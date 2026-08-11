//! Network state and actions, over NetworkManager's D-Bus interface.
//!
//! The types and the public functions are unchanged from when this file drove
//! `nmcli`; only the bodies moved. See `nm.rs` for why.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use super::nm;

/// How long to wait for a rescan's results before drawing what we have.
///
/// A scan is asynchronous now: `RequestScan` returns immediately and the
/// access-point list fills in behind it, so this is a poll on `LastScan`
/// rather than a wait on a process. Shorter than the old 15 s process
/// timeout because there is no process to hang — only a driver that may not
/// answer, and 6 s of a spinner is already longer than anyone wants.
const WIFI_SCAN_TIMEOUT: Duration = Duration::from_secs(6);

// ── Nerd Font icons ───────────────────────────────────────────────────────────
pub const ICON_SIGNAL_NONE: &str = "󰤯";
pub const ICON_SIGNAL_WEAK: &str = "󰤟";
pub const ICON_SIGNAL_OK: &str = "󰤢";
pub const ICON_SIGNAL_GOOD: &str = "󰤥";
pub const ICON_SIGNAL_EXCELLENT: &str = "󰤨";
pub const ICON_LOCK: &str = "";
pub const ICON_ETHERNET: &str = "󰈀";
pub const ICON_DISCONNECTED: &str = "󰤭";
pub const ICON_VPN: &str = "󰦝";

/// Maximum number of networks shown before a "Show all" button appears.
pub const MAX_VISIBLE_NETWORKS: usize = 8;

// ── NetworkManager connection type identifiers ────────────────────────────────
const NM_TYPE_WIFI: &str = "802-11-wireless";
const NM_TYPE_ETHERNET: &str = "802-3-ethernet";
const NM_TYPE_VPN: &str = "vpn";
const NM_TYPE_WIREGUARD: &str = "wireguard";

// ── Result type for the actions the panel can take ────────────────────────────

#[derive(Debug)]
pub enum NmResult {
    Success,
    Failure(String),
}

impl From<Result<(), String>> for NmResult {
    fn from(result: Result<(), String>) -> Self {
        match result {
            Ok(()) => NmResult::Success,
            Err(message) => NmResult::Failure(message),
        }
    }
}

/// Do something on the bus, turning a connection failure into the same
/// `NmResult` the operation itself would produce.
fn acting(f: impl FnOnce(&zbus::blocking::Connection) -> Result<(), String>) -> NmResult {
    match nm::system() {
        Ok(conn) => f(&conn).into(),
        Err(e) => NmResult::Failure(e),
    }
}

// ── Signal strength helpers ───────────────────────────────────────────────────

pub fn signal_icon(strength: u8) -> &'static str {
    match strength {
        0..=20 => ICON_SIGNAL_NONE,
        21..=40 => ICON_SIGNAL_WEAK,
        41..=60 => ICON_SIGNAL_OK,
        61..=80 => ICON_SIGNAL_GOOD,
        _ => ICON_SIGNAL_EXCELLENT,
    }
}

pub fn signal_css_class(strength: u8) -> &'static str {
    match strength {
        0..=20 => "network-signal-none",
        21..=40 => "network-signal-weak",
        41..=60 => "network-signal-ok",
        61..=80 => "network-signal-good",
        _ => "network-signal-excellent",
    }
}

// ── Data types ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct WifiNetwork {
    pub ssid: String,
    pub signal: u8,
    pub security: String,
    pub in_use: bool,
    pub is_known: bool,
    pub freq_mhz: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ActiveConnection {
    Wifi {
        ssid: String,
        signal: u8,
        device: String,
        freq_mhz: Option<u32>,
    },
    Ethernet {
        device: String,
    },
    Disconnected,
}

#[derive(Debug, Clone)]
pub struct VpnConnection {
    pub name: String,
    pub active: bool,
}

#[derive(Debug, Clone)]
pub struct NetworkInterface {
    pub device: String,
    pub iface_type: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConnectivityState {
    Full,
    Limited,
    Portal,
    None,
    Unknown,
}

impl ConnectivityState {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Full => "Connected",
            Self::Limited => "Limited — No internet",
            Self::Portal => "Captive portal",
            Self::None => "Disconnected",
            Self::Unknown => "Unknown",
        }
    }

    pub fn summary_badge(&self) -> Option<&'static str> {
        match self {
            Self::Portal => Some(" · ⚠ Portal"),
            Self::Limited => Some(" · ⚠ Limited"),
            Self::None => Some(" · ⚠ Offline"),
            _ => Option::None,
        }
    }
}

