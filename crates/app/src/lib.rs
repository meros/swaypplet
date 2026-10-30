mod command;
pub mod preview;
use std::cell::RefCell;
use std::fs;
use std::rc::Rc;

use gio::prelude::*;
use gtk4::Application;
use gtk4::glib;
use gtk4::prelude::*;

use command::{JumpAction, KeybindsAction, PanelCommand, PinAction};
use swaypplet_bar::BarManager;
use swaypplet_base::sway::ipc::SwayService;
use swaypplet_jump::jump::Jump;
use swaypplet_jump::keybinds::Keybinds;
use swaypplet_launcher::Launcher;
use swaypplet_notifications::stack::PopupManager;
use swaypplet_notifyd::dbus;
use swaypplet_notifyd::store::NotificationStore;
use swaypplet_osd::{Osd, OsdCommand};
use swaypplet_panel::Panel;
use swaypplet_ui::shell::{Namespace, Surface};
use swaypplet_ui::theme;

const APP_ID: &str = "dev.swaypplet.panel";

/// The unit that owns the panel process. Nothing else may start one.
const PANEL_UNIT: &str = "swaypplet.service";

/// The panel's layer surface: Pilot's Helm popup, a floating card at the
/// optical sweet spot (~25-28% down, placed by `shell::fit`) over a
/// full-screen transparent backdrop that dismisses it. Exclusive keyboard,
/// for the omnibox.
pub(crate) fn panel_surface(app: &gtk4::Application) -> Surface {
    Surface::builder(app, Namespace::Panel)
        .fill()
        .keyboard(gtk4_layer_shell::KeyboardMode::Exclusive)
        .card(swaypplet_ui::ui::Card::Floating)
        .slide(gtk4::Orientation::Vertical, swaypplet_ui::anim::SLIDE_PX)
        .build()
}

struct AppState {
    panel: Option<Panel>,
    /// Settings' own surface; shown instead of the Helm, never with it.
    settings: Option<Rc<swaypplet_settings::window::SettingsWindow>>,
    osd: Option<Osd>,
    launcher: Option<Launcher>,
    keybinds: Option<Rc<Keybinds>>,
    jump: Option<Rc<Jump>>,
    pins: Option<swaypplet_jump::jump::pin::Pins>,
    labels: Option<swaypplet_label::Labels>,
    /// Keep-alive only: the bar follows monitor hotplug by itself and has
    /// no external control surface.
    _bar: Option<Rc<BarManager>>,
}

/// The Helm's key: it closes settings when settings is what is open (one
/// key, one surface to put away), else shows or hides the panel. Nothing
/// before the panel is built.
fn toggle_panel(st: &AppState) {
    let Some(ref panel) = st.panel else { return };
    match st.settings.as_ref().filter(|s| s.is_shown()) {
        Some(settings) => settings.hide(),
        None => panel.toggle(),
    }
}

/// Build the panel's surfaces if `built` says this one is not there yet:
/// the first request of a session activates, which builds them all. The
/// state is not borrowed across the activation, which borrows it itself.
fn ensure_built(
    app: &gtk4::Application,
    state: &RefCell<AppState>,
    built: impl Fn(&AppState) -> bool,
) {
    if !built(&state.borrow()) {
        app.activate();
    }
}

