//! `swaypplet greet` — greetd greeter reusing the lock-screen UI.
//!
//! Runs as the greeter user inside a minimal cage/sway session; the wrapper
//! config execs `swaypplet greet` and tears the compositor down when it
//! returns. Unlike the lock (own PAM handle + direct fprintd), all auth here
//! flows through greetd's PAM conversation — only greetd can start the
//! session, and only its own PAM success lets it. pam_fprintd provides
//! fingerprint, pam_unix the password.
//!
//! The greetd protocol is strictly synchronous, which makes concurrent
//! password-and-fingerprint awkward: once pam_fprintd's "place your finger"
//! info message is acked, the reader arms and greetd blocks — nothing, not
//! even a cancel, gets through until the fingerprint resolves. So:
//!
//! * The finger info message is deliberately left unacked for a grace
//!   period. While unacked, the conversation is still cancelable.
//! * The password field left empty past the grace period means the user
//!   wants fingerprint → ack, arm the reader.
//! * A password submitted while cancelable triggers the dance: cancel the
//!   session, claim the fprintd device ourselves (pam_fprintd then bails
//!   instantly on recreate), recreate, and auto-answer the password prompt.
//! * A password submitted while armed can only wait — it auto-submits the
//!   moment pam_fprintd times out and pam_unix asks.
//!
//! Env: SWAYPPLET_GREET_USER (prefilled username), SWAYPPLET_GREET_CMD
//! (session command, default "sway"), SWAYPPLET_LOCK_WALLPAPER (shared with
//! the lock UI).

mod fpblock;
mod ipc;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use gtk4::glib;
use gtk4::prelude::*;
use greetd_ipc::AuthMessageType;

use crate::lock::ui::{StatusKind, SurfaceSet};

const EXIT_STARTED: i32 = 0;
const EXIT_ERROR: i32 = 1;

/// How long the password field must stay empty before the unacked finger
/// prompt is acked (arming the reader, giving up cancelability).
const FP_ARM_TICK: Duration = Duration::from_millis(500);
const FP_ARM_TICKS: u32 = 3;

#[derive(PartialEq)]
enum Prompt {
    /// Secret/Visible question outstanding — Enter answers it directly.
    Input,
    /// pam_fprintd's finger message outstanding, deliberately unacked.
    FpInfo,
}

struct Pending {
    user: String,
    password: String,
}

struct State {
    tx: mpsc::Sender<ipc::Req>,
    surfaces: SurfaceSet,
    default_user: String,
    session_cmd: String,
    /// User the live greetd session was created for.
    session_user: Option<String>,
    prompt: Option<Prompt>,
    /// A request is in flight; greetd hears nothing new until it replies
    /// (possibly the whole fingerprint wait).
    blocked: bool,
    /// cancel_session sent — drop conversation events until Canceled.
    canceling: bool,
    pending: Option<Pending>,
    recreate_user: Option<String>,
    fp_block: Option<fpblock::Handle>,
    /// The unack-and-wait treatment is given to the first finger message
    /// only; retry messages mid-verify are acked instantly.
    fp_held_once: bool,
}

impl State {
    fn send(&mut self, req: ipc::Req) {
        self.blocked = true;
        let _ = self.tx.send(req);
    }

    fn username(&self) -> Option<String> {
        self.surfaces
            .username()
            .or_else(|| Some(self.default_user.clone()).filter(|u| !u.is_empty()))
    }
}

