use std::collections::HashSet;
use std::process::Command;
use std::sync::mpsc;
use std::thread;

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

// ── Result type for async nmcli operations ────────────────────────────────────

#[derive(Debug)]
pub enum NmResult {
    Success,
    Failure(String),
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

#[derive(Debug, Clone, PartialEq)]
pub enum ActiveConnection {
    Wifi { ssid: String, signal: u8, device: String, freq_mhz: Option<u32> },
    Ethernet { device: String },
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

// ── nmcli availability check ──────────────────────────────────────────────────

pub fn nmcli_available() -> bool {
    Command::new("nmcli").arg("--version").output().is_ok()
}

pub fn wifi_adapter_present() -> bool {
    let Ok(out) = Command::new("nmcli")
        .args(["-t", "-f", "DEVICE,TYPE", "device", "status"])
        .output()
    else {
        return false;
    };
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines().any(|line| {
        let parts: Vec<&str> = line.splitn(2, ':').collect();
        parts.len() == 2 && parts[1].trim() == "wifi"
    })
}

// ── WiFi radio state ──────────────────────────────────────────────────────────

pub fn wifi_radio_enabled() -> bool {
    let Ok(out) = Command::new("nmcli").args(["radio", "wifi"]).output() else {
        return false;
    };
    String::from_utf8_lossy(&out.stdout).trim() == "enabled"
}

pub fn set_wifi_radio_async(enable: bool, tx: mpsc::Sender<NmResult>) {
    let state: &str = if enable { "on" } else { "off" };
    let state = state.to_owned();
    thread::spawn(move || {
        let out = Command::new("nmcli")
            .args(["radio", "wifi", &state])
            .output();
        let result = match out {
            Ok(o) if o.status.success() => NmResult::Success,
            Ok(o) => NmResult::Failure(String::from_utf8_lossy(&o.stderr).trim().to_string()),
            Err(e) => NmResult::Failure(e.to_string()),
        };
        let _ = tx.send(result);
    });
}

// ── Backend helpers ───────────────────────────────────────────────────────────

pub fn parse_wifi_list(output: &str, known_ssids: &[String]) -> Vec<WifiNetwork> {
    let mut networks: Vec<WifiNetwork> = Vec::new();
    for line in output.lines() {
        let parts: Vec<&str> = line.splitn(5, ':').collect();
        if parts.len() < 4 {
            continue;
        }
        let ssid = parts[0].replace("\\:", ":").trim().to_string();
        if ssid.is_empty() {
            continue;
        }
        let signal: u8 = parts[1].trim().parse().unwrap_or(0);
        let security = parts[2].trim().to_string();
        let in_use = parts[3].trim() == "*";
        let is_known = known_ssids.iter().any(|k| k == &ssid);
        let freq_mhz: Option<u32> = parts.get(4).and_then(|s| s.trim().parse().ok());

        if let Some(existing) = networks.iter_mut().find(|n| n.ssid == ssid) {
            if signal > existing.signal {
                existing.signal = signal;
                existing.freq_mhz = freq_mhz;
            }
            existing.in_use = existing.in_use || in_use;
            existing.is_known = existing.is_known || is_known;
            continue;
        }

        networks.push(WifiNetwork { ssid, signal, security, in_use, is_known, freq_mhz });
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
    let Ok(out) = Command::new("nmcli")
        .args(["-t", "-f", "NAME,TYPE", "connection", "show"])
        .output()
    else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.splitn(2, ':').collect();
            if parts.len() == 2 && parts[1].trim() == NM_TYPE_WIFI {
                Some(parts[0].replace("\\:", ":").trim().to_string())
            } else {
                None
            }
        })
        .collect()
}

pub fn scan_wifi_raw() -> String {
    let out = Command::new("nmcli")
        .args(["-t", "-f", "SSID,SIGNAL,SECURITY,IN-USE,FREQ", "device", "wifi", "list", "--rescan", "yes"])
        .output();
    match out {
        Ok(o) => String::from_utf8_lossy(&o.stdout).into_owned(),
        Err(_) => String::new(),
    }
}

pub fn get_active_connection() -> ActiveConnection {
    let Ok(out) = Command::new("nmcli")
        .args(["-t", "-f", "NAME,TYPE,DEVICE", "connection", "show", "--active"])
        .output()
    else {
        return ActiveConnection::Disconnected;
    };

    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        let parts: Vec<&str> = line.splitn(3, ':').collect();
        if parts.len() < 3 {
            continue;
        }
        let conn_type = parts[1].trim();
        let device = parts[2].trim().to_string();
        match conn_type {
            NM_TYPE_WIFI => {
                let ssid = parts[0].replace("\\:", ":").trim().to_string();
                let (signal, freq_mhz) = get_active_wifi_info(&ssid);
                return ActiveConnection::Wifi { ssid, signal, device, freq_mhz };
            }
            NM_TYPE_ETHERNET => {
                return ActiveConnection::Ethernet { device };
            }
            _ => {}
        }
    }
    ActiveConnection::Disconnected
}

