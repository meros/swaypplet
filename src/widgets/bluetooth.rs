use std::cell::RefCell;
use std::process::Command;
use std::rc::Rc;
use std::sync::mpsc;

use gtk4::prelude::*;
use gtk4::{Box, Button, Label, Orientation, Revealer, RevealerTransitionType, Spinner};

// ── Nerd Font icons ───────────────────────────────────────────────────────────
const ICON_HEADPHONES: &str = "󰋋";
const ICON_KEYBOARD: &str = "󰌌";
const ICON_MOUSE: &str = "󰍽";
const ICON_PHONE: &str = "󰏲";
const ICON_COMPUTER: &str = "󰍹";
const ICON_BLUETOOTH: &str = "󰂯";

// ── Data types ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct BtDevice {
    mac: String,
    name: String,
    /// Value of the "Icon:" field from `bluetoothctl info`, e.g. "audio-headset".
    icon_hint: Option<String>,
    connected: bool,
    /// Battery percentage (0–100) if available.
    battery: Option<u8>,
}

/// Result sent from a background connect/disconnect thread back to the UI.
#[derive(Debug)]
enum ConnectResult {
    Success,
    Failure(String),
}

// ── Backend helpers ───────────────────────────────────────────────────────────

/// Returns `true` if Bluetooth is powered on.
fn bt_is_powered() -> bool {
    let Ok(out) = Command::new("bluetoothctl").arg("show").output() else {
        return false;
    };
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Powered: ") {
            return rest.trim() == "yes";
        }
    }
    false
}

/// Returns `true` if `bluetoothctl` is available on the system.
fn bt_available() -> bool {
    Command::new("bluetoothctl")
        .arg("--version")
        .output()
        .is_ok()
}

/// Run `bluetoothctl devices` and return every known device MAC address.
fn bt_list_macs() -> Vec<String> {
    let Ok(out) = Command::new("bluetoothctl").arg("devices").output() else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            // Format: "Device XX:XX:XX:XX:XX:XX Name…"
            let mut parts = line.splitn(3, ' ');
            if parts.next() == Some("Device") {
                parts.next().map(str::to_owned)
            } else {
                None
            }
        })
        .collect()
}

/// Query `bluetoothctl info <MAC>` and build a `BtDevice`.
fn bt_info(mac: &str) -> Option<BtDevice> {
    let out = Command::new("bluetoothctl")
        .args(["info", mac])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);

    if text.trim().is_empty() {
        return None;
    }

    let mut name = mac.to_owned();
    let mut connected = false;
    let mut icon_hint: Option<String> = None;
    let mut battery: Option<u8> = None;

    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Name: ") {
            name = rest.to_owned();
        } else if let Some(rest) = line.strip_prefix("Connected: ") {
            connected = rest.trim() == "yes";
        } else if let Some(rest) = line.strip_prefix("Icon: ") {
            icon_hint = Some(rest.trim().to_owned());
        } else if let Some(rest) = line.strip_prefix("Battery Percentage: ") {
            // Format is "0x4b (75)" — grab the decimal inside the parens.
            if let Some(inner) = rest.find('(').and_then(|s| {
                let after = &rest[s + 1..];
                after.find(')').map(|e| &after[..e])
            }) {
                battery = inner.trim().parse().ok();
            }
        }
    }

    Some(BtDevice {
        mac: mac.to_owned(),
        name,
        icon_hint,
        connected,
        battery,
    })
}

/// Connect to a device in the calling thread (blocking). Returns a `ConnectResult`.
fn bt_connect_blocking(mac: &str) -> ConnectResult {
    let result = Command::new("bluetoothctl").args(["connect", mac]).output();

    match result {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            let combined = format!("{stdout}{stderr}");
            if combined.contains("Connection successful") || combined.contains("Connected: yes") {
                ConnectResult::Success
            } else {
                // Try to extract a useful reason from the output.
                let reason = combined
                    .lines()
                    .find(|l| l.contains("Failed") || l.contains("Error") || l.contains("not"))
                    .map(|l| l.trim().to_owned())
                    .unwrap_or_else(|| "Unknown error".to_owned());
                ConnectResult::Failure(reason)
            }
        }
        Err(e) => ConnectResult::Failure(e.to_string()),
    }
}

