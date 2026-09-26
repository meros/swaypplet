//! The section's whole state, read from NetworkManager in one pass, and the
//! actions that change it.
//!
//! Blocking, on a worker, like the rest of this module. The read asks for
//! nothing to be done: no scan, no connectivity probe. It is what
//! NetworkManager already knows, which is also why it is fast enough to run
//! on every change signal (`watch.rs`) instead of on a timer.

use std::collections::{HashMap, HashSet};

use zbus::blocking::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

use super::model::{Activation, Details, Metered, SavedNetwork, Snapshot, stage_text};
use super::nm;
use super::{ActiveConnection, NmResult, WifiNetwork, merge_and_rank};

type Settings = HashMap<String, HashMap<String, OwnedValue>>;

const IFACE_IP6: &str = "org.freedesktop.NetworkManager.IP6Config";
/// `NM_DEVICE_TYPE_MODEM`.
const DEVICE_TYPE_MODEM: u32 = 8;
/// `NM_DEVICE_STATE_ACTIVATED`.
const DEVICE_STATE_ACTIVATED: u32 = 100;

/// A stored Wi-Fi connection with its object path and settings.
struct Stored {
    path: String,
    saved: SavedNetwork,
    settings: Settings,
}

fn settings_of(conn: &Connection, path: &str) -> Option<Settings> {
    nm::proxy(conn, path, nm::IFACE_CONNECTION)
        .ok()?
        .call("GetSettings", &())
        .ok()
}

fn stored_wifi(conn: &Connection) -> Vec<Stored> {
    let list: Vec<OwnedObjectPath> = nm::proxy(conn, nm::SETTINGS_PATH, nm::IFACE_SETTINGS)
        .and_then(|p| p.call("ListConnections", &()).map_err(|e| e.to_string()))
        .unwrap_or_default();
    list.into_iter()
        .filter_map(|p| {
            let path = p.as_str().to_string();
            let settings = settings_of(conn, &path)?;
            let c = settings.get("connection")?;
            if c.get("type").and_then(nm::as_string).as_deref() != Some("802-11-wireless") {
                return None;
            }
            let id = c.get("id").and_then(nm::as_string)?;
            let ssid = settings
                .get("802-11-wireless")
                .and_then(|w| w.get("ssid"))
                .and_then(|v| v.downcast_ref::<zbus::zvariant::Array>().ok())
                .map(|a| {
                    a.iter()
                        .filter_map(|b| b.downcast_ref::<u8>().ok())
                        .collect::<Vec<u8>>()
                })
                .map(|b| super::model::ssid_from_bytes(&b))
                .unwrap_or_else(|| id.clone());
            let autoconnect = c
                .get("autoconnect")
                .and_then(|v| v.downcast_ref::<bool>().ok())
                .unwrap_or(true);
            let last_used = c
                .get("timestamp")
                .and_then(|v| v.downcast_ref::<u64>().ok())
                .unwrap_or(0);
            Some(Stored {
                path,
                saved: SavedNetwork {
                    id,
                    ssid,
                    autoconnect,
                    last_used,
                },
                settings,
            })
        })
        .collect()
}

