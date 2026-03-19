use std::cell::RefCell;
use std::process::Command;
use std::rc::Rc;

use gtk4::prelude::*;

use crate::notifications::store::NotificationStore;
use crate::spawn;

pub struct HeaderSection {
    root: gtk4::Box,
    wifi_btn: RefCell<gtk4::ToggleButton>,
    bluetooth_btn: RefCell<gtk4::ToggleButton>,
    dnd_btn: RefCell<gtk4::ToggleButton>,
    night_btn: RefCell<gtk4::ToggleButton>,
    idle_btn: RefCell<gtk4::ToggleButton>,
    camera_btn: RefCell<gtk4::ToggleButton>,
    touchpad_btn: RefCell<gtk4::ToggleButton>,
    #[allow(dead_code)]
    color_btn: RefCell<gtk4::Button>,
    store: Rc<RefCell<NotificationStore>>,
}

impl HeaderSection {
    pub fn new(store: Rc<RefCell<NotificationStore>>) -> Self {
        let root = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .build();
        root.add_css_class("section");
        root.add_css_class("quick-toggles");

        // Row 1
        let toggles_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(8)
            .homogeneous(true)
            .build();

        let (wifi_toggle, wifi_btn) = make_toggle("󰤨", "Wi-Fi");
        let (bt_toggle, bluetooth_btn) = make_toggle("󰂯", "Bluetooth");
        let (dnd_toggle, dnd_btn) = make_toggle("󰍷", "DND");
        let (night_toggle, night_btn) = make_toggle("󰖔", "Night Light");

        toggles_box.append(&wifi_toggle);
        toggles_box.append(&bt_toggle);
        toggles_box.append(&dnd_toggle);
        toggles_box.append(&night_toggle);

        // Row 2
        let toggles_box2 = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(8)
            .homogeneous(true)
            .build();

        let (idle_toggle, idle_btn) = make_toggle("󰈈", "Idle");
        let (camera_toggle, camera_btn) = make_toggle("󰄀", "Camera");
        let (touchpad_toggle, touchpad_btn) = make_toggle("󰟸", "Touchpad");
        let (color_toggle, color_btn) = make_action_btn("󰏘", "Color");

        toggles_box2.append(&idle_toggle);
        toggles_box2.append(&camera_toggle);
        toggles_box2.append(&touchpad_toggle);
        toggles_box2.append(&color_toggle);

        root.append(&toggles_box);
        root.append(&toggles_box2);

        // Wire up click handlers
        {
            let btn = wifi_btn.clone();
            wifi_btn.connect_clicked(move |_| {
                let new_active = btn.is_active();
                update_wifi_tooltip(&btn, new_active);

                let btn_clone = btn.clone();
                spawn::spawn_work(
                    move || {
                        let arg = if new_active { "on" } else { "off" };
                        let result = Command::new("nmcli")
                            .args(["radio", "wifi", arg])
                            .spawn()
                            .and_then(|mut c| c.wait());
                        let success = result.map(|s| s.success()).unwrap_or_else(|e| {
                            if e.kind() == std::io::ErrorKind::NotFound {
                                log::warn!("nmcli not found");
                            } else {
                                log::warn!("nmcli radio wifi {arg} failed: {e}");
                            }
                            false
                        });
                        success
                    },
                    move |success| {
                        if !success {
                            let b = btn_clone.clone();
                            glib::timeout_add_local_once(
                                std::time::Duration::from_secs(2),
                                move || {
                                    b.set_active(!new_active);
                                    update_wifi_tooltip(&b, !new_active);
                                },
                            );
                        }
                    },
                );
            });
        }

        {
            let btn = bluetooth_btn.clone();
            bluetooth_btn.connect_clicked(move |_| {
                let new_active = btn.is_active();
                update_bluetooth_tooltip(&btn, new_active);

                let btn_clone = btn.clone();
                spawn::spawn_work(
                    move || {
                        let arg = if new_active { "on" } else { "off" };
                        let result = Command::new("bluetoothctl")
                            .args(["power", arg])
                            .spawn()
                            .and_then(|mut c| c.wait());
                        result.map(|s| s.success()).unwrap_or_else(|e| {
                            if e.kind() == std::io::ErrorKind::NotFound {
                                log::warn!("bluetoothctl not found");
                            } else {
                                log::warn!("bluetoothctl power {arg} failed: {e}");
                            }
                            false
                        })
                    },
                    move |success| {
                        if !success {
                            let b = btn_clone.clone();
                            glib::timeout_add_local_once(
                                std::time::Duration::from_secs(2),
                                move || {
                                    b.set_active(!new_active);
                                    update_bluetooth_tooltip(&b, !new_active);
                                },
                            );
                        }
                    },
                );
            });
        }

        {
            let btn = dnd_btn.clone();
            let store_dnd = store.clone();
            dnd_btn.connect_clicked(move |_| {
                let new_active = btn.is_active();
                update_dnd_tooltip(&btn, new_active);
                store_dnd.borrow_mut().set_dnd(new_active);
            });
        }

        {
            let btn = night_btn.clone();
            night_btn.connect_clicked(move |_| {
                let new_active = btn.is_active();
                update_night_tooltip(&btn, new_active);

                let btn_clone = btn.clone();
                spawn::spawn_work(
                    move || {
                        let action = if new_active { "start" } else { "stop" };
                        let result = Command::new("systemctl")
                            .args(["--user", action, "gammastep.service"])
                            .spawn()
                            .and_then(|mut c| c.wait());
                        result.map(|s| s.success()).unwrap_or_else(|e| {
                            log::warn!("systemctl --user {action} gammastep.service failed: {e}");
                            false
                        })
                    },
                    move |success| {
                        if !success {
                            let b = btn_clone.clone();
                            glib::timeout_add_local_once(
                                std::time::Duration::from_secs(2),
                                move || {
                                    b.set_active(!new_active);
                                    update_night_tooltip(&b, !new_active);
                                },
                            );
                        }
                    },
                );
            });
        }

        {
            let btn = idle_btn.clone();
            idle_btn.connect_clicked(move |_| {
                let new_active = btn.is_active();
                update_idle_tooltip(&btn, new_active);

                let btn_clone = btn.clone();
                spawn::spawn_work(
                    move || {
                        if new_active {
                            // Toggle on: spawn systemd-inhibit in background, save PID
                            match Command::new("systemd-inhibit")
                                .args([
                                    "--what=idle:sleep",
                                    "--who=swaypplet",
                                    "--why=User toggled",
                                    "sleep",
                                    "infinity",
                                ])
                                .spawn()
                            {
                                Ok(child) => {
                                    let pid = child.id();
                                    match std::fs::write(
                                        "/tmp/swaypplet-idle-inhibit.pid",
                                        pid.to_string(),
                                    ) {
                                        Ok(_) => true,
                                        Err(e) => {
                                            log::warn!("Failed to write idle inhibit PID file: {e}");
                                            false
                                        }
                                    }
                                }
                                Err(e) => {
                                    if e.kind() == std::io::ErrorKind::NotFound {
                                        log::warn!("systemd-inhibit not found");
                                    } else {
                                        log::warn!("systemd-inhibit spawn failed: {e}");
                                    }
                                    false
                                }
                            }
                        } else {
                            // Toggle off: read PID, kill process, remove file
                            const PID_FILE: &str = "/tmp/swaypplet-idle-inhibit.pid";
                            match std::fs::read_to_string(PID_FILE) {
                                Ok(contents) => {
                                    let pid_str = contents.trim().to_string();
                                    match pid_str.parse::<u32>() {
                                        Ok(pid) => {
                                            let killed = Command::new("kill")
                                                .arg(pid.to_string())
                                                .spawn()
                                                .and_then(|mut c| c.wait())
                                                .map(|s| s.success())
                                                .unwrap_or(false);
                                            if killed {
                                                let _ = std::fs::remove_file(PID_FILE);
                                            } else {
                                                log::warn!("Failed to kill idle inhibit process (pid {pid})");
                                            }
                                            killed
                                        }
                                        Err(e) => {
                                            log::warn!("Failed to parse idle inhibit PID: {e}");
                                            false
                                        }
                                    }
                                }
                                Err(e) => {
                                    log::warn!("Failed to read idle inhibit PID file: {e}");
                                    false
                                }
                            }
                        }
                    },
                    move |success| {
                        if !success {
                            let b = btn_clone.clone();
                            glib::timeout_add_local_once(
                                std::time::Duration::from_secs(2),
                                move || {
                                    b.set_active(!new_active);
                                    update_idle_tooltip(&b, !new_active);
                                },
                            );
                        }
                    },
                );
            });
        }

        {
            let btn = camera_btn.clone();
            camera_btn.connect_clicked(move |_| {
                let new_active = btn.is_active();
                update_camera_tooltip(&btn, new_active);

                let btn_clone = btn.clone();
                spawn::spawn_work(
                    move || {
                        let action = if new_active { "start" } else { "stop" };
                        let result = Command::new("systemctl")
                            .args(["--user", action, "icamerasrc-v4l2loopback.service"])
                            .spawn()
                            .and_then(|mut c| c.wait());
                        result.map(|s| s.success()).unwrap_or_else(|e| {
                            if e.kind() == std::io::ErrorKind::NotFound {
                                log::warn!("systemctl not found");
                            } else {
                                log::warn!(
                                    "systemctl --user {action} icamerasrc-v4l2loopback.service failed: {e}"
                                );
                            }
                            false
                        })
                    },
                    move |success| {
                        if !success {
                            let b = btn_clone.clone();
                            glib::timeout_add_local_once(
                                std::time::Duration::from_secs(2),
                                move || {
                                    b.set_active(!new_active);
                                    update_camera_tooltip(&b, !new_active);
                                },
                            );
                        }
                    },
                );
            });
        }

        {
            let btn = touchpad_btn.clone();
            touchpad_btn.connect_clicked(move |_| {
                let new_active = btn.is_active();
                update_touchpad_tooltip(&btn, new_active);

                let btn_clone = btn.clone();
                spawn::spawn_work(
                    move || {
                        // enabled = touchpad on (active), disabled = touchpad off
                        let events = if new_active { "enabled" } else { "disabled" };
                        let result = Command::new("swaymsg")
                            .args(["input", "type:touchpad", "events", events])
                            .spawn()
                            .and_then(|mut c| c.wait());
                        result.map(|s| s.success()).unwrap_or_else(|e| {
                            log::warn!("swaymsg input type:touchpad events {events} failed: {e}");
                            false
                        })
                    },
                    move |success| {
                        if !success {
                            let b = btn_clone.clone();
                            glib::timeout_add_local_once(
                                std::time::Duration::from_secs(2),
                                move || {
                                    b.set_active(!new_active);
                                    update_touchpad_tooltip(&b, !new_active);
                                },
                            );
                        }
                    },
                );
            });
        }

        {
            // Color Picker: one-shot action, no toggle state
            color_btn.connect_clicked(move |_| {
                match Command::new("hyprpicker").arg("-a").spawn() {
                    Ok(_) => {}
                    Err(e) => {
                        if e.kind() == std::io::ErrorKind::NotFound {
                            log::warn!("hyprpicker not found");
                        } else {
                            log::warn!("hyprpicker -a failed: {e}");
                        }
                    }
                }
            });
        }

        let section = Self {
            root,
            wifi_btn: RefCell::new(wifi_btn),
            bluetooth_btn: RefCell::new(bluetooth_btn),
            dnd_btn: RefCell::new(dnd_btn),
            night_btn: RefCell::new(night_btn),
            idle_btn: RefCell::new(idle_btn),
            camera_btn: RefCell::new(camera_btn),
            touchpad_btn: RefCell::new(touchpad_btn),
            color_btn: RefCell::new(color_btn),
            store,
        };

        // Initialise states off the main thread, then apply results on it
        section.schedule_state_read();

        section
    }

