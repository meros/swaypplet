//! Dev-only component preview harness.
//!
//! `swaypplet --preview <component>` renders one piece of UI (or the whole
//! panel) in a plain toplevel window — no layer-shell, no SIGUSR1 toggle, no
//! single-instance handoff to a running session copy. Paired with the headless
//! render script (`dev/render.sh --mode preview:<component>`) this gives a
//! component-up visual-validation loop: screenshot a widget in isolation,
//! iterate on its CSS/layout, then assemble.
//!
//! Sections are intentionally leaked (`Box::leak` / `mem::forget`): the process
//! renders once and exits, so freeing them buys nothing and complicates the
//! `'static` lifetime the hosted widget references need.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{Application, ApplicationWindow};

use crate::notifications::store::NotificationStore;
use crate::panel::Panel;
use crate::theme;
use crate::widgets::{
    audio::AudioSection, bluetooth::BluetoothSection, brightness::BrightnessSection,
    clipboard::ClipboardSection, display::DisplaySection, media::MediaSection,
    network::NetworkSection, notifications::NotificationsSection, power::PowerSection, tiles,
};

pub fn run(component: &str) {
    let component = component.to_string();
    let app = Application::builder()
        .application_id("dev.swaypplet.preview")
        .build();

    app.connect_activate(move |app| {
        theme::load_css();
        let store = Rc::new(RefCell::new(NotificationStore::new()));

        // Full panel: host the real panel content in a normal toplevel.
        if component == "panel" {
            let window: gtk4::Window = ApplicationWindow::builder()
                .application(app)
                .default_width(820)
                .default_height(720)
                .build()
                .upcast();
            window.add_css_class("panel");
            let panel = Panel::new(window, store.clone());
            panel.window.set_visible(true);
            std::mem::forget(panel);
            return;
        }

        // Lock screen: full-window content in a plain toplevel — no session
        // lock is taken, so it's safe to iterate on styling while unlocked.
        // Submitting "ok" flashes success; anything else shakes.
        if component == "lock" {
            let window: gtk4::Window = ApplicationWindow::builder()
                .application(app)
                .default_width(1280)
                .default_height(800)
                .build()
                .upcast();
            window.add_css_class("lock");
            let set = crate::lock::ui::SurfaceSet::new();
            let feedback = set.clone();
            let content = set.build_content(
                &window,
                Rc::new(move |password: String| {
                    if password == "ok" {
                        feedback.flash_success();
                    } else {
                        feedback.set_status("Wrong password", crate::lock::ui::StatusKind::Error);
                        feedback.shake();
                    }
                }),
            );
            window.set_child(Some(&content));
            window.present();
            std::mem::forget(set);
            return;
        }

        // Single component: wrap its widget in a small window carrying the panel
        // surface classes so it inherits the same styling context.
        let window = ApplicationWindow::builder()
            .application(app)
            .default_width(440)
            .default_height(600)
            .build();
        window.add_css_class("panel");
        window.add_css_class("startmenu");

        let host = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(10)
            .build();
        host.add_css_class("startmenu-quick");
        host.add_css_class("preview-host");

        match component.as_str() {
            "tiles" => {
                let grid = gtk4::Grid::builder()
                    .row_spacing(8)
                    .column_spacing(8)
                    .column_homogeneous(true)
                    .build();
                grid.add_css_class("startmenu-tile-grid");
                let specs = tiles::tile_specs();
                let mut col = 0i32;
                let mut row = 0i32;
                for spec in specs.iter() {
                    let btn = tiles::build_tile(spec);
                    tiles::init_tile_state(&btn, spec);
                    grid.attach(&btn, col, row, 1, 1);
                    col += 1;
                    if col >= 2 {
                        col = 0;
                        row += 1;
                    }
                }
                let dnd = tiles::build_dnd_tile(store.clone());
                grid.attach(&dnd, col, row, 1, 1);
                host.append(&grid);
            }
            "audio" => {
                let s = Box::leak(Box::new(AudioSection::new()));
                host.append(s.widget());
            }
            "brightness" => {
                let s = Box::leak(Box::new(BrightnessSection::new()));
                host.append(s.widget());
            }
            "network" => {
                let s = Box::leak(Box::new(NetworkSection::new()));
                host.append(s.widget());
            }
            "bluetooth" => {
                let s = BluetoothSection::new();
                host.append(s.widget());
                std::mem::forget(s);
            }
            "display" => {
                let s = Box::leak(Box::new(DisplaySection::new()));
                s.expand_for_page();
                host.append(s.widget());
            }
            "media" => {
                let s = Box::leak(Box::new(MediaSection::new()));
                host.append(s.widget());
            }
            "notifications" => {
                let s = Box::leak(Box::new(NotificationsSection::new(store.clone())));
                s.expand_for_page();
                host.append(s.widget());
            }
            "clipboard" => {
                let s = Box::leak(Box::new(ClipboardSection::new()));
                s.expand_for_page();
                host.append(s.widget());
            }
            "power" => {
                let s = Box::leak(Box::new(PowerSection::new()));
                s.expand_for_page();
                host.append(s.widget());
            }
            other => {
                host.append(&gtk4::Label::new(Some(&format!(
                    "unknown preview component: {other}\n\nknown: panel, lock, tiles, audio, \
                     brightness, network, bluetooth, display, media, notifications, clipboard, power"
                ))));
            }
        }

        window.set_child(Some(&host));
        window.present();
    });

    // Run without forwarding our own argv (which contains `--preview <name>`)
    // so GApplication doesn't try to parse it as GTK options.
    app.run_with_args(&["swaypplet"]);
}
