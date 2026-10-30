//! `swaypplet greet` — greetd greeter reusing the lock-screen UI.
//!
//! Runs as the greeter user inside a minimal cage/sway session; the wrapper
//! config execs `swaypplet greet` and tears the compositor down when it
//! returns. All password auth flows through greetd's PAM conversation —
//! only greetd can start the session, and only its own PAM success lets it.
//!
//! Fingerprint is out-of-band (see `swaypplet_fp::agent`): greetd's PAM stack is
//! password-only, because the greetd protocol is strictly synchronous and an
//! armed pam_fprintd parks the whole conversation — cancels and user
//! switches would hang for the length of a fingerprint wait. Instead the
//! root `swaypplet fp-agent` claims the reader as the selected user and
//! verifies directly against fprintd; on a match it hands us a single-use
//! token which we submit as the password answer, and a pam_exec rule
//! (`swaypplet fp-check`) in greetd's stack accepts it. The conversation
//! therefore always rests at an answerable password prompt: typing works in
//! parallel with the armed reader, and switching users retargets the reader
//! and cancels/recreates the conversation instantly.
//!
//! Every user authenticates here, signed in or not, once. For a user who
//! already has a graphical session, greetd then runs the session command as
//! that user, and the dispatcher's `swaypplet switch-user --resume` asks
//! logind to activate and unlock the running session instead of starting a
//! second one: the pattern GDM, LightDM and SDDM share. The greeter only
//! says which of the two is about to happen ("Resuming…"); it needs no
//! privilege for either. It used to jump to a running session without
//! asking anything, which landed on that session's lock screen and asked
//! for the password a second time.
//!
//! Env: SWAYPPLET_GREET_USER (prefilled username), SWAYPPLET_GREET_USERS
//! (comma-separated users shown as clickable chips; first one is the
//! default unless SWAYPPLET_GREET_USER says otherwise), SWAYPPLET_GREET_CMD
//! (session command, default "sway" — runs as whichever user authenticated,
//! so a dispatcher script can pick a per-user session), SWAYPPLET_GREET_ENV
//! (whitespace-separated KEY=VALUE pairs put into the session's PAM
//! environment via greetd — e.g. XDG_SESSION_TYPE=wayland so logind
//! registers a graphical session, which GNOME requires; values cannot
//! contain whitespace), SWAYPPLET_FP_SOCK (fp-agent socket override).
//!
//! No wallpaper variable: the greeter's own sway draws it (`output * bg`),
//! the same way the compositor draws the lock's, and this process paints a
//! transparent surface over it. It used to read SWAYPPLET_LOCK_WALLPAPER and
//! blit the image itself, which is what forced its card to fake its glass.
//!
//! The richer chip data (avatars, presence, fingerprint enrollment) and the
//! jump-to-a-running-session path both come from [`swaypplet_switch_user`],
//! which is configured by /etc/swaypplet/switch-user.json rather than the
//! environment.

mod conversation;
mod ipc;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc;

use gtk4::glib;
use gtk4::prelude::*;

use swaypplet_auth_card::{SurfaceSet, UserChip};
use swaypplet_fp::agent::Cmd as FpCmd;
use swaypplet_switch_user as switch_user;

use conversation::{Conversation, Effect, Input};

const EXIT_STARTED: i32 = 0;
const EXIT_ERROR: i32 = 1;

/// Where a [`Conversation`]'s effects land: the cards, greetd's socket, the
/// fp-agent, and the main loop when it is time to go.
struct Greeter {
    conversation: RefCell<Conversation>,
    surfaces: SurfaceSet,
    greetd: mpsc::Sender<ipc::Req>,
    fp: tokio::sync::mpsc::UnboundedSender<FpCmd>,
    main_loop: glib::MainLoop,
    exit_code: Cell<i32>,
}

impl Greeter {
    /// Run `input` through the conversation, then carry out what it asks.
    /// The borrow ends before any effect runs, so a widget signal fired by
    /// one can never find the conversation held.
    fn on(&self, input: Input) {
        let effects = self.conversation.borrow_mut().on(input);
        for effect in effects {
            self.apply(effect);
        }
    }

    fn apply(&self, effect: Effect) {
        match effect {
            Effect::Greetd(req) => {
                let _ = self.greetd.send(req);
            }
            Effect::Fp(cmd) => {
                let _ = self.fp.send(cmd);
            }
            Effect::Status(text, tone) => self.surfaces.set_status(&text, tone),
            Effect::Verifying(on) => self.surfaces.set_verifying(on),
            Effect::Username(user) => self.surfaces.set_username(&user),
            Effect::FpArmed(armed) => self.surfaces.set_fp_armed(armed),
            Effect::FpHint(hint) => self.surfaces.fp_hint(&hint),
            Effect::Shake => self.surfaces.shake(),
            Effect::FlashSuccess => self.surfaces.flash_success(),
            Effect::Started => {
                self.exit_code.set(EXIT_STARTED);
                self.main_loop.quit();
            }
            Effect::Fatal(text) => {
                eprintln!("swaypplet greet: {text}");
                self.exit_code.set(EXIT_ERROR);
                self.main_loop.quit();
            }
        }
    }
}