    fn schedule_state_read(&self) {
        let wifi = self.wifi_btn.borrow().clone();
        let bt = self.bluetooth_btn.borrow().clone();
        let dnd = self.dnd_btn.borrow().clone();
        let night = self.night_btn.borrow().clone();
        let idle = self.idle_btn.borrow().clone();
        let camera = self.camera_btn.borrow().clone();
        let touchpad = self.touchpad_btn.borrow().clone();
        let store = self.store.clone();

        // DND state comes from our own store (main thread), read it now
        let dnd_active = store.borrow().is_dnd();
        apply_toggle_state(&dnd, if dnd_active { ToggleState::Active } else { ToggleState::Inactive });
        update_dnd_tooltip(&dnd, dnd_active);

        // Other states require blocking I/O — read on a background thread
        spawn::spawn_work(
            move || {
                (
                    read_wifi_state(),
                    read_bluetooth_state(),
                    read_night_state(),
                    read_idle_state(),
                    read_camera_state(),
                    read_touchpad_state(),
                )
            },
            move |(ws, bs, ns, is, cs, ts)| {
                apply_toggle_state(&wifi, ws);
                update_wifi_tooltip(&wifi, matches!(ws, ToggleState::Active));
                apply_toggle_state(&bt, bs);
                update_bluetooth_tooltip(&bt, matches!(bs, ToggleState::Active));
                apply_toggle_state(&night, ns);
                update_night_tooltip(&night, matches!(ns, ToggleState::Active));
                apply_toggle_state(&idle, is);
                update_idle_tooltip(&idle, matches!(is, ToggleState::Active));
                apply_toggle_state(&camera, cs);
                update_camera_tooltip(&camera, matches!(cs, ToggleState::Active));
                apply_toggle_state(&touchpad, ts);
                update_touchpad_tooltip(&touchpad, matches!(ts, ToggleState::Active));
            },
        );
    }