// ── Is the daemon there? ──────────────────────────────────────────────────────

/// Is NetworkManager there to talk to?
///
/// Named for what it answers rather than for the binary it used to look for;
/// the callers ask "can this section draw anything".
pub fn network_manager_available() -> bool {
    let Ok(conn) = nm::system() else {
        return false;
    };
    nm::prop::<u32>(&conn, nm::MANAGER_PATH, nm::IFACE_MANAGER, "State").is_some()
}

pub fn wifi_adapter_present() -> bool {
    let Ok(conn) = nm::system() else {
        return false;
    };
    nm::devices(&conn)
        .iter()
        .any(|d| d.device_type == nm::DEVICE_TYPE_WIFI)
}

// ── WiFi radio state ──────────────────────────────────────────────────────────

pub fn wifi_radio_enabled() -> bool {
    let Ok(conn) = nm::system() else {
        return false;
    };
    nm::prop::<bool>(
        &conn,
        nm::MANAGER_PATH,
        nm::IFACE_MANAGER,
        "WirelessEnabled",
    )
    .unwrap_or(false)
}

pub fn set_wifi_radio(enable: bool) -> NmResult {
    acting(|conn| {
        nm::proxy(conn, nm::MANAGER_PATH, nm::IFACE_MANAGER)?
            .set_property("WirelessEnabled", enable)
            .map_err(|e| e.to_string())
    })
}

// ── Backend helpers ───────────────────────────────────────────────────────────

/// Collapse an access-point list into one row per network, best signal wins,
/// then rank it the way the list draws it.
///
/// A network with three access points is one network to the person choosing
/// it, and the strongest radio is the one they will actually associate with.
/// Pure, so the ordering rules stay testable without a radio.
pub fn merge_and_rank(found: Vec<WifiNetwork>) -> Vec<WifiNetwork> {
    let mut networks: Vec<WifiNetwork> = Vec::new();

    for network in found {
        if network.ssid.is_empty() {
            continue;
        }
        if let Some(existing) = networks.iter_mut().find(|n| n.ssid == network.ssid) {
            if network.signal > existing.signal {
                existing.signal = network.signal;
                existing.freq_mhz = network.freq_mhz;
                existing.security = network.security;
            }
            existing.in_use |= network.in_use;
            existing.is_known |= network.is_known;
            continue;
        }
        networks.push(network);
    }

    networks.sort_by(|a, b| {
        if a.in_use != b.in_use {
            return b.in_use.cmp(&a.in_use);
        }
        if a.is_known != b.is_known {
            return b.is_known.cmp(&a.is_known);
        }
        b.signal.cmp(&a.signal)
    });

    networks
}

pub fn get_known_ssids() -> Vec<String> {
    let Ok(conn) = nm::system() else {
        return Vec::new();
    };
    nm::stored_connections(&conn)
        .into_iter()
        .filter(|(_, _, kind)| kind == NM_TYPE_WIFI)
        .map(|(_, id, _)| id)
        .collect()
}

