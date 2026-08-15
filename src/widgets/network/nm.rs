//! NetworkManager over D-Bus.
//!
//! `backend.rs` used to be seventeen `nmcli` invocations, each one a process
//! spawn whose answer was recovered by splitting terse output on colons — and
//! `nmcli` escapes colons inside values as `\:`, so every parse also had an
//! unescape step that had to be remembered. A network name with a colon in it
//! was one forgotten `.replace()` from being a different network.
//!
//! NetworkManager's own interface is the same information without the round
//! trip through text: typed properties, on the bus the daemon already
//! publishes. This module is the thin layer that reads it; `backend.rs` keeps
//! its shape and its callers, and its function bodies became lookups.
//!
//! Blocking on purpose. Every caller already runs on a worker thread through
//! `spawn::spawn_work`, and the async plumbing would buy nothing but a runtime
//! on a path that is idle except when the panel is open.

use std::collections::HashMap;

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

pub const SERVICE: &str = "org.freedesktop.NetworkManager";
pub const MANAGER_PATH: &str = "/org/freedesktop/NetworkManager";
pub const SETTINGS_PATH: &str = "/org/freedesktop/NetworkManager/Settings";

pub const IFACE_MANAGER: &str = "org.freedesktop.NetworkManager";
pub const IFACE_SETTINGS: &str = "org.freedesktop.NetworkManager.Settings";
pub const IFACE_CONNECTION: &str = "org.freedesktop.NetworkManager.Settings.Connection";
pub const IFACE_ACTIVE: &str = "org.freedesktop.NetworkManager.Connection.Active";
pub const IFACE_DEVICE: &str = "org.freedesktop.NetworkManager.Device";
pub const IFACE_WIRELESS: &str = "org.freedesktop.NetworkManager.Device.Wireless";
pub const IFACE_AP: &str = "org.freedesktop.NetworkManager.AccessPoint";
pub const IFACE_IP4: &str = "org.freedesktop.NetworkManager.IP4Config";

/// `NM_DEVICE_TYPE_WIFI`. The other type numbers are named in
/// `backend::device_type_name`, where they are turned into strings.
pub const DEVICE_TYPE_WIFI: u32 = 2;

/// `NM_DEVICE_STATE_DISCONNECTED`. Anything at or below this is a device
/// that cannot carry traffic without something else happening first.
pub const DEVICE_STATE_DISCONNECTED: u32 = 30;

/// `NM_802_11_AP_SEC_*` bits, for turning an access point's flags into the
/// short label the list draws.
const AP_SEC_PAIR_WEP40: u32 = 0x1;
const AP_SEC_PAIR_WEP104: u32 = 0x2;
const AP_SEC_KEY_MGMT_PSK: u32 = 0x100;
const AP_SEC_KEY_MGMT_802_1X: u32 = 0x200;
const AP_SEC_KEY_MGMT_SAE: u32 = 0x400;
const AP_SEC_KEY_MGMT_OWE: u32 = 0x800;

/// `NM_802_11_AP_FLAGS_PRIVACY`.
const AP_FLAGS_PRIVACY: u32 = 0x1;

/// One connection to the system bus, opened per call.
///
/// zbus caches the underlying socket per process, so this is cheaper than it
/// reads; making it a long-lived field would mean handing a `!Send` handle
/// across the worker threads that use it.
pub fn system() -> Result<Connection, String> {
    Connection::system().map_err(|e| format!("system bus: {e}"))
}

/// A proxy on a NetworkManager object.
pub fn proxy<'a>(conn: &Connection, path: &'a str, iface: &'a str) -> Result<Proxy<'a>, String> {
    Proxy::new(conn, SERVICE, path, iface).map_err(|e| format!("{iface} at {path}: {e}"))
}

/// Read one property, or `None` if anything at all went wrong.
///
/// Absent and unreadable collapse deliberately: a device that vanished
/// between two calls and a device that never had the property both mean "do
/// not draw this", and every caller here treats them the same.
pub fn prop<T>(conn: &Connection, path: &str, iface: &str, name: &str) -> Option<T>
where
    T: TryFrom<OwnedValue>,
{
    let value = proxy(conn, path, iface)
        .ok()?
        .get_property::<OwnedValue>(name);
    match value {
        Ok(v) => T::try_from(v).ok(),
        Err(e) => {
            log::debug!("nm: {iface}.{name} at {path}: {e}");
            None
        }
    }
}