/// Disconnect a device in the calling thread (blocking).
fn bt_disconnect_blocking(mac: &str) -> ConnectResult {
    let result = Command::new("bluetoothctl")
        .args(["disconnect", mac])
        .output();

    match result {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            let combined = format!("{stdout}{stderr}");
            if combined.contains("Successful") || combined.contains("Connected: no") {
                ConnectResult::Success
            } else {
                let reason = combined
                    .lines()
                    .find(|l| l.contains("Failed") || l.contains("Error"))
                    .map(|l| l.trim().to_owned())
                    .unwrap_or_else(|| "Unknown error".to_owned());
                ConnectResult::Failure(reason)
            }
        }
        Err(e) => ConnectResult::Failure(e.to_string()),
    }
}

/// Remove a paired device from bluetoothctl.
fn bt_forget(mac: &str) {
    let _ = Command::new("bluetoothctl").args(["remove", mac]).spawn();
}

/// Nerd Font glyph for a device based on its icon hint string.
fn device_icon(hint: Option<&str>) -> &'static str {
    match hint {
        Some(h)
            if h.contains("headphone")
                || h.contains("headset")
                || h.contains("audio-card")
                || h.starts_with("audio") =>
        {
            ICON_HEADPHONES
        }
        Some(h) if h.contains("keyboard") => ICON_KEYBOARD,
        Some(h) if h.contains("mouse") => ICON_MOUSE,
        Some(h) if h.contains("phone") => ICON_PHONE,
        Some(h) if h.contains("computer") || h.contains("laptop") => ICON_COMPUTER,
        _ => ICON_BLUETOOTH,
    }
}

// ── Internal state ────────────────────────────────────────────────────────────

struct State {
    scanning: bool,
}

// ── BluetoothSection ──────────────────────────────────────────────────────────

#[allow(dead_code)] // Fields kept alive for GObject ref-counting
pub struct BluetoothSection {
    root: Box,
    connected_list: Box,
    available_list: Box,
    revealer: Revealer,
    scan_spinner: Spinner,
    scan_btn: Button,
    scan_status_lbl: Label,
    state: Rc<RefCell<State>>,
}