/// Ask every wifi device to rescan, then read what they can see.
///
/// `RequestScan` is asynchronous: it returns as soon as the driver accepts,
/// and `LastScan` moves when results land. Waiting on that is the honest
/// version of the old bounded wait on an `nmcli` process — and unlike a
/// process, nothing here can be left running after we stop caring.
pub fn scan_wifi() -> Result<Vec<WifiNetwork>, String> {
    let conn = nm::system()?;

    let radios: Vec<String> = nm::devices(&conn)
        .into_iter()
        .filter(|d| d.device_type == nm::DEVICE_TYPE_WIFI)
        .map(|d| d.path)
        .collect();

    if radios.is_empty() {
        return Err("No WiFi adapter".to_string());
    }

    // With the radio off there is nothing to wait for, and waiting the full
    // timeout to report an empty list reads as a broken scan rather than a
    // switched-off one.
    if !nm::prop::<bool>(
        &conn,
        nm::MANAGER_PATH,
        nm::IFACE_MANAGER,
        "WirelessEnabled",
    )
    .unwrap_or(false)
    {
        return Err("WiFi is off".to_string());
    }

    let before: Vec<i64> = radios
        .iter()
        .map(|path| nm::prop::<i64>(&conn, path, nm::IFACE_WIRELESS, "LastScan").unwrap_or(-1))
        .collect();

    for path in &radios {
        // A refused scan is not fatal: the device may have scanned a second
        // ago, and its access-point list is still worth drawing.
        if let Ok(proxy) = nm::proxy(&conn, path, nm::IFACE_WIRELESS) {
            let options: std::collections::HashMap<&str, zbus::zvariant::Value> =
                std::collections::HashMap::new();
            let _ = proxy.call::<_, _, ()>("RequestScan", &(options,));
        }
    }

    let deadline = Instant::now() + WIFI_SCAN_TIMEOUT;
    while Instant::now() < deadline {
        let moved = radios.iter().zip(&before).any(|(path, was)| {
            nm::prop::<i64>(&conn, path, nm::IFACE_WIRELESS, "LastScan").unwrap_or(-1) > *was
        });
        if moved {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }

    let known = get_known_ssids();
    let mut found = Vec::new();

    for path in &radios {
        let active = nm::path_prop(&conn, path, nm::IFACE_WIRELESS, "ActiveAccessPoint");
        for ap in nm::paths(&conn, path, nm::IFACE_WIRELESS, "AccessPoints") {
            let Some(ssid) = nm::ssid_of(&conn, &ap) else {
                continue;
            };
            let flags = nm::prop::<u32>(&conn, &ap, nm::IFACE_AP, "Flags").unwrap_or(0);
            let wpa = nm::prop::<u32>(&conn, &ap, nm::IFACE_AP, "WpaFlags").unwrap_or(0);
            let rsn = nm::prop::<u32>(&conn, &ap, nm::IFACE_AP, "RsnFlags").unwrap_or(0);

            found.push(WifiNetwork {
                is_known: known.contains(&ssid),
                in_use: active.as_deref() == Some(ap.as_str()),
                signal: nm::prop::<u8>(&conn, &ap, nm::IFACE_AP, "Strength").unwrap_or(0),
                security: nm::security_label(flags, wpa, rsn),
                freq_mhz: nm::prop::<u32>(&conn, &ap, nm::IFACE_AP, "Frequency"),
                ssid,
            });
        }
    }

    Ok(merge_and_rank(found))
}

pub fn get_active_connection() -> ActiveConnection {
    let Ok(conn) = nm::system() else {
        return ActiveConnection::Disconnected;
    };

    for (_, kind, devices) in nm::active_connections(&conn) {
        let Some(device) = devices.into_iter().next() else {
            continue;
        };
        match kind.as_str() {
            NM_TYPE_WIFI => {
                let (ssid, signal, freq_mhz) = active_wifi(&conn).unwrap_or_default();
                return ActiveConnection::Wifi {
                    ssid,
                    signal,
                    device,
                    freq_mhz,
                };
            }
            NM_TYPE_ETHERNET => return ActiveConnection::Ethernet { device },
            _ => {}
        }
    }
    ActiveConnection::Disconnected
}

/// The access point a wifi device is actually associated with.
///
/// Read from the device rather than matched by name against a scan: the
/// association is a property NetworkManager already holds, and two networks
/// sharing an SSID used to make the old lookup pick whichever came first.
fn active_wifi(conn: &zbus::blocking::Connection) -> Option<(String, u8, Option<u32>)> {
    for device in nm::devices(conn) {
        if device.device_type != nm::DEVICE_TYPE_WIFI {
            continue;
        }
        let Some(ap) = nm::path_prop(conn, &device.path, nm::IFACE_WIRELESS, "ActiveAccessPoint")
        else {
            continue;
        };
        return Some((
            nm::ssid_of(conn, &ap)?,
            nm::prop::<u8>(conn, &ap, nm::IFACE_AP, "Strength").unwrap_or(0),
            nm::prop::<u32>(conn, &ap, nm::IFACE_AP, "Frequency"),
        ));
    }
    None
}

pub fn freq_band_label(freq_mhz: u32) -> &'static str {
    if freq_mhz < 3000 {
        "2.4 GHz"
    } else if freq_mhz < 6000 {
        "5 GHz"
    } else {
        "6 GHz"
    }
}

