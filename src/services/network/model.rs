//! What the network section draws, as plain data, and the rules that turn
//! NetworkManager's numbers into it. Pure: every rule here is tested
//! without a radio, and the fixture states (`fixture.rs`) are built from the
//! same types the live reads fill.

use super::{ActiveConnection, ConnectivityState, NetworkInterface, VpnConnection, WifiNetwork};

/// Everything the section shows at one moment. Read whole (`snapshot.rs`)
/// whenever NetworkManager says something moved, never polled.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    /// NetworkManager is on the bus.
    pub available: bool,
    pub has_wifi: bool,
    pub wifi_enabled: bool,
    /// Mobile broadband, for airplane mode: `None` with no modem.
    pub wwan_enabled: Option<bool>,
    /// The Bluetooth adapter's power, for airplane mode: `None` with none.
    pub bluetooth_powered: Option<bool>,
    pub active: ActiveConnection,
    pub details: Option<Details>,
    pub connectivity: ConnectivityState,
    /// Where NetworkManager checks for a captive portal; opening it in a
    /// browser is what gets redirected to the login page.
    pub portal_uri: Option<String>,
    pub networks: Vec<WifiNetwork>,
    pub saved: Vec<SavedNetwork>,
    pub vpns: Vec<VpnConnection>,
    pub interfaces: Vec<NetworkInterface>,
    /// A Wi-Fi connection on its way up, and how far it got.
    pub activating: Option<Activation>,
    /// The Wi-Fi device's `NMDeviceState` and the reason it got there. A
    /// failed join shows as a disconnected device with a failure reason;
    /// the section knows which network it was joining and names it.
    pub wifi_state: u32,
    pub wifi_reason: u32,
    /// When the Wi-Fi device last finished a scan (NetworkManager's
    /// `LastScan`, ms on the boot clock); moves when fresh results land.
    pub last_scan: i64,
}

/// The connected network, in more detail than its row.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Details {
    pub device: String,
    pub hw_address: Option<String>,
    pub ip4: Option<String>,
    pub ip6: Option<String>,
    pub gateway: Option<String>,
    pub dns: Vec<String>,
    /// Link speed in Mb/s (`Device.Wireless.Bitrate` is kb/s).
    pub bitrate_mbps: Option<u32>,
    pub freq_mhz: Option<u32>,
    pub security: String,
    pub metered: Metered,
    /// The stored connection behind it, for the switches that edit it.
    pub connection_id: Option<String>,
    pub power_saving: bool,
}

/// `NMMetered`, as the device reports it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Metered {
    #[default]
    Unknown,
    Yes,
    No,
    GuessYes,
    GuessNo,
}

impl Metered {
    pub fn from_nm(v: u32) -> Metered {
        match v {
            1 => Metered::Yes,
            2 => Metered::No,
            3 => Metered::GuessYes,
            4 => Metered::GuessNo,
            _ => Metered::Unknown,
        }
    }

    /// Whether traffic on it should be treated as costly.
    pub fn is_metered(self) -> bool {
        matches!(self, Metered::Yes | Metered::GuessYes)
    }
}

/// A stored Wi-Fi connection.
#[derive(Debug, Clone, PartialEq)]
pub struct SavedNetwork {
    /// The connection's name, which is not always its SSID.
    pub id: String,
    pub ssid: String,
    pub autoconnect: bool,
    /// Seconds since the epoch it was last up; 0 for never.
    pub last_used: u64,
}

/// A connection being brought up.
#[derive(Debug, Clone, PartialEq)]
pub struct Activation {
    pub ssid: String,
    pub stage: &'static str,
}

/// Why the last attempt failed, and whether a password would fix it.
#[derive(Debug, Clone, PartialEq)]
pub struct Failure {
    pub ssid: String,
    pub text: String,
    pub needs_password: bool,
}

