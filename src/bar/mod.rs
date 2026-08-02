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

mod battery;
mod board;
mod clock;
mod media;
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
use crate::layer_shell::{self, LayerShellConfig};
use crate::sway_ipc::SwayService;
use crate::task_state::TaskStateService;
use crate::theme;

const APP_ID: &str = "dev.swaypplet.bar";

// Waybar's mainBar geometry: bottom card, 38px, insets matching sway's
// `gaps inner 4` so card edges align with tiled window edges. swayfx
// layer_effects keyed on the namespace blur the wallpaper behind it.
static BAR_CONFIG: LayerShellConfig = LayerShellConfig {
    namespace: "swaypplet-bar",
    layer: Layer::Top,
    exclusive: true,
    default_width: None,
    default_height: Some(38),
    anchors: &[
        (Edge::Bottom, true),
        (Edge::Left, true),
        (Edge::Right, true),
    ],
    margins: &[(Edge::Right, 4), (Edge::Bottom, 4), (Edge::Left, 4)],
    keyboard_mode: gtk4_layer_shell::KeyboardMode::None,
};

struct BarWindow {
    monitor: gdk::Monitor,
    window: gtk4::Window,
}

/// Keeps one bar window per connected output, following monitor hotplug.
pub struct BarManager {
    app: gtk4::Application,
    monitors: gio::ListModel,
    windows: RefCell<Vec<BarWindow>>,
    sway: Rc<SwayService>,
    tasks: Rc<TaskStateService>,
    tray: Rc<tray::TrayService>,
    /// What the start button does. In-process hosting passes a direct
    /// `panel.toggle()`; the standalone bar passes the cross-process
    /// SIGUSR1 fallback (see `start::toggle_panel_fallback`).
    toggle_panel: Rc<dyn Fn()>,
}

impl BarManager {
    pub fn new(
        app: &gtk4::Application,
        sway: Rc<SwayService>,
        toggle_panel: Rc<dyn Fn()>,
    ) -> Rc<Self> {
        let display = gdk::Display::default().expect("no gdk display");
        let tasks = TaskStateService::start(&sway);
        let manager = Rc::new(Self {
            app: app.clone(),
            monitors: display.monitors(),
            windows: RefCell::new(Vec::new()),
            sway,
            tasks,
            tray: tray::TrayService::start(),
            toggle_panel,
        });

        // Intentional Rc cycle (monitors → handler → manager → monitors):
        // the manager lives for the process, so it never needs to drop.
        let for_sync = manager.clone();
        manager
            .monitors
            .connect_items_changed(move |_, _, _, _| for_sync.sync());
        manager.sync();

        manager
    }

    /// Reconcile windows against the current monitor list. A layer surface
    /// is bound to its wl_output, so a window whose monitor vanished is
    /// destroyed and a fresh one built for any new monitor — never migrated.
    fn sync(&self) {
        let current: Vec<gdk::Monitor> = self
            .monitors
            .iter::<gdk::Monitor>()
            .filter_map(|m| m.ok())
            .collect();

        self.windows.borrow_mut().retain(|bar| {
            let alive = current.contains(&bar.monitor);
            if !alive {
                bar.window.destroy();
            }
            alive
        });

        for monitor in current {
            let known = self
                .windows
                .borrow()
                .iter()
                .any(|bar| bar.monitor == monitor);
            if !known {
                // build_bar_window maps the window itself (Reveal enter).
                let window = build_bar_window(
                    &self.app,
                    &monitor,
                    &self.sway,
                    &self.tasks,
                    &self.tray,
                    self.toggle_panel.clone(),
                );
                self.windows
                    .borrow_mut()
                    .push(BarWindow { monitor, window });
            }
        }
    }
}

fn build_bar_window(
    app: &gtk4::Application,
    monitor: &gdk::Monitor,
    sway: &Rc<SwayService>,
    tasks: &Rc<TaskStateService>,
    tray: &Rc<tray::TrayService>,
    toggle_panel: Rc<dyn Fn()>,
) -> gtk4::Window {
    let window = layer_shell::create_layer_window_on(app, &BAR_CONFIG, Some(monitor));
    // Resizable stays ON: the left+right anchors mean the compositor's
    // configure sets the width, and a non-resizable GTK window pins to its
    // natural (content) size instead — the card then ends after the clock
    // rather than spanning the output.
    window.set_decorated(false);

    // Pane/content split for the enter transition (motion on glass,
    // anim.rs): `root` carries the frosted .bar-root card so its tint can
    // land within GLASS_MS, while the clusters on it fade over the full
    // enter.
    let root = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .css_classes(["bar-root"])
        .build();

    // CenterBox, not Box: the center slot must stay screen-centered
    // regardless of how the left/right clusters grow.
    let content = gtk4::CenterBox::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .hexpand(true)
        .build();

    let left = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .build();
    left.append(&start::build(toggle_panel));
    left.append(&workspaces::build(sway));
    let center = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .build();
    center.append(&media::build(sway));
    let right = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .build();
    // Tray sits left of the track, matching waybar's modules-right order.
    right.append(&tray::build(tray));
    // Battery + board + clock fuse into one segmented track (waybar's
    // group/right-track); a batteryless machine skips the segment so the
    // board keeps the rounded left end.
    let track = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .css_classes(["bar-track"])
        .build();
    if let Some(bat) = battery::build() {
        track.append(&bat);
    }
    // gdk connector names match sway output names under wlroots.
    track.append(&board::build(
        sway,
        tasks,
        monitor.connector().map(|c| c.to_string()),
        &root,
    ));
    track.append(&clock::build());
    right.append(&track);

    content.set_start_widget(Some(&left));
    content.set_center_widget(Some(&center));
    content.set_end_widget(Some(&right));
    root.append(&content);

    let slide = anim::SlideBin::new();
    slide.set_child(&root);
    window.set_child(Some(&slide));

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

    // Enter: fade + short settle up from below the surface edge, per bar
    // window (so hotplugged outputs get it too). The exclusive zone is a
    // property of the mapped surface, so tiled windows take their final
    // size on frame one — only render nodes move. Bars never hide; the
    // Reveal drops once the enter finishes.
    anim::Reveal::new(&window, &root)
        .content(&content)
        .slide(&slide, anim::SLIDE_PX)
        .show();

    window
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
        // Keeps itself alive through its main-context event loop.
        let sway = SwayService::start();
        *slot = Some(BarManager::new(
            app,
            sway,
            Rc::new(start::toggle_panel_fallback),
        ));
    });

    // Empty argv: run() would parse std::env::args and treat the `bar`
    // subcommand word as a file to open, which FLAGS_NONE rejects.
    app.run_with_args::<&str>(&[]);
}