fn get_active_wifi_info(ssid: &str) -> (u8, Option<u32>) {
    let Ok(out) = Command::new("nmcli")
        .args(["-t", "-f", "SSID,SIGNAL,IN-USE,FREQ", "device", "wifi", "list"])
        .output()
    else {
        return (0, None);
    };
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        let parts: Vec<&str> = line.splitn(4, ':').collect();
        if parts.len() < 3 {
            continue;
        }
        let line_ssid = parts[0].replace("\\:", ":").trim().to_string();
        if line_ssid == ssid {
            let signal = parts[1].trim().parse().unwrap_or(0);
            let freq = parts.get(3).and_then(|s| s.trim().parse().ok());
            return (signal, freq);
        }
    }
    (0, None)
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
    let Ok(out) = Command::new("nmcli")
        .args(["-t", "-f", "NAME,TYPE", "connection", "show"])
        .output()
    else {
        return Vec::new();
    };

    let text = String::from_utf8_lossy(&out.stdout);
    let all_vpns: Vec<String> = text
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.splitn(2, ':').collect();
            if parts.len() == 2 {
                let t = parts[1].trim();
                if t == NM_TYPE_VPN || t == NM_TYPE_WIREGUARD {
                    return Some(parts[0].replace("\\:", ":").trim().to_string());
                }
            }
            None
        })
        .collect();

    if all_vpns.is_empty() {
        return Vec::new();
    }

    let active_set: HashSet<String> = Command::new("nmcli")
        .args(["-t", "-f", "NAME,TYPE", "connection", "show", "--active"])
        .output()
        .map(|active_out| {
            let at = String::from_utf8_lossy(&active_out.stdout).into_owned();
            at.lines()
                .filter_map(|line| {
                    let parts: Vec<&str> = line.splitn(2, ':').collect();
                    if parts.len() == 2 {
                        let t = parts[1].trim();
                        if t == NM_TYPE_VPN || t == NM_TYPE_WIREGUARD {
                            return Some(parts[0].replace("\\:", ":").trim().to_string());
                        }
                    }
                    None
                })
                .collect()
        })
        .unwrap_or_default();

    all_vpns
        .into_iter()
        .map(|name| {
            let active = active_set.contains(&name);
            VpnConnection { name, active }
        })
        .collect()
}

pub fn vpn_up_async(name: String, tx: mpsc::Sender<NmResult>) {
    thread::spawn(move || {
        let out = Command::new("nmcli")
            .args(["connection", "up", &name])
            .output();
        let result = match out {
            Ok(o) if o.status.success() => NmResult::Success,
            Ok(o) => NmResult::Failure(String::from_utf8_lossy(&o.stderr).trim().to_string()),
            Err(e) => NmResult::Failure(e.to_string()),
        };
        let _ = tx.send(result);
    });
}