impl BluetoothSection {
    pub fn new() -> Self {
        // ── Root section box ──────────────────────────────────────────────────
        let root = Box::builder()
            .orientation(Orientation::Vertical)
            .spacing(4)
            .build();
        root.add_css_class("section");

        // ── Section title ─────────────────────────────────────────────────────
        let title = Label::builder().label("BLUETOOTH").xalign(0.0).build();
        title.add_css_class("section-title");
        root.append(&title);

        // ── Early-exit if BlueZ unavailable ───────────────────────────────────
        if !bt_available() {
            let msg = Label::builder()
                .label("BlueZ not available")
                .xalign(0.0)
                .build();
            msg.add_css_class("bt-unavailable");
            root.append(&msg);

            // Return a stub with no real functionality.
            let state = Rc::new(RefCell::new(State { scanning: false }));
            return Self {
                root,
                connected_list: Box::new(Orientation::Vertical, 0),
                available_list: Box::new(Orientation::Vertical, 0),
                revealer: Revealer::new(),
                scan_spinner: Spinner::new(),
                scan_btn: Button::new(),
                scan_status_lbl: Label::new(None),
                state,
            };
        }

        // ── Connected devices list ────────────────────────────────────────────
        let connected_list = Box::builder()
            .orientation(Orientation::Vertical)
            .spacing(2)
            .build();
        connected_list.add_css_class("device-list");
        root.append(&connected_list);

        // ── Revealer toggle button ────────────────────────────────────────────
        let toggle_btn = Button::with_label("▸ Available Devices");
        toggle_btn.add_css_class("revealer-toggle");
        root.append(&toggle_btn);

        // ── Revealer content ──────────────────────────────────────────────────
        let revealer = Revealer::builder()
            .transition_type(RevealerTransitionType::SlideDown)
            .transition_duration(200)
            .reveal_child(false)
            .build();

        let revealer_box = Box::builder()
            .orientation(Orientation::Vertical)
            .spacing(4)
            .build();
        revealer_box.add_css_class("revealer-content");

        // Scan row: button + spinner + status label
        let scan_row = Box::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .build();

        let scan_btn = Button::with_label("Scan");
        scan_btn.add_css_class("scan-button");

        let scan_spinner = Spinner::new();
        scan_spinner.set_visible(false);

        let scan_status_lbl = Label::builder().label("").xalign(0.0).hexpand(true).build();
        scan_status_lbl.add_css_class("scan-status");

        scan_row.append(&scan_btn);
        scan_row.append(&scan_spinner);
        scan_row.append(&scan_status_lbl);
        revealer_box.append(&scan_row);

        // Available devices list
        let available_list = Box::builder()
            .orientation(Orientation::Vertical)
            .spacing(2)
            .build();
        available_list.add_css_class("device-list");
        revealer_box.append(&available_list);

        revealer.set_child(Some(&revealer_box));
        root.append(&revealer);

        // ── Wire up toggle ────────────────────────────────────────────────────
        {
            let revealer_c = revealer.clone();
            let toggle_btn_c = toggle_btn.clone();
            toggle_btn.connect_clicked(move |_| {
                let revealed = !revealer_c.reveals_child();
                revealer_c.set_reveal_child(revealed);
                if revealed {
                    toggle_btn_c.set_label("▾ Available Devices");
                } else {
                    toggle_btn_c.set_label("▸ Available Devices");
                }
            });
        }

        // ── Wire up scan button ───────────────────────────────────────────────
        let state = Rc::new(RefCell::new(State { scanning: false }));

        {
            let state_c = state.clone();
            let scan_spinner_c = scan_spinner.clone();
            let scan_btn_c = scan_btn.clone();
            let scan_status_c = scan_status_lbl.clone();
            let available_list_c = available_list.clone();

            scan_btn.connect_clicked(move |_| {
                let scanning = state_c.borrow().scanning;

                if scanning {
                    // "Stop Scan" pressed — stop immediately.
                    let _ = Command::new("bluetoothctl").args(["scan", "off"]).spawn();
                    scan_spinner_c.stop();
                    scan_spinner_c.set_visible(false);
                    scan_btn_c.set_label("Scan");
                    scan_status_c.set_label("");
                    state_c.borrow_mut().scanning = false;
                    populate_available_list(&available_list_c);
                    return;
                }

                state_c.borrow_mut().scanning = true;
                scan_spinner_c.set_visible(true);
                scan_spinner_c.start();
                scan_btn_c.set_label("Stop Scan");
                scan_status_c.set_label("Scanning.");

                let _ = Command::new("bluetoothctl").args(["scan", "on"]).spawn();

                // Animate dots and refresh list every 2 seconds; stop after 10 s (5 ticks).
                let state_tick = state_c.clone();
                let spinner_tick = scan_spinner_c.clone();
                let btn_tick = scan_btn_c.clone();
                let status_tick = scan_status_c.clone();
                let list_tick = available_list_c.clone();
                let tick_count = Rc::new(RefCell::new(0u8));

                glib::timeout_add_local(std::time::Duration::from_secs(2), move || {
                    if !state_tick.borrow().scanning {
                        return glib::ControlFlow::Break;
                    }

                    *tick_count.borrow_mut() += 1;
                    let ticks = *tick_count.borrow();

                    // Update animated dots (1–3, cycling).
                    let dots = ".".repeat(((ticks - 1) % 3 + 1) as usize);
                    status_tick.set_label(&format!("Scanning{dots}"));

                    // Refresh available list with newly found devices.
                    populate_available_list(&list_tick);

                    if ticks >= 5 {
                        // 10 seconds elapsed — stop.
                        let _ = Command::new("bluetoothctl").args(["scan", "off"]).spawn();
                        spinner_tick.stop();
                        spinner_tick.set_visible(false);
                        btn_tick.set_label("Scan");
                        status_tick.set_label("");
                        state_tick.borrow_mut().scanning = false;
                        return glib::ControlFlow::Break;
                    }

                    glib::ControlFlow::Continue
                });
            });
        }

        let section = Self {
            root,
            connected_list,
            available_list,
            revealer,
            scan_spinner,
            scan_btn,
            scan_status_lbl,
            state,
        };

        section.refresh();
        section
    }

    /// Re-query bluetoothctl and rebuild both device lists.
    pub fn refresh(&self) {
        // Verify Bluetooth is powered on before populating.
        if !bt_is_powered() {
            // Clear connected list and show "Bluetooth is off" message.
            while let Some(child) = self.connected_list.first_child() {
                self.connected_list.remove(&child);
            }
            while let Some(child) = self.available_list.first_child() {
                self.available_list.remove(&child);
            }

            // Show a single informational label in place of the scan button.
            self.scan_btn.set_visible(false);
            self.scan_status_lbl.set_label("Bluetooth is off");
            self.connected_list.set_visible(false);
            return;
        }

        self.scan_btn.set_visible(true);
        if !self.state.borrow().scanning {
            self.scan_status_lbl.set_label("");
        }

        // Clear existing rows.
        while let Some(child) = self.connected_list.first_child() {
            self.connected_list.remove(&child);
        }
        while let Some(child) = self.available_list.first_child() {
            self.available_list.remove(&child);
        }

        let macs = bt_list_macs();
        let mut has_connected = false;

        for mac in &macs {
            let Some(dev) = bt_info(mac) else { continue };

            if dev.connected {
                has_connected = true;
                self.connected_list
                    .append(&make_connected_row(&dev, &self.connected_list));
            }
        }

        self.connected_list.set_visible(has_connected);

        populate_available_list(&self.available_list);
    }

