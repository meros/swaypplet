//! Canned network states, for screenshots.
//!
//! The render harness talks to the machine's real NetworkManager, and the
//! states worth a picture ("connecting", "wrong password", "sign-in
//! required", airplane mode) are ones nobody should cause on a real
//! machine to take one. `SWAYPPLET_NET_FIXTURE=<name>` makes the section
//! draw one of these instead, with every action turned into a no-op.

use super::model::{Activation, Details, Metered, SavedNetwork, Snapshot};
use super::tailscale::{Peer, Status};
use super::{ActiveConnection, ConnectivityState, NetworkInterface, VpnConnection, WifiNetwork};

/// The fixture the environment asks for, if any.
pub fn requested() -> Option<String> {
    std::env::var("SWAYPPLET_NET_FIXTURE")
        .ok()
        .filter(|s| !s.is_empty())
}

fn wifi(ssid: &str, signal: u8, security: &str, freq: u32, known: bool, in_use: bool) -> WifiNetwork {
    WifiNetwork {
        ssid: ssid.into(),
        signal,
        security: security.into(),
        in_use,
        is_known: known,
        freq_mhz: Some(freq),
    }
}

fn base() -> Snapshot {
    Snapshot {
        available: true,
        has_wifi: true,
        wifi_enabled: true,
        wwan_enabled: None,
        bluetooth_powered: Some(true),
        active: ActiveConnection::Wifi {
            ssid: "Björkhagen 5G".into(),
            signal: 82,
            device: "wlp0s20f3".into(),
            freq_mhz: Some(5180),
        },
        details: Some(Details {
            device: "wlp0s20f3".into(),
            hw_address: Some("02:00:00:00:00:01".into()),
            ip4: Some("192.168.1.42".into()),
            ip6: Some("2a02:aa1:1024:9e00::42".into()),
            gateway: Some("192.168.1.1".into()),
            dns: vec!["192.168.1.1".into(), "1.1.1.1".into()],
            bitrate_mbps: Some(866),
            freq_mhz: Some(5180),
            security: "WPA3".into(),
            metered: Metered::No,
            connection_id: Some("Björkhagen 5G".into()),
            power_saving: false,
        }),
        connectivity: ConnectivityState::Full,
        portal_uri: Some("http://nmcheck.gnome.org/check_network_status.txt".into()),
        networks: vec![
            wifi("Björkhagen 5G", 82, "WPA3", 5180, true, true),
            wifi("Björkhagen", 74, "WPA2", 2437, true, false),
            wifi("Kafé Torget", 61, "", 2412, false, false),
            wifi("Router-4F2C", 55, "WPA2", 5500, false, false),
            wifi("iPhone (Ada)", 44, "WPA2 WPA3", 5745, false, false),
            wifi("Hus 12 gäst", 28, "WPA2", 2462, false, false),
        ],
        saved: vec![
            SavedNetwork {
                id: "Björkhagen 5G".into(),
                ssid: "Björkhagen 5G".into(),
                autoconnect: true,
                last_used: 1_790_400_000,
            },
            SavedNetwork {
                id: "Björkhagen".into(),
                ssid: "Björkhagen".into(),
                autoconnect: true,
                last_used: 1_790_300_000,
            },
            SavedNetwork {
                id: "SJ Resenär".into(),
                ssid: "SJ Resenär".into(),
                autoconnect: false,
                last_used: 1_780_000_000,
            },
        ],
        vpns: vec![
            VpnConnection {
                name: "Office".into(),
                active: false,
                vpn_type: "WireGuard".into(),
            },
            VpnConnection {
                name: "Mullvad SE".into(),
                active: true,
                vpn_type: "WireGuard".into(),
            },
        ],
        interfaces: vec![NetworkInterface {
            device: "enp0s31f6".into(),
            iface_type: "ethernet".into(),
            enabled: false,
        }],
        activating: None,
        wifi_state: 100,
        wifi_reason: 0,
        last_scan: 1,
    }
}

/// The snapshot for `name`; unknown names get the connected state.
pub fn snapshot(name: &str) -> Snapshot {
    let mut s = base();
    match name {
        "connecting" => {
            s.active = ActiveConnection::Disconnected;
            s.details = None;
            s.connectivity = ConnectivityState::None;
            s.networks[0].in_use = false;
            s.wifi_state = 70;
            s.activating = Some(Activation {
                ssid: "Björkhagen 5G".into(),
                stage: "Getting an address…",
            });
        }
        "wrong-password" => {
            s.active = ActiveConnection::Disconnected;
            s.details = None;
            s.connectivity = ConnectivityState::None;
            s.networks[0].in_use = false;
            s.wifi_state = 30;
            s.wifi_reason = 8;
        }
        "portal" => {
            s.active = ActiveConnection::Wifi {
                ssid: "Kafé Torget".into(),
                signal: 61,
                device: "wlp0s20f3".into(),
                freq_mhz: Some(2412),
            };
            if let Some(d) = s.details.as_mut() {
                d.freq_mhz = Some(2412);
                d.bitrate_mbps = Some(72);
                d.security = String::new();
                d.connection_id = Some("Kafé Torget".into());
                d.metered = Metered::GuessYes;
            }
            s.connectivity = ConnectivityState::Portal;
            s.networks[0].in_use = false;
            s.networks[2].in_use = true;
            s.networks[2].is_known = true;
        }
        "airplane" => {
            s.wifi_enabled = false;
            s.bluetooth_powered = Some(false);
            s.active = ActiveConnection::Disconnected;
            s.details = None;
            s.connectivity = ConnectivityState::None;
            s.networks.clear();
            s.vpns.iter_mut().for_each(|v| v.active = false);
        }
        _ => {}
    }
    s
}

/// The joining network a fixture failure belongs to.
pub fn pending(name: &str) -> Option<String> {
    (name == "wrong-password").then(|| "Björkhagen 5G".to_string())
}

pub fn tailscale(name: &str) -> Option<Status> {
    (name != "airplane").then(|| Status {
        state: "Running".into(),
        tailnet: Some("example.github".into()),
        self_ip: Some("100.64.0.1".into()),
        self_name: Some("laptop".into()),
        exit_node: None,
        exit_options: vec![
            Peer {
                name: "server".into(),
                ip: "100.64.0.2".into(),
                online: true,
            },
            Peer {
                name: "cabin-pi".into(),
                ip: "100.64.0.3".into(),
                online: false,
            },
        ],
        peers_online: 2,
        peers_total: 3,
    })
}