    pub fn refresh(&self) {
        self.schedule_state_read();
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }
}

// ---------------------------------------------------------------------------
// Toggle state type
// ---------------------------------------------------------------------------

/// Result of reading an external tool's state. `Unavailable` means the tool
/// wasn't found or failed — the toggle is shown as disabled.
#[derive(Clone, Copy)]
enum ToggleState {
    Active,
    Inactive,
    Unavailable,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a vertical Box containing a ToggleButton and a Label beneath it.
fn make_toggle(icon: &str, label_text: &str) -> (gtk4::Box, gtk4::ToggleButton) {
    let vbox = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .build();

    let btn = gtk4::ToggleButton::builder().label(icon).build();
    btn.add_css_class("toggle-btn");

    let label = gtk4::Label::builder().label(label_text).build();
    label.add_css_class("toggle-label");

    vbox.append(&btn);
    vbox.append(&label);

    // Sync CSS classes with toggle state for styling
    btn.connect_toggled(|btn| {
        if btn.is_active() {
            btn.add_css_class("active");
        } else {
            btn.remove_css_class("active");
        }
        if let Some(parent) = btn.parent() {
            if btn.is_active() {
                parent.add_css_class("toggle-on");
            } else {
                parent.remove_css_class("toggle-on");
            }
        }
    });

    (vbox, btn)
}

/// Build a vertical Box containing a regular Button and a Label beneath it
/// (for one-shot actions like color picker).
fn make_action_btn(icon: &str, label_text: &str) -> (gtk4::Box, gtk4::Button) {
    let vbox = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .build();

    let btn = gtk4::Button::builder().label(icon).build();
    btn.add_css_class("toggle-btn");

    let label = gtk4::Label::builder().label(label_text).build();
    label.add_css_class("toggle-label");

    vbox.append(&btn);
    vbox.append(&label);

    (vbox, btn)
}

fn apply_toggle_state(btn: &gtk4::ToggleButton, state: ToggleState) {
    match state {
        ToggleState::Active => {
            btn.set_sensitive(true);
            btn.set_active(true);
        }
        ToggleState::Inactive => {
            btn.set_sensitive(true);
            btn.set_active(false);
        }
        ToggleState::Unavailable => {
            btn.set_sensitive(false);
            btn.set_active(false);
        }
    }
}


// ---------------------------------------------------------------------------
// Tooltip updaters
// ---------------------------------------------------------------------------

fn update_wifi_tooltip(btn: &gtk4::ToggleButton, active: bool) {
    btn.set_tooltip_text(Some(if active {
        "Wi-Fi: enabled"
    } else {
        "Wi-Fi: disabled"
    }));
}

fn update_bluetooth_tooltip(btn: &gtk4::ToggleButton, active: bool) {
    btn.set_tooltip_text(Some(if active {
        "Bluetooth: powered on"
    } else {
        "Bluetooth: powered off"
    }));
}

fn update_dnd_tooltip(btn: &gtk4::ToggleButton, active: bool) {
    btn.set_tooltip_text(Some(if active {
        "Do Not Disturb: active"
    } else {
        "Do Not Disturb: off"
    }));
}

fn update_night_tooltip(btn: &gtk4::ToggleButton, active: bool) {
    btn.set_tooltip_text(Some(if active {
        "Night Light: active"
    } else {
        "Night Light: off"
    }));
}

fn update_idle_tooltip(btn: &gtk4::ToggleButton, active: bool) {
    btn.set_tooltip_text(Some(if active {
        "Idle Inhibitor: active"
    } else {
        "Idle Inhibitor: off"
    }));
}

fn update_camera_tooltip(btn: &gtk4::ToggleButton, active: bool) {
    btn.set_tooltip_text(Some(if active {
        "Camera: active"
    } else {
        "Camera: off"
    }));
}

fn update_touchpad_tooltip(btn: &gtk4::ToggleButton, active: bool) {
    btn.set_tooltip_text(Some(if active {
        "Touchpad: enabled"
    } else {
        "Touchpad: disabled"
    }));
}

// ---------------------------------------------------------------------------
// State readers (blocking — always call from a background thread)
// ---------------------------------------------------------------------------

fn read_wifi_state() -> ToggleState {
    match Command::new("nmcli").args(["radio", "wifi"]).output() {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            log::warn!("nmcli not found; Wi-Fi toggle disabled");
            ToggleState::Unavailable
        }
        Err(e) => {
            log::warn!("nmcli radio wifi failed: {e}");
            ToggleState::Unavailable
        }
        Ok(out) => {
            let enabled =
                String::from_utf8_lossy(&out.stdout).trim().to_lowercase() == "enabled";
            if enabled {
                ToggleState::Active
            } else {
                ToggleState::Inactive
            }
        }
    }
}

