use std::cell::RefCell;
use std::process::Command;

use gtk4::prelude::*;

pub struct HeaderSection {
    root: gtk4::Box,
    wifi_btn: RefCell<gtk4::Button>,
    bluetooth_btn: RefCell<gtk4::Button>,
    dnd_btn: RefCell<gtk4::Button>,
    night_btn: RefCell<gtk4::Button>,
    idle_btn: RefCell<gtk4::Button>,
    camera_btn: RefCell<gtk4::Button>,
    #[allow(dead_code)]
    color_btn: RefCell<gtk4::Button>,
}

impl HeaderSection {
    pub fn new() -> Self {
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
        let (color_toggle, color_btn) = make_toggle("󰏘", "Color");

        toggles_box2.append(&idle_toggle);
        toggles_box2.append(&camera_toggle);
        toggles_box2.append(&color_toggle);

        root.append(&toggles_box);
        root.append(&toggles_box2);

        // Wire up click handlers
        {
            let btn = wifi_btn.clone();
            wifi_btn.connect_clicked(move |_| {
                if btn.has_css_class("disabled") {
                    return;
                }
                let currently_active = btn.has_css_class("active");
                // Optimistic UI: flip immediately
                set_active(&btn, !currently_active);
                update_wifi_tooltip(&btn, !currently_active);

                let btn_clone = btn.clone();
                spawn_toggle_command(
                    move || {
                        let arg = if currently_active { "off" } else { "on" };
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
                            // Revert after 2 s
                            let b = btn_clone.clone();
                            glib::timeout_add_local_once(
                                std::time::Duration::from_secs(2),
                                move || {
                                    set_active(&b, currently_active);
                                    update_wifi_tooltip(&b, currently_active);
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
                if btn.has_css_class("disabled") {
                    return;
                }
                let currently_active = btn.has_css_class("active");
                set_active(&btn, !currently_active);
                update_bluetooth_tooltip(&btn, !currently_active);

                let btn_clone = btn.clone();
                spawn_toggle_command(
                    move || {
                        let arg = if currently_active { "off" } else { "on" };
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
                                    set_active(&b, currently_active);
                                    update_bluetooth_tooltip(&b, currently_active);
                                },
                            );
                        }
                    },
                );
            });
        }

        {
            let btn = dnd_btn.clone();
            dnd_btn.connect_clicked(move |_| {
                if btn.has_css_class("disabled") {
                    return;
                }
                let currently_active = btn.has_css_class("active");
                set_active(&btn, !currently_active);
                update_dnd_tooltip(&btn, !currently_active);

                let btn_clone = btn.clone();
                spawn_toggle_command(
                    move || {
                        let flag = if currently_active { "-r" } else { "-a" };
                        let result = Command::new("makoctl")
                            .args(["mode", flag, "do-not-disturb"])
                            .spawn()
                            .and_then(|mut c| c.wait());
                        result.map(|s| s.success()).unwrap_or_else(|e| {
                            if e.kind() == std::io::ErrorKind::NotFound {
                                log::warn!("makoctl not found");
                            } else {
                                log::warn!("makoctl mode {flag} do-not-disturb failed: {e}");
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
                                    set_active(&b, currently_active);
                                    update_dnd_tooltip(&b, currently_active);
                                },
                            );
                        }
                    },
                );
            });
        }

        {
            let btn = night_btn.clone();
            night_btn.connect_clicked(move |_| {
                if btn.has_css_class("disabled") {
                    return;
                }
                let currently_active = btn.has_css_class("active");
                set_active(&btn, !currently_active);
                update_night_tooltip(&btn, !currently_active);

                let btn_clone = btn.clone();
                spawn_toggle_command(
                    move || {
                        let action = if currently_active { "stop" } else { "start" };
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
                                    set_active(&b, currently_active);
                                    update_night_tooltip(&b, currently_active);
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
                if btn.has_css_class("disabled") {
                    return;
                }
                let currently_active = btn.has_css_class("active");
                set_active(&btn, !currently_active);
                update_idle_tooltip(&btn, !currently_active);

                let btn_clone = btn.clone();
                spawn_toggle_command(
                    move || {
                        if currently_active {
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
                        } else {
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
                        }
                    },
                    move |success| {
                        if !success {
                            let b = btn_clone.clone();
                            glib::timeout_add_local_once(
                                std::time::Duration::from_secs(2),
                                move || {
                                    set_active(&b, currently_active);
                                    update_idle_tooltip(&b, currently_active);
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
                if btn.has_css_class("disabled") {
                    return;
                }
                let currently_active = btn.has_css_class("active");
                set_active(&btn, !currently_active);
                update_camera_tooltip(&btn, !currently_active);

                let btn_clone = btn.clone();
                spawn_toggle_command(
                    move || {
                        let action = if currently_active { "stop" } else { "start" };
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
                                    set_active(&b, currently_active);
                                    update_camera_tooltip(&b, currently_active);
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
            color_btn: RefCell::new(color_btn),
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

        let (tx, rx) = std::sync::mpsc::channel::<(
            ToggleState,
            ToggleState,
            ToggleState,
            ToggleState,
            ToggleState,
            ToggleState,
        )>();

        std::thread::spawn(move || {
            let states = (
                read_wifi_state(),
                read_bluetooth_state(),
                read_dnd_state(),
                read_night_state(),
                read_idle_state(),
                read_camera_state(),
            );
            let _ = tx.send(states);
        });

        glib::idle_add_local_once(move || {
            // Poll until the thread finishes (usually instant)
            fn poll(
                rx: std::sync::mpsc::Receiver<(
                    ToggleState,
                    ToggleState,
                    ToggleState,
                    ToggleState,
                    ToggleState,
                    ToggleState,
                )>,
                wifi: gtk4::Button,
                bt: gtk4::Button,
                dnd: gtk4::Button,
                night: gtk4::Button,
                idle: gtk4::Button,
                camera: gtk4::Button,
            ) {
                match rx.try_recv() {
                    Ok((ws, bs, ds, ns, is, cs)) => {
                        apply_toggle_state(&wifi, ws);
                        apply_toggle_state(&bt, bs);
                        apply_toggle_state(&dnd, ds);
                        apply_toggle_state(&night, ns);
                        apply_toggle_state(&idle, is);
                        apply_toggle_state(&camera, cs);
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        glib::idle_add_local_once(move || {
                            poll(rx, wifi, bt, dnd, night, idle, camera)
                        });
                    }
                    Err(_) => {}
                }
            }
            poll(rx, wifi, bt, dnd, night, idle, camera);
        });
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

/// Build a vertical Box containing a toggle Button and a Label beneath it.
fn make_toggle(icon: &str, label_text: &str) -> (gtk4::Box, gtk4::Button) {
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

fn set_active(btn: &gtk4::Button, active: bool) {
    if active {
        btn.add_css_class("active");
        btn.remove_css_class("inactive");
    } else {
        btn.remove_css_class("active");
        btn.add_css_class("inactive");
    }
}

fn apply_toggle_state(btn: &gtk4::Button, state: ToggleState) {
    match state {
        ToggleState::Active => {
            btn.remove_css_class("disabled");
            btn.set_sensitive(true);
            set_active(btn, true);
        }
        ToggleState::Inactive => {
            btn.remove_css_class("disabled");
            btn.set_sensitive(true);
            set_active(btn, false);
        }
        ToggleState::Unavailable => {
            btn.add_css_class("disabled");
            btn.set_sensitive(false);
            btn.remove_css_class("active");
            btn.remove_css_class("inactive");
        }
    }
}

/// Spawn `work` on a background thread. When it finishes, call `on_done` on
/// the GTK main thread via `glib::idle_add_local_once`.
///
/// `on_done` may capture `!Send` GTK objects. It is never called from the
/// background thread — only from the GTK main thread inside idle callbacks.
fn spawn_toggle_command<W, D>(work: W, on_done: D)
where
    W: FnOnce() -> bool + Send + 'static,
    D: FnOnce(bool) + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel::<bool>();

    std::thread::spawn(move || {
        let success = work();
        let _ = tx.send(success);
    });

    glib::idle_add_local_once(move || {
        fn poll(rx: std::sync::mpsc::Receiver<bool>, on_done: impl FnOnce(bool) + 'static) {
            match rx.try_recv() {
                Ok(success) => on_done(success),
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    glib::idle_add_local_once(move || poll(rx, on_done));
                }
                Err(_) => {}
            }
        }
        poll(rx, on_done);
    });
}

// ---------------------------------------------------------------------------
// Tooltip updaters
// ---------------------------------------------------------------------------

fn update_wifi_tooltip(btn: &gtk4::Button, active: bool) {
    btn.set_tooltip_text(Some(if active {
        "Wi-Fi: enabled"
    } else {
        "Wi-Fi: disabled"
    }));
}

fn update_bluetooth_tooltip(btn: &gtk4::Button, active: bool) {
    btn.set_tooltip_text(Some(if active {
        "Bluetooth: powered on"
    } else {
        "Bluetooth: powered off"
    }));
}

fn update_dnd_tooltip(btn: &gtk4::Button, active: bool) {
    btn.set_tooltip_text(Some(if active {
        "Do Not Disturb: active"
    } else {
        "Do Not Disturb: off"
    }));
}

fn update_night_tooltip(btn: &gtk4::Button, active: bool) {
    btn.set_tooltip_text(Some(if active {
        "Night Light: active"
    } else {
        "Night Light: off"
    }));
}

fn update_idle_tooltip(btn: &gtk4::Button, active: bool) {
    btn.set_tooltip_text(Some(if active {
        "Idle Inhibitor: active"
    } else {
        "Idle Inhibitor: off"
    }));
}

fn update_camera_tooltip(btn: &gtk4::Button, active: bool) {
    btn.set_tooltip_text(Some(if active {
        "Camera: active"
    } else {
        "Camera: off"
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

fn read_dnd_state() -> ToggleState {
    match Command::new("makoctl").arg("mode").output() {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            log::warn!("makoctl not found; DND toggle disabled");
            ToggleState::Unavailable
        }
        Err(e) => {
            log::warn!("makoctl mode failed: {e}");
            ToggleState::Unavailable
        }
        Ok(out) => {
            let active = String::from_utf8_lossy(&out.stdout)
                .lines()
                .any(|l| l.trim() == "do-not-disturb");
            if active {
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