pub fn run() -> ! {
    if let Err(e) = gtk4::init() {
        eprintln!("swaypplet greet: GTK init failed: {e}");
        std::process::exit(EXIT_ERROR);
    }
    crate::theme::load_css();

    let default_user = std::env::var("SWAYPPLET_GREET_USER").unwrap_or_default();
    let session_cmd = std::env::var("SWAYPPLET_GREET_CMD").unwrap_or_else(|_| "sway".to_string());

    let (tx, rx) = ipc::start();
    let main_loop = glib::MainLoop::new(None, false);
    let exit_code = Rc::new(RefCell::new(EXIT_ERROR));

    let surfaces = SurfaceSet::new();
    surfaces.enable_user_field(&default_user);

    let st = Rc::new(RefCell::new(State {
        tx,
        surfaces: surfaces.clone(),
        default_user,
        session_cmd,
        session_user: None,
        prompt: None,
        blocked: false,
        canceling: false,
        pending: None,
        recreate_user: None,
        fp_block: None,
        fp_held_once: false,
    }));

    let on_submit: Rc<dyn Fn(String)> = {
        let st = st.clone();
        Rc::new(move |password: String| submit(&st, password))
    };
    let window = surfaces.build_surface(on_submit);
    window.fullscreen();
    window.present();

    glib::timeout_add_seconds_local(1, {
        let surfaces = surfaces.clone();
        move || {
            surfaces.tick();
            glib::ControlFlow::Continue
        }
    });
    surfaces.tick();

    // Kick off the conversation for the prefilled user right away — the
    // common case fingerprints straight in without touching the keyboard.
    {
        let mut s = st.borrow_mut();
        if let Some(user) = s.username() {
            s.session_user = Some(user.clone());
            s.send(ipc::Req::Create { username: user });
        }
    }

    {
        let st = st.clone();
        let ml = main_loop.clone();
        let code = exit_code.clone();
        glib::timeout_add_local(Duration::from_millis(40), move || {
            while let Ok(ev) = rx.try_recv() {
                handle_event(&st, ev, &ml, &code);
            }
            glib::ControlFlow::Continue
        });
    }

    main_loop.run();
    // Flush remaining main-context work before the Wayland socket goes away.
    let ctx = glib::MainContext::default();
    while ctx.iteration(false) {}
    std::process::exit(*exit_code.borrow());
}

fn submit(st: &Rc<RefCell<State>>, password: String) {
    let mut s = st.borrow_mut();
    if password.is_empty() && s.prompt != Some(Prompt::Input) {
        return;
    }
    let Some(user) = s.username() else {
        s.surfaces.set_status("Enter a username", StatusKind::Error);
        return;
    };

    if s.canceling {
        // Dance already in flight — this password rides along.
        s.pending = Some(Pending { user: user.clone(), password });
        s.recreate_user = Some(user);
        return;
    }

    let same_user = s.session_user.as_deref() == Some(user.as_str());
    if s.prompt == Some(Prompt::Input) && same_user {
        s.prompt = None;
        s.surfaces.set_status("", StatusKind::Info);
        s.surfaces.set_verifying(true);
        s.send(ipc::Req::Respond(Some(password)));
        return;
    }

    s.pending = Some(Pending {
        user: user.clone(),
        password,
    });
    s.surfaces.set_verifying(true);
    if s.blocked {
        // Reader already armed; greetd can't hear a cancel until PAM
        // resolves. The pending password auto-submits at the next prompt.
        s.surfaces.set_status(
            "Fingerprint reader active — touch it, or your password submits when it times out",
            StatusKind::Info,
        );
    } else {
        s.surfaces.set_status("", StatusKind::Info);
        start_dance(&mut s, user);
    }
}

/// Cancel the current session and recreate it with the fprintd device
/// claimed by us, so pam_fprintd steps aside and the password prompt
/// arrives immediately.
fn start_dance(s: &mut State, user: String) {
    s.prompt = None;
    s.canceling = true;
    s.recreate_user = Some(user);
    if s.fp_block.is_none() {
        s.fp_block = Some(fpblock::claim());
    }
    s.surfaces.show_fp(false, "");
    s.send(ipc::Req::Cancel);
}

