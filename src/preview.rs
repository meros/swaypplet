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

mod components;

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{Application, ApplicationWindow, glib};

use crate::panel::Panel;
use crate::services::notifications::store::NotificationStore;
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
        // SWAYPPLET_PREVIEW_SETTINGS=1 reads the settings file first (the
        // render harness's SWPP_SETTINGS=1, with an XDG_CONFIG_HOME of its
        // own), so a preview shows the Look inputs it names (a tint, an
        // accent) rather than the binary's defaults.
        if std::env::var_os("SWAYPPLET_PREVIEW_SETTINGS").is_some() {
            crate::settings::store::init();
        }
        theme::load_css();
        let store = Rc::new(RefCell::new(NotificationStore::new()));

        // Full panel: the real panel on its own layer surface, shown.
        if component == "panel" {
            // The panel's own layer surface, as the panel process builds it.
            let panel = Panel::new(
                crate::app::panel_surface(app),
                store.clone(),
                crate::services::audio::AudioService::start(),
                // The harness shows the Helm alone; settings has its own
                // component (`settings`).
                Rc::new(|_| {}),
            );
            panel.toggle();
            std::mem::forget(panel);
            return;
        }

        // The report card (src/quality/report.rs), on its own glass.
        // `SWAYPPLET_PREVIEW_IMAGE=<png>` stands in for the capture; without
        // one the card opens as it does after Escape. Send prints the draft,
        // and with `SWAYPPLET_DRY_RUN=1` goes on to print the issue.
        if component == "report" {
            let image = std::env::var("SWAYPPLET_PREVIEW_IMAGE")
                .ok()
                .and_then(|p| crate::screenshot::deliver::load_png(std::path::Path::new(&p)).ok());
            let store = store.clone();
            let surface = crate::quality::report::card(app, image, move |draft| {
                eprintln!(
                    "report preview: send {} chars, picture {}, log {}",
                    draft.description.len(),
                    draft.image.is_some(),
                    draft.attach_log
                );
                if crate::quality::dry_run() {
                    crate::quality::report::send(&store, draft);
                }
            });
            std::mem::forget(surface);
            return;
        }

        // Lock screen: full-window content in a plain toplevel — no session
        // lock is taken, so it's safe to iterate on styling while unlocked.
        // Submitting "ok" flashes success; anything else shakes.
        //
        // What it shows is what the locker draws and no more: the clock's
        // plate and the card. The wallpaper under a real lock surface and the glass behind
        // its card are the compositor's, from `layer_effects "session-lock"`,
        // and no plain toplevel gets either.
        if component == "lock" {
            crate::theme::reload();
            // SWAYPPLET_PREVIEW_LAYER=1 hosts the lock content on a layer
            // surface named like the session lock, so the harness's real
            // `layer_effects "session-lock"` material draws behind it.
            let layer_surface = std::env::var_os("SWAYPPLET_PREVIEW_LAYER").map(|_| {
                crate::settings::glass::apply_saved();
                crate::shell::Surface::builder(app, crate::shell::Namespace::SessionLock)
                    .fill()
                    .no_card()
                    .build()
            });
            let window: gtk4::Window = match &layer_surface {
                Some(surface) => surface.window().clone(),
                None => ApplicationWindow::builder()
                    .application(app)
                    .default_width(1280)
                    .default_height(800)
                    .build()
                    .upcast(),
            };
            let set = crate::lock::ui::SurfaceSet::new();
            // Greeter-mode preview: SWAYPPLET_GREET_USERS=meros,melvin adds
            // the user row under the lock card.
            let users: Vec<String> = std::env::var("SWAYPPLET_GREET_USERS")
                .unwrap_or_default()
                .split(',')
                .map(str::trim)
                .filter(|u| !u.is_empty())
                .map(str::to_string)
                .collect();
            if let Some(first) = users.first() {
                set.enable_greeter(first);
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
            match &layer_surface {
                Some(surface) => {
                    content.set_hexpand(true);
                    content.set_vexpand(true);
                    surface.root().append(&content);
                }
                None => window.set_child(Some(&content)),
            }
            // The preview process lives as long as its one window.
            std::mem::forget(layer_surface);
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
                    "face" => set.show_face(Some(crate::ui::FaceState::Looking), "Looking for you"),
                    "face-ok" => set.show_face(Some(crate::ui::FaceState::Ok), "Recognised you"),
                    "face-fail" => {
                        set.show_face(Some(crate::ui::FaceState::Fail), "Didn't recognise you")
                    }
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
            use crate::polkit::dialog::{Callbacks, Card, Methods, PolkitDialog, StatusKind};
            if std::env::var_os("SWAYPPLET_PREVIEW_LAYER").is_some() {
                crate::settings::glass::apply_saved();
            }
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
            let card = Card {
                title: "Authentication Required",
                message: request.message.as_str(),
                icon_name: "",
                action_id: request.action_id.as_str(),
                command: std::env::var("SWAYPPLET_PREVIEW_POLKIT_STATE")
                    .unwrap_or_default()
                    .contains("command")
                    .then_some("sudo nixos-rebuild switch --flake .#laptop"),
                identities: &request.identities,
                details: crate::polkit::dialog::format_details(&request),
                password: true,
            };
            dialog.present(
                &card,
                Callbacks {
                    on_submit: std::rc::Rc::new(move |password: String| {
                        if password == "ok" {
                            d.set_status("Authenticated", StatusKind::Success);
                            d.flash_success();
                        } else {
                            d.reject("Authentication failed");
                        }
                    }),
                    on_cancel: std::rc::Rc::new(|| std::process::exit(0)),
                    ..Callbacks::default()
                },
            );
            // SWAYPPLET_PREVIEW_POLKIT_STATE picks which of the late
            // arrivals to draw. The card's geometry must be the same in every
            // one of them: that is the property the shots are taken to check.
            let mut methods = Methods {
                password: true,
                ..Methods::default()
            };
            for state in std::env::var("SWAYPPLET_PREVIEW_POLKIT_STATE")
                .unwrap_or_else(|_| "fp,prompt".into())
                .split(',')
                .map(str::trim)
            {
                match state {
                    "fp" => methods.fp = true,
                    "face" => methods.face = true,
                    "prompt" => dialog.set_password_prompt("Password"),
                    "error" => dialog.set_status("Authentication failed", StatusKind::Error),
                    _ => {}
                }
            }
            dialog.set_methods(methods);
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
                    ["Personal", "Work", "Clients", "Consulting", "Testing"]
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
        //
        // Settings fills the Helm card's width (740 to 1033 px), so its
        // preview opens at that width: at the 440 single-component default
        // the tab strip clips and the columns read wrong.
        // The component sheet (`components.<page>`, src/preview/components.rs)
        // lays its specimens out in lines across a 1440 output.
        let width = if component.starts_with("settings") {
            1100
        } else if component.starts_with("components") {
            1400
        } else {
            440
        };
        // SWAYPPLET_PREVIEW_LAYER=1 (the render harness's default) puts the
        // component in a panel card on a layer surface, the way the Helm shows
        // it, so the compositor's glass and the mode's material are behind it.
        // A plain toplevel has neither, and in light mode its dark text then
        // sits on nothing.
        let surface = std::env::var_os("SWAYPPLET_PREVIEW_LAYER").map(|_| {
            crate::settings::glass::apply_saved();
            crate::shell::Surface::builder(app, crate::shell::Namespace::Panel)
                .card(crate::ui::Card::Floating)
                .width(width)
                .build()
        });
        let window: gtk4::Window = match &surface {
            Some(surface) => surface.window().clone(),
            None => ApplicationWindow::builder()
                .application(app)
                .default_width(width)
                .default_height(600)
                .build()
                .upcast(),
        };

        let host = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(crate::tokens::space(4))
            .build();
        crate::ui::surface::adopt(&host);
        host.add_css_class("startmenu-quick");

        match component.as_str() {
            "tiles" => {
                let grid = gtk4::Grid::builder()
                    .row_spacing(crate::tokens::space(3))
                    .column_spacing(crate::tokens::space(3))
                    .column_homogeneous(true)
                    .build();
                grid.add_css_class("startmenu-tile-grid");
                // `SWAYPPLET_PREVIEW_TILES=on` draws every tile on with a
                // status line, from made-up words: no reading, and no
                // action (`set_active` emits `toggled`; the actions are on
                // `clicked`).
                let fixture = std::env::var_os("SWAYPPLET_PREVIEW_TILES").is_some();
                let fixture_status = |label: &str| match label {
                    "Night Light" => "Sun · 3500 K",
                    "No Sleep" => "Until 14:30",
                    "No Lock" => "Until turned off",
                    _ => "",
                };
                let specs = tiles::tile_specs();
                let mut col = 0i32;
                let mut row = 0i32;
                let mut place = |w: &gtk4::Widget| {
                    grid.attach(w, col, row, 1, 1);
                    col += 1;
                    if col >= 2 {
                        col = 0;
                        row += 1;
                    }
                };
                for spec in specs.iter() {
                    if spec.status.is_some() {
                        let tile = tiles::build_split(spec, |_| {});
                        if fixture {
                            tile.toggle.set_active(true);
                            crate::ui::set_tile_status(&tile, fixture_status(spec.label));
                        } else {
                            tiles::init_tile_state(&tile.toggle, spec);
                            tiles::refresh_status(&tile.status, spec);
                        }
                        place(tile.root.upcast_ref());
                    } else {
                        let btn = tiles::build_tile(spec);
                        if fixture {
                            btn.set_active(true);
                        } else {
                            tiles::init_tile_state(&btn, spec);
                        }
                        place(btn.upcast_ref());
                    }
                }
                let dnd = tiles::build_dnd_split(store.clone(), |_| {});
                if fixture {
                    dnd.toggle.set_active(true);
                    crate::ui::set_tile_status(&dnd, "Quiet until 07:00");
                }
                place(dnd.root.upcast_ref());
                host.append(&grid);
            }
            // Audio and Bluetooth draw fixtures: every state worth a look at
            // once, and nothing a click in the preview could change on the
            // real sound server or adapter. SWAYPPLET_PREVIEW_LIVE=1 reads
            // the real ones instead.
            "audio" => {
                let service = if live() {
                    crate::services::audio::AudioService::start()
                } else {
                    let s = crate::services::audio::AudioService::fixture(fixtures::audio());
                    s.set_fixture_level(0.42);
                    s
                };
                let s = Box::leak(Box::new(AudioSection::new(service)));
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
            // `settings` or `settings.<tab>` (look, idle, bar, input, alerts,
            // launcher, displays, glass). Displays and Input drive the nested
            // compositor the preview runs in, as `display` does.
            c if c == "settings" || c.starts_with("settings.") => {
                if c == "settings.displays" {
                    crate::services::displays::start(store.clone());
                }
                if c == "settings.input" {
                    crate::services::input::follow();
                }
                let s = Box::leak(Box::new(crate::settings::SettingsSection::new()));
                if let Some(tab) = c.strip_prefix("settings.") {
                    s.show(tab);
                }
                // The Input tab reads the devices on refresh, as the panel
                // does when it opens.
                if c == "settings.input" {
                    s.refresh();
                    // SWAYPPLET_PREVIEW_LAYOUT_QUERY=dvorak: the layout
                    // picker open on that search.
                    if let Some(q) = std::env::var_os("SWAYPPLET_PREVIEW_LAYOUT_QUERY")
                        .filter(|q| !q.is_empty())
                    {
                        s.demo_layout_pick(&q.to_string_lossy());
                    }
                }
                // SWAYPPLET_PREVIEW_DISPLAYS_APPLY=1: after two seconds the
                // Displays tab moves an output and applies it, so the keep
                // question and its revert can be shot (docs/SETTINGS.md).
                // `=leave` also hides the page three seconds later, as
                // closing the panel does, which must revert at once.
                if c == "settings.displays"
                    && let Some(how) = std::env::var_os("SWAYPPLET_PREVIEW_DISPLAYS_APPLY")
                {
                    let s: &'static crate::settings::SettingsSection = s;
                    glib::timeout_add_local_once(std::time::Duration::from_secs(2), move || {
                        s.demo_displays();
                    });
                    if how == "leave" {
                        glib::timeout_add_local_once(std::time::Duration::from_secs(5), move || {
                            s.widget().set_visible(false);
                        });
                    }
                }
                host.append(s.widget());
            }
            // The component sheet: every component in every state, one page
            // at a time (`components.controls`, `.inputs`, `.lists`).
            c if c == "components" || c.starts_with("components.") => {
                let page = c.strip_prefix("components.").unwrap_or(components::PAGES[0]);
                host.append(&components::page(page));
            }
            "network" => {
                let s = Box::leak(Box::new(NetworkSection::new()));
                // The panel opens this as a subsheet page (panel.rs), which
                // is the only state it is ever seen in. Without this the
                // harness rendered the collapsed summary row on an empty
                // canvas and every screenshot of the section was of nothing.
                s.expand_for_page();
                host.append(s.widget());
            }
            "bluetooth" => {
                let service = if live() {
                    crate::services::bluetooth::BluetoothService::start()
                } else {
                    crate::services::bluetooth::BluetoothService::fixture(fixtures::bluetooth())
                };
                let s = BluetoothSection::new(service);
                s.expand_for_page();
                host.append(s.widget());
                std::mem::forget(s);
            }
            "display" => {
                // The profiles come from the service, as in the panel; here
                // it drives the nested compositor the preview runs in.
                crate::services::displays::start(store.clone());
                let s = Box::leak(Box::new(DisplaySection::new()));
                s.expand_for_page();
                host.append(s.widget());
            }
            "media" => {
                let s = Box::leak(Box::new(MediaSection::new()));
                host.append(s.widget());
            }
            "notifications" => {
                // A sender with a burst and two single ones, so the centre's
                // grouping (`services::notifications::group`) is on screen.
                for (app, summary, body) in [
                    ("CI", "Pipeline #411 failed", "test: 2 of 418 failed on main."),
                    ("Kalender", "Möte nu — Åsa, Öresund", "Startade 10:45."),
                    ("CI", "Pipeline #412 failed", "test: 1 of 418 failed on main."),
                    ("Chat", "Ada Lovelace", "Short one."),
                    ("CI", "Pipeline #413 passed", "All 418 tests passed."),
                ] {
                    crate::services::notifications::store::store_add(
                        &store,
                        crate::services::notifications::Notification {
                            app_name: app.into(),
                            summary: summary.into(),
                            body: body.into(),
                            timestamp: std::time::SystemTime::now(),
                            ..Default::default()
                        },
                    );
                }
                let s = Box::leak(Box::new(NotificationsSection::new(store.clone())));
                s.expand_for_page();
                host.append(s.widget());
            }
            "clipboard" => {
                let s = Box::leak(Box::new(ClipboardSection::new()));
                s.expand_for_page();
                host.append(s.widget());
            }
            "users" => {
                let s = Box::leak(Box::new(crate::widgets::users::UserSection::new()));
                s.refresh();
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
                let card = crate::ui::overline("Behind the selector", crate::ui::Tone::Fg);
                card.set_xalign(0.5);
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
                     brightness, network, bluetooth, display, media, notifications, clipboard, power, \
                     components.controls, components.inputs, components.lists"
                ))));
            }
        }

        match surface {
            Some(surface) => {
                crate::ui::pad(&host, 5);
                surface.card().append(&host);
                surface.set_content(&host);
                surface.show();
                std::mem::forget(surface);
            }
            None => {
                window.set_child(Some(&host));
                window.present();
            }
        }
    });

    // Run without forwarding our own argv (which contains `--preview <name>`)
    // so GApplication doesn't try to parse it as GTK options.
    app.run_with_args(&["swaypplet"]);
}

