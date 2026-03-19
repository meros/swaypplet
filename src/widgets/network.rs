use std::cell::RefCell;
use std::process::Command;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::thread;

use gtk4::prelude::*;
use gtk4::{
    Box, Button, Label, ListBox, ListBoxRow, Orientation, PasswordEntry, Revealer,
    RevealerTransitionType, Spinner,
};

// ── Nerd Font icons ───────────────────────────────────────────────────────────
const ICON_SIGNAL_NONE: &str = "󰤯";
const ICON_SIGNAL_WEAK: &str = "󰤟";
const ICON_SIGNAL_OK: &str = "󰤢";
const ICON_SIGNAL_GOOD: &str = "󰤥";
const ICON_SIGNAL_EXCELLENT: &str = "󰤨";
const ICON_LOCK: &str = "";
const ICON_ETHERNET: &str = "󰈀";
const ICON_DISCONNECTED: &str = "󰤭";
const ICON_VPN: &str = "󰦝";

// Maximum number of networks shown before a "Show all" button appears.
const MAX_VISIBLE_NETWORKS: usize = 8;

// ── Signal strength helpers ───────────────────────────────────────────────────

fn signal_icon(strength: u8) -> &'static str {
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
struct WifiNetwork {
    ssid: String,
    signal: u8,
    security: String,
    in_use: bool,
    is_known: bool,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
enum ActiveConnection {
    Wifi { ssid: String, signal: u8 },
    Ethernet { device: String },
    Disconnected,
}

#[derive(Debug, Clone)]
struct VpnConnection {
    name: String,
    active: bool,
}

// ── nmcli availability check ──────────────────────────────────────────────────

fn nmcli_available() -> bool {
    Command::new("nmcli").arg("--version").output().is_ok()
}

fn wifi_adapter_present() -> bool {
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

// ── Backend helpers ───────────────────────────────────────────────────────────

fn parse_wifi_list(output: &str, known_ssids: &[String]) -> Vec<WifiNetwork> {
    let mut networks: Vec<WifiNetwork> = Vec::new();
    for line in output.lines() {
        // Format: SSID:SIGNAL:SECURITY:IN-USE (nmcli -t escapes colons as \:)
        let parts: Vec<&str> = line.splitn(4, ':').collect();
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

        // Deduplicate: keep strongest signal per SSID, union in_use flag.
        if let Some(existing) = networks.iter_mut().find(|n| n.ssid == ssid) {
            if signal > existing.signal {
                existing.signal = signal;
            }
            existing.in_use = existing.in_use || in_use;
            existing.is_known = existing.is_known || is_known;
            continue;
        }

        networks.push(WifiNetwork {
            ssid,
            signal,
            security,
            in_use,
            is_known,
        });
    }

    // Sort: active first, then known sorted by signal desc, then unknown by signal desc.
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

fn get_known_ssids() -> Vec<String> {
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
            if parts.len() == 2 && parts[1].trim() == "802-11-wireless" {
                Some(parts[0].replace("\\:", ":").trim().to_string())
            } else {
                None
            }
        })
        .collect()
}

/// Runs `nmcli device wifi list --rescan yes` (blocking, may take 2-5 s).
/// Returns the raw stdout string.
fn scan_wifi_raw() -> String {
    let out = Command::new("nmcli")
        .args([
            "-t",
            "-f",
            "SSID,SIGNAL,SECURITY,IN-USE",
            "device",
            "wifi",
            "list",
            "--rescan",
            "yes",
        ])
        .output();
    match out {
        Ok(o) => String::from_utf8_lossy(&o.stdout).into_owned(),
        Err(_) => String::new(),
    }
}

fn get_active_connection() -> ActiveConnection {
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
            "802-11-wireless" => {
                let ssid = parts[0].replace("\\:", ":").trim().to_string();
                let signal = get_active_wifi_signal(&ssid);
                return ActiveConnection::Wifi { ssid, signal };
            }
            "802-3-ethernet" => {
                return ActiveConnection::Ethernet { device };
            }
            _ => {}
        }
    }
    ActiveConnection::Disconnected
}