fn handle_event(
    st: &Rc<RefCell<State>>,
    ev: ipc::Ev,
    ml: &glib::MainLoop,
    code: &Rc<RefCell<i32>>,
) {
    let mut s = st.borrow_mut();
    s.blocked = false;
    match ev {
        ipc::Ev::AuthMessage { kind, text } => {
            if s.canceling {
                return; // stale message from the dying session
            }
            match kind {
                AuthMessageType::Secret | AuthMessageType::Visible => {
                    s.surfaces.show_fp(false, "");
                    if matches!(kind, AuthMessageType::Visible) {
                        s.surfaces.set_status(&text, StatusKind::Info);
                    }
                    match s.pending.take() {
                        Some(p) if s.session_user.as_deref() == Some(p.user.as_str()) => {
                            s.send(ipc::Req::Respond(Some(p.password)));
                        }
                        Some(p) => {
                            // Username changed while waiting — start over.
                            let user = p.user.clone();
                            s.pending = Some(p);
                            start_dance(&mut s, user);
                        }
                        None => {
                            s.prompt = Some(Prompt::Input);
                            s.surfaces.set_verifying(false);
                        }
                    }
                }
                AuthMessageType::Info | AuthMessageType::Error => {
                    let lower = text.to_lowercase();
                    let fingery = lower.contains("finger") || lower.contains("swipe");
                    if fingery {
                        s.surfaces.show_fp(true, text.trim());
                    } else if matches!(kind, AuthMessageType::Info) {
                        s.surfaces.set_status(&text, StatusKind::Info);
                    } else {
                        s.surfaces.set_status(&text, StatusKind::Error);
                    }
                    if fingery && !s.fp_held_once && s.pending.is_none() {
                        s.fp_held_once = true;
                        s.prompt = Some(Prompt::FpInfo);
                        arm_fp_when_idle(st);
                    } else {
                        s.send(ipc::Req::Respond(None));
                    }
                }
            }
        }
        ipc::Ev::SessionReady => {
            s.fp_block = None;
            s.surfaces.flash_success();
            let cmd = s.session_cmd.clone();
            s.send(ipc::Req::Start { cmd });
        }
        ipc::Ev::SessionStarted => {
            *code.borrow_mut() = EXIT_STARTED;
            ml.quit();
        }
        ipc::Ev::Canceled => {
            s.canceling = false;
            if let Some(user) = s.recreate_user.take() {
                s.session_user = Some(user.clone());
                s.send(ipc::Req::Create { username: user });
            }
        }
        ipc::Ev::Error { auth, text } => {
            if s.canceling {
                return; // error from the dying session; Canceled follows
            }
            // The session is gone either way; reset and start a fresh one.
            // fp_block is kept: after a failed password the user is typing,
            // and the instant re-prompt matters more than the reader.
            s.session_user = None;
            s.prompt = None;
            s.pending = None;
            s.surfaces.set_verifying(false);
            if auth {
                log::info!("auth failed: {text}");
                s.surfaces.set_status("Login failed", StatusKind::Error);
                s.surfaces.shake();
            } else {
                log::warn!("greetd error: {text}");
                s.surfaces.set_status(&text, StatusKind::Error);
            }
            if let Some(user) = s.username() {
                s.session_user = Some(user.clone());
                s.send(ipc::Req::Create { username: user });
            }
        }
        ipc::Ev::Fatal(text) => {
            eprintln!("swaypplet greet: {text}");
            *code.borrow_mut() = EXIT_ERROR;
            ml.quit();
        }
    }
}

/// Ack the held finger prompt once the password field has stayed empty for
/// the grace period — the user is reaching for the reader, not the keys.
fn arm_fp_when_idle(st: &Rc<RefCell<State>>) {
    let st = st.clone();
    let mut ticks = 0u32;
    glib::timeout_add_local(FP_ARM_TICK, move || {
        let mut s = st.borrow_mut();
        if s.prompt != Some(Prompt::FpInfo) || s.canceling {
            return glib::ControlFlow::Break;
        }
        ticks += 1;
        if ticks >= FP_ARM_TICKS && s.surfaces.password_empty() {
            s.prompt = None;
            s.send(ipc::Req::Respond(None));
            return glib::ControlFlow::Break;
        }
        glib::ControlFlow::Continue
    });
}
