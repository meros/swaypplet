//! Native status bar — waybar replacement.
//!
//! One layer surface per output, bottom-anchored frosted card matching the
//! waybar geometry it replaces (height 38, margins 0 4 4 4, radius 14 —
//! see users/modules/waybar.nix in the nixos repo). `Layer::Top` with an
//! auto exclusive zone so tiled windows sit above the bar + margins, while
//! the Overlay panel/OSD/launcher surfaces still stack over it.
//!
//! During development the bar runs standalone (`swaypplet bar`, own
//! GApplication id) next to the live panel + waybar.

mod battery;
mod clock;
mod media;
mod start;
mod task;
mod workspaces;

use std::cell::RefCell;
use std::rc::Rc;

use gio::prelude::*;
use gtk4::gdk;
use gtk4::prelude::*;
use gtk4_layer_shell::{Edge, Layer};

use crate::layer_shell::{self, LayerShellConfig};
use crate::sway_ipc::SwayService;
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
        let manager = Rc::new(Self {
            app: app.clone(),
            monitors: display.monitors(),
            windows: RefCell::new(Vec::new()),
            sway,
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
                let window =
                    build_bar_window(&self.app, &monitor, &self.sway, self.toggle_panel.clone());
                window.present();
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
    toggle_panel: Rc<dyn Fn()>,
) -> gtk4::Window {
    let window = layer_shell::create_layer_window_on(app, &BAR_CONFIG, Some(monitor));
    window.set_resizable(false);
    window.set_decorated(false);

    // CenterBox, not Box: the center slot must stay screen-centered
    // regardless of how the left/right clusters grow.
    let root = gtk4::CenterBox::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .css_classes(["bar-root"])
        .build();

    let left = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .css_classes(["bar-left"])
        .build();
    left.append(&start::build(toggle_panel));
    left.append(&workspaces::build(sway));
    let center = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .css_classes(["bar-center"])
        .build();
    center.append(&media::build(sway));
    let right = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .css_classes(["bar-right"])
        .build();
    // Battery + task pill + clock fuse into one segmented track (waybar's
    // group/right-track); a batteryless machine skips the segment so the
    // task pill keeps the rounded left end.
    let track = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .css_classes(["bar-track"])
        .build();
    if let Some(bat) = battery::build() {
        track.append(&bat);
    }
    // gdk connector names match sway output names under wlroots.
    track.append(&task::build(
        sway,
        monitor.connector().map(|c| c.to_string()),
        &root,
    ));
    track.append(&clock::build());
    right.append(&track);

    root.set_start_widget(Some(&left));
    root.set_center_widget(Some(&center));
    root.set_end_widget(Some(&right));

    window.set_child(Some(&root));
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
