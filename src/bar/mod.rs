//! Native status bar — waybar replacement.
//!
//! One layer surface per output, bottom-anchored frosted card matching the
//! waybar geometry it replaces (height 38, margins 0 4 4 4, radius 14 —
//! see users/modules/waybar.nix in the nixos repo). `Layer::Top` with an
//! auto exclusive zone so tiled windows sit above the bar + margins, while
//! the Overlay panel/OSD/launcher surfaces still stack over it.
//!
//! The default swaypplet process hosts the bar (app.rs); `swaypplet bar`
//! (own GApplication id) still runs it standalone for development next to
//! a live panel, and `SWAYPPLET_NO_BAR=1` keeps the hosted bar off while
//! an external bar owns the strip (see app.rs).

mod backup;
mod battery;
mod board;
mod clock;
mod decision;
mod hazards;
mod media;
mod peek;
mod pins;
mod popover;
mod presence;
mod start;
mod tray;
mod workspaces;

use std::cell::RefCell;
use std::rc::Rc;

use gio::prelude::*;
use gtk4::gdk;
use gtk4::prelude::*;
use gtk4_layer_shell::{Edge, Layer};

use crate::anim;
use crate::services::task_state::TaskStateService;
use crate::shell::{Namespace, PerMonitor, Surface};
use crate::sway::ipc::SwayService;
use crate::theme;

const APP_ID: &str = "dev.swaypplet.bar";

/// One output's bar. Dropping it (its monitor left) destroys the surface.
struct BarWindow {
    #[allow(dead_code)] // held for its Drop: the surface lives as long as this
    surface: Surface,
    /// OSD interjections route here (BAR_VISION increment 5).
    decision: decision::DecisionSlot,
}

/// Keeps one bar window per connected output, following monitor hotplug.
pub struct BarManager {
    app: gtk4::Application,
    windows: Rc<PerMonitor<BarWindow>>,
    sway: Rc<SwayService>,
    /// One sound-server connection per bar process; the hazard lane's
    /// microphone glyph is its only reader here.
    audio: Rc<crate::services::audio::AudioService>,
    tasks: Rc<TaskStateService>,
    /// One status watcher per bar process; every output's backup segment
    /// reads the same snapshot.
    backup: Rc<crate::services::backup::BackupStatusService>,
    /// One MPRIS connection per bar process; every output's media mark
    /// reads the same snapshot.
    mpris: Rc<crate::services::mpris::MprisService>,
    tray: Rc<crate::services::tray::TrayService>,
    /// What the start button does. In-process hosting passes a direct
    /// `panel.toggle()`; the standalone bar passes the cross-process
    /// SIGUSR1 fallback (see `start::toggle_panel_fallback`).
    toggle_panel: Rc<dyn Fn()>,
}

impl BarManager {
    pub fn new(
        app: &gtk4::Application,
        sway: Rc<SwayService>,
        audio: Rc<crate::services::audio::AudioService>,
        toggle_panel: Rc<dyn Fn()>,
    ) -> Rc<Self> {
        let tasks = TaskStateService::start(&sway);
        let manager = Rc::new(Self {
            app: app.clone(),
            windows: PerMonitor::new(),
            sway,
            audio,
            tasks,
            backup: crate::services::backup::BackupStatusService::start(),
            mpris: crate::services::mpris::MprisService::start(),
            tray: crate::services::tray::TrayService::start(),
            toggle_panel,
        });

        // One bar per output, following hotplug. Weak: the manager owns
        // the set, and the set's build must not own the manager.
        let weak = Rc::downgrade(&manager);
        manager.windows.watch(move |monitor| {
            let manager = weak.upgrade().expect("the bar manager outlives its bars");
            // build_bar_window maps the window itself (Reveal enter).
            let (surface, decision) = build_bar_window(&manager, monitor);
            BarWindow { surface, decision }
        });

        manager
    }