    /// Return a reference to the root widget for embedding in the panel.
    pub fn widget(&self) -> &Box {
        &self.root
    }
}

// ── Row builders ──────────────────────────────────────────────────────────────

/// Build a row for a connected device with a disconnect button and forget (×) button.
fn make_connected_row(dev: &BtDevice, parent_list: &Box) -> Box {
    let row = Box::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .hexpand(true)
        .build();
    row.add_css_class("device-row");
    row.add_css_class("device-row--connected");

    let icon_lbl = Label::builder()
        .label(device_icon(dev.icon_hint.as_deref()))
        .build();
    icon_lbl.add_css_class("device-icon");

    // Name + optional battery percentage.
    let name_text = match dev.battery {
        Some(pct) => format!("{} {}%", dev.name, pct),
        None => dev.name.clone(),
    };
    let name_lbl = Label::builder()
        .label(&name_text)
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .build();
    name_lbl.add_css_class("device-name");

    // Status label — used for spinner/feedback during disconnect.
    let status_lbl = Label::builder().label("").build();
    status_lbl.add_css_class("device-status");

    let spinner = Spinner::new();
    spinner.set_visible(false);

    let disconnect_btn = Button::with_label("Disconnect");
    disconnect_btn.add_css_class("device-action");

    let forget_btn = Button::with_label("×");
    forget_btn.add_css_class("device-forget");

    // ── Disconnect handler ────────────────────────────────────────────────────
    {
        let mac = dev.mac.clone();
        let row_c = row.clone();
        let parent_c = parent_list.clone();
        let spinner_c = spinner.clone();
        let disconnect_btn_c = disconnect_btn.clone();
        let forget_btn_c = forget_btn.clone();
        let status_c = status_lbl.clone();

        disconnect_btn.connect_clicked(move |btn| {
            btn.set_sensitive(false);
            forget_btn_c.set_sensitive(false);
            spinner_c.set_visible(true);
            spinner_c.start();
            status_c.set_label("");

            let mac_bg = mac.clone();
            let (tx, rx) = mpsc::channel::<ConnectResult>();

            std::thread::spawn(move || {
                let result = bt_disconnect_blocking(&mac_bg);
                let _ = tx.send(result);
            });

            // Poll the channel from the main loop.
            let row_poll = row_c.clone();
            let parent_poll = parent_c.clone();
            let spinner_poll = spinner_c.clone();
            let btn_poll = disconnect_btn_c.clone();
            let forget_poll = forget_btn_c.clone();
            let status_poll = status_c.clone();

            glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
                match rx.try_recv() {
                    Ok(ConnectResult::Success) => {
                        // Remove the row from the connected list.
                        parent_poll.remove(&row_poll);
                        glib::ControlFlow::Break
                    }
                    Ok(ConnectResult::Failure(reason)) => {
                        spinner_poll.stop();
                        spinner_poll.set_visible(false);
                        btn_poll.set_sensitive(true);
                        forget_poll.set_sensitive(true);
                        status_poll.set_label(&format!("Error: {reason}"));
                        status_poll.add_css_class("device-status--error");
                        glib::ControlFlow::Break
                    }
                    Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        spinner_poll.stop();
                        spinner_poll.set_visible(false);
                        btn_poll.set_sensitive(true);
                        forget_poll.set_sensitive(true);
                        glib::ControlFlow::Break
                    }
                }
            });
        });
    }

    // ── Forget handler ────────────────────────────────────────────────────────
    {
        let mac = dev.mac.clone();
        let row_c = row.clone();
        let parent_c = parent_list.clone();

        forget_btn.connect_clicked(move |_| {
            bt_forget(&mac);
            parent_c.remove(&row_c);
        });
    }

    row.append(&icon_lbl);
    row.append(&name_lbl);
    row.append(&spinner);
    row.append(&status_lbl);
    row.append(&disconnect_btn);
    row.append(&forget_btn);
    row
}

