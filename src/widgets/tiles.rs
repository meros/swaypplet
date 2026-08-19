//! Declarative quick-settings toggle tiles for the start-menu quick strip.
//!
//! Each tile is described once as a [`TileSpec`]; a single [`build_tile`]
//! factory turns a spec into an optimistic toggle button with revert-on-failure
//! and the `loading` CSS class. This replaces the per-toggle copy-pasted
//! click-handler boilerplate that used to live in `header.rs`.
//!
//! A spec is presentation plus two function pointers, and that is the whole
//! contract: this module knows how to *draw* a toggle and nothing about what
//! any of them switch. The inhibitor tiles (No Sleep / No Lock) are generated
//! from [`crate::inhibit`], which owns their wording, their actions and their
//! state; Wi-Fi and Bluetooth delegate the same way to their own modules.

use std::cell::RefCell;
use std::process::Command;
use std::rc::Rc;
use std::sync::Arc;

use gtk4::prelude::*;

use crate::inhibit::{self, Inhibitor};
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

impl From<Option<bool>> for TileState {
    /// `None` is "could not reach the authority", which is exactly
    /// [`TileState::Unavailable`].
    fn from(state: Option<bool>) -> Self {
        match state {
            Some(true) => TileState::Active,
            Some(false) => TileState::Inactive,
            None => TileState::Unavailable,
        }
    }
}

/// One tile: icon glyph, label, on/off tooltips, the async on/off action and
/// the state reader. `action` and `read_state` run on a background thread,
/// hence `Arc<… + Send + Sync>`; `on_state` is main-thread only.
///
/// Cloning a spec is cheap (three `&'static str`s and three refcount bumps),
/// which is what lets the panel keep one beside each button for refresh.
#[derive(Clone)]
pub struct TileSpec {
    pub icon: &'static str,
    pub label: &'static str,
    pub tooltip_on: &'static str,
    pub tooltip_off: &'static str,
    /// Perform the toggle for the requested target state. Returns success.
    /// Runs on a background thread (blocking I/O allowed).
    pub action: Arc<dyn Fn(bool) -> bool + Send + Sync>,
    /// Read current state. Runs on a background thread.
    pub read_state: Arc<dyn Fn() -> TileState + Send + Sync>,
    /// Main-thread observer fired with each *established* state: every
    /// successful read and every successful toggle. Feeds in-process
    /// consumers (the bar's hazard lane) without them polling the tool.
    pub on_state: Option<Rc<dyn Fn(bool)>>,
}

/// Every tile the quick strip can show: Wi-Fi, Bluetooth, Night Light and
/// the two session inhibitors. DND is store-backed and handled specially by
/// the panel; the rest drive something outside this process.
pub fn tile_specs() -> Vec<TileSpec> {
    let mut specs = vec![wifi_spec(), bluetooth_spec(), night_light_spec()];
    specs.extend(Inhibitor::ALL.map(inhibitor_spec));
    specs
}

/// The subset the panel's flight deck carries, in deck order. Built from the
/// same constructors as [`tile_specs`] rather than indexed out of it — the
/// deck used to reach in by position, and every tile added shifted the
/// indices under it.
pub fn deck_specs() -> Vec<TileSpec> {
    let mut specs = vec![night_light_spec()];
    specs.extend(Inhibitor::ALL.map(inhibitor_spec));
    specs
}

fn wifi_spec() -> TileSpec {
    TileSpec {
        icon: "󰤨",
        label: "Wi-Fi",
        tooltip_on: "Wi-Fi: enabled",
        tooltip_off: "Wi-Fi: disabled",
        action: Arc::new(|on| {
            matches!(
                crate::widgets::network::set_wifi_radio(on),
                crate::widgets::network::NmResult::Success
            )
        }),
        read_state: Arc::new(read_wifi_state),
        on_state: None,
    }
}

