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
            .build();
        window.add_css_class("panel");

        window.init_layer_shell();
        window.set_layer(Layer::Overlay);
        window.set_namespace("swaypplet");
        window.set_anchor(Edge::Top, true);
        window.set_anchor(Edge::Bottom, true);
        window.set_anchor(Edge::Right, true);
        window.set_margin(Edge::Top, 8);
        window.set_margin(Edge::Bottom, 48);
        window.set_margin(Edge::Right, 8);
        window.set_keyboard_mode(KeyboardMode::OnDemand);

        let panel = Panel::new(window);
        panel.window.present();
        panel.window.set_visible(false);

        // ── OSD overlay ──────────────────────────────────────────────────────
        let osd = Osd::new(app);

        st.panel = Some(panel);
        st.osd = Some(osd);
    });

    // ── Command-line handling ────────────────────────────────────────────────
    let state_clone = state.clone();
    app.connect_command_line(move |app, cmdline| {
        let args: Vec<String> = cmdline
            .arguments()
            .iter()
            .map(|a| a.to_string_lossy().to_string())
            .collect();

        if args.len() > 1 && args[1] == "osd" {
            let osd_args: Vec<String> = args[2..].to_vec();
            if let Some(cmd) = OsdCommand::parse(&osd_args) {
                let st = state_clone.borrow();
                if st.osd.is_none() {
                    drop(st);
                    app.activate();
                    let st = state_clone.borrow();
                    if let Some(ref osd) = st.osd {
                        osd.trigger(&cmd);
                    }
                    // Sync panel sliders with the new value
                    if let Some(ref panel) = st.panel {
                        match cmd {
                            OsdCommand::OutputVolumeRaise
                            | OsdCommand::OutputVolumeLower
                            | OsdCommand::OutputVolumeMuteToggle
                            | OsdCommand::InputVolumeMuteToggle => panel.refresh_audio(),
                            OsdCommand::BrightnessRaise
                            | OsdCommand::BrightnessLower => panel.refresh_brightness(),
                            _ => {}
                        }
                    }
                } else {
                    if let Some(ref osd) = st.osd {
                        osd.trigger(&cmd);
                    }
                    // Sync panel sliders with the new value
                    if let Some(ref panel) = st.panel {
                        match cmd {
                            OsdCommand::OutputVolumeRaise
                            | OsdCommand::OutputVolumeLower
                            | OsdCommand::OutputVolumeMuteToggle
                            | OsdCommand::InputVolumeMuteToggle => panel.refresh_audio(),
                            OsdCommand::BrightnessRaise
                            | OsdCommand::BrightnessLower => panel.refresh_brightness(),
                            _ => {}
                        }
                    }
                }
            } else {
                log::warn!("Unknown OSD command: {:?}", &args[2..]);
            }
        } else {
            app.activate();
        }

        0
    });

    app.connect_shutdown(|_| {
        let _ = fs::remove_file("/tmp/swaypplet.pid");
    });

    app.run();
}