/// Whether a preview reads the real services rather than fixtures.
fn live() -> bool {
    std::env::var_os("SWAYPPLET_PREVIEW_LIVE").is_some()
}

/// Made-up states for the previews: every row state at once.
mod fixtures {
    use crate::services::audio::{
        AudioState, Card, Device, DeviceKind, Profile, Stream, VolumeState,
    };
    use crate::services::bluetooth::{BtState, Op};
    use crate::services::bluez::{self, Snapshot};

    fn vol(volume: f64, muted: bool) -> VolumeState {
        VolumeState { volume, muted }
    }

    fn device(i: u32, name: &str, kind: DeviceKind, detail: &str, default: bool) -> Device {
        Device {
            id: format!("fixture.{i}"),
            index: i,
            name: name.into(),
            is_default: default,
            channels: 2,
            volume: vol(0.64, false),
            kind,
            detail: detail.into(),
            card: (kind == DeviceKind::Headphones).then_some(7),
            available: kind != DeviceKind::Hdmi,
        }
    }

    fn stream(i: u32, name: &str, icon: &str, volume: f64, muted: bool) -> Stream {
        Stream {
            index: i,
            name: name.into(),
            channels: 2,
            volume: vol(volume, muted),
            recording: false,
            icon: Some(icon.into()),
        }
    }