fn bluetooth_spec() -> TileSpec {
    TileSpec {
        icon: "󰂯",
        label: "Bluetooth",
        tooltip_on: "Bluetooth: powered on",
        tooltip_off: "Bluetooth: powered off",
        action: Arc::new(|on| crate::widgets::bluez::set_powered(on).is_ok()),
        read_state: Arc::new(read_bluetooth_state),
        on_state: None,
    }
}

fn night_light_spec() -> TileSpec {
    TileSpec {
        icon: "󰖔",
        label: "Night Light",
        tooltip_on: "Night Light: active",
        tooltip_off: "Night Light: off",
        action: Arc::new(|on| {
            run_ok(Command::new("systemctl").args([
                "--user",
                if on { "start" } else { "stop" },
                "gammastep.service",
            ]))
        }),
        read_state: Arc::new(read_night_state),
        on_state: None,
    }
}

/// One tile per session inhibitor, all of it delegated: wording, action and
/// reading come from [`crate::inhibit`], and the established state goes back
/// there for the bar's hazard lane to observe. Adding a third inhibitor is a
/// change in that module alone.
fn inhibitor_spec(which: Inhibitor) -> TileSpec {
    TileSpec {
        icon: which.icon(),
        label: which.label(),
        tooltip_on: which.tooltip_on(),
        tooltip_off: which.tooltip_off(),
        action: Arc::new(move |on| which.arm(on)),
        read_state: Arc::new(move || which.read().into()),
        on_state: Some(Rc::new(move |armed| inhibit::publish(which, armed))),
    }
}

/// Build a tile (vertical Box: toggle button + label) from a spec, wiring the
/// optimistic-toggle + revert-on-failure + `loading` behavior once.
pub fn build_tile(spec: &TileSpec) -> gtk4::ToggleButton {
    let btn = make_toggle(spec.icon, spec.label);

    let spec = spec.clone();
    let btn_h = btn.clone();
    btn.connect_clicked(move |_| {
        let target = btn_h.is_active();
        let (label, tooltip_on, tooltip_off) = (spec.label, spec.tooltip_on, spec.tooltip_off);
        set_tooltip(&btn_h, target, tooltip_on, tooltip_off);
        log::info!("tile[{label}]: toggle requested — target {target}");

        btn_h.add_css_class("loading");
        let btn_done = btn_h.clone();
        let action = spec.action.clone();
        let on_state = spec.on_state.clone();
        spawn::spawn_work(
            move || action(target),
            move |success| {
                btn_done.remove_css_class("loading");
                if success {
                    log::info!("tile[{label}]: now {target}");
                    // Established, not optimistic: a failed toggle never
                    // reaches in-process consumers.
                    if let Some(observe) = on_state {
                        observe(target);
                    }
                } else {
                    log::warn!("tile[{label}]: toggle failed — reverting in 2s");
                    let b = btn_done.clone();
                    glib::timeout_add_local_once(std::time::Duration::from_secs(2), move || {
                        b.set_active(!target);
                        set_tooltip(&b, !target, tooltip_on, tooltip_off);
                    });
                }
            },
        );
    });

    btn
}

/// Build a bare reveal tile (container + toggle button, icon + label content)
/// with no radio action wired — used for reveal-only tiles like Display, where
/// the panel attaches its own reveal handler.
pub fn build_reveal_tile(icon: &str, label: &str) -> gtk4::ToggleButton {
    make_toggle(icon, label)
}

/// Wrap a tile's toggle button in an overlay with a reveal chevron floating at
/// the right edge. The chevron takes no layout width, so chevron and non-chevron
/// tiles stay the same size in a homogeneous grid. Returns the grid-cell widget
/// (the overlay) and the chevron button for the caller to wire.
pub fn wrap_with_chevron(btn: &gtk4::ToggleButton) -> (gtk4::Overlay, gtk4::Button) {
    let overlay = gtk4::Overlay::new();
    overlay.add_css_class("startmenu-tile");
    // Reserve room on the right so the label never slides under the chevron.
    btn.add_css_class("has-chevron");
    overlay.set_child(Some(btn));

    let chevron = tile_chevron();
    chevron.set_halign(gtk4::Align::End);
    chevron.set_valign(gtk4::Align::Center);
    overlay.add_overlay(&chevron);

    (overlay, chevron)
}