pub fn run() -> ! {
    if let Err(e) = gtk4::init() {
        eprintln!("swaypplet greet: GTK init failed: {e}");
        std::process::exit(EXIT_ERROR);
    }
    // The system's Look, as Nix writes it (/etc/swaypplet/settings.json;
    // the greeter user has no settings of its own), resolved here: `auto`
    // follows the sun at the time zone's city. The machine's owner
    // decides it in the flake, which is the only place a system-wide
    // setting lives. Then follow it, and put the matching glass on this
    // compositor, whose config ships the dark material.
    // The greeter is alone on its compositor, so it owns the theme there,
    // as the panel does in a session (`theme::own_mode`).
    swaypplet_model::store::init();
    swaypplet_ui::theme::own_mode();
    swaypplet_ui::theme::load_css();
    swaypplet_ui::glass::apply_greeter(swaypplet_ui::theme::shown());
    swaypplet_ui::theme::watch(swaypplet_ui::glass::apply_greeter);

    // Start from the cheap SWAYPPLET_GREET_USERS name list so the window can
    // present without blocking on the session query (a logind round trip per
    // user, plus fprintd, which may be cold at boot). The richer data —
    // avatars, presence, fingerprint enrollment — is fetched off-thread below
    // and upgrades the chips once it lands.
    let users: Vec<String> = std::env::var("SWAYPPLET_GREET_USERS")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|u| !u.is_empty())
        .map(str::to_string)
        .collect();
    let chips = users.iter().map(|u| UserChip::plain(u)).collect::<Vec<_>>();
    let default_user = std::env::var("SWAYPPLET_GREET_USER")
        .ok()
        .filter(|u| !u.is_empty())
        .or_else(|| users.first().cloned())
        .unwrap_or_default();
    let session_cmd = std::env::var("SWAYPPLET_GREET_CMD").unwrap_or_else(|_| "sway".to_string());
    let session_env: Vec<String> = std::env::var("SWAYPPLET_GREET_ENV")
        .unwrap_or_default()
        .split_whitespace()
        .filter(|e| e.contains('='))
        .map(str::to_string)
        .collect();

    let (greetd, greetd_rx) = ipc::start();
    let (fp, fp_rx) = swaypplet_fp::agent::client::start();
    let main_loop = glib::MainLoop::new(None, false);

    let surfaces = SurfaceSet::new();
    surfaces.enable_greeter(&default_user);

    let greeter = Rc::new(Greeter {
        conversation: RefCell::new(Conversation::new(default_user, session_cmd, session_env)),
        surfaces: surfaces.clone(),
        greetd,
        fp,
        main_loop: main_loop.clone(),
        exit_code: Cell::new(EXIT_ERROR),
    });

    if chips.len() > 1 {
        let greeter = greeter.clone();
        surfaces.enable_user_chips(
            &chips,
            Rc::new(move |user| greeter.on(Input::SwitchUser(user))),
        );
    }

    let on_submit: Rc<dyn Fn(String)> = {
        let greeter = greeter.clone();
        Rc::new(move |password: String| greeter.on(Input::Submit(password)))
    };
    // One surface per monitor, every one of them a full card. `SurfaceSet`
    // already mirrors status, chips, the reader pill and the username across
    // its surfaces for the lock; the greeter used to build only one and let
    // sway place it, so every other output showed bare wallpaper and no way
    // in. A monitor that appears later (dock, DPMS wake) gets its own card,
    // and one that goes away takes its window with it — a layer surface is
    // bound to its wl_output and is rebuilt, never migrated (as in the bar).
    //
    // Held until the main loop returns: the set watches the monitors
    // weakly, and dropping it destroys every card.
    let windows = swaypplet_ui::shell::PerMonitor::new();
    {
        let surfaces = surfaces.clone();
        windows.after_change(move || surfaces.focus_entry());
    }
    {
        let surfaces = surfaces.clone();
        windows.watch(move |monitor| {
            let window = surfaces.build_surface(on_submit.clone(), Some(monitor));
            // Before `present`, which is where GTK asks for the surface.
            swaypplet_ui::shell::layer::make_layer_window(&window, &CONFIG, Some(monitor));
            window.present();
            GreeterWindow(window)
        });
    }

    glib::timeout_add_seconds_local(1, {
        let surfaces = surfaces.clone();
        move || {
            surfaces.tick();
            glib::ControlFlow::Continue
        }
    });
    surfaces.tick();
    surfaces.follow_caps_lock();

    // Kick off the conversation for the prefilled user right away and arm
    // the reader for them — the common case fingerprints straight in
    // without touching the keyboard.
    greeter.on(Input::Start);

    // Upgrade the env-name chips to the full picker data (avatars, presence,
    // fingerprint enrollment) off the main thread — the window is already up.
    // Until this lands, unknown enrollment is treated as enrolled (the reader
    // arms) and unknown presence as logged-out (a chip click starts a greetd
    // conversation rather than jumping); both self-correct once it resolves.
    {
        let greeter = greeter.clone();
        let surfaces = surfaces.clone();
        swaypplet_base::spawn::spawn_work(switch_user::list, move |list| {
            let Some(list) = list.filter(|l| !l.is_empty()) else {
                return;
            };
            let chips: Vec<UserChip> = list
                .iter()
                .map(|u| UserChip {
                    user: u.user.clone(),
                    logged_in: u.logged_in,
                    icon: u.icon.clone(),
                })
                .collect();
            // Only record a definite enrollment verdict; `None` (host
            // couldn't tell — cold fprintd) is left absent so `fp_enrolled`
            // falls back to armed rather than tearing the reader down on a
            // false negative.
            let enrolled = list
                .iter()
                .filter_map(|u| u.fingerprint.map(|fp| (u.user.clone(), fp)))
                .collect();
            let logged_in = list.iter().map(|u| (u.user.clone(), u.logged_in)).collect();
            surfaces.set_user_chips(&chips);
            greeter.on(Input::Roster {
                enrolled,
                logged_in,
            });
        });
    }

    // greetd's replies and the reader's events, each awaited as it lands.
    {
        let greeter = greeter.clone();
        glib::spawn_future_local(async move {
            while let Ok(ev) = greetd_rx.recv().await {
                greeter.on(Input::Greetd(ev));
            }
        });
    }
    {
        let greeter = greeter.clone();
        glib::spawn_future_local(async move {
            while let Ok(ev) = fp_rx.recv().await {
                greeter.on(Input::Fp(ev));
            }
        });
    }

    main_loop.run();
    // The cards stay up to the end, as they always have: the process is
    // leaving, and destroying them here would only race the session start.
    std::mem::forget(windows);
    // Flush remaining main-context work before the Wayland socket goes away.
    let ctx = glib::MainContext::default();
    while ctx.iteration(false) {}
    std::process::exit(greeter.exit_code.get());
}