/// Object paths as plain strings, which is what the rest of this module
/// passes around.
pub fn paths(conn: &Connection, path: &str, iface: &str, name: &str) -> Vec<String> {
    prop::<Vec<OwnedObjectPath>>(conn, path, iface, name)
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.as_str().to_string())
        .collect()
}

pub fn path_prop(conn: &Connection, path: &str, iface: &str, name: &str) -> Option<String> {
    let p = prop::<OwnedObjectPath>(conn, path, iface, name)?;
    // NetworkManager spells "nothing here" as the root path rather than as an
    // absent property, and treating that as an object leads to a lookup that
    // fails much further away.
    (p.as_str() != "/").then(|| p.as_str().to_string())
}

/// An access point's SSID is a byte array, because it is: 802.11 does not say
/// it is text. Everything above wants a string, so invalid UTF-8 becomes the
/// lossy rendering rather than dropping the network from the list entirely.
pub fn ssid_of(conn: &Connection, ap: &str) -> Option<String> {
    let bytes: Vec<u8> = prop(conn, ap, IFACE_AP, "Ssid")?;
    if bytes.is_empty() {
        return None; // a hidden network broadcasting no name
    }
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// The short security label the network list draws, from an access point's
/// three flag words.
///
/// Deliberately close to what `nmcli` printed, because the strings end up in
/// front of the same person: "WPA2", "WPA3", "802.1X", "WEP", or empty for an
/// open network.
pub fn security_label(flags: u32, wpa: u32, rsn: u32) -> String {
    let mut parts: Vec<&str> = Vec::new();

    if wpa != 0 {
        parts.push("WPA1");
    }
    if rsn & AP_SEC_KEY_MGMT_SAE != 0 {
        parts.push("WPA3");
    } else if rsn & (AP_SEC_KEY_MGMT_PSK | AP_SEC_KEY_MGMT_802_1X) != 0 {
        parts.push("WPA2");
    }
    if rsn & AP_SEC_KEY_MGMT_OWE != 0 {
        parts.push("OWE");
    }
    if (wpa | rsn) & AP_SEC_KEY_MGMT_802_1X != 0 {
        parts.push("802.1X");
    }
    if parts.is_empty()
        && flags & AP_FLAGS_PRIVACY != 0
        && (wpa | rsn) & (AP_SEC_PAIR_WEP40 | AP_SEC_PAIR_WEP104) == 0
    {
        // Privacy set with no key management at all is the original WEP.
        parts.push("WEP");
    }

    parts.join(" ")
}

/// Every device, with the two properties nearly every caller wants.
pub struct DeviceInfo {
    pub path: String,
    pub interface: String,
    pub device_type: u32,
    pub state: u32,
}

pub fn devices(conn: &Connection) -> Vec<DeviceInfo> {
    paths(conn, MANAGER_PATH, IFACE_MANAGER, "Devices")
        .into_iter()
        .filter_map(|path| {
            Some(DeviceInfo {
                interface: prop(conn, &path, IFACE_DEVICE, "Interface")?,
                device_type: prop(conn, &path, IFACE_DEVICE, "DeviceType")?,
                state: prop(conn, &path, IFACE_DEVICE, "State")?,
                path,
            })
        })
        .collect()
}

/// The settings of one stored connection, as `(id, type)`.
///
/// `GetSettings` returns the whole nested dictionary; only the `connection`
/// section's `id` and `type` are ever read here.
pub fn connection_id_and_type(conn: &Connection, path: &str) -> Option<(String, String)> {
    let settings: HashMap<String, HashMap<String, OwnedValue>> =
        proxy(conn, path, IFACE_CONNECTION)
            .ok()?
            .call("GetSettings", &())
            .ok()?;
    let section = settings.get("connection")?;
    let id = section.get("id").and_then(as_string)?;
    let kind = section.get("type").and_then(as_string)?;
    Some((id, kind))
}

/// Stored connections as `(path, id, type)`.
pub fn stored_connections(conn: &Connection) -> Vec<(String, String, String)> {
    let list: Vec<OwnedObjectPath> = proxy(conn, SETTINGS_PATH, IFACE_SETTINGS)
        .and_then(|p| p.call("ListConnections", &()).map_err(|e| e.to_string()))
        .unwrap_or_default();

    list.into_iter()
        .filter_map(|path| {
            let path = path.as_str().to_string();
            let (id, kind) = connection_id_and_type(conn, &path)?;
            Some((path, id, kind))
        })
        .collect()
}

/// Stored connections with refined VPN subtype (OpenVPN, WireGuard, Cisco, etc.)
pub fn stored_connections_with_vpn_type(conn: &Connection) -> Vec<(String, String, String, String)> {
    let list: Vec<OwnedObjectPath> = proxy(conn, SETTINGS_PATH, IFACE_SETTINGS)
        .and_then(|p| p.call("ListConnections", &()).map_err(|e| e.to_string()))
        .unwrap_or_default();

    list.into_iter()
        .filter_map(|path| {
            let path = path.as_str().to_string();
            let settings: HashMap<String, HashMap<String, OwnedValue>> =
                proxy(conn, &path, IFACE_CONNECTION)
                    .ok()?
                    .call("GetSettings", &())
                    .ok()?;
            let section = settings.get("connection")?;
            let id = section.get("id").and_then(as_string)?;
            let kind = section.get("type").and_then(as_string)?;

            let vpn_sub = if kind == "wireguard" {
                "WireGuard".to_string()
            } else if let Some(vpn_sec) = settings.get("vpn") {
                let st = vpn_sec.get("service-type").and_then(as_string).unwrap_or_default();
                if st.contains("openvpn") {
                    "OpenVPN".to_string()
                } else if st.contains("wireguard") {
                    "WireGuard".to_string()
                } else if st.contains("openconnect") {
                    "OpenConnect".to_string()
                } else if st.contains("vpnc") || st.contains("cisco") {
                    "Cisco".to_string()
                } else {
                    "VPN".to_string()
                }
            } else if kind == "vpn" {
                "VPN".to_string()
            } else {
                kind.clone()
            };

            Some((path, id, kind, vpn_sub))
        })
        .collect()
}

/// Active connections as `(id, type, device interfaces)`.
pub fn active_connections(conn: &Connection) -> Vec<(String, String, Vec<String>)> {
    paths(conn, MANAGER_PATH, IFACE_MANAGER, "ActiveConnections")
        .into_iter()
        .filter_map(|path| {
            let id: String = prop(conn, &path, IFACE_ACTIVE, "Id")?;
            let kind: String = prop(conn, &path, IFACE_ACTIVE, "Type")?;
            let devices = paths(conn, &path, IFACE_ACTIVE, "Devices")
                .into_iter()
                .filter_map(|d| prop::<String>(conn, &d, IFACE_DEVICE, "Interface"))
                .collect();
            Some((id, kind, devices))
        })
        .collect()
}

/// Activate a stored connection by its id, letting NetworkManager pick the
/// device — which is what `nmcli connection up <name>` did.
pub fn activate_by_id(conn: &Connection, id: &str) -> Result<(), String> {
    let path = stored_connections(conn)
        .into_iter()
        .find(|(_, name, _)| name == id)
        .map(|(path, _, _)| path)
        .ok_or_else(|| format!("no connection named {id}"))?;

    let root = ObjectPath::try_from("/").map_err(|e| e.to_string())?;
    let target = ObjectPath::try_from(path.as_str()).map_err(|e| e.to_string())?;

    proxy(conn, MANAGER_PATH, IFACE_MANAGER)?
        .call::<_, _, OwnedObjectPath>("ActivateConnection", &(&target, &root, &root))
        .map(|_| ())
        .map_err(|e| dbus_message(&e))
}

/// Deactivate whichever active connection carries this id.
pub fn deactivate_by_id(conn: &Connection, id: &str) -> Result<(), String> {
    let active = paths(conn, MANAGER_PATH, IFACE_MANAGER, "ActiveConnections")
        .into_iter()
        .find(|path| prop::<String>(conn, path, IFACE_ACTIVE, "Id").as_deref() == Some(id))
        .ok_or_else(|| format!("{id} is not active"))?;

    let target = ObjectPath::try_from(active.as_str()).map_err(|e| e.to_string())?;
    proxy(conn, MANAGER_PATH, IFACE_MANAGER)?
        .call::<_, _, ()>("DeactivateConnection", &(&target,))
        .map_err(|e| dbus_message(&e))
}

/// Join a network that has no stored connection yet.
///
/// The settings dictionary is the minimum NetworkManager needs to build one:
/// the SSID, the key management, the passphrase, and — for a network that
/// does not broadcast — the flag that makes it probe by name instead of
/// waiting to see it in a scan.
pub fn add_and_activate(
    conn: &Connection,
    device_path: &str,
    ssid: &str,
    password: &str,
    hidden: bool,
) -> Result<(), String> {
    let mut wireless: HashMap<&str, Value> = HashMap::new();
    wireless.insert("ssid", Value::from(ssid.as_bytes().to_vec()));
    if hidden {
        wireless.insert("hidden", Value::from(true));
    }

    let mut connection: HashMap<&str, Value> = HashMap::new();
    connection.insert("id", Value::from(ssid));
    connection.insert("type", Value::from("802-11-wireless"));

    let mut settings: HashMap<&str, HashMap<&str, Value>> = HashMap::new();
    settings.insert("connection", connection);
    settings.insert("802-11-wireless", wireless);

    if !password.is_empty() {
        let mut security: HashMap<&str, Value> = HashMap::new();
        // WPA-PSK covers WPA2 and, for a router that offers both, WPA3's
        // transition mode; NetworkManager negotiates SAE from there.
        security.insert("key-mgmt", Value::from("wpa-psk"));
        security.insert("psk", Value::from(password));
        settings.insert("802-11-wireless-security", security);
        settings
            .get_mut("802-11-wireless")
            .expect("just inserted")
            .insert("security", Value::from("802-11-wireless-security"));
    }

    let device = ObjectPath::try_from(device_path).map_err(|e| e.to_string())?;
    let root = ObjectPath::try_from("/").map_err(|e| e.to_string())?;

    proxy(conn, MANAGER_PATH, IFACE_MANAGER)?
        .call::<_, _, (OwnedObjectPath, OwnedObjectPath)>(
            "AddAndActivateConnection",
            &(&settings, &device, &root),
        )
        .map(|_| ())
        .map_err(|e| dbus_message(&e))
}

/// A `OwnedValue` holding a string, as a `String`.
///
/// zvariant 4's `OwnedValue` is not `Clone`, so the usual `try_from(v.clone())`
/// silently clones the *reference* instead and fails to convert. Going through
/// `downcast_ref` says what is meant and cannot do that.
pub fn as_string(value: &OwnedValue) -> Option<String> {
    value.downcast_ref::<&str>().ok().map(str::to_string)
}

pub fn as_u32(value: &OwnedValue) -> Option<u32> {
    value.downcast_ref::<u32>().ok()
}

/// The human half of a D-Bus error.
///
/// A raw `zbus::Error` renders as the interface name and the method that
/// failed, which is true and useless in a status label two centimetres wide.
/// NetworkManager's own message is the part worth showing.
pub fn dbus_message(error: &zbus::Error) -> String {
    match error {
        zbus::Error::MethodError(_, Some(message), _) => message.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_open_network_has_no_security_label() {
        assert_eq!(security_label(0, 0, 0), "");
    }

    #[test]
    fn wpa2_is_rsn_with_a_pre_shared_key() {
        assert_eq!(
            security_label(AP_FLAGS_PRIVACY, 0, AP_SEC_KEY_MGMT_PSK),
            "WPA2"
        );
    }

    #[test]
    fn wpa3_outranks_wpa2_rather_than_joining_it() {
        // A transition-mode router advertises both; calling it WPA2 WPA3
        // would be accurate and would also make every row wider.
        let label = security_label(
            AP_FLAGS_PRIVACY,
            0,
            AP_SEC_KEY_MGMT_PSK | AP_SEC_KEY_MGMT_SAE,
        );
        assert_eq!(label, "WPA3");
    }

    #[test]
    fn a_mixed_router_reports_both_generations() {
        let label = security_label(AP_FLAGS_PRIVACY, AP_SEC_KEY_MGMT_PSK, AP_SEC_KEY_MGMT_PSK);
        assert_eq!(label, "WPA1 WPA2");
    }

    #[test]
    fn enterprise_is_called_out_separately() {
        let label = security_label(AP_FLAGS_PRIVACY, 0, AP_SEC_KEY_MGMT_802_1X);
        assert_eq!(label, "WPA2 802.1X");
    }

    #[test]
    fn privacy_without_key_management_is_wep() {
        assert_eq!(security_label(AP_FLAGS_PRIVACY, 0, 0), "WEP");
    }
}
