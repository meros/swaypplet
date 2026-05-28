//! Declarative quick-settings toggle tiles for the start-menu quick strip.
//!
//! Each tile is described once in [`tile_specs`]; a single [`build_tile`]
//! factory turns a spec into an optimistic toggle button with revert-on-failure
//! and the `loading` CSS class. This replaces the per-toggle copy-pasted
//! click-handler boilerplate that used to live in `header.rs`.

use std::cell::RefCell;
use std::process::Command;
use std::rc::Rc;

use gtk4::prelude::*;

use crate::notifications::store::NotificationStore;
use crate::spawn;

/// Result of reading an external tool's state. `Unavailable` means the tool
/// wasn't found or failed — the tile is shown disabled.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TileState {
    Active,
    Inactive,
    Unavailable,
}

/// One tile: icon glyph, label, on/off tooltips, the async on/off action and
/// the state reader. `action` and `read_state` run on a background thread.
pub struct TileSpec {
    pub icon: &'static str,
    pub label: &'static str,
    pub tooltip_on: &'static str,
    pub tooltip_off: &'static str,
    /// Perform the toggle for the requested target state. Returns success.
    /// Runs on a background thread (blocking I/O allowed).
    pub action: fn(bool) -> bool,
    /// Read current state. Runs on a background thread.
    pub read_state: fn() -> TileState,
}

/// The declarative tile set for the quick strip: Wi-Fi, Bluetooth, DND,
/// Night Light, Idle. DND is store-backed and handled specially by the panel;
/// the rest drive external tools.
pub fn tile_specs() -> Vec<TileSpec> {
    vec![
        TileSpec {
            icon: "󰤨",
            label: "Wi-Fi",
            tooltip_on: "Wi-Fi: enabled",
            tooltip_off: "Wi-Fi: disabled",
            action: |on| {
                run_ok(Command::new("nmcli").args(["radio", "wifi", if on { "on" } else { "off" }]))
            },
            read_state: read_wifi_state,
        },
        TileSpec {
            icon: "󰂯",
            label: "Bluetooth",
            tooltip_on: "Bluetooth: powered on",
            tooltip_off: "Bluetooth: powered off",
            action: |on| {
                run_ok(Command::new("bluetoothctl").args(["power", if on { "on" } else { "off" }]))
            },
            read_state: read_bluetooth_state,
        },
        TileSpec {
            icon: "󰖔",
            label: "Night Light",
            tooltip_on: "Night Light: active",
            tooltip_off: "Night Light: off",
            action: |on| {
                run_ok(Command::new("systemctl").args([
                    "--user",
                    if on { "start" } else { "stop" },
                    "gammastep.service",
                ]))
            },
            read_state: read_night_state,
        },
        TileSpec {
            icon: "󰈈",
            label: "Idle",
            tooltip_on: "Idle Inhibitor: active",
            tooltip_off: "Idle Inhibitor: off",
            action: toggle_idle,
            read_state: read_idle_state,
        },
    ]
}

/// Build a tile (vertical Box: toggle button + label) from a spec, wiring the
/// optimistic-toggle + revert-on-failure + `loading` behavior once.
pub fn build_tile(spec: &TileSpec) -> (gtk4::Box, gtk4::ToggleButton) {
    let (vbox, btn) = make_toggle(spec.icon, spec.label);

    let action = spec.action;
    let tooltip_on = spec.tooltip_on;
    let tooltip_off = spec.tooltip_off;

    let btn_h = btn.clone();
    btn.connect_clicked(move |_| {
        let target = btn_h.is_active();
        set_tooltip(&btn_h, target, tooltip_on, tooltip_off);

        btn_h.add_css_class("loading");
        let btn_done = btn_h.clone();
        spawn::spawn_work(
            move || action(target),
            move |success| {
                btn_done.remove_css_class("loading");
                if !success {
                    let b = btn_done.clone();
                    glib::timeout_add_local_once(
                        std::time::Duration::from_secs(2),
                        move || {
                            b.set_active(!target);
                            set_tooltip(&b, !target, tooltip_on, tooltip_off);
                        },
                    );
                }
            },
        );
    });

    (vbox, btn)
}

/// Read the initial state for a tile (on a background thread) and apply it.
pub fn init_tile_state(btn: &gtk4::ToggleButton, spec: &TileSpec) {
    let btn = btn.clone();
    let read_state = spec.read_state;
    let tooltip_on = spec.tooltip_on;
    let tooltip_off = spec.tooltip_off;
    spawn::spawn_work(
        move || read_state(),
        move |state| {
            apply_tile_state(&btn, state);
            if state != TileState::Unavailable {
                set_tooltip(&btn, state == TileState::Active, tooltip_on, tooltip_off);
            }
        },
    );
}

/// DND is store-backed (main-thread state), so it gets a dedicated builder:
/// no background action, just flips the store.
pub fn build_dnd_tile(store: Rc<RefCell<NotificationStore>>) -> (gtk4::Box, gtk4::ToggleButton) {
    let (vbox, btn) = make_toggle("󰍷", "DND");

    let active = store.borrow().is_dnd();
    btn.set_active(active);
    set_tooltip(&btn, active, "Do Not Disturb: active", "Do Not Disturb: off");

    let store_c = store.clone();
    let btn_h = btn.clone();
    btn.connect_clicked(move |_| {
        let on = btn_h.is_active();
        set_tooltip(&btn_h, on, "Do Not Disturb: active", "Do Not Disturb: off");
        store_c.borrow_mut().set_dnd(on);
    });

    (vbox, btn)
}