pub fn freq_band_short(freq_mhz: u32) -> &'static str {
    if freq_mhz < 3000 {
        "2.4G"
    } else if freq_mhz < 6000 {
        "5G"
    } else {
        "6G"
    }
}

// ── VPN ───────────────────────────────────────────────────────────────────────

pub fn get_vpn_connections() -> Vec<VpnConnection> {
    let Ok(conn) = nm::system() else {
        return Vec::new();
    };

    let is_vpn = |kind: &str| kind == NM_TYPE_VPN || kind == NM_TYPE_WIREGUARD;

    let active: HashSet<String> = nm::active_connections(&conn)
        .into_iter()
        .filter(|(_, kind, _)| is_vpn(kind))
        .map(|(id, _, _)| id)
        .collect();

    nm::stored_connections(&conn)
        .into_iter()
        .filter(|(_, _, kind)| is_vpn(kind))
        .map(|(_, name, _)| VpnConnection {
            active: active.contains(&name),
            name,
        })
        .collect()
}

pub fn vpn_up(name: &str) -> NmResult {
    let name = name.to_string();
    acting(move |conn| nm::activate_by_id(conn, &name))
}

pub fn vpn_down(name: &str) -> NmResult {
    let name = name.to_string();
    acting(move |conn| nm::deactivate_by_id(conn, &name))
}

// ── WiFi connect/forget ───────────────────────────────────────────────────────

pub fn connect_known(ssid: &str) -> NmResult {
    let ssid = ssid.to_string();
    acting(move |conn| nm::activate_by_id(conn, &ssid))
}

pub fn connect_new(ssid: &str, password: &str, hidden: bool) -> NmResult {
    let (ssid, password) = (ssid.to_string(), password.to_string());
    acting(move |conn| {
        let device = nm::devices(conn)
            .into_iter()
            .find(|d| d.device_type == nm::DEVICE_TYPE_WIFI)
            .map(|d| d.path)
            .ok_or("No WiFi adapter")?;
        nm::add_and_activate(conn, &device, &ssid, &password, hidden)
    })
}

pub fn forget_network(ssid: &str) -> NmResult {
    let ssid = ssid.to_string();
    acting(move |conn| {
        let path = nm::stored_connections(conn)
            .into_iter()
            .find(|(_, id, kind)| id == &ssid && kind == NM_TYPE_WIFI)
            .map(|(path, _, _)| path)
            .ok_or_else(|| format!("no saved network named {ssid}"))?;

        nm::proxy(conn, &path, nm::IFACE_CONNECTION)?
            .call::<_, _, ()>("Delete", &())
            .map_err(|e| nm::dbus_message(&e))
    })
}

// ── Interface management ──────────────────────────────────────────────────────

pub fn get_network_interfaces() -> Vec<NetworkInterface> {
    let Ok(conn) = nm::system() else {
        return Vec::new();
    };
    nm::devices(&conn)
        .into_iter()
        .filter_map(|device| {
            let iface_type = device_type_name(device.device_type);
            if iface_type == "loopback" || iface_type == "bridge" || device.interface == "lo" {
                return None;
            }
            Some(NetworkInterface {
                enabled: device.state > nm::DEVICE_STATE_DISCONNECTED,
                device: device.interface,
                iface_type: iface_type.to_string(),
            })
        })
        .collect()
}

/// `NM_DEVICE_TYPE_*` as the strings the icon table and the filters above
/// already speak, which are `nmcli`'s names for the same numbers.
fn device_type_name(device_type: u32) -> &'static str {
    match device_type {
        1 => "ethernet",
        2 => "wifi",
        5 => "bluetooth",
        13 => "bridge",
        14 => "bond",
        16 => "tun",
        29 => NM_TYPE_WIREGUARD,
        // Loopback got its own device type in NM 1.42; before that it was
        // "generic" and filtered out by name.
        32 => "loopback",
        _ => "generic",
    }
}

