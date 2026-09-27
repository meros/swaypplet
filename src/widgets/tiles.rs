//! Declarative quick-settings toggle tiles for the start-menu quick strip.
//!
//! Each tile is described once as a [`TileSpec`]; a single [`build_tile`]
//! factory turns a spec into an optimistic toggle button with revert-on-failure
//! and the `loading` CSS class, drawn as the design system's tile
//! (`ui::tile_toggle`). This replaces the per-toggle copy-pasted
//! click-handler boilerplate that used to live in `header.rs`.
//!
//! A spec is presentation plus two function pointers, and that is the whole
//! contract: this module knows how to *draw* a toggle and nothing about what
//! any of them switch. The inhibitor tiles (No Sleep / No Lock) are generated
//! from [`crate::services::inhibit`], which owns their wording, their actions and their
//! state; Wi-Fi and Bluetooth delegate the same way to their own modules.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use gtk4::prelude::*;

use crate::services::inhibit::{self, Inhibitor};
use crate::services::notifications::store::NotificationStore;
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
    /// The split tile's status line: why the tile is in its state.
    pub status: Option<Status>,
}

/// Where a tile's status line comes from.
#[derive(Clone)]
pub enum Status {
    /// A blocking read (a unit's description), on a worker.
    Worker(Arc<dyn Fn() -> String + Send + Sync>),
    /// A read of main-thread state (the settings).
    Main(Rc<dyn Fn() -> String>),
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
                crate::services::network::set_wifi_radio(on),
                crate::services::network::NmResult::Success
            )
        }),
        read_state: Arc::new(read_wifi_state),
        on_state: None,
        status: None,
    }
}

fn bluetooth_spec() -> TileSpec {
    TileSpec {
        icon: "󰂯",
        label: "Bluetooth",
        tooltip_on: "Bluetooth: powered on",
        tooltip_off: "Bluetooth: powered off",
        action: Arc::new(|on| crate::services::bluez::set_powered(on).is_ok()),
        read_state: Arc::new(read_bluetooth_state),
        on_state: None,
        status: None,
    }
}

fn night_light_spec() -> TileSpec {
    TileSpec {
        icon: "󰖔",
        label: "Night Light",
        tooltip_on: "Night Light: active",
        tooltip_off: "Night Light: off",
        action: Arc::new(|on| {
            if !crate::services::gamma::available() {
                return false;
            }
            crate::services::gamma::set_enabled(on);
            true
        }),
        read_state: Arc::new(read_night_state),
        on_state: None,
        status: Some(Status::Main(Rc::new(night_status))),
    }
}

/// One tile per session inhibitor, all of it delegated: wording, action and
/// reading come from [`crate::services::inhibit`], and the established state goes back
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
        status: Some(Status::Worker(Arc::new(move || inhibitor_status(which)))),
    }
}

/// Build a tile (vertical Box: toggle button + label) from a spec, wiring the
/// optimistic-toggle + revert-on-failure + `loading` behavior once.
pub fn build_tile(spec: &TileSpec) -> gtk4::ToggleButton {
    let btn = make_toggle(spec.icon, spec.label);
    wire(&btn, spec);
    btn
}

/// A split tile from a spec: the body is the same toggle as [`build_tile`],
/// `on_detail` runs for the chevron.
pub fn build_split(spec: &TileSpec, on_detail: impl Fn(&gtk4::Button) + 'static) -> crate::ui::SplitTile {
    let tile = crate::ui::tile_split(spec.icon, spec.label);
    tile.root.set_hexpand(true);
    wire(&tile.toggle, spec);
    tile.detail.connect_clicked(on_detail);
    tile
}

/// Read a tile's status line again, from its spec's source.
pub fn refresh_status(label: &gtk4::Label, spec: &TileSpec) {
    let show = |label: &gtk4::Label, text: String| {
        label.set_visible(!text.is_empty());
        label.set_label(&text);
    };
    match &spec.status {
        None => {}
        Some(Status::Main(read)) => show(label, read()),
        Some(Status::Worker(read)) => {
            let (read, label) = (read.clone(), label.clone());
            spawn::spawn_work(move || read(), move |text| show(&label, text));
        }
    }
}

/// The durations a timed switch offers, in minutes; `None` is until the
/// switch is turned off.
const DURATIONS: [(&str, Option<u32>); 5] = [
    ("30 min", Some(30)),
    ("1 hour", Some(60)),
    ("2 hours", Some(120)),
    ("4 hours", Some(240)),
    ("Until turned off", None),
];