    /// Route a volume/brightness OSD into the decision slot of every bar,
    /// like the center card, which is on every output too: a key press has
    /// no output. `false` when no bar exists, so the caller falls back to
    /// the center-screen card.
    pub fn interject(&self, icon: &str, fraction: f64, text: &str) -> bool {
        self.windows
            .for_each(|bar| bar.decision.interject(icon, fraction, text));
        !self.windows.is_empty()
    }
}

fn build_bar_window(bar: &BarManager, monitor: &gdk::Monitor) -> (Surface, decision::DecisionSlot) {
    let BarManager {
        app,
        sway,
        audio,
        tasks,
        backup,
        mpris,
        tray,
        toggle_panel,
        ..
    } = bar;
    let toggle_panel = toggle_panel.clone();
    // Waybar's mainBar geometry: bottom card, 38px, insets matching sway's
    // `gaps inner 4` so card edges align with tiled window edges. Top, with
    // an exclusive zone, so tiled windows sit above the bar and its margins
    // while the overlay surfaces still stack over it. Stretched between
    // left and right, so the compositor sets its width (Surface keeps it
    // resizable).
    let surface = Surface::builder(app, Namespace::Bar)
        .monitor(Some(monitor))
        .layer(Layer::Top)
        .anchor(&[Edge::Bottom, Edge::Left, Edge::Right])
        .margin(Edge::Right, 4)
        .margin(Edge::Bottom, 4)
        .margin(Edge::Left, 4)
        .height(38)
        .exclusive()
        .card(crate::ui::Card::Thin)
        // Enter: fade + short settle up from below the surface edge, per bar
        // window (so hotplugged outputs get it too).
        .slide(gtk4::Orientation::Vertical, anim::SLIDE_PX)
        .build();
    let window = surface.window().clone();
    // The whole 38px: the root is a box, and the bar's card fills it.
    if let Some(slide) = surface.slide() {
        slide.set_vexpand(true);
    }

    // Pane/content split for the enter transition (motion on glass,
    // anim.rs): `root` carries the thin glass card so its tint lands with
    // the frost in one step, while the clusters on it fade over the full
    // enter.
    let root = surface.card().clone();

    // CenterBox, not Box: the center slot must stay screen-centered
    // regardless of how the left/right clusters grow.
    let content = gtk4::CenterBox::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .hexpand(true)
        // The card is a vertical box; the clusters take its whole height,
        // as they did in a horizontal one.
        .vexpand(true)
        .build();

    let left = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .build();
    left.append(&start::build(toggle_panel));
    // No per-output argument: the strip states the world identically on
    // every bar, and its groups say which screen each workspace is on.
    left.append(&workspaces::build(sway, tasks));
    // Center: the decision slot — priority-muxed single occupant, empty
    // at nominal (bar/decision.rs).
    let center = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .build();
    let decision = decision::DecisionSlot::build(tasks);
    center.append(decision.widget());
    let right = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .build();
    // Right cluster order per the vision: media mark, tray, hazard lane,
    // then the instrument track. The Bar tab's Segments group hides the
    // ones it names; the hazard lane and the clock are not optional.
    right.append(&follow_setting(media::build(mpris), |bar| bar.media));
    // Where pinned workspaces live when they are not floating (jump/pin.rs).
    right.append(&pins::build());
    right.append(&follow_setting(tray::build(tray), |bar| bar.tray));
    right.append(&hazards::build(sway, audio));
    // Battery + board + clock fuse into one segmented track (waybar's
    // group/right-track); a batteryless machine skips the segment so the
    // board keeps the rounded left end.
    let track = crate::ui::segmented();
    track.add_css_class("bar-track");
    if let Some(bat) = battery::build({
        // The decision slot's battery occupant rides this segment's 30 s
        // poll instead of polling on its own.
        let slot = decision.clone();
        move |bat| slot.set_battery(bat)
    }) {
        track.append(&follow_setting(bat, |bar| bar.battery));
    }
    // gdk connector names match sway output names under wlroots.
    let board = board::build(
        sway,
        tasks,
        monitor.connector().map(|c| c.to_string()),
        &root,
    );
    // Hidden by default rather than removed: the bays were not earning their
    // width, but the instrument may come back, and an invisible child costs
    // one layout skip. GTK gives no clicks to a hidden widget, so the bay
    // popovers are unreachable too.
    track.append(&follow_setting(board, |bar| bar.board));
    if let Some(presence) = presence::build() {
        track.append(&follow_setting(presence, |bar| bar.presence));
    }
    track.append(&follow_setting(backup::build(backup), |bar| bar.backup));
    track.append(&clock::build());
    right.append(&track);

    content.set_start_widget(Some(&left));
    content.set_center_widget(Some(&center));
    content.set_end_widget(Some(&right));
    root.append(&content);
    surface.set_content(&content);

    // Height forensics at map time: the surface only honours the requested
    // 38px if no cluster's minimum exceeds it, and a single padded widget
    // silently grows the whole bar. Log the offenders instead of guessing.
    {
        let (left, center, right, root) =
            (left.clone(), center.clone(), right.clone(), root.clone());
        window.connect_map(move |_| {
            for (name, w) in [
                ("root", root.upcast_ref::<gtk4::Widget>()),
                ("left", left.upcast_ref()),
                ("center", center.upcast_ref()),
                ("right", right.upcast_ref()),
            ] {
                let (min, nat, _, _) = w.measure(gtk4::Orientation::Vertical, -1);
                log::debug!("bar height: {name} min={min} nat={nat}");
            }
        });
    }

    // The exclusive zone is a property of the mapped surface, so tiled
    // windows take their final size on frame one: only render nodes move.
    // Bars never hide.
    surface.show();

    (surface, decision)
}