pub fn vpn_down(name: &str) {
    let _ = Command::new("nmcli")
        .args(["connection", "down", name])
        .spawn();
}

pub fn vpn_down_async(name: String, tx: mpsc::Sender<NmResult>) {
    thread::spawn(move || {
        let out = Command::new("nmcli")
            .args(["connection", "down", &name])
            .output();
        let result = match out {
            Ok(o) if o.status.success() => NmResult::Success,
            Ok(o) => NmResult::Failure(String::from_utf8_lossy(&o.stderr).trim().to_string()),
            Err(e) => NmResult::Failure(e.to_string()),
        };
        let _ = tx.send(result);
    });
}

// ── WiFi connect/forget ───────────────────────────────────────────────────────

pub fn connect_known_async(ssid: String, tx: mpsc::Sender<NmResult>) {
    thread::spawn(move || {
        let out = Command::new("nmcli")
            .args(["connection", "up", &ssid])
            .output();
        let result = match out {
            Ok(o) if o.status.success() => NmResult::Success,
            Ok(o) => NmResult::Failure(String::from_utf8_lossy(&o.stderr).trim().to_string()),
            Err(e) => NmResult::Failure(e.to_string()),
        };
        let _ = tx.send(result);
    });
}

pub fn connect_new_async(ssid: String, password: String, hidden: bool, tx: mpsc::Sender<NmResult>) {
    thread::spawn(move || {
        let mut cmd = Command::new("nmcli");
        cmd.args(["device", "wifi", "connect", &ssid]);
        if !password.is_empty() {
            cmd.args(["password", &password]);
        }
        if hidden {
            cmd.args(["hidden", "yes"]);
        }
        let out = cmd.output();
        let result = match out {
            Ok(o) if o.status.success() => NmResult::Success,
            Ok(o) => NmResult::Failure(String::from_utf8_lossy(&o.stderr).trim().to_string()),
            Err(e) => NmResult::Failure(e.to_string()),
        };
        let _ = tx.send(result);
    });
}

pub fn forget_network(ssid: &str) {
    let _ = Command::new("nmcli")
        .args(["connection", "delete", ssid])
        .output();
}

// ── Interface management ──────────────────────────────────────────────────────

pub fn get_network_interfaces() -> Vec<NetworkInterface> {
    let Ok(out) = Command::new("nmcli")
        .args(["-t", "-f", "DEVICE,TYPE,STATE", "device", "status"])
        .output()
    else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.splitn(3, ':').collect();
            if parts.len() < 3 {
                return None;
            }
            let device = parts[0].trim().to_string();
            let iface_type = parts[1].trim().to_string();
            let raw_state = parts[2].trim();
            if iface_type == "loopback" || iface_type == "bridge" || device == "lo" {
                return None;
            }
            let enabled = raw_state != "disconnected"
                && raw_state != "unavailable"
                && raw_state != "unmanaged";
            Some(NetworkInterface { device, iface_type, enabled })
        })
        .collect()
}

pub fn device_connect_async(device: String, tx: mpsc::Sender<NmResult>) {
    thread::spawn(move || {
        let out = Command::new("nmcli")
            .args(["device", "connect", &device])
            .output();
        let result = match out {
            Ok(o) if o.status.success() => NmResult::Success,
            Ok(o) => NmResult::Failure(String::from_utf8_lossy(&o.stderr).trim().to_string()),
            Err(e) => NmResult::Failure(e.to_string()),
        };
        let _ = tx.send(result);
    });
}