fn read_bluetooth_state() -> ToggleState {
    match Command::new("bluetoothctl").arg("show").output() {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            log::warn!("bluetoothctl not found; Bluetooth toggle disabled");
            ToggleState::Unavailable
        }
        Err(e) => {
            log::warn!("bluetoothctl show failed: {e}");
            ToggleState::Unavailable
        }
        Ok(out) => {
            let powered = String::from_utf8_lossy(&out.stdout)
                .lines()
                .any(|l| l.trim().eq_ignore_ascii_case("Powered: yes"));
            if powered {
                ToggleState::Active
            } else {
                ToggleState::Inactive
            }
        }
    }
}

fn read_night_state() -> ToggleState {
    match Command::new("systemctl")
        .args(["--user", "is-active", "gammastep.service"])
        .output()
    {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            log::warn!("systemctl not found; Night Light toggle disabled");
            ToggleState::Unavailable
        }
        Err(e) => {
            log::warn!("systemctl --user is-active gammastep.service failed: {e}");
            ToggleState::Unavailable
        }
        Ok(out) => {
            let status = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if status == "active" {
                ToggleState::Active
            } else {
                ToggleState::Inactive
            }
        }
    }
}

fn read_idle_state() -> ToggleState {
    const PID_FILE: &str = "/tmp/swaypplet-idle-inhibit.pid";
    match std::fs::read_to_string(PID_FILE) {
        Ok(contents) => {
            let pid_str = contents.trim().to_string();
            match pid_str.parse::<u32>() {
                Ok(pid) => {
                    // Check if process is alive via /proc/{pid}
                    let alive = std::path::Path::new(&format!("/proc/{pid}")).exists();
                    if alive {
                        ToggleState::Active
                    } else {
                        // Stale PID file — clean it up
                        let _ = std::fs::remove_file(PID_FILE);
                        ToggleState::Inactive
                    }
                }
                Err(_) => {
                    let _ = std::fs::remove_file(PID_FILE);
                    ToggleState::Inactive
                }
            }
        }
        Err(_) => ToggleState::Inactive,
    }
}

