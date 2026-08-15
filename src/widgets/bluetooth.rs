use std::cell::RefCell;

use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{Box, Button, Label, Orientation, Revealer, RevealerTransitionType, Spinner};

use crate::icons;
use crate::spawn::spawn_work;
use crate::widgets::bluez;

// ── Nerd Font icons ───────────────────────────────────────────────────────────
const ICON_HEADPHONES: &str = "󰋋";
const ICON_KEYBOARD: &str = "󰌌";
const ICON_MOUSE: &str = "󰍽";
const ICON_PHONE: &str = "󰏲";
const ICON_BLUETOOTH: &str = "󰂯";
const ICON_BLUETOOTH_OFF: &str = "󰂲";

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
//
// Thin wrappers over `bluez`, which does the D-Bus work. They stay as named
// functions because the shapes they return (`ConnectResult`, `BtDevice`)
// belong to this section rather than to the bus.

impl From<bluez::Device> for BtDevice {
    fn from(device: bluez::Device) -> Self {
        BtDevice {
            mac: device.mac,
            name: device.name,
            icon_hint: device.icon_hint,
            connected: device.connected,
            battery: device.battery,
        }
    }
}

impl From<Result<(), String>> for ConnectResult {
    fn from(result: Result<(), String>) -> Self {
        match result {
            Ok(()) => ConnectResult::Success,
            Err(message) => ConnectResult::Failure(message),
        }
    }
}

/// Connect in the calling thread (blocking).
///
/// Success is the method returning. The old version decided by looking for
/// "Connection successful" in `bluetoothctl`'s output, which made the answer
/// depend on the wording of a message.
fn bt_connect_blocking(mac: &str) -> ConnectResult {
    bluez::connect(mac).into()
}

fn bt_disconnect_blocking(mac: &str) -> ConnectResult {
    bluez::disconnect(mac).into()
}

/// Unpair a device.
fn bt_forget(mac: &str) {
    if let Err(e) = bluez::forget(mac) {
        log::warn!("bluetooth: could not forget {mac}: {e}");
    }
}

/// Start or stop scanning. `InProgress` means the adapter is already in the
/// state being asked for, which is not worth a log line.
fn bt_set_discovery(on: bool) {
    if let Err(e) = bluez::set_discovery(on)
        && !e.contains("InProgress")
        && !e.contains("progress")
    {
        log::debug!("bluetooth: discovery {on}: {e}");
    }
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
        Some(h) if h.contains("computer") || h.contains("laptop") => icons::DISPLAY,
        _ => ICON_BLUETOOTH,
    }
}

// ── Background state fetch ───────────────────────────────────────────────────

/// All Bluetooth state fetched from a background thread.
struct BluetoothState {
    available: bool,
    powered: bool,
    devices: Vec<BtDevice>,
}