fn get_active_wifi_signal(ssid: &str) -> u8 {
    let Ok(out) = Command::new("nmcli")
        .args(["-t", "-f", "SSID,SIGNAL,IN-USE", "device", "wifi", "list"])
        .output()
    else {
        return 0;
    };
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        let parts: Vec<&str> = line.splitn(3, ':').collect();
        if parts.len() < 3 {
            continue;
        }
        let line_ssid = parts[0].replace("\\:", ":").trim().to_string();
        if line_ssid == ssid {
            return parts[1].trim().parse().unwrap_or(0);
        }
    }
    0
}

fn get_vpn_connections() -> Vec<VpnConnection> {
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
                if t == "vpn" || t == "wireguard" {
                    return Some(parts[0].replace("\\:", ":").trim().to_string());
                }
            }
            None
        })
        .collect();

    if all_vpns.is_empty() {
        return Vec::new();
    }

    // Find which VPNs are active.
    let active_set: std::collections::HashSet<String> = Command::new("nmcli")
        .args(["-t", "-f", "NAME,TYPE", "connection", "show", "--active"])
        .output()
        .map(|active_out| {
            let at = String::from_utf8_lossy(&active_out.stdout).into_owned();
            at.lines()
                .filter_map(|line| {
                    let parts: Vec<&str> = line.splitn(2, ':').collect();
                    if parts.len() == 2 {
                        let t = parts[1].trim();
                        if t == "vpn" || t == "wireguard" {
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

/// Spawn nmcli to connect to a known saved connection. Returns the process
/// exit status via `Arc<Mutex<Option<bool>>>` (true = success).
fn nmcli_connect_known_async(ssid: String, result: Arc<Mutex<Option<bool>>>) {
    thread::spawn(move || {
        let status = Command::new("nmcli")
            .args(["connection", "up", &ssid])
            .status();
        let ok = status.map(|s| s.success()).unwrap_or(false);
        *result.lock().unwrap() = Some(ok);
    });
}

fn nmcli_connect_new_async(ssid: String, password: String, result: Arc<Mutex<Option<bool>>>) {
    thread::spawn(move || {
        let mut cmd = Command::new("nmcli");
        cmd.args(["device", "wifi", "connect", &ssid]);
        if !password.is_empty() {
            cmd.args(["password", &password]);
        }
        let status = cmd.status();
        let ok = status.map(|s| s.success()).unwrap_or(false);
        *result.lock().unwrap() = Some(ok);
    });
}

fn nmcli_vpn_up(name: String) {
    let _ = Command::new("nmcli")
        .args(["connection", "up", &name])
        .spawn();
}

fn nmcli_vpn_down(name: String) {
    let _ = Command::new("nmcli")
        .args(["connection", "down", &name])
        .spawn();
}

// ── Internal state ────────────────────────────────────────────────────────────

struct NetworkState {
    active: ActiveConnection,
    networks: Vec<WifiNetwork>,
    vpns: Vec<VpnConnection>,
    list_visible: bool,
    show_all: bool,
    scanning: bool,
}

// ── NetworkSection ────────────────────────────────────────────────────────────

#[allow(dead_code)] // Fields kept alive for GObject ref-counting
pub struct NetworkSection {
    root: Box,
    state: Rc<RefCell<NetworkState>>,
    // Status row widgets
    current_icon_label: Label,
    current_ssid_label: Label,
    current_signal_label: Label,
    // Scan status
    scan_spinner: Spinner,
    scan_status_label: Label,
    // Toggle / lists
    toggle_button: Button,
    revealer: Revealer,
    network_list_box: ListBox,
    vpn_list_box: ListBox,
}

impl NetworkSection {
    pub fn new() -> Self {
        let root = Box::builder()
            .orientation(Orientation::Vertical)
            .spacing(6)
            .build();
        root.add_css_class("section");

        // ── Section title ─────────────────────────────────────────────────────
        let title = Label::builder()
            .label("NETWORK")
            .halign(gtk4::Align::Start)
            .build();
        title.add_css_class("section-title");
        root.append(&title);

        // ── nmcli / adapter guard ─────────────────────────────────────────────
        if !nmcli_available() {
            let placeholder = Label::builder()
                .label("NetworkManager not available")
                .halign(gtk4::Align::Start)
                .build();
            placeholder.add_css_class("network-placeholder");
            root.append(&placeholder);

            // Return a minimal (non-functional) instance.
            return Self {
                root,
                state: Rc::new(RefCell::new(NetworkState {
                    active: ActiveConnection::Disconnected,
                    networks: Vec::new(),
                    vpns: Vec::new(),
                    list_visible: false,
                    show_all: false,
                    scanning: false,
                })),
                current_icon_label: Label::new(None),
                current_ssid_label: Label::new(None),
                current_signal_label: Label::new(None),
                scan_spinner: Spinner::new(),
                scan_status_label: Label::new(None),
                toggle_button: Button::new(),
                revealer: Revealer::new(),
                network_list_box: ListBox::new(),
                vpn_list_box: ListBox::new(),
            };
        }

        // ── Current connection row ────────────────────────────────────────────
        let current_row = Box::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .build();
        current_row.add_css_class("network-current");

        let current_icon_label = Label::builder()
            .label(ICON_DISCONNECTED)
            .halign(gtk4::Align::Start)
            .build();
        current_icon_label.add_css_class("network-icon");

        let current_ssid_label = Label::builder()
            .label("Disconnected")
            .halign(gtk4::Align::Start)
            .hexpand(true)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .build();
        current_ssid_label.add_css_class("network-ssid");

        let current_signal_label = Label::builder()
            .label("")
            .halign(gtk4::Align::End)
            .build();
        current_signal_label.add_css_class("network-signal");

        current_row.append(&current_icon_label);
        current_row.append(&current_ssid_label);
        current_row.append(&current_signal_label);
        root.append(&current_row);

        // ── Scan status row (spinner + label) ─────────────────────────────────
        let scan_row = Box::builder()
            .orientation(Orientation::Horizontal)
            .spacing(6)
            .halign(gtk4::Align::Start)
            .build();

        let scan_spinner = Spinner::new();
        scan_spinner.set_visible(false);

        let scan_status_label = Label::builder()
            .label("")
            .halign(gtk4::Align::Start)
            .build();
        scan_status_label.add_css_class("network-scan-status");
        scan_status_label.set_visible(false);

        scan_row.append(&scan_spinner);
        scan_row.append(&scan_status_label);
        root.append(&scan_row);

        // ── Toggle button ─────────────────────────────────────────────────────
        let toggle_button = Button::builder()
            .label("▸ Available Networks")
            .hexpand(true)
            .build();
        toggle_button.add_css_class("section-expander");
        root.append(&toggle_button);

        // ── Revealer ──────────────────────────────────────────────────────────
        let revealer = Revealer::builder()
            .transition_type(RevealerTransitionType::SlideDown)
            .transition_duration(200)
            .reveal_child(false)
            .build();

        let revealer_box = Box::builder()
            .orientation(Orientation::Vertical)
            .spacing(8)
            .margin_top(4)
            .build();

        // WiFi adapter warning (shown if no adapter, but nmcli exists)
        let no_adapter_label = Label::builder()
            .label("No WiFi adapter")
            .halign(gtk4::Align::Start)
            .build();
        no_adapter_label.add_css_class("network-placeholder");
        no_adapter_label.set_visible(!wifi_adapter_present());
        revealer_box.append(&no_adapter_label);

        let network_list_box = ListBox::builder()
            .selection_mode(gtk4::SelectionMode::None)
            .build();
        network_list_box.add_css_class("network-list");

        revealer_box.append(&network_list_box);

        // VPN subsection
        let vpn_title = Label::builder()
            .label("VPN")
            .halign(gtk4::Align::Start)
            .build();
        vpn_title.add_css_class("network-subsection-title");
        revealer_box.append(&vpn_title);

        let vpn_list_box = ListBox::builder()
            .selection_mode(gtk4::SelectionMode::None)
            .build();
        vpn_list_box.add_css_class("network-list");
        revealer_box.append(&vpn_list_box);

        revealer.set_child(Some(&revealer_box));
        root.append(&revealer);

        let state = Rc::new(RefCell::new(NetworkState {
            active: ActiveConnection::Disconnected,
            networks: Vec::new(),
            vpns: Vec::new(),
            list_visible: false,
            show_all: false,
            scanning: false,
        }));

        // ── Wire up toggle ────────────────────────────────────────────────────
        {
            let revealer_c = revealer.clone();
            toggle_button.connect_clicked(move |btn| {
                let revealed = !revealer_c.reveals_child();
                revealer_c.set_reveal_child(revealed);
                btn.set_label(if revealed {
                    "▾ Available Networks"
                } else {
                    "▸ Available Networks"
                });
            });
        }

        let section = Self {
            root,
            state,
            current_icon_label,
            current_ssid_label,
            current_signal_label,
            scan_spinner,
            scan_status_label,
            toggle_button,
            revealer,
            network_list_box,
            vpn_list_box,
        };

        section.refresh();
        section
    }

    pub fn refresh(&self) {
        // Update active connection (fast).
        let active = get_active_connection();
        self.update_active_display(&active);

        {
            let mut s = self.state.borrow_mut();
            s.active = active;
        }

        // Update VPN list (fast).
        let vpns = get_vpn_connections();
        {
            self.state.borrow_mut().vpns = vpns;
        }
        self.rebuild_vpn_list();

        // Kick off background WiFi scan.
        self.start_wifi_scan();
    }

    fn update_active_display(&self, active: &ActiveConnection) {
        match active {
            ActiveConnection::Wifi { ssid, signal } => {
                self.current_icon_label.set_label(signal_icon(*signal));
                self.current_ssid_label.set_label(ssid);
                self.current_signal_label
                    .set_label(&format!("{}%", signal));
            }
            ActiveConnection::Ethernet { .. } => {
                self.current_icon_label.set_label(ICON_ETHERNET);
                self.current_ssid_label.set_label("Ethernet");
                self.current_signal_label.set_label("");
            }
            ActiveConnection::Disconnected => {
                self.current_icon_label.set_label(ICON_DISCONNECTED);
                self.current_ssid_label.set_label("Disconnected");
                self.current_signal_label.set_label("");
            }
        }
    }

    fn start_wifi_scan(&self) {
        // Avoid concurrent scans.
        if self.state.borrow().scanning {
            return;
        }
        self.state.borrow_mut().scanning = true;

        // Show scanning indicator.
        self.scan_spinner.set_visible(true);
        self.scan_spinner.start();
        self.scan_status_label.set_label("Scanning…");
        self.scan_status_label.set_visible(true);

        // Launch blocking scan on a background thread. Only plain data
        // crosses the thread boundary; GTK widgets stay on the main thread.
        let scan_result: Arc<Mutex<Option<Vec<WifiNetwork>>>> = Arc::new(Mutex::new(None));
        let scan_result_writer = scan_result.clone();

        thread::spawn(move || {
            let raw = scan_wifi_raw();
            let known = get_known_ssids();
            let networks = parse_wifi_list(&raw, &known);
            *scan_result_writer.lock().unwrap() = Some(networks);
        });

        // Poll for the result on the GTK main loop every 200 ms.
        let scan_spinner_c = self.scan_spinner.clone();
        let scan_status_c = self.scan_status_label.clone();
        let network_list_box_c = self.network_list_box.clone();
        let state_c = self.state.clone();

        glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
            let done = scan_result.lock().unwrap().is_some();
            if !done {
                return glib::ControlFlow::Continue;
            }

            let networks = scan_result.lock().unwrap().take().unwrap();

            scan_spinner_c.stop();
            scan_spinner_c.set_visible(false);
            scan_status_c.set_visible(false);

            {
                let mut s = state_c.borrow_mut();
                s.networks = networks;
                s.scanning = false;
                s.show_all = false;
            }

            rebuild_wifi_list_into(&network_list_box_c, &state_c);

            glib::ControlFlow::Break
        });
    }

    #[allow(dead_code)]
    fn rebuild_network_list(&self) {
        rebuild_wifi_list_into(&self.network_list_box, &self.state);
    }

    fn rebuild_vpn_list(&self) {
        while let Some(child) = self.vpn_list_box.first_child() {
            self.vpn_list_box.remove(&child);
        }

        let vpns = self.state.borrow().vpns.clone();

        if vpns.is_empty() {
            let empty_lbl = Label::builder()
                .label("No VPN connections configured")
                .halign(gtk4::Align::Start)
                .build();
            empty_lbl.add_css_class("network-placeholder");
            let row = ListBoxRow::builder().build();
            row.set_child(Some(&empty_lbl));
            row.add_css_class("network-row");
            self.vpn_list_box.append(&row);
            return;
        }

        for vpn in vpns {
            let row_box = Box::builder()
                .orientation(Orientation::Horizontal)
                .spacing(8)
                .margin_top(4)
                .margin_bottom(4)
                .margin_start(4)
                .margin_end(4)
                .build();

            let icon_lbl = Label::builder().label(ICON_VPN).build();
            icon_lbl.add_css_class("network-icon");
            if vpn.active {
                icon_lbl.add_css_class("network-vpn-active");
            }

            let name_lbl = Label::builder()
                .label(&vpn.name)
                .halign(gtk4::Align::Start)
                .hexpand(true)
                .ellipsize(gtk4::pango::EllipsizeMode::End)
                .build();
            name_lbl.add_css_class("network-ssid");

            let btn_label = if vpn.active { "Disconnect" } else { "Connect" };
            let action_btn = Button::builder().label(btn_label).build();
            action_btn.add_css_class("network-connect-btn");

            {
                let name_clone = vpn.name.clone();
                let is_active = vpn.active;
                action_btn.connect_clicked(move |_| {
                    if is_active {
                        nmcli_vpn_down(name_clone.clone());
                    } else {
                        nmcli_vpn_up(name_clone.clone());
                    }
                });
            }

            row_box.append(&icon_lbl);
            row_box.append(&name_lbl);
            row_box.append(&action_btn);

            let list_row = ListBoxRow::builder().build();
            list_row.set_child(Some(&row_box));
            list_row.add_css_class("network-row");
            self.vpn_list_box.append(&list_row);
        }
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }
}

// ── WiFi list builder (free function so it can be called from idle callback) ──

fn rebuild_wifi_list_into(list: &ListBox, state: &Rc<RefCell<NetworkState>>) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }

    let (networks, show_all) = {
        let s = state.borrow();
        (s.networks.clone(), s.show_all)
    };

    let total = networks.len();
    let visible_count = if show_all {
        total
    } else {
        total.min(MAX_VISIBLE_NETWORKS)
    };

    for network in networks.iter().take(visible_count) {
        let list_row = build_wifi_row(network);
        list.append(&list_row);
    }

    // "Show all" / "Show fewer" button when more networks exist.
    if total > MAX_VISIBLE_NETWORKS {
        let btn_label = if show_all {
            "Show fewer".to_string()
        } else {
            format!("Show all ({})", total)
        };
        let more_btn = Button::builder()
            .label(&btn_label)
            .halign(gtk4::Align::Center)
            .build();
        more_btn.add_css_class("network-show-all-btn");

        let state_c = state.clone();
        let list_c = list.clone();
        more_btn.connect_clicked(move |_| {
            {
                let mut s = state_c.borrow_mut();
                s.show_all = !s.show_all;
            }
            rebuild_wifi_list_into(&list_c, &state_c);
        });

        let row = ListBoxRow::builder().build();
        row.set_child(Some(&more_btn));
        row.add_css_class("network-row");
        list.append(&row);
    }
}

fn build_wifi_row(network: &WifiNetwork) -> ListBoxRow {
    let connect_area = Box::builder()
        .orientation(Orientation::Vertical)
        .spacing(4)
        .build();

    // ── Main info row ─────────────────────────────────────────────────────────
    let row_box = Box::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .margin_top(4)
        .margin_bottom(4)
        .margin_start(4)
        .margin_end(4)
        .build();

    let signal_lbl = Label::builder()
        .label(signal_icon(network.signal))
        .tooltip_text(format!("{}%", network.signal))
        .build();
    signal_lbl.add_css_class("network-icon");

    let ssid_lbl = Label::builder()
        .label(&network.ssid)
        .halign(gtk4::Align::Start)
        .hexpand(true)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .build();
    ssid_lbl.add_css_class("network-ssid");
    if network.in_use {
        ssid_lbl.add_css_class("network-active");
    }

    let lock_lbl = Label::builder()
        .label(
            if network.security.is_empty() || network.security == "--" {
                ""
            } else {
                ICON_LOCK
            },
        )
        .build();
    lock_lbl.add_css_class("network-security");

    row_box.append(&signal_lbl);
    row_box.append(&ssid_lbl);
    row_box.append(&lock_lbl);
    connect_area.append(&row_box);

    // ── Connect controls (not shown for in-use network) ───────────────────────
    if !network.in_use {
        let needs_password =
            !network.security.is_empty() && network.security != "--" && !network.is_known;

        if network.is_known {
            // Known network: single Connect button with feedback.
            let btn_row = Box::builder()
                .orientation(Orientation::Horizontal)
                .halign(gtk4::Align::End)
                .spacing(6)
                .build();

            let spinner = Spinner::new();
            spinner.set_visible(false);

            let status_lbl = Label::builder().label("").build();
            status_lbl.add_css_class("network-conn-status");
            status_lbl.set_visible(false);

            let connect_btn = Button::builder().label("Connect").build();
            connect_btn.add_css_class("network-connect-btn");

            btn_row.append(&spinner);
            btn_row.append(&status_lbl);
            btn_row.append(&connect_btn);
            connect_area.append(&btn_row);

            wire_connect_known_button(
                &connect_btn,
                &spinner,
                &status_lbl,
                network.ssid.clone(),
            );
        } else if needs_password {
            // Unknown encrypted network: password revealer.
            let pw_revealer = Revealer::builder()
                .transition_type(RevealerTransitionType::SlideDown)
                .transition_duration(150)
                .reveal_child(false)
                .build();

            let pw_area = Box::builder()
                .orientation(Orientation::Vertical)
                .spacing(4)
                .build();

            let pw_row = Box::builder()
                .orientation(Orientation::Horizontal)
                .spacing(6)
                .build();

            let pw_entry = PasswordEntry::builder()
                .hexpand(true)
                .placeholder_text("Password")
                .show_peek_icon(true)
                .build();
            pw_entry.add_css_class("network-password-entry");

            let connect_btn = Button::builder().label("Connect").build();
            connect_btn.add_css_class("network-connect-btn");

            pw_row.append(&pw_entry);
            pw_row.append(&connect_btn);
            pw_area.append(&pw_row);

            // Feedback row (spinner + status label).
            let fb_row = Box::builder()
                .orientation(Orientation::Horizontal)
                .halign(gtk4::Align::End)
                .spacing(6)
                .build();

            let spinner = Spinner::new();
            spinner.set_visible(false);

            let status_lbl = Label::builder().label("").build();
            status_lbl.add_css_class("network-conn-status");
            status_lbl.set_visible(false);

            fb_row.append(&spinner);
            fb_row.append(&status_lbl);
            pw_area.append(&fb_row);

            pw_revealer.set_child(Some(&pw_area));
            connect_area.append(&pw_revealer);

            // Enter key in password field triggers connect.
            wire_connect_new_button(
                &connect_btn,
                &pw_entry,
                &spinner,
                &status_lbl,
                network.ssid.clone(),
            );

            // Toggle revealer on row_box click.
            let click = gtk4::GestureClick::new();
            {
                let rev_c = pw_revealer.clone();
                let entry_c = pw_entry.clone();
                click.connect_released(move |_, _, _, _| {
                    let visible = rev_c.reveals_child();
                    rev_c.set_reveal_child(!visible);
                    if !visible {
                        entry_c.grab_focus();
                    }
                });
            }
            row_box.add_controller(click);
        } else {
            // Unknown open network: Connect with feedback.
            let btn_row = Box::builder()
                .orientation(Orientation::Horizontal)
                .halign(gtk4::Align::End)
                .spacing(6)
                .build();

            let spinner = Spinner::new();
            spinner.set_visible(false);

            let status_lbl = Label::builder().label("").build();
            status_lbl.add_css_class("network-conn-status");
            status_lbl.set_visible(false);

            let connect_btn = Button::builder().label("Connect").build();
            connect_btn.add_css_class("network-connect-btn");

            btn_row.append(&spinner);
            btn_row.append(&status_lbl);
            btn_row.append(&connect_btn);
            connect_area.append(&btn_row);

            // Open network: connect_new with empty password.
            wire_connect_new_button_open(
                &connect_btn,
                &spinner,
                &status_lbl,
                network.ssid.clone(),
            );
        }
    }

    let list_row = ListBoxRow::builder().build();
    list_row.set_child(Some(&connect_area));
    list_row.add_css_class("network-row");
    list_row
}

// ── Connection wiring helpers ─────────────────────────────────────────────────

fn wire_connect_known_button(
    btn: &Button,
    spinner: &Spinner,
    status_lbl: &Label,
    ssid: String,
) {
    let btn_c = btn.clone();
    let spinner_c = spinner.clone();
    let status_c = status_lbl.clone();

    btn.connect_clicked(move |_| {
        btn_c.set_sensitive(false);
        spinner_c.set_visible(true);
        spinner_c.start();
        status_c.set_visible(false);

        let result: Arc<Mutex<Option<bool>>> = Arc::new(Mutex::new(None));
        let result_c = result.clone();

        nmcli_connect_known_async(ssid.clone(), result_c);

        // Poll for result every 200 ms.
        let btn_poll = btn_c.clone();
        let spinner_poll = spinner_c.clone();
        let status_poll = status_c.clone();
        let result_poll = result.clone();

        glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
            let done = result_poll.lock().unwrap().is_some();
            if !done {
                return glib::ControlFlow::Continue;
            }

            let ok = result_poll.lock().unwrap().unwrap();
            spinner_poll.stop();
            spinner_poll.set_visible(false);
            btn_poll.set_sensitive(true);

            if ok {
                status_poll.set_label("✓");
                status_poll.add_css_class("network-status-ok");
                status_poll.remove_css_class("network-status-err");
            } else {
                status_poll.set_label("Failed");
                status_poll.add_css_class("network-status-err");
                status_poll.remove_css_class("network-status-ok");
            }
            status_poll.set_visible(true);

            // Hide status after 4 s.
            let status_hide = status_poll.clone();
            glib::timeout_add_local_once(std::time::Duration::from_secs(4), move || {
                status_hide.set_visible(false);
            });

            glib::ControlFlow::Break
        });
    });
}