/// Bring a device up by activating whatever connection it last used.
///
/// NetworkManager has no "connect this device" call — `nmcli device connect`
/// picks a connection itself. The same choice is made here: the device's own
/// `AvailableConnections`, first entry, which is the list NM keeps in
/// preference order.
pub fn device_connect(device: &str) -> NmResult {
    let device = device.to_string();
    acting(move |conn| {
        let path = nm::devices(conn)
            .into_iter()
            .find(|d| d.interface == device)
            .map(|d| d.path)
            .ok_or_else(|| format!("no device named {device}"))?;

        let available = nm::paths(conn, &path, nm::IFACE_DEVICE, "AvailableConnections");
        let target = available
            .first()
            .ok_or_else(|| format!("{device} has no connection to bring up"))?;

        let target =
            zbus::zvariant::ObjectPath::try_from(target.as_str()).map_err(|e| e.to_string())?;
        let device_path =
            zbus::zvariant::ObjectPath::try_from(path.as_str()).map_err(|e| e.to_string())?;
        let root = zbus::zvariant::ObjectPath::try_from("/").map_err(|e| e.to_string())?;

        nm::proxy(conn, nm::MANAGER_PATH, nm::IFACE_MANAGER)?
            .call::<_, _, zbus::zvariant::OwnedObjectPath>(
                "ActivateConnection",
                &(&target, &device_path, &root),
            )
            .map(|_| ())
            .map_err(|e| nm::dbus_message(&e))
    })
}

pub fn device_disconnect(device: &str) -> NmResult {
    let device = device.to_string();
    acting(move |conn| {
        let path = nm::devices(conn)
            .into_iter()
            .find(|d| d.interface == device)
            .map(|d| d.path)
            .ok_or_else(|| format!("no device named {device}"))?;

        nm::proxy(conn, &path, nm::IFACE_DEVICE)?
            .call::<_, _, ()>("Disconnect", &())
            .map_err(|e| nm::dbus_message(&e))
    })
}

pub fn iface_type_icon(iface_type: &str) -> &'static str {
    match iface_type {
        "wifi" => ICON_SIGNAL_EXCELLENT,
        "ethernet" => ICON_ETHERNET,
        NM_TYPE_WIREGUARD | NM_TYPE_VPN => ICON_VPN,
        _ => "󰛳",
    }
}

// ── IP info ───────────────────────────────────────────────────────────────────

/// The device's IPv4 address, from NetworkManager's own view of it.
///
/// This used to shell out to `ip`; the daemon has the same answer and is
/// already being asked about the device on the line above.
pub fn get_device_ip(device: &str) -> Option<String> {
    let conn = nm::system().ok()?;
    let ip4 = ip4_config(&conn, device)?;
    let addresses: Vec<std::collections::HashMap<String, zbus::zvariant::OwnedValue>> =
        nm::prop(&conn, &ip4, nm::IFACE_IP4, "AddressData")?;
    addresses.first()?.get("address").and_then(nm::as_string)
}

pub fn get_default_gateway() -> Option<String> {
    let conn = nm::system().ok()?;
    let primary = nm::path_prop(
        &conn,
        nm::MANAGER_PATH,
        nm::IFACE_MANAGER,
        "PrimaryConnection",
    )?;
    let device = nm::paths(&conn, &primary, nm::IFACE_ACTIVE, "Devices")
        .into_iter()
        .next()?;
    let ip4 = nm::path_prop(&conn, &device, nm::IFACE_DEVICE, "Ip4Config")?;
    let gateway: String = nm::prop(&conn, &ip4, nm::IFACE_IP4, "Gateway")?;
    (!gateway.is_empty()).then_some(gateway)
}

pub fn get_dns_servers(device: &str) -> Vec<String> {
    let Ok(conn) = nm::system() else {
        return Vec::new();
    };
    let Some(ip4) = ip4_config(&conn, device) else {
        return Vec::new();
    };
    nm::prop::<Vec<std::collections::HashMap<String, zbus::zvariant::OwnedValue>>>(
        &conn,
        &ip4,
        nm::IFACE_IP4,
        "NameserverData",
    )
    .unwrap_or_default()
    .into_iter()
    .filter_map(|entry| entry.get("address").and_then(nm::as_string))
    .collect()
}