/// Read the initial state for a tile (on a background thread) and apply it.
pub fn init_tile_state(btn: &gtk4::ToggleButton, spec: &TileSpec) {
    let btn = btn.clone();
    let read_state = spec.read_state.clone();
    let (tooltip_on, tooltip_off) = (spec.tooltip_on, spec.tooltip_off);
    let on_state = spec.on_state.clone();
    spawn::spawn_work(
        move || read_state(),
        move |state| {
            apply_tile_state(&btn, state);
            if state != TileState::Unavailable {
                set_tooltip(&btn, state == TileState::Active, tooltip_on, tooltip_off);
                if let Some(observe) = on_state {
                    observe(state == TileState::Active);
                }
            }
        },
    );
}

/// DND is store-backed (main-thread state), so it gets a dedicated builder:
/// no background action, just flips the store.
pub fn build_dnd_tile(store: Rc<RefCell<NotificationStore>>) -> gtk4::ToggleButton {
    let btn = make_toggle("󰍷", "DND");

    let active = store.borrow().is_dnd();
    btn.set_active(active);
    set_tooltip(
        &btn,
        active,
        "Do Not Disturb: active",
        "Do Not Disturb: off",
    );

    let store_c = store.clone();
    let btn_h = btn.clone();
    btn.connect_clicked(move |_| {
        let on = btn_h.is_active();
        set_tooltip(&btn_h, on, "Do Not Disturb: active", "Do Not Disturb: off");
        store_c.borrow_mut().set_dnd(on);
    });

    btn
}

// ── Widget helpers ──────────────────────────────────────────────────────────

/// The icon + label content shared by all tiles: glyph on the left, text label
/// filling the rest, left-aligned. Lives inside the toggle button.
fn tile_content(icon: &str, label_text: &str) -> gtk4::Box {
    let content = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(10)
        .build();
    content.add_css_class("tile-content");

    let icon_lbl = gtk4::Label::builder().label(icon).build();
    icon_lbl.add_css_class("tile-icon");

    let text_lbl = gtk4::Label::builder().label(label_text).xalign(0.0).build();
    text_lbl.add_css_class("toggle-label");
    text_lbl.set_hexpand(true);

    content.append(&icon_lbl);
    content.append(&text_lbl);
    content
}

/// A reveal chevron that sits flush beside a tile's toggle button (same height).
/// Callers wire its click handler and flip the glyph via [`set_chevron_open`].
pub fn tile_chevron() -> gtk4::Button {
    let chevron = gtk4::Button::builder().label("›").build();
    chevron.add_css_class("startmenu-tile-chevron");
    chevron
}

/// Flip a chevron between collapsed (›) and expanded (⌄) glyphs.
pub fn set_chevron_open(chevron: &gtk4::Button, open: bool) {
    chevron.set_label(if open { "⌄" } else { "›" });
}

fn make_toggle(icon: &str, label_text: &str) -> gtk4::ToggleButton {
    let btn = gtk4::ToggleButton::builder().hexpand(true).build();
    btn.add_css_class("toggle-btn");
    btn.set_child(Some(&tile_content(icon, label_text)));

    // Active state colors the whole button (icon + label) via `.toggle-btn.active`.
    btn.connect_toggled(|btn| {
        if btn.is_active() {
            btn.add_css_class("active");
        } else {
            btn.remove_css_class("active");
        }
    });

    btn
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
    if !crate::widgets::network::network_manager_available() {
        return TileState::Unavailable;
    }
    if crate::widgets::network::wifi_radio_enabled() {
        TileState::Active
    } else {
        TileState::Inactive
    }
}

fn read_bluetooth_state() -> TileState {
    let snapshot = crate::widgets::bluez::snapshot();
    if !snapshot.available {
        return TileState::Unavailable;
    }
    if snapshot.powered {
        TileState::Active
    } else {
        TileState::Inactive
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