    pub fn audio() -> AudioState {
        let sinks = vec![
            device(1, "WH-1000XM5", DeviceKind::Headphones, "Bluetooth", true),
            device(2, "Speakers", DeviceKind::Speakers, "Built-in", false),
            device(3, "LG 28H2U", DeviceKind::Hdmi, "HDMI", false),
            device(4, "USB DAC", DeviceKind::Usb, "USB", false),
        ];
        let sources = vec![
            device(11, "Microphone", DeviceKind::Microphone, "Built-in", true),
            device(12, "Webcam", DeviceKind::Webcam, "USB", false),
        ];
        AudioState {
            sink: Some(vol(0.64, false)),
            source: Some(vol(0.8, false)),
            sinks,
            sources,
            streams: vec![
                stream(21, "Spotify", "spotify-client", 0.9, false),
                stream(22, "Firefox", "firefox", 0.5, true),
                stream(23, "Slack", "com.slack.Slack", 0.3, false),
            ],
            recorders: Vec::new(),
            cards: vec![Card {
                index: 7,
                profiles: vec![
                    Profile {
                        name: "a2dp-sink".into(),
                        description: "High Fidelity Playback (A2DP Sink)".into(),
                        available: true,
                    },
                    Profile {
                        name: "headset-head-unit".into(),
                        description: "Headset Head Unit (HSP/HFP)".into(),
                        available: true,
                    },
                ],
                active: Some("a2dp-sink".into()),
            }],
            connected: true,
        }
    }