fn wire_connect_new_button(
    btn: &Button,
    pw_entry: &PasswordEntry,
    spinner: &Spinner,
    status_lbl: &Label,
    ssid: String,
) {
    // Enter key in password field.
    {
        let btn_enter = btn.clone();
        pw_entry.connect_activate(move |_| {
            btn_enter.emit_clicked();
        });
    }

    let btn_c = btn.clone();
    let pw_c = pw_entry.clone();
    let spinner_c = spinner.clone();
    let status_c = status_lbl.clone();

    btn.connect_clicked(move |_| {
        let password = pw_c.text().to_string();
        btn_c.set_sensitive(false);
        spinner_c.set_visible(true);
        spinner_c.start();
        status_c.set_visible(false);

        let result: Arc<Mutex<Option<bool>>> = Arc::new(Mutex::new(None));
        let result_c = result.clone();

        nmcli_connect_new_async(ssid.clone(), password, result_c);

        let btn_poll = btn_c.clone();
        let spinner_poll = spinner_c.clone();
        let status_poll = status_c.clone();
        let result_poll = result.clone();

        glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
            let done = result_poll.lock().unwrap().is_some();
            if !done {
                return glib::ControlFlow::Continue;
            }

            let ok = result_poll.lock().unwrap().unwrap();
            spinner_poll.stop();
            spinner_poll.set_visible(false);
            btn_poll.set_sensitive(true);

            if ok {
                status_poll.set_label("✓");
                status_poll.add_css_class("network-status-ok");
                status_poll.remove_css_class("network-status-err");
            } else {
                status_poll.set_label("Failed");
                status_poll.add_css_class("network-status-err");
                status_poll.remove_css_class("network-status-ok");
            }
            status_poll.set_visible(true);

            let status_hide = status_poll.clone();
            glib::timeout_add_local_once(std::time::Duration::from_secs(4), move || {
                status_hide.set_visible(false);
            });

            glib::ControlFlow::Break
        });
    });
}