/// Everything, as NetworkManager has it now.
pub fn read() -> Snapshot {
    let Ok(conn) = nm::system() else {
        return Snapshot::default();
    };
    let Some(_) = nm::prop::<u32>(&conn, nm::MANAGER_PATH, nm::IFACE_MANAGER, "State") else {
        return Snapshot::default();
    };
    let manager = |name: &str| nm::prop::<bool>(&conn, nm::MANAGER_PATH, nm::IFACE_MANAGER, name);

    let devices = nm::devices(&conn);
    let wifi = devices
        .iter()
        .find(|d| d.device_type == nm::DEVICE_TYPE_WIFI);
    let has_modem = devices.iter().any(|d| d.device_type == DEVICE_TYPE_MODEM);

    let stored = stored_wifi(&conn);
    let saved_ssids: HashSet<&str> = stored.iter().map(|s| s.saved.ssid.as_str()).collect();

    let mut s = Snapshot {
        available: true,
        has_wifi: wifi.is_some(),
        wifi_enabled: manager("WirelessEnabled").unwrap_or(false),
        wwan_enabled: has_modem.then(|| manager("WwanEnabled").unwrap_or(false)),
        bluetooth_powered: {
            let b = crate::services::bluez::snapshot();
            (b.available && b.adapter.is_some()).then_some(b.powered)
        },
        active: super::get_active_connection(),
        connectivity: super::check_connectivity(),
        portal_uri: nm::prop::<String>(
            &conn,
            nm::MANAGER_PATH,
            nm::IFACE_MANAGER,
            "ConnectivityCheckUri",
        )
        .filter(|u| !u.is_empty()),
        vpns: super::get_vpn_connections(),
        interfaces: super::get_network_interfaces(),
        ..Default::default()
    };

    if let Some(dev) = wifi {
        let (state, reason) =
            nm::prop::<(u32, u32)>(&conn, &dev.path, nm::IFACE_DEVICE, "StateReason")
                .unwrap_or((dev.state, 0));
        s.wifi_state = state;
        s.wifi_reason = reason;
        s.last_scan =
            nm::prop::<i64>(&conn, &dev.path, nm::IFACE_WIRELESS, "LastScan").unwrap_or(-1);

        let active_ap = nm::path_prop(&conn, &dev.path, nm::IFACE_WIRELESS, "ActiveAccessPoint");
        let mut found = Vec::new();
        for ap in nm::paths(&conn, &dev.path, nm::IFACE_WIRELESS, "AccessPoints") {
            let Some(ssid) = nm::ssid_of(&conn, &ap) else {
                continue;
            };
            let flags = nm::prop::<u32>(&conn, &ap, nm::IFACE_AP, "Flags").unwrap_or(0);
            let wpa = nm::prop::<u32>(&conn, &ap, nm::IFACE_AP, "WpaFlags").unwrap_or(0);
            let rsn = nm::prop::<u32>(&conn, &ap, nm::IFACE_AP, "RsnFlags").unwrap_or(0);
            found.push(WifiNetwork {
                is_known: saved_ssids.contains(ssid.as_str()),
                in_use: active_ap.as_deref() == Some(ap.as_str())
                    && state == DEVICE_STATE_ACTIVATED,
                signal: nm::prop::<u8>(&conn, &ap, nm::IFACE_AP, "Strength").unwrap_or(0),
                security: nm::security_label(flags, wpa, rsn),
                freq_mhz: nm::prop::<u32>(&conn, &ap, nm::IFACE_AP, "Frequency"),
                ssid,
            });
        }
        s.networks = merge_and_rank(found);

        let active_conn = nm::path_prop(&conn, &dev.path, nm::IFACE_DEVICE, "ActiveConnection");
        let active_id = active_conn
            .as_deref()
            .and_then(|p| nm::prop::<String>(&conn, p, nm::IFACE_ACTIVE, "Id"));
        if let (Some(stage), Some(id)) = (stage_text(state), active_id.clone()) {
            s.activating = Some(Activation {
                ssid: stored
                    .iter()
                    .find(|st| st.saved.id == id)
                    .map_or(id, |st| st.saved.ssid.clone()),
                stage,
            });
        }
    }

    s.details = details(&conn, &s.active, &stored);
    s.saved = {
        let mut v: Vec<SavedNetwork> = stored.into_iter().map(|st| st.saved).collect();
        v.sort_by_key(|s| std::cmp::Reverse(s.last_used));
        v
    };
    s
}

fn details(conn: &Connection, active: &ActiveConnection, stored: &[Stored]) -> Option<Details> {
    let device = match active {
        ActiveConnection::Wifi { device, .. } | ActiveConnection::Ethernet { device } => device,
        ActiveConnection::Disconnected => return None,
    };
    let dev = nm::devices(conn)
        .into_iter()
        .find(|d| d.interface == *device)?;
    let ip4 = nm::path_prop(conn, &dev.path, nm::IFACE_DEVICE, "Ip4Config");
    let ip6 = nm::path_prop(conn, &dev.path, nm::IFACE_DEVICE, "Ip6Config");
    let first_address = |cfg: &Option<String>, iface: &str| -> Option<String> {
        let data: Vec<HashMap<String, OwnedValue>> =
            nm::prop(conn, cfg.as_deref()?, iface, "AddressData")?;
        data.iter()
            .filter_map(|e| e.get("address").and_then(nm::as_string))
            // A link-local IPv6 address says nothing a person needs.
            .find(|a| !a.starts_with("fe80"))
    };
    let active_conn = nm::path_prop(conn, &dev.path, nm::IFACE_DEVICE, "ActiveConnection");
    let connection_id = active_conn
        .as_deref()
        .and_then(|p| nm::prop::<String>(conn, p, nm::IFACE_ACTIVE, "Id"));
    let (freq, security) = match active {
        ActiveConnection::Wifi { .. } => {
            let ap = nm::path_prop(conn, &dev.path, nm::IFACE_WIRELESS, "ActiveAccessPoint");
            let sec = ap.as_deref().map(|ap| {
                nm::security_label(
                    nm::prop::<u32>(conn, ap, nm::IFACE_AP, "Flags").unwrap_or(0),
                    nm::prop::<u32>(conn, ap, nm::IFACE_AP, "WpaFlags").unwrap_or(0),
                    nm::prop::<u32>(conn, ap, nm::IFACE_AP, "RsnFlags").unwrap_or(0),
                )
            });
            (
                ap.as_deref()
                    .and_then(|ap| nm::prop::<u32>(conn, ap, nm::IFACE_AP, "Frequency")),
                sec.unwrap_or_default(),
            )
        }
        _ => (None, String::new()),
    };
    let power_saving = connection_id
        .as_deref()
        .and_then(|id| stored.iter().find(|s| s.saved.id == id))
        .and_then(|s| s.settings.get("802-11-wireless")?.get("powersave").and_then(nm::as_u32))
        == Some(3);
    Some(Details {
        device: device.clone(),
        hw_address: nm::prop::<String>(conn, &dev.path, nm::IFACE_DEVICE, "HwAddress"),
        ip4: first_address(&ip4, nm::IFACE_IP4),
        ip6: first_address(&ip6, IFACE_IP6),
        gateway: ip4
            .as_deref()
            .and_then(|c| nm::prop::<String>(conn, c, nm::IFACE_IP4, "Gateway"))
            .filter(|g| !g.is_empty()),
        dns: ip4
            .as_deref()
            .and_then(|c| {
                nm::prop::<Vec<HashMap<String, OwnedValue>>>(
                    conn,
                    c,
                    nm::IFACE_IP4,
                    "NameserverData",
                )
            })
            .unwrap_or_default()
            .iter()
            .filter_map(|e| e.get("address").and_then(nm::as_string))
            .collect(),
        bitrate_mbps: nm::prop::<u32>(conn, &dev.path, nm::IFACE_WIRELESS, "Bitrate")
            .map(|kbps| kbps / 1000),
        freq_mhz: freq,
        security,
        metered: Metered::from_nm(
            nm::prop::<u32>(conn, &dev.path, nm::IFACE_DEVICE, "Metered").unwrap_or(0),
        ),
        connection_id,
        power_saving,
    })
}