fn ip4_config(conn: &zbus::blocking::Connection, device: &str) -> Option<String> {
    let path = nm::devices(conn)
        .into_iter()
        .find(|d| d.interface == device)
        .map(|d| d.path)?;
    nm::path_prop(conn, &path, nm::IFACE_DEVICE, "Ip4Config")
}

// ── Connectivity ──────────────────────────────────────────────────────────────

pub fn check_connectivity() -> ConnectivityState {
    let Ok(conn) = nm::system() else {
        return ConnectivityState::Unknown;
    };
    // The cached property, not `CheckConnectivity()` — that one runs a live
    // probe and blocks for seconds, which is what the old comment here was
    // avoiding by reading `nmcli networking connectivity` instead of
    // `connectivity check`.
    match nm::prop::<u32>(&conn, nm::MANAGER_PATH, nm::IFACE_MANAGER, "Connectivity") {
        Some(1) => ConnectivityState::None,
        Some(2) => ConnectivityState::Portal,
        Some(3) => ConnectivityState::Limited,
        Some(4) => ConnectivityState::Full,
        _ => ConnectivityState::Unknown,
    }
}

// ── WiFi power saving ─────────────────────────────────────────────────────────

/// `802-11-wireless.powersave`: 0 default, 1 ignore, 2 disable, 3 enable.
const POWERSAVE_DISABLE: u32 = 2;
const POWERSAVE_ENABLE: u32 = 3;

pub fn get_wifi_power_saving(conn_name: &str) -> bool {
    let Ok(conn) = nm::system() else {
        return false;
    };
    let Some(path) = wifi_connection_path(&conn, conn_name) else {
        return false;
    };
    let Ok(proxy) = nm::proxy(&conn, &path, nm::IFACE_CONNECTION) else {
        return false;
    };
    let settings: std::collections::HashMap<
        String,
        std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
    > = match proxy.call("GetSettings", &()) {
        Ok(s) => s,
        Err(e) => {
            log::debug!("nm: GetSettings for {conn_name}: {e}");
            return false;
        }
    };

    settings
        .get(NM_TYPE_WIFI)
        .and_then(|section| section.get("powersave"))
        .and_then(nm::as_u32)
        == Some(POWERSAVE_ENABLE)
}

pub fn set_wifi_power_saving(conn_name: &str, enable: bool) -> NmResult {
    let conn_name = conn_name.to_string();
    acting(move |conn| {
        let path = wifi_connection_path(conn, &conn_name)
            .ok_or_else(|| format!("no saved network named {conn_name}"))?;
        let proxy = nm::proxy(conn, &path, nm::IFACE_CONNECTION)?;

        // Read-modify-write: `Update` replaces the whole connection, so
        // anything not carried over here would be silently dropped.
        let mut settings: std::collections::HashMap<
            String,
            std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
        > = proxy
            .call("GetSettings", &())
            .map_err(|e| nm::dbus_message(&e))?;

        let value = if enable {
            POWERSAVE_ENABLE
        } else {
            POWERSAVE_DISABLE
        };
        settings
            .entry(NM_TYPE_WIFI.to_string())
            .or_default()
            .insert(
                "powersave".to_string(),
                zbus::zvariant::Value::from(value)
                    .try_into()
                    .map_err(|e| format!("powersave: {e}"))?,
            );

        proxy
            .call::<_, _, ()>("Update", &(settings,))
            .map_err(|e| nm::dbus_message(&e))
    })
}

fn wifi_connection_path(conn: &zbus::blocking::Connection, name: &str) -> Option<String> {
    nm::stored_connections(conn)
        .into_iter()
        .find(|(_, id, kind)| id == name && kind == NM_TYPE_WIFI)
        .map(|(path, _, _)| path)
}

/// The NM connection name of the active WiFi connection.
pub fn get_active_wifi_conn_name() -> Option<String> {
    let conn = nm::system().ok()?;
    nm::active_connections(&conn)
        .into_iter()
        .find(|(_, kind, _)| kind == NM_TYPE_WIFI)
        .map(|(id, _, _)| id)
}

// ── Shared UI helpers ─────────────────────────────────────────────────────────

