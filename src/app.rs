use std::cell::RefCell;
use std::fs;
use std::rc::Rc;

use gio::prelude::*;
use glib::unix_signal_add_local;
use gtk4::prelude::*;
use gtk4::Application;
use gtk4_layer_shell::Edge;

use crate::launcher::Launcher;
use crate::layer_shell::{self, LayerShellConfig};
use crate::notifications::store::NotificationStore;
use crate::notifications::{dbus, popup::PopupManager};
use crate::osd::{Osd, OsdCommand};
use crate::panel::Panel;
use crate::theme;

const APP_ID: &str = "dev.swaypplet.panel";

static PANEL_CONFIG: LayerShellConfig = LayerShellConfig {
    namespace: "swaypplet",
    default_width: Some(380),
    default_height: None,
    anchors: &[
        (Edge::Top, true),
        (Edge::Bottom, true),
        (Edge::Right, true),
    ],
    margins: &[(Edge::Top, 8), (Edge::Bottom, 48), (Edge::Right, 8)],
    keyboard_mode: gtk4_layer_shell::KeyboardMode::OnDemand,
};

struct AppState {
    panel: Option<Panel>,
    osd: Option<Osd>,
    launcher: Option<Launcher>,
}

pub fn run() {
    let app = Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();

    let state: Rc<RefCell<AppState>> = Rc::new(RefCell::new(AppState {
        panel: None,
        osd: None,
        launcher: None,
    }));

    // Shared notification store — lives on the GTK main thread (Rc, no Arc)
    let store = Rc::new(RefCell::new(NotificationStore::new()));

    let state_clone = state.clone();
    let store_startup = store.clone();
    app.connect_startup(move |_app| {
        let _ = fs::write("/tmp/swaypplet.pid", std::process::id().to_string());
        theme::load_css();

        // Start D-Bus notification server
        dbus::start_server(store_startup.clone());

        // SIGUSR1 toggles panel visibility
        let s = state_clone.clone();
        unix_signal_add_local(10 /* SIGUSR1 */, move || {
            if let Some(ref panel) = s.borrow().panel {
                panel.toggle();
            }
            glib::ControlFlow::Continue
        });

        // SIGUSR2 toggles launcher
        let s = state_clone.clone();
        unix_signal_add_local(12 /* SIGUSR2 */, move || {
            if let Some(ref launcher) = s.borrow().launcher {
                launcher.toggle();
            }
            glib::ControlFlow::Continue
        });
    });

    let state_clone = state.clone();
    let store_activate = store.clone();
    app.connect_activate(move |app| {
        let mut st = state_clone.borrow_mut();
        if let Some(ref panel) = st.panel {
            panel.toggle();
            return;
        }

        // ── Main panel window ────────────────────────────────────────────────
        let window = layer_shell::create_layer_window(app, &PANEL_CONFIG);
        window.add_css_class("panel");

        let panel = Panel::new(window, store_activate.clone());
        panel.window.present();
        panel.window.set_visible(false);

        // ── Popup manager ────────────────────────────────────────────────────
        PopupManager::register(app, store_activate.clone());

        // ── OSD overlay ──────────────────────────────────────────────────────
        let osd = Osd::new(app);

        // ── Launcher ────────────────────────────────────────────────────────
        let launcher = Launcher::new(app);

        st.panel = Some(panel);
        st.osd = Some(osd);
        st.launcher = Some(launcher);
    });

    // ── Command-line handling ────────────────────────────────────────────────
    let state_clone = state.clone();
    app.connect_command_line(move |app, cmdline| {
        let args: Vec<String> = cmdline
            .arguments()
            .iter()
            .map(|a| a.to_string_lossy().to_string())
            .collect();

        if args.len() > 1 && args[1] == "launcher" {
            let st = state_clone.borrow();
            if st.launcher.is_none() {
                drop(st);
                app.activate();
                let st = state_clone.borrow();
                if let Some(ref launcher) = st.launcher {
                    launcher.toggle();
                }
            } else if let Some(ref launcher) = st.launcher {
                launcher.toggle();
            }
        } else if args.len() > 1 && args[1] == "osd" {
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