/// The durations for the timed switches (No Sleep, No Lock), folded out
/// inline under the switch strip.
///
/// It used to be a `GtkPopover` on the chevron. On a layer-shell panel that
/// is a separate popup surface, outside the glass config and placed by the
/// compositor, and it came out broken. Inline it is part of the panel: one
/// strip, shared by both switches, on the panel's own glass. The chevron
/// opens it for its switch, a second press or the other switch's chevron
/// changes it, a pick arms the switch and folds it away, and it folds away
/// whenever the panel closes.
pub struct DurationFold {
    pub root: gtk4::Revealer,
    title: gtk4::Label,
    /// Which switch it is open for.
    open: Cell<Option<Inhibitor>>,
    /// The chevron that opened it, marked while it is open.
    anchor: RefCell<Option<gtk4::Button>>,
    /// What a pick does for the switch it is open for.
    on_pick: RefCell<Option<Rc<dyn Fn(Option<u32>)>>>,
    /// Set while a pick is arming, so a second click does not arm twice.
    busy: Cell<bool>,
}

impl DurationFold {
    pub fn new() -> Rc<DurationFold> {
        let root = crate::ui::revealer(
            gtk4::RevealerTransitionType::SlideDown,
            crate::tokens::motion::EXPAND,
        );
        let body = crate::ui::vbox(2);
        body.add_css_class("deck-fold");
        let title = crate::ui::heading("");
        title.set_halign(gtk4::Align::Start);
        let row = crate::ui::hbox(2);
        body.append(&title);
        body.append(&row);
        root.set_child(Some(&body));
        let fold = Rc::new(DurationFold {
            root,
            title,
            open: Cell::new(None),
            anchor: RefCell::new(None),
            on_pick: RefCell::new(None),
            busy: Cell::new(false),
        });
        for (label, minutes) in DURATIONS {
            let b = crate::ui::button_with(
                crate::ui::Face::Label(label),
                crate::ui::Kind::Secondary,
                crate::ui::Size::Small,
            );
            let weak = Rc::downgrade(&fold);
            b.connect_clicked(move |_| {
                let Some(fold) = weak.upgrade() else { return };
                if fold.busy.get() {
                    return;
                }
                let pick = fold.on_pick.borrow().clone();
                if let Some(pick) = pick {
                    pick(minutes);
                }
            });
            row.append(&b);
        }
        fold
    }

    /// Open for `which` from `anchor`, or close when it is already open for
    /// it. `done` runs after a pick with whether the switch took.
    pub fn toggle(
        self: &Rc<Self>,
        anchor: &gtk4::Button,
        which: Inhibitor,
        done: impl Fn(bool) + 'static,
    ) {
        if self.open.get() == Some(which) && self.root.reveals_child() {
            self.close();
            return;
        }
        self.mark(None);
        self.title.set_label(&format!("{} for", which.label()));
        let done: Rc<dyn Fn(bool)> = Rc::new(done);
        let weak = Rc::downgrade(self);
        *self.on_pick.borrow_mut() = Some(Rc::new(move |minutes| {
            let Some(fold) = weak.upgrade() else { return };
            fold.busy.set(true);
            let done = done.clone();
            let weak = Rc::downgrade(&fold);
            spawn::spawn_work(
                move || which.arm_for(true, minutes),
                move |ok| {
                    if ok {
                        inhibit::publish(which, true);
                    }
                    if let Some(fold) = weak.upgrade() {
                        fold.busy.set(false);
                        fold.close();
                    }
                    done(ok);
                },
            );
        }));
        self.open.set(Some(which));
        self.mark(Some(anchor));
        self.root.set_reveal_child(true);
    }

    /// Fold away and forget the switch it was open for.
    pub fn close(&self) {
        self.root.set_reveal_child(false);
        self.open.set(None);
        self.on_pick.borrow_mut().take();
        self.mark(None);
    }

    /// Mark the chevron that owns the open fold, and unmark the last one.
    fn mark(&self, anchor: Option<&gtk4::Button>) {
        if let Some(old) = self.anchor.borrow_mut().take() {
            old.remove_css_class("open");
        }
        if let Some(a) = anchor {
            a.add_css_class("open");
            *self.anchor.borrow_mut() = Some(a.clone());
        }
    }
}