fn wire_connect_new_button_open(
    btn: &Button,
    spinner: &Spinner,
    status_lbl: &Label,
    ssid: String,
) {
    let btn_c = btn.clone();
    let spinner_c = spinner.clone();
    let status_c = status_lbl.clone();

    btn.connect_clicked(move |_| {
        btn_c.set_sensitive(false);
        spinner_c.set_visible(true);
        spinner_c.start();
        status_c.set_visible(false);

        let result: Arc<Mutex<Option<bool>>> = Arc::new(Mutex::new(None));
        let result_c = result.clone();

        nmcli_connect_new_async(ssid.clone(), String::new(), result_c);

        let btn_poll = btn_c.clone();
        let spinner_poll = spinner_c.clone();
        let status_poll = status_c.clone();
        let result_poll = result.clone();

        glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
            let done = result_poll.lock().unwrap().is_some();
            if !done {
                return glib::ControlFlow::Continue;
            }

            let ok = result_poll.lock().unwrap().unwrap();
            spinner_poll.stop();
            spinner_poll.set_visible(false);
            btn_poll.set_sensitive(true);

            if ok {
                status_poll.set_label("✓");
                status_poll.add_css_class("network-status-ok");
                status_poll.remove_css_class("network-status-err");
            } else {
                status_poll.set_label("Failed");
                status_poll.add_css_class("network-status-err");
                status_poll.remove_css_class("network-status-ok");
            }
            status_poll.set_visible(true);

            let status_hide = status_poll.clone();
            glib::timeout_add_local_once(std::time::Duration::from_secs(4), move || {
                status_hide.set_visible(false);
            });

            glib::ControlFlow::Break
        });
    });
}