pub fn device_disconnect_async(device: String, tx: mpsc::Sender<NmResult>) {
    thread::spawn(move || {
        let out = Command::new("nmcli")
            .args(["device", "disconnect", &device])
            .output();
        let result = match out {
            Ok(o) if o.status.success() => NmResult::Success,
            Ok(o) => NmResult::Failure(String::from_utf8_lossy(&o.stderr).trim().to_string()),
            Err(e) => NmResult::Failure(e.to_string()),
        };
        let _ = tx.send(result);
    });
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

pub fn get_device_ip(device: &str) -> Option<String> {
    let out = Command::new("ip")
        .args(["-4", "-o", "addr", "show", device])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        if let Some(inet_pos) = line.find("inet ") {
            let rest = &line[inet_pos + 5..];
            if let Some(slash) = rest.find('/') {
                return Some(rest[..slash].trim().to_string());
            }
            return Some(rest.split_whitespace().next()?.to_string());
        }
    }
    None
}

pub fn get_default_gateway() -> Option<String> {
    let out = Command::new("ip")
        .args(["-4", "route", "show", "default"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 3 && parts[0] == "default" && parts[1] == "via" {
            return Some(parts[2].to_string());
        }
    }
    None
}

pub fn get_dns_servers(device: &str) -> Vec<String> {
    let Ok(out) = Command::new("nmcli")
        .args(["-t", "-f", "IP4.DNS", "device", "show", device])
        .output()
    else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines()
        .filter_map(|line| {
            // Format: IP4.DNS[1]:1.1.1.1
            let (_, addr) = line.split_once(':')?;
            let addr = addr.trim();
            if addr.is_empty() { None } else { Some(addr.to_string()) }
        })
        .collect()
}

// ── Connectivity ──────────────────────────────────────────────────────────────

pub fn check_connectivity() -> ConnectivityState {
    // Use `connectivity` (cached) not `connectivity check` (remote probe that blocks 1-5s).
    let Ok(out) = Command::new("nmcli")
        .args(["networking", "connectivity"])
        .output()
    else {
        return ConnectivityState::Unknown;
    };
    match String::from_utf8_lossy(&out.stdout).trim() {
        "full" => ConnectivityState::Full,
        "limited" => ConnectivityState::Limited,
        "portal" => ConnectivityState::Portal,
        "none" => ConnectivityState::None,
        _ => ConnectivityState::Unknown,
    }
}

// ── WiFi power saving ─────────────────────────────────────────────────────────

pub fn get_wifi_power_saving(conn_name: &str) -> bool {
    let Ok(out) = Command::new("nmcli")
        .args(["-t", "-f", "802-11-wireless.powersave", "connection", "show", conn_name])
        .output()
    else {
        return false;
    };
    let text = String::from_utf8_lossy(&out.stdout);
    // powersave: 3 = enabled, 2 = disabled, 0 = default, 1 = ignore
    text.trim().ends_with(":3")
}

pub fn set_wifi_power_saving_async(conn_name: String, enable: bool, tx: mpsc::Sender<NmResult>) {
    let value = if enable { "3" } else { "2" };
    let value = value.to_string();
    thread::spawn(move || {
        let out = Command::new("nmcli")
            .args(["connection", "modify", &conn_name, "802-11-wireless.powersave", &value])
            .output();
        let result = match out {
            Ok(o) if o.status.success() => NmResult::Success,
            Ok(o) => NmResult::Failure(String::from_utf8_lossy(&o.stderr).trim().to_string()),
            Err(e) => NmResult::Failure(e.to_string()),
        };
        let _ = tx.send(result);
    });
}

/// Get the NM connection name for the active WiFi connection.
pub fn get_active_wifi_conn_name() -> Option<String> {
    let Ok(out) = Command::new("nmcli")
        .args(["-t", "-f", "NAME,TYPE", "connection", "show", "--active"])
        .output()
    else {
        return None;
    };
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        let parts: Vec<&str> = line.splitn(2, ':').collect();
        if parts.len() == 2 && parts[1].trim() == NM_TYPE_WIFI {
            return Some(parts[0].replace("\\:", ":").trim().to_string());
        }
    }
    None
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
            let display = if msg.is_empty() { "Failed" } else { msg.as_str() };
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