/// `NMDeviceState` while a connection is coming up, in words a person can
/// act on. `None` for a state that is not part of an activation.
pub fn stage_text(device_state: u32) -> Option<&'static str> {
    Some(match device_state {
        40 => "Preparing…",
        50 => "Configuring…",
        60 => "Waiting for password…",
        70 => "Getting an address…",
        80 => "Checking the connection…",
        90 => "Starting…",
        _ => return None,
    })
}

/// `NMDeviceStateReason` as what went wrong, from the user's side. The
/// numbers are NetworkManager's; the table covers the reasons a Wi-Fi join
/// actually ends in, and names the rest by number rather than hiding them.
pub fn failure_text(reason: u32) -> String {
    match reason {
        7 => "Password needed".into(),
        8..=10 => "Wrong password".into(),
        11 => "The network refused the settings".into(),
        12..=17 => "Could not get an address".into(),
        53 => "The network did not answer".into(),
        54 => "Authentication failed".into(),
        55 | 56 => "The Wi-Fi driver failed".into(),
        57 | 58 => "Wi-Fi hardware problem".into(),
        36 => "Carrier lost".into(),
        39 => "Unplugged".into(),
        64 => "The network is out of range".into(),
        n => format!("Failed (reason {n})"),
    }
}

/// Whether a failure is one a password fixes, so the row opens its
/// password field instead of only saying so.
pub fn wants_password(reason: u32) -> bool {
    matches!(reason, 7..=10 | 54)
}

/// The order to draw `fresh` in, given the order the rows had (`prev`, by
/// SSID). The connected network leads and saved networks come before
/// strangers; inside those groups a network already on screen keeps its
/// place and a new one lands after them by signal. Rows therefore never
/// jump while the list is open, which is when a hand is aiming at one;
/// `prev` is empty on each open, so a fresh open is ranked by signal.
pub fn settle(prev: &[String], mut fresh: Vec<WifiNetwork>) -> Vec<WifiNetwork> {
    let group = |n: &WifiNetwork| {
        if n.in_use {
            0
        } else if n.is_known {
            1
        } else {
            2
        }
    };
    let place = |n: &WifiNetwork| prev.iter().position(|s| *s == n.ssid);
    fresh.sort_by(|a, b| {
        group(a).cmp(&group(b)).then_with(|| match (place(a), place(b)) {
            (Some(x), Some(y)) => x.cmp(&y),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => b.signal.cmp(&a.signal),
        })
    });
    fresh
}

/// A row's second line: security, band, and "Saved".
pub fn row_subtitle(n: &WifiNetwork) -> String {
    let mut parts: Vec<String> = Vec::new();
    parts.push(if n.security.is_empty() {
        "Open".to_string()
    } else {
        n.security.clone()
    });
    if let Some(f) = n.freq_mhz {
        parts.push(super::freq_band_label(f).to_string());
    }
    if n.is_known {
        parts.push("Saved".to_string());
    }
    parts.join(" · ")
}

/// The connected network's second line.
pub fn active_subtitle(conn: &ConnectivityState, details: Option<&Details>) -> String {
    let state = match conn {
        ConnectivityState::Full => "Connected",
        ConnectivityState::Limited => "No internet",
        ConnectivityState::Portal => "Sign-in required",
        ConnectivityState::None => "No internet",
        ConnectivityState::Unknown => "Connected",
    };
    let mut parts = vec![state.to_string()];
    if let Some(d) = details {
        if let Some(f) = d.freq_mhz {
            parts.push(super::freq_band_label(f).to_string());
        }
        if let Some(b) = d.bitrate_mbps.filter(|b| *b > 0) {
            parts.push(format!("{b} Mb/s"));
        }
        if d.metered.is_metered() {
            parts.push("Metered".to_string());
        }
    }
    parts.join(" · ")
}

