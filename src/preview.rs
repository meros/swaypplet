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
            let panel = Panel::new(window, store.clone(), crate::audio::AudioService::start());
            panel.window.set_visible(true);
            std::mem::forget(panel);
            return;
        }

        // Lock screen: full-window content in a plain toplevel — no session
        // lock is taken, so it's safe to iterate on styling while unlocked.
        // Submitting "ok" flashes success; anything else shakes.
        //
        // What it shows is what the locker draws and no more: scrim, clock,
        // card. The wallpaper under a real lock surface and the glass behind
        // its card are the compositor's, from `layer_effects "session-lock"`,
        // and no plain toplevel gets either.
        if component == "lock" {
            let window: gtk4::Window = ApplicationWindow::builder()
                .application(app)
                .default_width(1280)
                .default_height(800)
                .build()
                .upcast();
            window.add_css_class("lock");
            let set = crate::lock::ui::SurfaceSet::new();
            // Greeter-mode preview: SWAYPPLET_GREET_USERS=meros,melvin adds
            // the user chips + username row on top of the lock card.
            let users: Vec<String> = std::env::var("SWAYPPLET_GREET_USERS")
                .unwrap_or_default()
                .split(',')
                .map(str::trim)
                .filter(|u| !u.is_empty())
                .map(str::to_string)
                .collect();
            if let Some(first) = users.first() {
                set.enable_user_field(first);
                // Mock avatar data so the preview exercises the new chips:
                // the first user shows a logged-in presence dot, and
                // SWAYPPLET_PREVIEW_AVATAR=<image> gives it a real picture
                // (the others keep the monogram fallback).
                let icon = std::env::var("SWAYPPLET_PREVIEW_AVATAR").ok();
                let chips: Vec<crate::lock::ui::UserChip> = users
                    .iter()
                    .enumerate()
                    .map(|(i, u)| crate::lock::ui::UserChip {
                        user: u.clone(),
                        logged_in: i == 0,
                        icon: if i == 0 { icon.clone() } else { None },
                    })
                    .collect();
                let sel = set.clone();
                set.enable_user_chips(&chips, Rc::new(move |u| sel.set_username(&u)));
            }
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
                true,
            );
            window.set_child(Some(&content));
            window.present();
            // SWAYPPLET_PREVIEW_LOCK_STATE drives the card into one of the
            // states that used to arrive after it was on screen. The point of
            // rendering them separately is that the card's geometry is
            // identical in every shot: diff two captures and only the
            // contents of the reserved rows may differ.
            // Caps first: a rejection composes the warning onto its own line,
            // so the card has to know about the key before it hears about the
            // rejection. The real surfaces tick once a second and always do.
            set.tick();
            for state in std::env::var("SWAYPPLET_PREVIEW_LOCK_STATE")
                .unwrap_or_default()
                .split(',')
                .map(str::trim)
            {
                match state {
                    "fp" => set.set_fp_armed(true),
                    "fp-hint" => {
                        set.set_fp_armed(true);
                        set.fp_hint("Remove and try again");
                    }
                    "face" => set.show_face(true, "looking", "Looking for you"),
                    "face-ok" => set.show_face(true, "ok", "Recognised you"),
                    "face-fail" => set.show_face(true, "fail", "Didn't recognise you"),
                    "error" => set.set_status(
                        "Wrong password (3 attempts)",
                        crate::lock::ui::StatusKind::Error,
                    ),
                    "info" => set.set_status("Switching\u{2026}", crate::lock::ui::StatusKind::Info),
                    // The case the reserved second line exists for, and the
                    // one a stray `line-height` in the stylesheet breaks.
                    "long" => set.set_status(
                        "Your account has expired; please contact your system administrator",
                        crate::lock::ui::StatusKind::Error,
                    ),
                    _ => {}
                }
            }
            std::mem::forget(set);
            return;
        }

        // Polkit dialog: present the real layer-shell dialog (works under the
        // nested-sway harness) with a fake request. Both auth affordances are
        // shown so one screenshot covers fingerprint pill + password entry.
        // Password "ok" flashes success; anything else shakes.
        if component == "polkit" {
            use crate::polkit::dialog::PolkitDialog;
            let dialog = PolkitDialog::new(app);
            let request = crate::polkit::agent::AuthRequest {
                action_id: "org.freedesktop.policykit.exec".into(),
                message: "Authentication is required to run a program as another user".into(),
                icon_name: String::new(),
                details: std::collections::HashMap::from([(
                    "command_line".to_string(),
                    "/run/current-system/sw/bin/true".to_string(),
                )]),
                cookie: "preview".into(),
                identities: Vec::new(),
            };
            let d = dialog.clone();
            dialog.present(
                &request,
                Box::new(move |password: String| {
                    if password == "ok" {
                        d.set_status("Authenticated", crate::polkit::dialog::StatusKind::Success);
                        d.flash_success();
                    } else {
                        d.set_status(
                            "Authentication failed",
                            crate::polkit::dialog::StatusKind::Error,
                        );
                        d.shake();
                        d.set_verifying(false);
                    }
                }),
                Box::new(|| std::process::exit(0)),
                Box::new(|_uid| {}),
                Box::new(|| {}),
            );
            // SWAYPPLET_PREVIEW_POLKIT_STATE picks which of the late
            // arrivals to draw. The card's geometry must be the same in every
            // one of them: that is the property the shots are taken to check.
            for state in std::env::var("SWAYPPLET_PREVIEW_POLKIT_STATE")
                .unwrap_or_else(|_| "fp,prompt".into())
                .split(',')
                .map(str::trim)
            {
                match state {
                    "fp" => dialog.show_fingerprint(true, "Touch fingerprint reader"),
                    "prompt" => dialog.set_password_prompt("Password"),
                    "error" => dialog.set_status(
                        "Authentication failed",
                        crate::polkit::dialog::StatusKind::Error,
                    ),
                    _ => {}
                }
            }
            std::mem::forget(dialog);
            return;
        }

        // dmenu picker: the real layer-shell surface (works under the nested
        // sway harness) pre-filled with sample items, so the card, the prompt
        // header and the row rhythm can be screenshotted without a pipe.
        if component == "dmenu" {
            let items: Vec<String> = std::env::var("SWAYPPLET_PREVIEW_ITEMS")
                .ok()
                .filter(|v| !v.is_empty())
                .map(|v| v.split(',').map(|s| s.trim().to_string()).collect())
                .unwrap_or_else(|| {
                    ["Personal", "Work", "Norban", "Consulting", "Testing"]
                        .iter()
                        .map(|s| s.to_string())
                        .collect()
                });
            let prompt =
                std::env::var("SWAYPPLET_PREVIEW_PROMPT").unwrap_or_else(|_| "Chrome Profile".into());
            let picker = crate::dmenu::present_picker(app, &prompt, items, |_| std::process::exit(0));
            if let Ok(query) = std::env::var("SWAYPPLET_PREVIEW_QUERY") {
                picker.set_query(&query);
            }
            std::mem::forget(picker);
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
                let s = Box::leak(Box::new(AudioSection::new(crate::audio::AudioService::start())));
                s.expand_for_preview();
                host.append(s.widget());
            }
            "brightness" => {
                let s = Box::leak(Box::new(BrightnessSection::new()));
                host.append(s.widget());
            }
            // The settings pane. `SWAYPPLET_GLASS_CONFIG` points it at a
            // system config, so the glass group can be rendered without
            // /etc/swaypplet/glass.json existing on the build host; without
            // one it draws the "no glass configuration" note, which is the
            // other state worth a screenshot.
            "settings" => {
                let s = Box::leak(Box::new(crate::settings::SettingsSection::new()));
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
            "selector" => {
                // Exercises the whole flow in-process, which is what tells a
                // broken selector apart from a request that never arrived.
                // The window is filled first and left on screen: a dimmed
                // capture of an empty desktop is a black rectangle, which is
                // also what a selector that never mapped looks like.
                let card = gtk4::Label::new(Some("BEHIND THE SELECTOR"));
                card.add_css_class("keybinds-heading");
                card.set_vexpand(true);
                card.set_hexpand(true);
                host.append(&card);
                window.set_child(Some(&host));
                window.present();

                if let Some(app) = window.application() {
                    let store = store.clone();
                    glib::timeout_add_local_once(
                        std::time::Duration::from_millis(1500),
                        move || {
                            crate::screenshot::take(
                                &app,
                                &store,
                                crate::screenshot::Shot::Region,
                            );
                        },
                    );
                }
                return;
            }
            "annotate" => {
                // A gradient with a hard edge: enough structure to tell a
                // pixelated block from an untouched one at a glance.
                let (w, h) = (640u32, 400u32);
                let mut pixels = Vec::with_capacity((w * h * 4) as usize);
                for y in 0..h {
                    for x in 0..w {
                        let band = if (x / 40 + y / 40) % 2 == 0 { 60 } else { 0 };
                        pixels.extend_from_slice(&[
                            (x * 255 / w) as u8,
                            (y * 255 / h) as u8,
                            band,
                            255,
                        ]);
                    }
                }
                let image = crate::screenshot::capture::Image {
                    width: w,
                    height: h,
                    pixels,
                };
                if let Some(app) = window.application() {
                    crate::screenshot::annotate::open(&app, image, |_| {});
                }
                return;
            }
            other => {
                host.append(&gtk4::Label::new(Some(&format!(
                    "unknown preview component: {other}\n\nknown: panel, lock, polkit, tiles, audio, \
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
