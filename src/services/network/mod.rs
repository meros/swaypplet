//! Network state and actions, over NetworkManager's D-Bus interface.
//!
//! The reads and actions the tile and the section share. The section itself
//! reads through `snapshot` on NetworkManager's change signals (`watch`);
//! `model` is the pure part, `fixture` the canned states for screenshots,
//! `tailscale` the tailnet through its CLI. See `nm.rs` for why D-Bus and
//! not `nmcli`.

use std::collections::HashSet;

pub mod blocked;
pub mod fixture;
pub mod model;
mod nm;
pub mod snapshot;
pub mod tailscale;
pub mod watch;

// ── Nerd Font icons ───────────────────────────────────────────────────────────
pub const ICON_SIGNAL_NONE: &str = "󰤯";
pub const ICON_SIGNAL_WEAK: &str = "󰤟";
pub const ICON_SIGNAL_OK: &str = "󰤢";
pub const ICON_SIGNAL_GOOD: &str = "󰤥";
pub const ICON_SIGNAL_EXCELLENT: &str = "󰤨";
pub const ICON_ETHERNET: &str = "󰈀";
pub const ICON_DISCONNECTED: &str = "󰤭";
pub const ICON_VPN: &str = "󰦝";

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

#[derive(Debug, Clone, PartialEq, Default)]
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
    #[default]
    Disconnected,
}

#[derive(Debug, Clone)]
pub struct VpnConnection {
    pub name: String,
    pub active: bool,
    pub vpn_type: String,
}