/// Ask the Wi-Fi device to scan. Returns at once; the results arrive as
/// change signals. Harmless: it changes no configuration.
pub fn request_scan() {
    let Ok(conn) = nm::system() else { return };
    for d in nm::devices(&conn) {
        if d.device_type != nm::DEVICE_TYPE_WIFI {
            continue;
        }
        if let Ok(p) = nm::proxy(&conn, &d.path, nm::IFACE_WIRELESS) {
            let options: HashMap<&str, Value> = HashMap::new();
            let _ = p.call::<_, _, ()>("RequestScan", &(options,));
        }
    }
}

// ── Actions ─────────────────────────────────────────────────────────────

fn acting(f: impl FnOnce(&Connection) -> Result<(), String>) -> NmResult {
    match nm::system() {
        Ok(conn) => f(&conn).into(),
        Err(e) => NmResult::Failure(e),
    }
}

fn find_stored(conn: &Connection, pred: impl Fn(&SavedNetwork) -> bool) -> Option<Stored> {
    stored_wifi(conn).into_iter().find(|s| pred(&s.saved))
}

/// Replace a stored connection's settings (`Update` takes the whole set).
fn update(conn: &Connection, path: &str, settings: &Settings) -> Result<(), String> {
    nm::proxy(conn, path, nm::IFACE_CONNECTION)?
        .call::<_, _, ()>("Update", &(settings,))
        .map_err(|e| nm::dbus_message(&e))
}

fn owned(v: Value<'_>) -> Result<OwnedValue, String> {
    v.try_into().map_err(|e| format!("{e}"))
}

/// The security block for a join, from the access point's label.
fn security_block(security: &str, password: &str) -> Result<HashMap<String, OwnedValue>, String> {
    let mut sec = HashMap::new();
    if security.contains("WEP") {
        sec.insert("key-mgmt".to_string(), owned(Value::from("none"))?);
        sec.insert("wep-key0".to_string(), owned(Value::from(password))?);
        sec.insert("wep-key-type".to_string(), owned(Value::from(1u32))?);
    } else {
        // WPA3-only networks take SAE; the pre-shared key goes in `psk` for
        // both. The old join set `key-mgmt = sae` and no key at all, so a
        // WPA3 network could never be joined from the panel.
        let mgmt = if security.contains("WPA3") && !security.contains("WPA2") {
            "sae"
        } else {
            "wpa-psk"
        };
        sec.insert("key-mgmt".to_string(), owned(Value::from(mgmt))?);
        sec.insert("psk".to_string(), owned(Value::from(password))?);
    }
    Ok(sec)
}

