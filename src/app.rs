use std::cell::RefCell;
use std::fs;
use std::rc::Rc;

use gio::prelude::*;
use glib::unix_signal_add_local;
use gtk4::prelude::*;
use gtk4::Application;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

use crate::osd::{Osd, OsdCommand};
use crate::panel::Panel;
use crate::theme;

/// Tell the compositor this window surface may have transparent pixels,
/// so alpha compositing is applied (needed for rounded corners).
fn make_surface_transparent(window: &gtk4::Window) {
    window.connect_realize(|win| {
        if let Some(surface) = win.surface() {
            use gdk4::prelude::SurfaceExt;
            surface.set_opaque_region(None::<&cairo::Region>);
        }
    });
}

const APP_ID: &str = "dev.swaypplet.panel";

struct AppState {
    panel: Option<Panel>,
    osd: Option<Osd>,
}

pub fn run() {
    let app = Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();

    let state: Rc<RefCell<AppState>> = Rc::new(RefCell::new(AppState {
        panel: None,
        osd: None,
    }));

    let state_clone = state.clone();
    app.connect_startup(move |_app| {
        let _ = fs::write("/tmp/swaypplet.pid", std::process::id().to_string());
        theme::load_css();

        // SIGUSR1 toggles panel visibility
        let s = state_clone.clone();
        unix_signal_add_local(10 /* SIGUSR1 */, move || {
            if let Some(ref panel) = s.borrow().panel {
                panel.toggle();
            }
            glib::ControlFlow::Continue
        });
    });

    let state_clone = state.clone();
    app.connect_activate(move |app| {
        let mut st = state_clone.borrow_mut();
        if let Some(ref panel) = st.panel {
            panel.toggle();
            return;
        }

        // ── Main panel window ────────────────────────────────────────────────
        let window = gtk4::Window::builder()
            .application(app)
            .default_width(380)
            .resizable(false)
            .build();
        window.add_css_class("panel");

        window.init_layer_shell();
        window.set_layer(Layer::Overlay);
        window.set_namespace("swaypplet");
        window.set_anchor(Edge::Bottom, true);
        window.set_anchor(Edge::Right, true);
        window.set_margin(Edge::Bottom, 48);
        window.set_margin(Edge::Right, 8);
        window.set_keyboard_mode(KeyboardMode::OnDemand);

        make_surface_transparent(&window);
        let panel = Panel::new(window);
        panel.window.present();

        // ── Applet toggle button ─────────────────────────────────────────────
        let applet_window = gtk4::Window::builder()
            .application(app)
            .resizable(false)
            .decorated(false)
            .build();
        applet_window.add_css_class("applet-window");

        applet_window.init_layer_shell();
        applet_window.set_layer(Layer::Top);
        applet_window.set_namespace("swaypplet-applet");
        applet_window.set_anchor(Edge::Bottom, true);
        applet_window.set_anchor(Edge::Right, true);
        applet_window.set_margin(Edge::Bottom, 4);
        applet_window.set_margin(Edge::Right, 4);

        let applet_btn = gtk4::Button::builder()
            .label("\u{f013}") //  gear icon
            .build();
        applet_btn.add_css_class("applet-btn");

        {
            let sc = state_clone.clone();
            applet_btn.connect_clicked(move |_| {
                if let Some(ref p) = sc.borrow().panel {
                    p.toggle();
                }
            });
        }

        make_surface_transparent(&applet_window);
        applet_window.set_child(Some(&applet_btn));
        applet_window.present();

        // ── Escape key to close panel ────────────────────────────────────────
        let key_controller = gtk4::EventControllerKey::new();
        {
            let window_clone = panel.window.clone();
            key_controller.connect_key_pressed(move |_, key, _, _| {
                if key == gtk4::gdk::Key::Escape {
                    window_clone.set_visible(false);
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            });
        }
        panel.window.add_controller(key_controller);

        // ── OSD overlay ──────────────────────────────────────────────────────
        let osd = Osd::new(app);

        st.panel = Some(panel);
        st.osd = Some(osd);
    });

    // ── Command-line handling ────────────────────────────────────────────────
    // First launch: no args → activate (creates panel)
    // Subsequent: "osd --flag action" → trigger OSD
    // Subsequent: no args → toggle panel
    let state_clone = state.clone();
    app.connect_command_line(move |app, cmdline| {
        let args: Vec<String> = cmdline
            .arguments()
            .iter()
            .map(|a| a.to_string_lossy().to_string())
            .collect();

        // Check for OSD subcommand (args[0] is binary name)
        if args.len() > 1 && args[1] == "osd" {
            let osd_args: Vec<String> = args[2..].to_vec();
            if let Some(cmd) = OsdCommand::parse(&osd_args) {
                // Ensure windows are created
                let st = state_clone.borrow();
                if st.osd.is_none() {
                    drop(st);
                    app.activate();
                    let st = state_clone.borrow();
                    if let Some(ref osd) = st.osd {
                        osd.trigger(&cmd);
                    }
                } else if let Some(ref osd) = st.osd {
                    osd.trigger(&cmd);
                }
            } else {
                log::warn!("Unknown OSD command: {:?}", &args[2..]);
            }
        } else {
            // No OSD args → activate (toggle panel or create it)
            app.activate();
        }

        0
    });

    app.connect_shutdown(|_| {
        let _ = fs::remove_file("/tmp/swaypplet.pid");
    });

    app.run();
}