fn read_touchpad_state() -> ToggleState {
    // Query sway for touchpad events status via `swaymsg -t get_inputs --raw`
    match Command::new("swaymsg")
        .args(["-t", "get_inputs", "--raw"])
        .output()
    {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            log::warn!("swaymsg not found; Touchpad toggle disabled");
            ToggleState::Unavailable
        }
        Err(e) => {
            log::warn!("swaymsg -t get_inputs failed: {e}");
            ToggleState::Unavailable
        }
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout);
            // Look for a touchpad input and check its send_events status
            // Each input object has "type": "touchpad" and "send_events": "enabled"/"disabled"
            let mut found_touchpad = false;
            let mut in_touchpad = false;
            for line in text.lines() {
                let trimmed = line.trim();
                if trimmed.contains("\"type\"") && trimmed.contains("\"touchpad\"") {
                    in_touchpad = true;
                    found_touchpad = true;
                }
                if in_touchpad && trimmed.contains("\"send_events\"") {
                    if trimmed.contains("\"disabled\"") {
                        return ToggleState::Inactive;
                    } else {
                        return ToggleState::Active;
                    }
                }
                // Reset when hitting next input block
                if in_touchpad && trimmed == "}," {
                    in_touchpad = false;
                }
            }
            if found_touchpad {
                ToggleState::Active // default: enabled
            } else {
                ToggleState::Unavailable // no touchpad found
            }
        }
    }
}

fn read_camera_state() -> ToggleState {
    match Command::new("systemctl")
        .args(["--user", "is-active", "icamerasrc-v4l2loopback.service"])
        .output()
    {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            log::warn!("systemctl not found; Camera toggle disabled");
            ToggleState::Unavailable
        }
        Err(e) => {
            log::warn!(
                "systemctl --user is-active icamerasrc-v4l2loopback.service failed: {e}"
            );
            ToggleState::Unavailable
        }
        Ok(out) => {
            let status = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if status == "active" {
                ToggleState::Active
            } else {
                ToggleState::Inactive
            }
        }
    }
}