#[derive(Debug, Clone)]
pub struct NetworkInterface {
    pub device: String,
    pub iface_type: String,
    pub enabled: bool,
    /// Left alone by NetworkManager until unplugged ([`is_banned`]).
    pub banned: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum ConnectivityState {
    Full,
    Limited,
    Portal,
    None,
    #[default]
    Unknown,
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

    nm::stored_connections_with_vpn_type(&conn)
        .into_iter()
        .filter(|(_, _, kind, _)| is_vpn(kind))
        .map(|(_, name, _, vpn_type)| VpnConnection {
            active: active.contains(&name),
            name,
            vpn_type,
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






// ── Interface management ──────────────────────────────────────────────────────

pub fn get_network_interfaces() -> Vec<NetworkInterface> {
    let Ok(conn) = nm::system() else {
        return Vec::new();
    };
    nm::devices(&conn)
        .into_iter()
        .filter_map(|device| {
            // A banned adapter is unmanaged, which the filter below drops; it
            // stays in the list so the person can lift the ban.
            let banned = device.device_type == nm::DEVICE_TYPE_ETHERNET
                && device.state <= nm::DEVICE_STATE_UNMANAGED
                && nm::managed_and_real(&conn, &device.path).is_some_and(|(managed, real)| {
                    is_banned(&device.interface, device.device_type, managed, real)
                });
            if !banned
                && !is_user_facing_interface(&device.interface, device.device_type, device.state)
            {
                return None;
            }
            let iface_type = device_type_name(device.device_type);
            Some(NetworkInterface {
                enabled: device.state > nm::DEVICE_STATE_DISCONNECTED,
                device: device.interface,
                iface_type: iface_type.to_string(),
                banned,
            })
        })
        .collect()
}

/// A wired adapter NetworkManager has been told to leave alone for now.
///
/// A ban is NM's runtime `Managed = false` on a real ethernet device: what
/// the panel's Block button sets, and what the host's dispatcher sets when
/// the link drops while the adapter is still plugged in (a flaky dock link
/// that would otherwise break every call on it). The machine falls back to
/// Wi-Fi until the adapter is unplugged, which ends the ban with the device.
///
/// Read off `Managed`, not off the state: unmanaged is also where a docker
/// bridge or an external veth sits, and those are not banned, just not NM's.
/// The type and the name rule them out; `Real` rules out the placeholder
/// device NM keeps for a profile whose hardware is absent.
pub fn is_banned(interface: &str, device_type: u32, managed: bool, real: bool) -> bool {
    real && !managed && device_type == nm::DEVICE_TYPE_ETHERNET && !is_virtual_name(interface)
}

/// One real, physically named wired adapter, as the bar's hazard reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct Wired {
    /// NM's object path: one per plug-in, never reused for a replug.
    pub path: String,
    pub interface: String,
    pub managed: bool,
}

/// Every real wired adapter with a physical name, managed or not.
pub fn wired_adapters() -> Vec<Wired> {
    let Ok(conn) = nm::system() else {
        return Vec::new();
    };
    nm::devices(&conn)
        .into_iter()
        .filter(|d| d.device_type == nm::DEVICE_TYPE_ETHERNET && !is_virtual_name(&d.interface))
        .filter_map(|d| {
            let (managed, real) = nm::managed_and_real(&conn, &d.path)?;
            real.then_some(Wired {
                path: d.path,
                interface: d.interface,
                managed,
            })
        })
        .collect()
}

/// Ban (`true`) or lift the ban on (`false`) a wired adapter. See
/// [`is_banned`]; the ban lasts until the adapter is unplugged.
pub fn set_banned(device: &str, banned: bool) -> NmResult {
    let device = device.to_string();
    acting(move |conn| {
        let path = nm::devices(conn)
            .into_iter()
            .find(|d| d.interface == device)
            .map(|d| d.path)
            .ok_or_else(|| format!("no device named {device}"))?;
        nm::set_managed(conn, &path, !banned)
    })
}

fn is_user_facing_interface(interface: &str, device_type: u32, state: u32) -> bool {
    // Ignore unmanaged devices (e.g. veth pairs, external containers)
    if state <= nm::DEVICE_STATE_UNMANAGED {
        return false;
    }
    let iface_type = device_type_name(device_type);
    // Wi-Fi has its own dedicated top-level section; loopback/bridge are internal.
    if iface_type == "loopback"
        || iface_type == "bridge"
        || iface_type == "wifi"
        || interface == "lo"
    {
        return false;
    }
    !is_virtual_name(interface)
}

/// A virtual device, by the name its creator gave it.
fn is_virtual_name(interface: &str) -> bool {
    // Long enough that a prefix match is the whole test: nothing a person
    // would name an adapter starts with any of these.
    const VIRTUAL: [&str; 10] = [
        "veth",      // container pair ends: veth<random hex>
        "docker",    // docker0
        "br-",       // docker's user-defined bridges
        "virbr",     // libvirt
        "tailscale", // tailscale0
        "tunl",      // the kernel's IPIP device, and not "tun" plus a letter
        "dummy",
        "p2p-dev",
        "cni",
        "flannel",
    ];
    if VIRTUAL.iter().any(|prefix| interface.starts_with(prefix)) {
        return true;
    }

    // The short ones need a boundary. A udev rule may name a real adapter
    // anything, and "tun" on its own also swallows a device called
    // "tundra", which is a wired card the user then cannot find. So these
    // match only when what follows is not another letter — a number
    // ("tun0", "wg0"), a separator ("wg-office"), or nothing at all.
    const SHORT: [&str; 3] = ["tun", "tap", "wg"];
    SHORT.iter().any(|prefix| {
        interface
            .strip_prefix(prefix)
            .is_some_and(|rest| !rest.starts_with(|c: char| c.is_ascii_alphabetic()))
    })
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

    // ── The interface list's filter ──────────────────────────────────────
    //
    // What it decides is what the Advanced group shows. Wrong in one
    // direction it hides a real adapter the user is looking for; wrong in
    // the other it lists a docker bridge and a wireguard tunnel as if they
    // were hardware. Neither shows up in a test that only exercises Wi-Fi,
    // which is why these are here.

    const ETHERNET: u32 = 1;
    const WIFI: u32 = nm::DEVICE_TYPE_WIFI;
    const GENERIC: u32 = 20;
    const ACTIVE: u32 = nm::DEVICE_STATE_DISCONNECTED + 70; // NM_DEVICE_STATE_ACTIVATED

    #[test]
    fn a_wired_adapter_is_user_facing() {
        assert!(is_user_facing_interface("enp0s31f6", ETHERNET, ACTIVE));
        assert!(is_user_facing_interface("eth0", ETHERNET, ACTIVE));
        // Down is still a device someone may want to bring up.
        assert!(is_user_facing_interface(
            "enp0s31f6",
            ETHERNET,
            nm::DEVICE_STATE_DISCONNECTED
        ));
    }

    #[test]
    fn an_unmanaged_real_wired_adapter_is_banned() {
        assert!(is_banned("enp0s13f0u2u1", ETHERNET, false, true));
        // Managed is the normal case, not a ban.
        assert!(!is_banned("enp0s13f0u2u1", ETHERNET, true, true));
        // NM's placeholder for a profile whose hardware is absent.
        assert!(!is_banned("enp0s13f0u2u1", ETHERNET, false, false));
    }

    #[test]
    fn unmanaged_virtual_and_wireless_devices_are_not_banned() {
        // Docker's devices sit unmanaged by design; they are not NM's to ban.
        for iface in ["docker0", "br-1a2b3c", "veth9f2a", "tailscale0"] {
            assert!(!is_banned(iface, ETHERNET, false, true), "{iface}");
        }
        assert!(!is_banned("wlp0s20f3", WIFI, false, true));
    }

    #[test]
    fn wifi_is_not_in_the_interface_list() {
        // It has the whole section above it; listing it twice was the bug
        // this filter exists to stop.
        assert!(!is_user_facing_interface("wlp2s0", WIFI, ACTIVE));
    }

    #[test]
    fn an_unmanaged_device_is_not_user_facing() {
        assert!(!is_user_facing_interface(
            "enp0s31f6",
            ETHERNET,
            nm::DEVICE_STATE_UNMANAGED
        ));
        // The states below unmanaged are unknown and unavailable, which are
        // not devices to offer either.
        assert!(!is_user_facing_interface("enp0s31f6", ETHERNET, 0));
    }

    #[test]
    fn the_virtual_zoo_is_filtered_by_name() {
        for iface in [
            "lo",
            "docker0",
            "br-1a2b3c",
            "virbr0",
            "veth9f2a",
            "tailscale0",
            "tun0",
            "tap0",
            "wg0",
            "dummy0",
            "p2p-dev-wlp2s0",
            "cni0",
            "flannel.1",
        ] {
            assert!(
                !is_user_facing_interface(iface, GENERIC, ACTIVE),
                "{iface} should not be offered as an interface"
            );
        }
    }

    #[test]
    fn a_real_name_that_starts_like_a_virtual_one_is_kept() {
        // The prefixes are matched at the start of the name, so a device
        // whose name merely contains one stays. `tunl0` and `wgX` are the
        // near misses worth pinning: both really are virtual, and both are
        // caught by prefix, while an ethernet named `tundra` is not.
        assert!(is_user_facing_interface("tundra", ETHERNET, ACTIVE));
        assert!(!is_user_facing_interface("tunl0", GENERIC, ACTIVE));
        assert!(!is_user_facing_interface("wg-office", GENERIC, ACTIVE));
    }

    #[test]
    fn loopback_is_out_by_type_as_well_as_by_name() {
        // NM 1.42 gave loopback its own device type; before that it was
        // generic and only the name caught it. Both paths still have to.
        assert!(!is_user_facing_interface("lo", 32, ACTIVE));
        assert!(!is_user_facing_interface("lo", GENERIC, ACTIVE));
    }
}

#[cfg(test)]
mod live {
    /// Every read path against the running daemon. Ignored: needs
    /// NetworkManager on the system bus. Touches nothing.
    #[test]
    #[ignore]
    fn read_the_session() {
        let at = std::time::Instant::now();
        let s = super::snapshot::read();
        println!("read in {:?}", at.elapsed());
        println!("available {} wifi {} on {} airplane {}", s.available, s.has_wifi, s.wifi_enabled, super::model::airplane(&s));
        println!("active {:?} connectivity {:?} portal {:?}", s.active, s.connectivity, s.portal_uri);
        println!("details {:?}", s.details);
        println!("state {} reason {} activating {:?} last_scan {}", s.wifi_state, s.wifi_reason, s.activating, s.last_scan);
        for n in s.networks.iter().take(12) {
            println!("  {:>3}% {:<10} {:<24} known={} in_use={}", n.signal, n.security, n.ssid, n.is_known, n.in_use);
        }
        println!("saved {:?}", s.saved.iter().map(|x| (&x.id, &x.ssid, x.autoconnect)).collect::<Vec<_>>());
        println!("vpns {:?}", s.vpns);
        println!("tailscale {:?}", super::tailscale::status());
    }
}