    fn bt(i: u8, name: &str, hint: &str, connected: bool, paired: bool) -> bluez::Device {
        let mac = format!("00:1A:7D:DA:71:{i:02X}");
        bluez::Device {
            path: format!("/org/bluez/hci0/dev_{}", mac.replace(':', "_")),
            mac,
            name: name.into(),
            icon_hint: Some(hint.into()),
            connected,
            paired,
            trusted: paired,
            battery: None,
            rssi: (!paired).then_some(-50 - i16::from(i)),
            named: true,
        }
    }

    pub fn bluetooth() -> BtState {
        let mut buds = bt(1, "WH-1000XM5", "audio-headphones", true, true);
        buds.battery = Some(72);
        let keyboard = bt(2, "MX Keys", "input-keyboard", true, true);
        let speaker = bt(3, "Kitchen speaker", "audio-card", false, true);
        let mouse = bt(4, "MX Master 3S", "input-mouse", false, true);
        let phone = bt(5, "Pixel 9", "phone", false, false);
        let watch = bt(6, "Garmin Venu", "watch", false, false);
        let ops = [
            (speaker.mac.clone(), Op::Connecting),
            (mouse.mac.clone(), Op::Failed("Not responding. Is it on and in range?".into())),
            (phone.mac.clone(), Op::Confirm("482 915".into())),
        ]
        .into_iter()
        .collect();
        BtState {
            snapshot: Snapshot {
                available: true,
                powered: true,
                discovering: true,
                devices: vec![buds, keyboard, speaker, mouse, phone, watch],
                adapter: Some("/org/bluez/hci0".into()),
            },
            ops,
        }
    }
}