/// A layer surface, not a fullscreened toplevel, and the namespace is the
/// whole point: `layer_effects` keys on layer-shell namespaces, so this is
/// what lets the greeter's card be the same liquid glass as the bar and as
/// the lock card. It is also the only way the wallpaper reaches the screen at
/// all now that the greeter's compositor draws it rather than this process:
/// sway disables the background layer whenever a workspace holds a fullscreen
/// container (`shell_background` in sway/desktop/transaction.c) and paints an
/// opaque black rect behind it instead, so a fullscreened toplevel would sit
/// on black no matter what `output * bg` was set to.
///
/// Anchored to all four edges rather than sized: that is what makes a layer
/// surface fill the output, and it keeps working across a mode change with
/// nothing to recompute. Exclusive keyboard because this is a login prompt
/// and it must own the keyboard from the moment it maps, with no pointer
/// anywhere near it. With one card per output sway gives the keyboard to one
/// of them; a click on another moves it there, as on the lock screen.
static CONFIG: swaypplet_ui::shell::layer::LayerShellConfig =
    swaypplet_ui::shell::layer::LayerShellConfig {
        namespace: swaypplet_ui::shell::Namespace::Greeter,
        layer: gtk4_layer_shell::Layer::Overlay,
        default_width: None,
        default_height: None,
        anchors: &[
            (gtk4_layer_shell::Edge::Top, true),
            (gtk4_layer_shell::Edge::Bottom, true),
            (gtk4_layer_shell::Edge::Left, true),
            (gtk4_layer_shell::Edge::Right, true),
        ],
        margins: &[],
        keyboard_mode: gtk4_layer_shell::KeyboardMode::Exclusive,
        // Nothing else is on this compositor to reserve space from.
        exclusive: false,
    };

/// One output's greeter card. Dropped when its monitor goes away, which
/// destroys the window (`SurfaceSet` forgets it on ::destroy).
struct GreeterWindow(gtk4::Window);

impl Drop for GreeterWindow {
    fn drop(&mut self) {
        self.0.destroy();
    }
}
