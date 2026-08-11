use std::cell::RefCell;
use std::fs;
use std::rc::Rc;

use gio::prelude::*;
use gtk4::Application;
use gtk4::prelude::*;
use gtk4_layer_shell::Edge;

use crate::bar::BarManager;
use crate::keybinds::Keybinds;
use crate::launcher::Launcher;
use crate::layer_shell::{self, LayerShellConfig};
use crate::notifications::store::NotificationStore;
use crate::notifications::{dbus, popup::PopupManager};
use crate::osd::{Osd, OsdCommand};
use crate::panel::Panel;
use crate::sway_ipc::SwayService;
use crate::theme;

const APP_ID: &str = "dev.swaypplet.panel";

/// Pid file lives in the per-user runtime dir (mode 0700), so no other user
/// can forge or clobber it. The /tmp fallback matches the wrapper scripts'
/// `$XDG_RUNTIME_DIR/swaypplet.pid` with the same /tmp fallback. Shared with
/// the standalone bar's start button (`bar::start`), which signals this pid.
pub(crate) fn pid_file_path() -> std::path::PathBuf {
    let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
    std::path::Path::new(&dir).join("swaypplet.pid")
}

// Start-menu popup: bottom-left anchored, ~780x700, clearing the waybar.
static PANEL_CONFIG: LayerShellConfig = LayerShellConfig {
    namespace: "swaypplet",
    layer: gtk4_layer_shell::Layer::Overlay,
    exclusive: false,
    default_width: Some(780),
    default_height: Some(700),
    anchors: &[(Edge::Bottom, true), (Edge::Left, true)],
    // The native bar reserves an exclusive zone of 42px (38px card + 4px
    // bottom margin, auto zone in src/bar/mod.rs — the same numbers waybar
    // reserved before it). This surface keeps exclusive_zone 0, so the
    // compositor places it above that strip and the bottom margin only needs
    // to be a small visual gap, not the bar's full height (a 48px margin
    // double-counted it and left a large gap above the bar). 4px matches the
    // sway window gaps and the bar's screen-edge margins, so the menu aligns
    // with the bar's left edge and floats 4px above its top edge. Reasoned
    // from the zone arithmetic; not runtime-verified here (headless).
    margins: &[(Edge::Bottom, 4), (Edge::Left, 4)],
    keyboard_mode: gtk4_layer_shell::KeyboardMode::Exclusive,
};

struct AppState {
    panel: Option<Panel>,
    osd: Option<Osd>,
    launcher: Option<Launcher>,
    keybinds: Option<Rc<Keybinds>>,
    /// Keep-alive only: the bar follows monitor hotplug by itself and has
    /// no external control surface.
    _bar: Option<Rc<BarManager>>,
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
        keybinds: None,
        _bar: None,
    }));

    // Shared notification store — lives on the GTK main thread (Rc, no Arc)
    let store = Rc::new(RefCell::new(NotificationStore::new()));

    let state_clone = state.clone();
    let store_startup = store.clone();
    app.connect_startup(move |app| {
        theme::load_css();

        // Start D-Bus notification server
        dbus::start_server(store_startup.clone());

        // Picker server: `swaypplet dmenu` clients hand their request to
        // this warm process instead of cold-starting GTK.
        crate::dmenu::start_server(app);

        // SIGUSR1 toggles panel visibility
        let s = state_clone.clone();
        crate::glib_unix::signal_add_local(10 /* SIGUSR1 */, move || {
            if let Some(ref panel) = s.borrow().panel {
                panel.toggle();
            }
            glib::ControlFlow::Continue
        });

        // SIGUSR2 toggles launcher
        let s = state_clone.clone();
        crate::glib_unix::signal_add_local(12 /* SIGUSR2 */, move || {
            if let Some(ref launcher) = s.borrow().launcher {
                launcher.toggle();
            }
            glib::ControlFlow::Continue
        });

        // Written only after both signal handlers are installed, so a
        // `kill -USR1/-USR2` that lands as soon as the pid file exists can't
        // hit the default (terminating) disposition.
        let _ = fs::write(pid_file_path(), std::process::id().to_string());
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

        // ── Keybinding sheet ────────────────────────────────────────────────
        let keybinds = Keybinds::new(app);

        // ── Native bar (one card per output, src/bar/) ──────────────────────
        // SWAYPPLET_NO_BAR=1 skips it so an external bar (waybar) can keep
        // the strip — the nixos side sets this only during the migration
        // window. Delete the guard once waybar is gone from the config.
        if std::env::var_os("SWAYPPLET_NO_BAR").is_none_or(|v| v != "1") {
            // In-process hosting: the start button toggles the panel
            // directly instead of the standalone bar's SIGUSR1 fallback.
            let s = state_clone.clone();
            let toggle: Rc<dyn Fn()> = Rc::new(move || {
                if let Some(ref panel) = s.borrow().panel {
                    panel.toggle();
                }
            });
            let sway = SwayService::start();
            // Stop-notification policy (vision O2): the store resolves a
            // notification's claude-pid hint to task + visibility here,
            // where the sway model lives — same /proc parent-chain hop and
            // ":tN" parse the task scan uses. Bar disabled → no resolver →
            // hinted notifications follow normal rules.
            {
                let sway = sway.clone();
                store_activate
                    .borrow_mut()
                    .set_task_resolver(Box::new(move |pid| {
                        let snap = sway.snapshot();
                        let ws = crate::task_state::workspace_of_pid(pid, &snap.pid_workspaces)?;
                        let task = crate::task_state::task_of_name(&ws)?;
                        let visible = snap.workspaces.iter().any(|w| w.name == ws && w.visible);
                        Some(crate::notifications::store::TaskRef { task, visible })
                    }));
            }
            let bar = BarManager::new(app, sway.clone(), toggle);
            // OSD interjection (BAR_VISION increment 5): volume/brightness
            // render in the bar's decision slot unless the focused window
            // is fullscreen — then the center-screen card stays.
            {
                let bar = bar.clone();
                osd.set_bar_route(move |icon, fraction, text| {
                    if sway.snapshot().focused_fullscreen {
                        return false;
                    }
                    bar.interject(icon, fraction, text)
                });
            }
            st._bar = Some(bar);
        }

        st.panel = Some(panel);
        st.osd = Some(osd);
        st.launcher = Some(launcher);
        st.keybinds = Some(keybinds);
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
        } else if args.len() > 1 && args[1] == "keybinds" {
            // The overlay is driven by key press and release, so the client
            // says which edge it saw rather than asking for a toggle it
            // cannot reason about.
            let action = args.get(2).map(String::as_str).unwrap_or("toggle");
            let st = state_clone.borrow();
            if st.keybinds.is_none() {
                drop(st);
                app.activate();
            }
            let st = state_clone.borrow();
            if let Some(ref keybinds) = st.keybinds {
                match action {
                    "show" => keybinds.show(),
                    "hide" => keybinds.hide(),
                    "reload" => keybinds.invalidate(),
                    _ => keybinds.toggle(),
                }
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
                            OsdCommand::BrightnessRaise | OsdCommand::BrightnessLower => {
                                panel.refresh_brightness()
                            }
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
                            OsdCommand::BrightnessRaise | OsdCommand::BrightnessLower => {
                                panel.refresh_brightness()
                            }
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

        glib::ExitCode::SUCCESS
    });

    app.connect_shutdown(|_| {
        let _ = fs::remove_file(pid_file_path());
    });

    app.run();
}