/// Apply an `NmResult` to a status label: set text, CSS class, and visibility.
pub fn apply_nm_result(status_lbl: &gtk4::Label, result: &NmResult) {
    use gtk4::prelude::*;
    match result {
        NmResult::Success => {
            status_lbl.set_label("✓");
            status_lbl.add_css_class("network-status-ok");
            status_lbl.remove_css_class("network-status-err");
        }
        NmResult::Failure(msg) => {
            let display = if msg.is_empty() {
                "Failed"
            } else {
                msg.as_str()
            };
            status_lbl.set_label(display);
            status_lbl.add_css_class("network-status-err");
            status_lbl.remove_css_class("network-status-ok");
        }
    }
    status_lbl.set_visible(true);
}

/// Auto-hide a status label after 4 seconds.
pub fn auto_hide_status(status_lbl: &gtk4::Label) {
    use gtk4::prelude::*;
    let status_hide = status_lbl.clone();
    glib::timeout_add_local_once(std::time::Duration::from_secs(4), move || {
        status_hide.set_visible(false);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn net(ssid: &str, signal: u8, known: bool, in_use: bool) -> WifiNetwork {
        WifiNetwork {
            ssid: ssid.into(),
            signal,
            security: "WPA2".into(),
            in_use,
            is_known: known,
            freq_mhz: Some(5180),
        }
    }

    #[test]
    fn one_network_per_ssid_keeps_the_strongest_radio() {
        let merged = merge_and_rank(vec![
            net("office", 40, false, false),
            net("office", 78, false, false),
        ]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].signal, 78);
    }

    #[test]
    fn a_weaker_radio_can_still_contribute_the_flags() {
        // The strong one was seen first and is not the associated AP; the
        // weak one is. Dropping it would lose the "connected" mark.
        let merged = merge_and_rank(vec![
            net("office", 78, false, false),
            net("office", 40, true, true),
        ]);
        assert_eq!(merged.len(), 1);
        assert!(merged[0].in_use);
        assert!(merged[0].is_known);
        assert_eq!(merged[0].signal, 78);
    }

    #[test]
    fn the_connected_network_sorts_first_then_saved_then_signal() {
        let merged = merge_and_rank(vec![
            net("weak-stranger", 20, false, false),
            net("strong-stranger", 90, false, false),
            net("saved", 30, true, false),
            net("connected", 10, true, true),
        ]);
        let order: Vec<&str> = merged.iter().map(|n| n.ssid.as_str()).collect();
        assert_eq!(
            order,
            vec!["connected", "saved", "strong-stranger", "weak-stranger"]
        );
    }

    #[test]
    fn a_nameless_access_point_is_not_a_network() {
        assert!(merge_and_rank(vec![net("", 90, false, false)]).is_empty());
    }
}

#[cfg(test)]
mod live {
    /// Every read path against the running daemon. Ignored: needs
    /// NetworkManager on the system bus. Touches nothing.
    #[test]
    #[ignore]
    fn read_the_session() {
        use super::*;
        println!("available:    {}", network_manager_available());
        println!("wifi adapter: {}", wifi_adapter_present());
        println!("wifi radio:   {}", wifi_radio_enabled());
        println!("active:       {:?}", get_active_connection());
        println!("connectivity: {:?}", check_connectivity());
        println!("gateway:      {:?}", get_default_gateway());
        println!("known ssids:  {:?}", get_known_ssids());
        println!("vpns:         {:?}", get_vpn_connections());
        for iface in get_network_interfaces() {
            println!(
                "  iface {:<12} {:<10} enabled={} ip={:?} dns={:?}",
                iface.device,
                iface.iface_type,
                iface.enabled,
                get_device_ip(&iface.device),
                get_dns_servers(&iface.device),
            );
        }
        if let Some(name) = get_active_wifi_conn_name() {
            println!(
                "active wifi conn: {name} powersave={}",
                get_wifi_power_saving(&name)
            );
        }
    }

    /// A rescan. Ignored and separate: it asks the radio to do something,
    /// even though it changes no configuration.
    #[test]
    #[ignore]
    fn scan() {
        match super::scan_wifi() {
            Ok(networks) => {
                for n in networks.iter().take(12) {
                    println!(
                        "{:>3}% {:<10} {:<24} known={} in_use={}",
                        n.signal, n.security, n.ssid, n.is_known, n.in_use
                    );
                }
                println!("{} networks", networks.len());
            }
            Err(e) => println!("scan failed: {e}"),
        }
    }
}