/// Show `widget` while `wanted` says so, now and on every settings change.
/// A weak ref, so a bar whose output was unplugged does not keep its
/// segments alive through the observer list.
fn follow_setting<W: IsA<gtk4::Widget>>(
    widget: W,
    wanted: fn(&crate::settings::store::Bar) -> bool,
) -> W {
    let shown = move || crate::settings::store::with(|s| wanted(&s.bar()));
    widget.set_visible(shown());
    let weak = widget.downgrade();
    crate::settings::store::observe(move || {
        if let Some(widget) = weak.upgrade() {
            widget.set_visible(shown());
        }
    });
    widget
}

pub fn run() {
    let app = gtk4::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::FLAGS_NONE)
        .build();

    let manager: Rc<RefCell<Option<Rc<BarManager>>>> = Rc::new(RefCell::new(None));

    let manager_activate = manager.clone();
    app.connect_activate(move |app| {
        let mut slot = manager_activate.borrow_mut();
        if slot.is_some() {
            // Remote activation of an already-running instance.
            return;
        }
        theme::load_css();
        // The bar outlives every wallpaper change in a session, so it follows
        // the theme inputs (the wallpaper's hue the panel samples among them)
        // instead of keeping the ones it started with. Nothing here writes
        // them; the glass goes back through the settings' replay, as in the
        // panel.
        theme::watch(crate::settings::glass::apply_saved_for);
        crate::settings::store::init();
        // Keeps itself alive through its main-context event loop.
        let sway = SwayService::start();
        // The standalone bar starts its own sound-server connection: the
        // microphone hazard is the only consumer here, and one connection
        // costs a socket.
        *slot = Some(BarManager::new(
            app,
            sway,
            crate::services::audio::AudioService::start(),
            Rc::new(start::toggle_panel_fallback),
        ));
    });

    // Empty argv: run() would parse std::env::args and treat the `bar`
    // subcommand word as a file to open, which FLAGS_NONE rejects.
    app.run_with_args::<&str>(&[]);
}