/// Build a row for an available (not connected) device with a connect button and forget (×).
fn make_available_row(dev: &BtDevice, parent_list: &Box) -> Box {
    let row = Box::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .hexpand(true)
        .build();
    row.add_css_class("device-row");
    row.add_css_class("device-row--available");

    let icon_lbl = Label::builder()
        .label(device_icon(dev.icon_hint.as_deref()))
        .build();
    icon_lbl.add_css_class("device-icon");

    let name_lbl = Label::builder()
        .label(&dev.name)
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .build();
    name_lbl.add_css_class("device-name");

    // Status label — shows spinner feedback and error/success messages.
    let status_lbl = Label::builder().label("").build();
    status_lbl.add_css_class("device-status");

    let spinner = Spinner::new();
    spinner.set_visible(false);

    let connect_btn = Button::with_label("Connect");
    connect_btn.add_css_class("device-action");

    let forget_btn = Button::with_label("×");
    forget_btn.add_css_class("device-forget");

    // ── Connect handler ───────────────────────────────────────────────────────
    {
        let mac = dev.mac.clone();
        let row_c = row.clone();
        let parent_c = parent_list.clone();
        let spinner_c = spinner.clone();
        let connect_btn_c = connect_btn.clone();
        let forget_btn_c = forget_btn.clone();
        let status_c = status_lbl.clone();

        connect_btn.connect_clicked(move |btn| {
            btn.set_sensitive(false);
            forget_btn_c.set_sensitive(false);
            spinner_c.set_visible(true);
            spinner_c.start();
            status_c.set_label("");
            status_c.remove_css_class("device-status--error");
            status_c.remove_css_class("device-status--success");

            let mac_bg = mac.clone();
            let (tx, rx) = mpsc::channel::<ConnectResult>();

            std::thread::spawn(move || {
                let result = bt_connect_blocking(&mac_bg);
                let _ = tx.send(result);
            });

            // Poll the channel from the main loop.
            let row_poll = row_c.clone();
            let parent_poll = parent_c.clone();
            let spinner_poll = spinner_c.clone();
            let btn_poll = connect_btn_c.clone();
            let forget_poll = forget_btn_c.clone();
            let status_poll = status_c.clone();

            glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
                match rx.try_recv() {
                    Ok(ConnectResult::Success) => {
                        spinner_poll.stop();
                        spinner_poll.set_visible(false);
                        status_poll.set_label("✓");
                        status_poll.add_css_class("device-status--success");

                        // Brief flash of the checkmark, then remove the row
                        // (caller's refresh() will add it to connected list).
                        let row_rm = row_poll.clone();
                        let parent_rm = parent_poll.clone();
                        glib::timeout_add_local_once(
                            std::time::Duration::from_millis(1200),
                            move || {
                                parent_rm.remove(&row_rm);
                            },
                        );
                        glib::ControlFlow::Break
                    }
                    Ok(ConnectResult::Failure(reason)) => {
                        spinner_poll.stop();
                        spinner_poll.set_visible(false);
                        btn_poll.set_sensitive(true);
                        forget_poll.set_sensitive(true);
                        status_poll.set_label(&format!("Connection failed: {reason}"));
                        status_poll.add_css_class("device-status--error");
                        glib::ControlFlow::Break
                    }
                    Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        spinner_poll.stop();
                        spinner_poll.set_visible(false);
                        btn_poll.set_sensitive(true);
                        forget_poll.set_sensitive(true);
                        glib::ControlFlow::Break
                    }
                }
            });
        });
    }

    // ── Forget handler ────────────────────────────────────────────────────────
    {
        let mac = dev.mac.clone();
        let row_c = row.clone();
        let parent_c = parent_list.clone();

        forget_btn.connect_clicked(move |_| {
            bt_forget(&mac);
            parent_c.remove(&row_c);
        });
    }

    row.append(&icon_lbl);
    row.append(&name_lbl);
    row.append(&spinner);
    row.append(&status_lbl);
    row.append(&connect_btn);
    row.append(&forget_btn);
    row
}

/// Clear `list` and repopulate it with every non-connected known device.
fn populate_available_list(list: &Box) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }

    for mac in bt_list_macs() {
        let Some(dev) = bt_info(&mac) else { continue };
        if !dev.connected {
            list.append(&make_available_row(&dev, list));
        }
    }
}