/// Join `ssid` with `password`. A network already saved keeps its stored
/// connection (its metered flag, its auto-connect, its name) and only gets
/// the new password; a new one is added and removed again if the join
/// fails, so a typo does not leave a broken entry behind.
pub fn join(ssid: &str, password: &str, security: &str, hidden: bool) -> NmResult {
    let (ssid, password, security) = (ssid.to_string(), password.to_string(), security.to_string());
    acting(move |conn| {
        if let Some(mut st) = find_stored(conn, |s| s.ssid == ssid) {
            if !password.is_empty() {
                st.settings
                    .insert("802-11-wireless-security".into(), security_block(&security, &password)?);
                st.settings
                    .entry("802-11-wireless".into())
                    .or_default()
                    .insert("security".into(), owned(Value::from("802-11-wireless-security"))?);
                update(conn, &st.path, &st.settings)?;
            }
            return nm::activate_by_id(conn, &st.saved.id);
        }
        let device = nm::devices(conn)
            .into_iter()
            .find(|d| d.device_type == nm::DEVICE_TYPE_WIFI)
            .map(|d| d.path)
            .ok_or("No Wi-Fi adapter")?;
        nm::add_and_activate(conn, &device, &ssid, &password, &security, hidden)
    })
}

/// Bring up a saved network.
pub fn connect_saved(id: &str) -> NmResult {
    let id = id.to_string();
    acting(move |conn| nm::activate_by_id(conn, &id))
}

pub fn forget(id: &str) -> NmResult {
    let id = id.to_string();
    acting(move |conn| {
        let st = find_stored(conn, |s| s.id == id).ok_or_else(|| format!("no saved network {id}"))?;
        nm::proxy(conn, &st.path, nm::IFACE_CONNECTION)?
            .call::<_, _, ()>("Delete", &())
            .map_err(|e| nm::dbus_message(&e))
    })
}

fn edit_connection(
    id: &str,
    edit: impl FnOnce(&mut Settings) -> Result<(), String> + Send + 'static,
) -> NmResult {
    let id = id.to_string();
    acting(move |conn| {
        let mut st =
            find_stored(conn, |s| s.id == id).ok_or_else(|| format!("no saved network {id}"))?;
        edit(&mut st.settings)?;
        update(conn, &st.path, &st.settings)
    })
}

pub fn set_autoconnect(id: &str, on: bool) -> NmResult {
    edit_connection(id, move |s| {
        s.entry("connection".into())
            .or_default()
            .insert("autoconnect".into(), owned(Value::from(on))?);
        Ok(())
    })
}

/// Mark the connection metered or not, and reapply it on its device so the
/// flag counts now, not from the next join.
pub fn set_metered(id: &str, device: &str, metered: bool) -> NmResult {
    let device = device.to_string();
    let edited = edit_connection(id, move |s| {
        s.entry("connection".into())
            .or_default()
            .insert("metered".into(), owned(Value::from(if metered { 1i32 } else { 2i32 }))?);
        Ok(())
    });
    if matches!(edited, NmResult::Failure(_)) {
        return edited;
    }
    acting(move |conn| {
        let path = nm::devices(conn)
            .into_iter()
            .find(|d| d.interface == device)
            .map(|d| d.path)
            .ok_or("device gone")?;
        let empty: HashMap<&str, HashMap<&str, Value>> = HashMap::new();
        nm::proxy(conn, &path, nm::IFACE_DEVICE)?
            .call::<_, _, ()>("Reapply", &(empty, 0u64, 0u32))
            .map_err(|e| nm::dbus_message(&e))
    })
}

pub fn set_power_saving(id: &str, on: bool) -> NmResult {
    super::set_wifi_power_saving(id, on)
}

/// Airplane mode: every radio off, or every radio back on.
pub fn set_airplane(on: bool) -> NmResult {
    let radios = acting(move |conn| {
        let p = nm::proxy(conn, nm::MANAGER_PATH, nm::IFACE_MANAGER)?;
        p.set_property("WirelessEnabled", !on)
            .map_err(|e| e.to_string())?;
        // No modem is not an error; the property is still there.
        let _ = p.set_property("WwanEnabled", !on);
        Ok(())
    });
    if crate::services::bluez::snapshot().adapter.is_some() {
        let _ = crate::services::bluez::set_powered(!on);
    }
    radios
}

pub fn disconnect_wifi() -> NmResult {
    acting(nm::deactivate_active_wifi)
}

pub fn vpn(name: &str, up: bool) -> NmResult {
    if up {
        super::vpn_up(name)
    } else {
        super::vpn_down(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(v: &OwnedValue) -> String {
        nm::as_string(v).unwrap()
    }

    #[test]
    fn a_wpa3_join_carries_its_key() {
        let s = security_block("WPA3", "hunter22").unwrap();
        assert_eq!(text(&s["key-mgmt"]), "sae");
        assert_eq!(text(&s["psk"]), "hunter22");
        let s = security_block("WPA2", "hunter22").unwrap();
        assert_eq!(text(&s["key-mgmt"]), "wpa-psk");
        let s = security_block("WEP", "abcde").unwrap();
        assert_eq!(text(&s["key-mgmt"]), "none");
        assert_eq!(text(&s["wep-key0"]), "abcde");
    }
}
