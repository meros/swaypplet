use std::cell::RefCell;
use std::fs;
use std::rc::Rc;

use gio::prelude::*;
use gtk4::Application;
use gtk4::glib;
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
use crate::switcher::Switcher;
use crate::theme;

const APP_ID: &str = "dev.swaypplet.panel";

/// The unit that owns the panel process. Nothing else may start one.
const PANEL_UNIT: &str = "swaypplet.service";

/// Pid file lives in the per-user runtime dir (mode 0700), so no other user
/// can forge or clobber it. The /tmp fallback matches the wrapper scripts'
/// `$XDG_RUNTIME_DIR/swaypplet.pid` with the same /tmp fallback. Shared with
/// the standalone bar's start button (`bar::start`), which signals this pid.
pub(crate) fn pid_file_path() -> std::path::PathBuf {
    let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
    std::path::Path::new(&dir).join("swaypplet.pid")
}

// Pilot's Helm popup: centered optical HUD (~25-28% Y-offset), 740px wide,
// with full-screen dismiss backdrop.
static PANEL_CONFIG: LayerShellConfig = LayerShellConfig {
    namespace: "swaypplet",
    layer: gtk4_layer_shell::Layer::Overlay,
    exclusive: false,
    default_width: None,
    default_height: None,
    anchors: &[
        (Edge::Top, true),
        (Edge::Bottom, true),
        (Edge::Left, true),
        (Edge::Right, true),
    ],
    margins: &[],
    keyboard_mode: gtk4_layer_shell::KeyboardMode::Exclusive,
};

struct AppState {
    panel: Option<Panel>,
    osd: Option<Osd>,
    launcher: Option<Launcher>,
    keybinds: Option<Rc<Keybinds>>,
    switcher: Option<Rc<Switcher>>,
    /// Keep-alive only: the bar follows monitor hotplug by itself and has
    /// no external control surface.
    _bar: Option<Rc<BarManager>>,
}

/// Does something already own [`APP_ID`] on the session bus?
///
/// Asked over the bus rather than by registering our own `GApplication`,
/// because registering is not a question: the first process to ask *becomes*
/// the owner, which is the whole problem below. On any error this answers
/// "yes" — a client that cannot reach the bus should fail the way it always
/// did, not start a second panel.
fn panel_listening() -> bool {
    let Ok(bus) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) else {
        return true;
    };
    let reply = bus.call_sync(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "NameHasOwner",
        Some(&(APP_ID,).to_variant()),
        Some(glib::VariantTy::new("(b)").unwrap()),
        gio::DBusCallFlags::NONE,
        1000,
        gio::Cancellable::NONE,
    );
    match reply {
        Ok(reply) => reply.child_value(0).get::<bool>().unwrap_or(true),
        Err(e) => {
            log::warn!("NameHasOwner({APP_ID}) failed: {e}");
            true
        }
    }
}

/// A subcommand is a request to the running shell, never a way to start one.
///
/// `GApplication`'s default is the opposite: the first process to register
/// becomes the primary instance, so `swaypplet keybinds show` with no panel
/// up quietly *became* the panel. Everything downstream of that went wrong at
/// once — the shell ran outside `swaypplet.service` with its logs sent to the
/// caller's /dev/null; the unit's own restart found the bus name taken,
/// dispatched as a remote instance and exited 0, so systemd reported
/// `inactive` while a panel was on screen; and the Super-hold watcher that
/// had run the client waits for it to return, so it never read another key
/// and the sheet stayed up until the process was killed.
///
/// So a client that finds nobody home asks systemd for the panel and leaves.
/// The keypress that prompted it is dropped — the next one lands on a real
/// panel, which is the cheap half of the trade.
fn defer_to_service() -> bool {
    if panel_listening() {
        return false;
    }
    log::warn!("no panel owns {APP_ID}; starting {PANEL_UNIT} rather than becoming one");
    // --no-block: nothing here waits on the unit, and the caller may be a
    // watcher loop that must get its thread back.
    let started = std::process::Command::new("systemctl")
        .args(["--user", "start", "--no-block", PANEL_UNIT])
        .status();
    if let Err(e) = started {
        log::warn!("could not start {PANEL_UNIT}: {e}");
    }
    true
}

pub fn run() {
    // Only the bare `swaypplet` invocation — the unit's own ExecStart — is
    // allowed to be the panel.
    if std::env::args().nth(1).is_some() && defer_to_service() {
        return;
    }

    let app = Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();

    let state: Rc<RefCell<AppState>> = Rc::new(RefCell::new(AppState {
        panel: None,
        osd: None,
        launcher: None,
        keybinds: None,
        switcher: None,
        _bar: None,
    }));

    // Shared notification store — lives on the GTK main thread (Rc, no Arc)
    let store = Rc::new(RefCell::new(NotificationStore::new()));

    let state_clone = state.clone();
    let store_startup = store.clone();
    app.connect_startup(move |app| {
        theme::load_css();

        // Put the saved glass override back on the compositor. The sway config
        // has already applied the system material by now, so this is a no-op
        // on a session that has never been tuned — which is why it is a plain
        // call rather than something the settings pane has to remember to do
        // the first time it is opened.
        crate::settings::glass::apply_saved();

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

        // One connection to the sound server for the whole process: the
        // panel section reads it, and (BAR_VISION increment 7) the hazard
        // lane's microphone glyph reads the same snapshot.
        let audio = crate::audio::AudioService::start();

        let panel = Panel::new(window, store_activate.clone(), audio.clone());
        panel.window.present();
        panel.window.set_visible(false);

        // ── Popup manager ────────────────────────────────────────────────────
        PopupManager::register(app, store_activate.clone());

        // ── OSD overlay ──────────────────────────────────────────────────────
        let osd = Osd::new(app);
        osd.set_audio(audio.clone());

        // ── Launcher ────────────────────────────────────────────────────────
        let launcher = Launcher::new(app);

        // ── Keybinding sheet ────────────────────────────────────────────────
        let keybinds = Keybinds::new(app);

        // ── Window switcher ─────────────────────────────────────────────────
        let switcher = Switcher::new(app);

        // Screenshot card buttons (Annotate / Open / Delete). Registered on
        // activate rather than startup because the editor needs the
        // application to parent its window to.
        crate::screenshot::install(app, &store_activate);

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
            let bar = BarManager::new(app, sway.clone(), audio.clone(), toggle);
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
        st.switcher = Some(switcher);
    });

    // ── Command-line handling ────────────────────────────────────────────────
    let state_clone = state.clone();
    let store_cmdline = store.clone();
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
        } else if args.len() > 1 && args[1] == "switcher" {
            let st = state_clone.borrow();
            if st.switcher.is_none() {
                drop(st);
                app.activate();
            }
            let st = state_clone.borrow();
            if let Some(ref switcher) = st.switcher {
                switcher.toggle();
            }
        } else if args.len() > 1 && args[1] == "screenshot" {
            let shot = crate::screenshot::Shot::parse(args.get(2).map(String::as_str));
            let st = state_clone.borrow();
            if st.panel.is_none() {
                drop(st);
                app.activate();
            }
            crate::screenshot::take(app, &store_cmdline, shot);
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