/// Fetch all Bluetooth state in one D-Bus call.
/// Must be called from a background thread — never the GTK main thread.
fn read_bt_state_blocking() -> BluetoothState {
    let snapshot = bluez::snapshot();
    BluetoothState {
        available: snapshot.available,
        powered: snapshot.powered,
        // Paired devices only: the section's two lists are "connected" and
        // "available to connect", and an unpaired stranger seen mid-scan
        // belongs to neither until the scan sheet asks for it.
        devices: snapshot
            .devices
            .into_iter()
            .filter(|d| d.paired)
            .map(BtDevice::from)
            .collect(),
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
    summary_btn: Button,
    summary_icon: Label,
    summary_text: Label,
    summary_arrow: Label,
    detail_revealer: Revealer,
    connected_list: Box,
    available_list: Box,
    revealer: Revealer,
    scan_spinner: Spinner,
    scan_btn: Button,
    scan_status_lbl: Label,
    state: Rc<RefCell<State>>,
}

impl BluetoothSection {
    pub fn new() -> Rc<Self> {
        // ── Root section box ──────────────────────────────────────────────────
        let root = Box::builder()
            .orientation(Orientation::Vertical)
            .spacing(4)
            .build();
        root.add_css_class("section");

        // ── Summary row (always visible) ──────────────────────────────────────
        let summary_content = Box::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .hexpand(true)
            .build();

        let summary_icon = Label::builder().label(ICON_BLUETOOTH).build();
        summary_icon.add_css_class("section-summary-icon");

        let summary_text = Label::builder()
            .label("")
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .build();
        summary_text.add_css_class("section-summary-label");

        let summary_arrow = Label::builder().label("▸").build();
        summary_arrow.add_css_class("section-expand-arrow");

        summary_content.append(&summary_icon);
        summary_content.append(&summary_text);
        summary_content.append(&summary_arrow);

        let summary_btn = Button::builder().child(&summary_content).build();
        summary_btn.add_css_class("section-summary");
        root.append(&summary_btn);

        // ── Detail revealer ───────────────────────────────────────────────────
        let detail_revealer = Revealer::builder()
            .transition_type(RevealerTransitionType::SlideDown)
            .transition_duration(200)
            .reveal_child(false)
            .build();

        // ── Detail content box (lives inside detail_revealer) ─────────────────
        let detail_box = Box::builder()
            .orientation(Orientation::Vertical)
            .spacing(4)
            .build();

        // ── Connected devices list ────────────────────────────────────────────
        let connected_list = Box::builder()
            .orientation(Orientation::Vertical)
            .spacing(2)
            .build();
        connected_list.add_css_class("device-list");
        detail_box.append(&connected_list);

        // ── Revealer toggle button (available devices) ────────────────────────
        let toggle_btn = Button::builder()
            .label("▸ Available Devices")
            .hexpand(true)
            .build();
        toggle_btn.add_css_class("section-expander");
        detail_box.append(&toggle_btn);

        // ── Revealer content (available devices) ──────────────────────────────
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

        // ── Advanced Bluetooth Settings launcher (blueman / bluetoothctl) ───
        let adv_btn = Button::builder()
            .label("󰂯  Advanced Bluetooth Manager (blueman / bluetoothctl)")
            .halign(gtk4::Align::Fill)
            .build();
        adv_btn.add_css_class("network-adv-btn");
        adv_btn.connect_clicked(|_| {
            let _ = std::process::Command::new("blueman-manager")
                .spawn()
                .or_else(|_| {
                    std::process::Command::new("ghostty")
                        .args(["-e", "bluetoothctl"])
                        .spawn()
                })
                .or_else(|_| {
                    std::process::Command::new("foot")
                        .args(["-e", "bluetoothctl"])
                        .spawn()
                });
        });
        revealer_box.append(&adv_btn);

        revealer.set_child(Some(&revealer_box));
        detail_box.append(&revealer);

        detail_revealer.set_child(Some(&detail_box));
        root.append(&detail_revealer);

        // ── Wire up summary row toggle ────────────────────────────────────────
        {
            let detail_revealer_c = detail_revealer.clone();
            let summary_arrow_c = summary_arrow.clone();
            summary_btn.connect_clicked(move |_| {
                let revealed = !detail_revealer_c.reveals_child();
                detail_revealer_c.set_reveal_child(revealed);
                summary_arrow_c.set_label(if revealed { "▾" } else { "▸" });
            });
        }

        // ── Wire up available-devices toggle ──────────────────────────────────
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
                    bt_set_discovery(false);
                    scan_spinner_c.stop();
                    scan_spinner_c.set_visible(false);
                    scan_btn_c.set_label("Scan");
                    scan_status_c.set_label("");
                    state_c.borrow_mut().scanning = false;
                    schedule_populate_available(&available_list_c);
                    return;
                }

                state_c.borrow_mut().scanning = true;
                scan_spinner_c.set_visible(true);
                scan_spinner_c.start();
                scan_btn_c.set_label("Stop Scan");
                scan_status_c.set_label("Scanning.");

                bt_set_discovery(true);

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
                    schedule_populate_available(&list_tick);

                    if ticks >= 5 {
                        // 10 seconds elapsed — stop.
                        bt_set_discovery(false);
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

        let section = Rc::new(Self {
            root,
            summary_btn,
            summary_icon,
            summary_text,
            summary_arrow,
            detail_revealer,
            connected_list,
            available_list,
            revealer,
            scan_spinner,
            scan_btn,
            scan_status_lbl,
            state,
        });

        section.refresh();
        section
    }

    /// Schedule an async refresh: fetch Bluetooth state on a background thread,
    /// then apply the result on the GTK main thread.
    pub fn refresh(self: &Rc<Self>) {
        let section = Rc::clone(self);
        spawn_work(read_bt_state_blocking, move |state| {
            section.apply_state(state);
        });
    }

    /// Apply pre-fetched Bluetooth state to the UI (runs on the main thread).
    fn apply_state(&self, state: BluetoothState) {
        if !state.available {
            self.summary_icon.set_label(ICON_BLUETOOTH_OFF);
            self.summary_text.set_label("Unavailable");
            return;
        }

        if !state.powered {
            // Clear both lists.
            clear_box(&self.connected_list);
            clear_box(&self.available_list);

            self.scan_btn.set_visible(false);
            self.scan_status_lbl.set_label("Bluetooth is off");
            self.connected_list.set_visible(false);

            self.summary_icon.set_label(ICON_BLUETOOTH_OFF);
            self.summary_text.set_label("Bluetooth off");
            return;
        }

        self.scan_btn.set_visible(true);
        if !self.state.borrow().scanning {
            self.scan_status_lbl.set_label("");
        }

        // Clear existing rows.
        clear_box(&self.connected_list);
        clear_box(&self.available_list);

        let mut connected_devices: Vec<&BtDevice> = Vec::new();

        for dev in &state.devices {
            if dev.connected {
                connected_devices.push(dev);
                self.connected_list
                    .append(&make_connected_row(dev, &self.connected_list));
            } else {
                self.available_list
                    .append(&make_available_row(dev, &self.available_list));
            }
        }

        let has_connected = !connected_devices.is_empty();
        self.connected_list.set_visible(has_connected);

        // Update summary row.
        self.summary_icon.set_label(ICON_BLUETOOTH);
        match connected_devices.len() {
            0 => {
                self.summary_text.set_label("No devices");
            }
            1 => {
                let dev = connected_devices[0];
                let text = match dev.battery {
                    Some(pct) => format!("{} {}%", dev.name, pct),
                    None => dev.name.clone(),
                };
                self.summary_text.set_label(&text);
            }
            n => {
                self.summary_text
                    .set_label(&format!("{n} devices connected"));
            }
        }
    }

    pub fn expand_for_page(&self) {
        self.summary_btn.set_visible(false);
        self.detail_revealer.set_reveal_child(true);
        self.revealer.set_reveal_child(true);
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
    row.add_css_class("connected");

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
            let row_poll = row_c.clone();
            let parent_poll = parent_c.clone();
            let spinner_poll = spinner_c.clone();
            let btn_poll = disconnect_btn_c.clone();
            let forget_poll = forget_btn_c.clone();
            let status_poll = status_c.clone();

            spawn_work(
                move || bt_disconnect_blocking(&mac_bg),
                move |result| match result {
                    ConnectResult::Success => {
                        // Remove the row from the connected list.
                        parent_poll.remove(&row_poll);
                    }
                    ConnectResult::Failure(reason) => {
                        spinner_poll.stop();
                        spinner_poll.set_visible(false);
                        btn_poll.set_sensitive(true);
                        forget_poll.set_sensitive(true);
                        status_poll.set_label(&format!("Error: {reason}"));
                        status_poll.add_css_class("error");
                    }
                },
            );
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
    row.add_css_class("available");

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
            status_c.remove_css_class("error");
            status_c.remove_css_class("success");

            let mac_bg = mac.clone();
            let row_poll = row_c.clone();
            let parent_poll = parent_c.clone();
            let spinner_poll = spinner_c.clone();
            let btn_poll = connect_btn_c.clone();
            let forget_poll = forget_btn_c.clone();
            let status_poll = status_c.clone();

            spawn_work(
                move || bt_connect_blocking(&mac_bg),
                move |result| match result {
                    ConnectResult::Success => {
                        spinner_poll.stop();
                        spinner_poll.set_visible(false);
                        status_poll.set_label("✓");
                        status_poll.add_css_class("success");

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
                    }
                    ConnectResult::Failure(reason) => {
                        spinner_poll.stop();
                        spinner_poll.set_visible(false);
                        btn_poll.set_sensitive(true);
                        forget_poll.set_sensitive(true);
                        status_poll.set_label(&format!("Connection failed: {reason}"));
                        status_poll.add_css_class("error");
                    }
                },
            );
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

/// Remove all children from a GTK Box.
fn clear_box(b: &Box) {
    while let Some(child) = b.first_child() {
        b.remove(&child);
    }
}

/// Re-fetch available devices on a background thread and repopulate the list.
fn schedule_populate_available(list: &Box) {
    let list_c = list.clone();
    spawn_work(
        || {
            // Everything the adapter can see and is not already using —
            // paired or not, because this list is what a scan is for.
            bluez::snapshot()
                .devices
                .into_iter()
                .filter(|d| !d.connected)
                .map(BtDevice::from)
                .collect::<Vec<_>>()
        },
        move |devices| {
            clear_box(&list_c);
            for dev in &devices {
                list_c.append(&make_available_row(dev, &list_c));
            }
        },
    );
}