/// The optimistic toggle, its revert on failure, and the `loading` state,
/// on any toggle button.
fn wire(btn: &gtk4::ToggleButton, spec: &TileSpec) {
    let spec = spec.clone();
    let btn_h = btn.clone();
    btn.connect_clicked(move |_| {
        let target = btn_h.is_active();
        let (label, tooltip_on, tooltip_off) = (spec.label, spec.tooltip_on, spec.tooltip_off);
        set_tooltip(&btn_h, target, tooltip_on, tooltip_off);
        log::info!("tile[{label}]: toggle requested — target {target}");

        crate::ui::set_loading(&btn_h, true);
        let btn_done = btn_h.clone();
        let action = spec.action.clone();
        let on_state = spec.on_state.clone();
        spawn::spawn_work(
            move || action(target),
            move |success| {
                crate::ui::set_loading(&btn_done, false);
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

/// Do Not Disturb as a split tile: the body flips the store, the chevron
/// runs `on_detail`, and the status line says why ([`dnd_status`]).
pub fn build_dnd_split(
    store: Rc<RefCell<NotificationStore>>,
    on_detail: impl Fn(&gtk4::Button) + 'static,
) -> crate::ui::SplitTile {
    let tile = crate::ui::tile_split("󰍷", "DND");
    tile.root.set_hexpand(true);
    let active = store.borrow().is_dnd();
    tile.toggle.set_active(active);
    crate::ui::set_tile_status(&tile, &dnd_status(active));
    let status = tile.status.clone();
    tile.toggle.connect_clicked(move |b| {
        let on = b.is_active();
        store.borrow_mut().set_dnd(on);
        let text = dnd_status(on);
        status.set_visible(!text.is_empty());
        status.set_label(&text);
    });
    tile.detail.connect_clicked(on_detail);
    tile
}

// ── Widget helpers ──────────────────────────────────────────────────────────

/// The tile component (`ui::tile_toggle`): glyph and name on one button,
/// the accent fill when on.
fn make_toggle(icon: &str, label_text: &str) -> gtk4::ToggleButton {
    let btn = crate::ui::tile_toggle(icon, label_text);
    btn.set_hexpand(true);
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

// ── State readers (blocking — always called from a background thread) ─────────

fn read_wifi_state() -> TileState {
    if !crate::services::network::network_manager_available() {
        return TileState::Unavailable;
    }
    if crate::services::network::wifi_radio_enabled() {
        TileState::Active
    } else {
        TileState::Inactive
    }
}

fn read_bluetooth_state() -> TileState {
    let snapshot = crate::services::bluez::snapshot();
    if !snapshot.available {
        return TileState::Unavailable;
    }
    if snapshot.powered {
        TileState::Active
    } else {
        TileState::Inactive
    }
}

/// The night light is in this process (`services::gamma`): available when
/// the compositor gave it the gamma tables, on when the setting says so.
fn read_night_state() -> TileState {
    use crate::services::gamma;
    if !gamma::available() {
        TileState::Unavailable
    } else if gamma::enabled() {
        TileState::Active
    } else {
        TileState::Inactive
    }
}

// ── Status lines ─────────────────────────────────────────────────────────────

/// "Until 14:30" for a timed inhibitor, "Until turned off" for one armed
/// without an end, empty when off. Blocking.
fn inhibitor_status(which: Inhibitor) -> String {
    if !which.armed() {
        return String::new();
    }
    match which.until() {
        Some(t) => format!("Until {t}"),
        None => "Until turned off".to_string(),
    }
}

/// "Sun · 3500 K" or "All day · 3500 K": short enough for a tile. Main
/// thread.
fn night_status() -> String {
    use crate::settings::store::{self, NightSchedule};
    let n = store::current().night_light();
    let when = match n.schedule {
        NightSchedule::Sun => "Sun",
        NightSchedule::Always => "All day",
    };
    format!("{when} · {} K", n.night_k)
}

/// Why Do Not Disturb is what it is: the quiet hours' end when they armed
/// it, their start when they will, and "Until turned off" otherwise.
pub fn dnd_status(on: bool) -> String {
    let alerts = crate::settings::store::current().alerts();
    let hour = gtk4::glib::DateTime::now_local().map(|t| t.hour() as u8).unwrap_or(12);
    match (on, alerts.quiet, alerts.quiet && alerts.in_quiet_hours(hour)) {
        (true, _, true) => format!("Quiet until {:02}:00", alerts.quiet_to_h),
        (true, _, false) => "Until turned off".to_string(),
        (false, true, false) => format!("Quiet from {:02}:00", alerts.quiet_from_h),
        _ => String::new(),
    }
}