// ── Widget helpers ──────────────────────────────────────────────────────────

fn make_toggle(icon: &str, label_text: &str) -> (gtk4::Box, gtk4::ToggleButton) {
    let vbox = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .build();
    vbox.add_css_class("startmenu-tile");

    let btn = gtk4::ToggleButton::builder().label(icon).build();
    btn.add_css_class("toggle-btn");

    let label = gtk4::Label::builder().label(label_text).build();
    label.add_css_class("toggle-label");

    vbox.append(&btn);
    vbox.append(&label);

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

fn set_tooltip(btn: &gtk4::ToggleButton, active: bool, on: &str, off: &str) {
    btn.set_tooltip_text(Some(if active { on } else { off }));
}

fn apply_tile_state(btn: &gtk4::ToggleButton, state: TileState) {
    match state {
        TileState::Active => {
            btn.set_sensitive(true);
            btn.set_active(true);
        }
        TileState::Inactive => {
            btn.set_sensitive(true);
            btn.set_active(false);
        }
        TileState::Unavailable => {
            btn.set_sensitive(false);
            btn.set_active(false);
        }
    }
}

/// Spawn a command, wait for it, return whether it exited successfully. Logs
/// the failure. Used by the simple on/off tile actions.
fn run_ok(cmd: &mut Command) -> bool {
    match cmd.spawn().and_then(|mut c| c.wait()) {
        Ok(status) => status.success(),
        Err(e) => {
            if e.kind() == std::io::ErrorKind::NotFound {
                log::warn!("command not found: {:?}", cmd.get_program());
            } else {
                log::warn!("command {:?} failed: {e}", cmd.get_program());
            }
            false
        }
    }
}

// ── State readers (blocking — always called from a background thread) ─────────

fn read_wifi_state() -> TileState {
    match Command::new("nmcli").args(["radio", "wifi"]).output() {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            log::warn!("nmcli not found; Wi-Fi toggle disabled");
            TileState::Unavailable
        }
        Err(e) => {
            log::warn!("nmcli radio wifi failed: {e}");
            TileState::Unavailable
        }
        Ok(out) => {
            if String::from_utf8_lossy(&out.stdout).trim().eq_ignore_ascii_case("enabled") {
                TileState::Active
            } else {
                TileState::Inactive
            }
        }
    }
}

fn read_bluetooth_state() -> TileState {
    match Command::new("bluetoothctl").arg("show").output() {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            log::warn!("bluetoothctl not found; Bluetooth toggle disabled");
            TileState::Unavailable
        }
        Err(e) => {
            log::warn!("bluetoothctl show failed: {e}");
            TileState::Unavailable
        }
        Ok(out) => {
            let powered = String::from_utf8_lossy(&out.stdout)
                .lines()
                .any(|l| l.trim().eq_ignore_ascii_case("Powered: yes"));
            if powered {
                TileState::Active
            } else {
                TileState::Inactive
            }
        }
    }
}

fn read_night_state() -> TileState {
    match Command::new("systemctl")
        .args(["--user", "is-active", "gammastep.service"])
        .output()
    {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            log::warn!("systemctl not found; Night Light toggle disabled");
            TileState::Unavailable
        }
        Err(e) => {
            log::warn!("systemctl --user is-active gammastep.service failed: {e}");
            TileState::Unavailable
        }
        Ok(out) => {
            if String::from_utf8_lossy(&out.stdout).trim() == "active" {
                TileState::Active
            } else {
                TileState::Inactive
            }
        }
    }
}

const IDLE_PID_FILE: &str = "/tmp/swaypplet-idle-inhibit.pid";

fn toggle_idle(on: bool) -> bool {
    if on {
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
            Ok(child) => match std::fs::write(IDLE_PID_FILE, child.id().to_string()) {
                Ok(_) => true,
                Err(e) => {
                    log::warn!("Failed to write idle inhibit PID file: {e}");
                    false
                }
            },
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
        match std::fs::read_to_string(IDLE_PID_FILE) {
            Ok(contents) => match contents.trim().parse::<u32>() {
                Ok(pid) => {
                    let killed = Command::new("kill")
                        .arg(pid.to_string())
                        .spawn()
                        .and_then(|mut c| c.wait())
                        .map(|s| s.success())
                        .unwrap_or(false);
                    if killed {
                        let _ = std::fs::remove_file(IDLE_PID_FILE);
                    } else {
                        log::warn!("Failed to kill idle inhibit process (pid {pid})");
                    }
                    killed
                }
                Err(e) => {
                    log::warn!("Failed to parse idle inhibit PID: {e}");
                    false
                }
            },
            Err(e) => {
                log::warn!("Failed to read idle inhibit PID file: {e}");
                false
            }
        }
    }
}

fn read_idle_state() -> TileState {
    match std::fs::read_to_string(IDLE_PID_FILE) {
        Ok(contents) => match contents.trim().parse::<u32>() {
            Ok(pid) => {
                if std::path::Path::new(&format!("/proc/{pid}")).exists() {
                    TileState::Active
                } else {
                    let _ = std::fs::remove_file(IDLE_PID_FILE);
                    TileState::Inactive
                }
            }
            Err(_) => {
                let _ = std::fs::remove_file(IDLE_PID_FILE);
                TileState::Inactive
            }
        },
        Err(_) => TileState::Inactive,
    }
}