/// Show `cmd` on the OSD, and bring the panel's slider for it up to date.
fn trigger_osd(st: &AppState, cmd: OsdCommand) {
    if let Some(ref osd) = st.osd {
        osd.trigger(&cmd);
    }
    let Some(ref panel) = st.panel else { return };
    match cmd {
        OsdCommand::OutputVolumeRaise
        | OsdCommand::OutputVolumeLower
        | OsdCommand::OutputVolumeMuteToggle
        | OsdCommand::InputVolumeMuteToggle => panel.refresh_audio(),
        OsdCommand::BrightnessRaise | OsdCommand::BrightnessLower => panel.refresh_brightness(),
        OsdCommand::CapsLock | OsdCommand::NumLock | OsdCommand::ScrollLock => {}
    }
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
    // A test run (SWAYPPLET_DRY_SESSION, dev/render.sh) must not start the
    // person's own panel unit through their user manager.
    if std::env::var_os("SWAYPPLET_DRY_SESSION").is_some() {
        log::warn!("no panel owns {APP_ID}; not starting {PANEL_UNIT} (dry run)");
        return true;
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

    // The launcher searches the settings: hand it the panes' tables.
    swaypplet_settings::install_search();

    let app = Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();

    let state: Rc<RefCell<AppState>> = Rc::new(RefCell::new(AppState {
        panel: None,
        settings: None,
        osd: None,
        launcher: None,
        keybinds: None,
        jump: None,
        pins: None,
        labels: None,
        _bar: None,
    }));

    // Shared notification store — lives on the GTK main thread (Rc, no Arc)
    let store = Rc::new(RefCell::new(NotificationStore::new()));

    let state_clone = state.clone();
    let store_startup = store.clone();
    app.connect_startup(move |app| {
        // Before the first resolution: this process resolves `auto` for
        // every other one (`theme::own_mode`).
        theme::own_mode();
        theme::load_css();
        swaypplet_base::frame_stats::init();

        // The settings file into its live copy, before anything that observes
        // it (the bar's clock, the OSD route) is built; then followed, so a
        // `swaypplet settings set` from a keybind lands here too.
        swaypplet_model::store::init();
        swaypplet_model::store::watch();

        // Put the saved glass override and the saved wallpaper back on the
        // compositor. The sway config has already applied the system material
        // and its own wallpaper by now, so both are no-ops on a session that
        // has never been tuned — which is why they are plain calls rather
        // than something the settings pane has to remember to do the first
        // time it is opened.
        swaypplet_ui::glass::apply_saved();
        swaypplet_model::wallpaper::apply_saved();

        // The wallpaper's hue for `look.tint`. This process is the only one
        // that samples it (`theme::wallpaper::follow_settings`, for why);
        // every other one reads the file it leaves behind. Then follow the
        // theme inputs: the stylesheet, sway's borders, and the glass, which
        // the theme hands back here because sending it is the settings'.
        swaypplet_ui::theme::wallpaper::follow_settings();
        theme::watch(swaypplet_ui::glass::apply_saved_for);
        // Apps follow the mode, the accent, contrast and motion
        // (`theme::apps`); the bar process watches the theme too, but only
        // this one publishes.
        theme::publish_to_apps();

        // The night light: this process holds the gamma tables, from the
        // same sun the automatic mode follows.
        swaypplet_ambient::gamma::follow_settings();

        // The animated wallpaper slows to a stop or goes black on battery when
        // the Look tab says so; this process owns the battery reading and the
        // theme it darkens.
        swaypplet_ambient::wallpaper_battery::follow(app);

        // Display profiles: kanshi's job, matched against the outputs the
        // compositor reports and applied in one step (`swaypplet_displays`).
        swaypplet_displays::start(store_startup.clone());

        // An extra screen for a TV (`swaypplet_cast`); this removes one the
        // panel before this one left behind.
        swaypplet_cast::start();

        // Keyboard, touchpad and mouse: the saved `input` section on sway,
        // again after a hotplug or a reload (`swaypplet_devices::input`).
        swaypplet_devices::input::follow();

        // Start D-Bus notification server
        dbus::start_server(store_startup.clone());

        // Picker server: `swaypplet dmenu` clients hand their request to
        // this warm process instead of cold-starting GTK.
        swaypplet_dmenu::start_server(app);

        // SIGUSR1 toggles panel visibility
        let s = state_clone.clone();
        swaypplet_base::glib_unix::signal_add_local(10 /* SIGUSR1 */, move || {
            toggle_panel(&s.borrow());
            glib::ControlFlow::Continue
        });

        // SIGUSR2 toggles launcher
        let s = state_clone.clone();
        swaypplet_base::glib_unix::signal_add_local(12 /* SIGUSR2 */, move || {
            if let Some(ref launcher) = s.borrow().launcher {
                launcher.toggle();
            }
            glib::ControlFlow::Continue
        });

        // Written only after both signal handlers are installed, so a
        // `kill -USR1/-USR2` that lands as soon as the pid file exists can't
        // hit the default (terminating) disposition.
        let _ = fs::write(
            swaypplet_base::pid::file_path(),
            std::process::id().to_string(),
        );
    });

    let state_clone = state.clone();
    let store_activate = store.clone();
    app.connect_activate(move |app| {
        let mut st = state_clone.borrow_mut();
        if st.panel.is_some() {
            toggle_panel(&st);
            return;
        }

        // ── Main panel window ────────────────────────────────────────────────

        // One connection to the sound server for the whole process: the
        // panel section reads it, and (BAR_VISION increment 7) the hazard
        // lane's microphone glyph reads the same snapshot.
        let audio = swaypplet_audio::AudioService::start();

        // Settings, and the one way into it: the Helm puts itself away and
        // settings opens where it was asked to.
        let settings = swaypplet_settings::window::SettingsWindow::new(app);
        settings.window().present();
        settings.window().set_visible(false);
        let on_settings: swaypplet_panel::OnSettings = {
            let state = Rc::downgrade(&state_clone);
            let settings = Rc::downgrade(&settings);
            Rc::new(move |open| {
                if let Some(state) = state.upgrade()
                    && let Ok(st) = state.try_borrow()
                    && let Some(panel) = st.panel.as_ref()
                {
                    panel.hide();
                }
                if let Some(settings) = settings.upgrade() {
                    settings.open(open);
                }
            })
        };
        {
            // A Helm page among settings' search hits: the other way round.
            let state = Rc::downgrade(&state_clone);
            let weak = Rc::downgrade(&settings);
            settings.set_on_page(move |prefix| {
                if let Some(settings) = weak.upgrade() {
                    settings.hide();
                }
                let prefix = prefix.to_string();
                let state = state.clone();
                // After the hide has let go of the keyboard.
                glib::idle_add_local_once(move || {
                    if let Some(state) = state.upgrade()
                        && let Ok(st) = state.try_borrow()
                        && let Some(panel) = st.panel.as_ref()
                    {
                        panel.open_page(&prefix);
                    }
                });
            });
        }
        st.settings = Some(settings);

        let panel = Panel::new(
            panel_surface(app),
            store_activate.clone(),
            audio.clone(),
            on_settings,
        );
        panel.window().present();
        panel.window().set_visible(false);
        {
            // Never both: the Helm opened by any door (the bar's start
            // button, a notification's card, a page from settings' search)
            // puts settings away, so two surfaces never hold the keyboard.
            let settings = st.settings.as_ref().map(Rc::downgrade);
            panel.window().connect_map(move |_| {
                if let Some(settings) = settings.as_ref().and_then(|w| w.upgrade()) {
                    settings.hide();
                }
            });
        }

        // ── Popup manager ────────────────────────────────────────────────────
        let popups = PopupManager::register(app, store_activate.clone());
        swaypplet_notifyd::quiet::install(store_activate.clone());

        // ── OSD overlay ──────────────────────────────────────────────────────
        let osd = Osd::new(app);
        osd.set_audio(audio.clone());

        // ── Launcher ────────────────────────────────────────────────────────
        let launcher = Launcher::new(app);
        // A settings row found from the launcher card opens in the panel,
        // which holds the settings.
        {
            let state = Rc::downgrade(&state_clone);
            launcher.set_on_setting(move |id| {
                let Some(state) = state.upgrade() else { return };
                let Ok(st) = state.try_borrow() else { return };
                if let Some(panel) = st.panel.as_ref() {
                    panel.open_setting(id);
                }
            });
        }

        // ── Keybinding sheet ────────────────────────────────────────────────
        let keybinds = Keybinds::new(app);

        // ── Jump: Super+Tab, back through the workspaces you came from ──────
        let jump = Jump::new(app);

        // ── Pins: a workspace kept in sight, live, in a corner ──────────────
        let pins = swaypplet_jump::jump::pin::Pins::new(app);
        {
            let pins = pins.clone();
            jump.set_pin(move |workspace| pins.toggle(workspace));
        }
        {
            let osd = osd.clone();
            pins.set_notice(move |icon, label| osd.notice(icon, label));
        }

        // Screenshot card buttons (Annotate / Open / Delete). Registered on
        // activate rather than startup because the editor needs the
        // application to parent its window to.
        swaypplet_screenshot::screenshot::install(app, &store_activate);
        // The report notification's Open button (crates/screenshot/src/quality/report.rs).
        swaypplet_screenshot::quality::report::install(&store_activate);

        // ── Native bar (one card per output, crates/bar/src/) ──────────────────────
        // SWAYPPLET_NO_BAR=1 skips it so an external bar (waybar) can keep
        // the strip — the nixos side sets this only during the migration
        // window. Delete the guard once waybar is gone from the config.
        let mut sway_model = None;
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
            sway_model = Some(sway.clone());
            pins.set_sway(sway.clone());
            popups.set_sway(sway.clone());
            // A `swaymsg reload` puts every layer_effects back to the config,
            // so the glass tuned in the settings pane would last only until
            // the next one. Put it back each time.
            {
                let last = std::cell::Cell::new(0);
                let watched = sway.clone();
                sway.connect_change(move || {
                    let reloads = watched.snapshot().reloads;
                    if reloads != last.replace(reloads) {
                        swaypplet_ui::glass::apply_saved();
                        swaypplet_displays::on_reload();
                    }
                });
            }
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
                        let ws = swaypplet_system::task_state::workspace_of_pid(
                            pid,
                            &snap.pid_workspaces,
                        )?;
                        let task = swaypplet_system::task_state::task_of_name(&ws)?;
                        let visible = snap.workspaces.iter().any(|w| w.name == ws && w.visible);
                        Some(swaypplet_notifyd::store::TaskRef { task, visible })
                    }));
            }
            // The extra-screen hazard's click: the Helm on its Displays
            // page, where the running cast's Stop is.
            let open_displays: Rc<dyn Fn()> = {
                let s = state_clone.clone();
                Rc::new(move || {
                    if let Some(ref panel) = s.borrow().panel {
                        panel.open_page(":disp");
                    }
                })
            };
            let bar = BarManager::new(
                app,
                sway.clone(),
                audio.clone(),
                toggle,
                Some(open_displays),
            );
            // Workspace names: a pill that follows the workspace on each
            // output (crates/label/src/). Needs the sway model, so it lives with
            // the bar.
            st.labels = Some(swaypplet_label::Labels::new(app, sway.clone()));
            // OSD interjection (BAR_VISION increment 5): volume and brightness
            // render in the bar's decision slot rather than as the
            // center-screen card.
            //
            // Off by default. The interjection was the better trade while
            // the card was a frosted slab that covered the middle of the
            // screen to say one number; it is a worse one against a material
            // you can read through, where the card costs a glance and the bar
            // costs a saccade to the bottom edge and back. The card is also
            // the only one of the two that can show a level while the bar is
            // busy with something else.
            //
            // The Bar tab of the settings pane turns it on, and the route
            // reads the setting per press rather than being installed
            // conditionally: which one reads better depends on the material,
            // and the material is editable from the same pane.
            {
                let bar = bar.clone();
                osd.set_bar_route(move |icon, fraction, text| {
                    if !swaypplet_model::store::with(|s| s.bar().osd_in_bar) {
                        return false;
                    }
                    if sway.snapshot().focused_fullscreen {
                        return false;
                    }
                    bar.interject(icon, fraction, text)
                });
            }
            st._bar = Some(bar);
        }

        // Quiet by context: popups held while a screen is shared, mirrored
        // or fullscreen, and one card at the end if anything was held.
        {
            let s = state_clone.clone();
            let open_centre: Rc<dyn Fn()> = Rc::new(move || {
                if let Some(ref panel) = s.borrow().panel {
                    panel.show_notifications();
                }
            });
            swaypplet_notifyd::context::install(
                store_activate.clone(),
                swaypplet_notifyd::context::Sources {
                    sway: sway_model,
                    capture: Some(swaypplet_system::capture::CaptureService::start()),
                    audio: Some(audio.clone()),
                    open_centre,
                },
            );
            let store = store_activate.clone();
            osd.set_hold(move || store.borrow().context().holds_osd());
        }

        st.panel = Some(panel);
        st.osd = Some(osd);
        st.launcher = Some(launcher);
        st.keybinds = Some(keybinds);
        st.jump = Some(jump);
        st.pins = Some(pins);
    });

    // ── Command-line handling ────────────────────────────────────────────────
    let state_clone = state.clone();
    let store_cmdline = store.clone();
    app.connect_command_line(move |app, cmdline| {
        let args: Vec<String> = cmdline
            .arguments()
            .iter()
            .skip(1)
            .map(|a| a.to_string_lossy().to_string())
            .collect();

        match PanelCommand::parse(&args) {
            PanelCommand::Launcher => {
                ensure_built(app, &state_clone, |st| st.launcher.is_some());
                if let Some(ref launcher) = state_clone.borrow().launcher {
                    launcher.toggle();
                }
            }
            PanelCommand::Jump(action) => {
                // Every press is its own client, so the surface has to exist
                // already: activating here would build it after the keypress
                // that wanted it. On a cold session `defer_to_service` has
                // started the panel by now and this first press is simply
                // dropped, which is the same trade the keybind sheet makes.
                if let Some(ref jump) = state_clone.borrow().jump {
                    match action {
                        JumpAction::Step => jump.step(),
                        JumpAction::StepBack => jump.step_back(),
                        JumpAction::Cancel => jump.cancel(),
                        JumpAction::Pin => jump.pin(),
                    }
                }
            }
            PanelCommand::Label(rest) => {
                // Like `jump`, a request to the running panel: a cold session
                // has no pill yet and the first press is dropped.
                if let Some(ref labels) = state_clone.borrow().labels {
                    let rest: Vec<&str> = rest.iter().map(String::as_str).collect();
                    labels.command(&rest);
                }
            }
            PanelCommand::Pin(action) => {
                let Some(pins) = state_clone.borrow().pins.clone() else {
                    return glib::ExitCode::SUCCESS;
                };
                match action {
                    PinAction::Region(area) => {
                        swaypplet_base::spawn::spawn_work(
                            move || {
                                let output = swaypplet_base::sway::ipc::focused_output()?;
                                let tree =
                                    swaypplet_base::sway::ipc::connect().ok()?.get_tree().ok()?;
                                Some((
                                    swaypplet_jump::jump::scene::window_at(&tree, &output, area)?,
                                    output,
                                ))
                            },
                            move |found| {
                                if let Some((region, output)) = found {
                                    pins.pin_region(region, Some(output));
                                }
                            },
                        );
                    }
                    // Drag a rectangle on the frozen screen, and pin that
                    // piece of the window under it.
                    PinAction::SelectRegion => {
                        swaypplet_screenshot::screenshot::select::region(
                            app,
                            swaypplet_screenshot::screenshot::select::Mode::Region,
                            move |selection| {
                                let Some(selection) = selection else { return };
                                let Some(area) = selection.area else { return };
                                let output = selection.output.clone();
                                swaypplet_base::spawn::spawn_work(
                                    {
                                        let output = output.clone();
                                        move || {
                                            let tree = swaypplet_base::sway::ipc::connect()
                                                .ok()?
                                                .get_tree()
                                                .ok()?;
                                            swaypplet_jump::jump::scene::window_at(
                                                &tree, &output, area,
                                            )
                                        }
                                    },
                                    move |region| {
                                        if let Some(region) = region {
                                            pins.pin_region(region, Some(output));
                                        }
                                    },
                                );
                            },
                        );
                    }
                    PinAction::Focused => pins.toggle_focused(),
                }
            }
            PanelCommand::Screenshot(kind) => {
                let shot = swaypplet_screenshot::screenshot::Shot::parse(kind.as_deref());
                ensure_built(app, &state_clone, |st| st.panel.is_some());
                swaypplet_screenshot::screenshot::take(app, &store_cmdline, shot);
            }
            PanelCommand::Report(area) => {
                // A screenshot and a description, on their way to an issue.
                let area = swaypplet_screenshot::quality::report::Area::parse(area.as_deref());
                ensure_built(app, &state_clone, |st| st.panel.is_some());
                swaypplet_screenshot::quality::report::start(app, &store_cmdline, area);
            }
            PanelCommand::Keybinds(action) => {
                ensure_built(app, &state_clone, |st| st.keybinds.is_some());
                if let Some(ref keybinds) = state_clone.borrow().keybinds {
                    match action {
                        KeybindsAction::Show => keybinds.show(),
                        KeybindsAction::Hide => keybinds.hide(),
                        KeybindsAction::Reload => keybinds.invalidate(),
                        KeybindsAction::Toggle => keybinds.toggle(),
                    }
                }
            }
            PanelCommand::Osd(cmd) => {
                ensure_built(app, &state_clone, |st| st.osd.is_some());
                trigger_osd(&state_clone.borrow(), cmd);
            }
            PanelCommand::BadOsd(words) => log::warn!("Unknown OSD command: {words:?}"),
            // Super+Space (the nixos repo's sway.nix). Over D-Bus to the
            // running panel, never a signal to a pid file's pid: a second
            // activation shows or hides the panel.
            PanelCommand::Toggle | PanelCommand::Activate => app.activate(),
            PanelCommand::Unknown(word) => {
                // Kept as it always was: an unknown word shows or hides the
                // panel. Said here, so a typo in a keybind is findable.
                log::warn!("Unknown command {word:?}; toggling the panel");
                app.activate();
            }
        }

        glib::ExitCode::SUCCESS
    });

    app.connect_shutdown(|_| {
        let _ = fs::remove_file(swaypplet_base::pid::file_path());
    });

    app.run();
}