/// Whether airplane mode is on: every radio there is, off. A machine with
/// no radios at all is not in airplane mode.
pub fn airplane(s: &Snapshot) -> bool {
    let radios = [
        s.has_wifi.then_some(s.wifi_enabled),
        s.wwan_enabled,
        s.bluetooth_powered,
    ];
    let present: Vec<bool> = radios.into_iter().flatten().collect();
    !present.is_empty() && present.iter().all(|on| !on)
}

/// The SSID a stored connection's `802-11-wireless.ssid` bytes spell.
pub fn ssid_from_bytes(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
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

    fn order(v: &[WifiNetwork]) -> Vec<&str> {
        v.iter().map(|n| n.ssid.as_str()).collect()
    }

    #[test]
    fn a_fresh_list_is_connected_then_saved_then_by_signal() {
        let got = settle(
            &[],
            vec![
                net("weak", 20, false, false),
                net("strong", 90, false, false),
                net("saved", 30, true, false),
                net("here", 10, true, true),
            ],
        );
        assert_eq!(order(&got), ["here", "saved", "strong", "weak"]);
    }

    #[test]
    fn rows_on_screen_keep_their_place_when_signals_move() {
        let prev: Vec<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
        // c is now the strongest; it stays third while the list is open.
        let got = settle(
            &prev,
            vec![
                net("c", 95, false, false),
                net("a", 10, false, false),
                net("b", 50, false, false),
            ],
        );
        assert_eq!(order(&got), ["a", "b", "c"]);
    }

    #[test]
    fn a_new_network_lands_after_the_ones_on_screen() {
        let prev: Vec<String> = ["a", "b"].iter().map(|s| s.to_string()).collect();
        let got = settle(
            &prev,
            vec![
                net("new-strong", 99, false, false),
                net("b", 10, false, false),
                net("a", 10, false, false),
                net("new-weak", 5, false, false),
            ],
        );
        assert_eq!(order(&got), ["a", "b", "new-strong", "new-weak"]);
    }

    #[test]
    fn joining_a_network_moves_it_to_the_top_anyway() {
        // The group outranks the place: a network that just connected is
        // not a jump, it is the answer to the click.
        let prev: Vec<String> = ["a", "b"].iter().map(|s| s.to_string()).collect();
        let got = settle(
            &prev,
            vec![net("a", 50, false, false), net("b", 50, true, true)],
        );
        assert_eq!(order(&got), ["b", "a"]);
    }

    #[test]
    fn password_failures_open_the_field() {
        for r in [7, 8, 9, 10, 54] {
            assert!(wants_password(r), "{r}");
        }
        assert!(!wants_password(53));
        assert_eq!(failure_text(8), "Wrong password");
        assert_eq!(failure_text(7), "Password needed");
        assert_eq!(failure_text(999), "Failed (reason 999)");
    }

    #[test]
    fn stages_are_only_the_activation_states() {
        assert_eq!(stage_text(70), Some("Getting an address…"));
        assert_eq!(stage_text(100), None);
        assert_eq!(stage_text(30), None);
    }

    #[test]
    fn airplane_needs_every_present_radio_off() {
        let mut s = Snapshot {
            has_wifi: true,
            wifi_enabled: false,
            bluetooth_powered: Some(false),
            ..Default::default()
        };
        assert!(airplane(&s));
        s.bluetooth_powered = Some(true);
        assert!(!airplane(&s));
        s.bluetooth_powered = None;
        assert!(airplane(&s));
        // No radios: not airplane mode, just a desktop.
        assert!(!airplane(&Snapshot::default()));
    }

    #[test]
    fn subtitles_say_what_matters() {
        let mut open = net("x", 50, true, false);
        open.security.clear();
        assert_eq!(row_subtitle(&open), "Open · 5 GHz · Saved");
        let d = Details {
            freq_mhz: Some(2412),
            bitrate_mbps: Some(144),
            metered: Metered::GuessYes,
            ..Default::default()
        };
        assert_eq!(
            active_subtitle(&ConnectivityState::Portal, Some(&d)),
            "Sign-in required · 2.4 GHz · 144 Mb/s · Metered"
        );
    }
}
