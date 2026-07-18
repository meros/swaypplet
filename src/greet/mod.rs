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
//! Env: SWAYPPLET_GREET_USER (prefilled username), SWAYPPLET_GREET_USERS
//! (comma-separated users shown as clickable chips; first one is the
//! default unless SWAYPPLET_GREET_USER says otherwise), SWAYPPLET_GREET_CMD
//! (session command, default "sway" — runs as whichever user authenticated,
//! so a dispatcher script can pick a per-user session), SWAYPPLET_GREET_ENV
//! (whitespace-separated KEY=VALUE pairs put into the session's PAM
//! environment via greetd — e.g. XDG_SESSION_TYPE=wayland so logind
//! registers a graphical session, which GNOME requires; values cannot
//! contain whitespace), SWAYPPLET_LOCK_WALLPAPER (shared with the lock UI).

mod fpblock;
mod ipc;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use gtk4::glib;
use gtk4::prelude::*;
use greetd_ipc::AuthMessageType;

use crate::lock::ui::{StatusKind, SurfaceSet, UserChip};
use crate::switch_user;

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
    /// KEY=VALUE pairs for start_session (SWAYPPLET_GREET_ENV).
    session_env: Vec<String>,
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
    /// Chip click that landed while a request was in flight — the
    /// conversation restarts for this user once greetd replies.
    switch_pending: Option<String>,
    fp_block: Option<fpblock::Handle>,
    /// The unack-and-wait treatment is given to the first finger message
    /// only; retry messages mid-verify are acked instantly.
    fp_held_once: bool,
    /// Per-user fprintd enrollment from `--list`. Governs whether the selected
    /// user gets the fingerprint hint / pam_fprintd, or whether we claim the
    /// reader to skip it. Empty (no `--list`) → assume enrolled (today's
    /// behaviour: let pam_fprintd run, show the hint).
    enrolled: HashMap<String, bool>,
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

    /// Whether `user` has an enrolled fingerprint. Unknown (no `--list`) →
    /// `true`, preserving the pre-enrollment-awareness behaviour.
    fn fp_enrolled(&self, user: &str) -> bool {
        self.enrolled.get(user).copied().unwrap_or(true)
    }

    /// Whether the currently selected user is fingerprint-enrolled.
    fn selected_fp_enrolled(&self) -> bool {
        self.username()
            .map(|u| self.fp_enrolled(&u))
            .unwrap_or(true)
    }
}

pub fn run() -> ! {
    if let Err(e) = gtk4::init() {
        eprintln!("swaypplet greet: GTK init failed: {e}");
        std::process::exit(EXIT_ERROR);
    }
    crate::theme::load_css();

    // Prefer the host `--list` (avatars, presence, fingerprint enrollment) and
    // fall back to the bare SWAYPPLET_GREET_USERS name list. Chip order follows
    // whichever source is used.
    let (users, chips, enrolled) = match switch_user::list() {
        Some(list) if !list.is_empty() => {
            let users = list.iter().map(|u| u.user.clone()).collect::<Vec<_>>();
            let chips = list
                .iter()
                .map(|u| UserChip {
                    user: u.user.clone(),
                    logged_in: u.logged_in,
                    icon: u.icon.clone(),
                })
                .collect::<Vec<_>>();
            let enrolled = list
                .iter()
                .map(|u| (u.user.clone(), u.fingerprint))
                .collect::<HashMap<_, _>>();
            (users, chips, enrolled)
        }
        _ => {
            let users: Vec<String> = std::env::var("SWAYPPLET_GREET_USERS")
                .unwrap_or_default()
                .split(',')
                .map(str::trim)
                .filter(|u| !u.is_empty())
                .map(str::to_string)
                .collect();
            let chips = users.iter().map(|u| UserChip::plain(u)).collect::<Vec<_>>();
            (users, chips, HashMap::new())
        }
    };
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
        session_env,
        session_user: None,
        prompt: None,
        blocked: false,
        canceling: false,
        pending: None,
        recreate_user: None,
        switch_pending: None,
        fp_block: None,
        fp_held_once: false,
        enrolled,
    }));

    if chips.len() > 1 {
        let st = st.clone();
        surfaces.enable_user_chips(&chips, Rc::new(move |user| switch_user(&st, user)));
    }

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
        apply_fp_policy(&mut s);
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

/// A user chip was clicked: mirror the name everywhere and restart the
/// greetd conversation for that user so the right PAM stack (fingerprint
/// included) is armed. If greetd can't hear us right now (request in
/// flight, e.g. the reader is armed for the old user), the old user's
/// fingerprint hint hides immediately and the restart is queued — it runs
/// the moment greetd replies (`switch_pending` in `handle_event`).
fn switch_user(st: &Rc<RefCell<State>>, user: String) {
    let mut s = st.borrow_mut();
    let already_current = s.session_user.as_deref() == Some(user.as_str())
        && s.username().as_deref() == Some(user.as_str());
    if already_current {
        return;
    }
    s.default_user = user.clone();
    s.surfaces.set_username(&user);
    s.pending = None;
    s.surfaces.set_status("", StatusKind::Info);
    s.surfaces.set_verifying(false);
    // Re-evaluate fingerprint policy for the newly selected user.
    apply_fp_policy(&mut s);
    if s.canceling {
        // A dance is mid-flight; just retarget the recreate.
        s.recreate_user = Some(user);
        return;
    }
    if s.blocked {
        // greetd can't hear a cancel until the in-flight request resolves
        // (possibly a whole fingerprint wait for the old user). The hint on
        // screen belongs to that old conversation — drop it now so the new
        // user isn't invited to touch a reader armed for someone else.
        s.surfaces.show_fp(false, "");
        s.switch_pending = Some(user);
        return;
    }
    s.prompt = None;
    s.canceling = true;
    s.recreate_user = Some(user);
    s.fp_held_once = false;
    s.surfaces.show_fp(false, "");
    s.send(ipc::Req::Cancel);
}

/// Align the fprintd block with the selected user's enrollment. Enrolled →
/// release the reader so pam_fprintd can verify it and the hint shows; not
/// enrolled → claim it so pam_fprintd bails instantly and no hint is offered.
/// Skipped mid-dance so it doesn't fight the cancel/recreate flow.
fn apply_fp_policy(s: &mut State) {
    if s.canceling {
        return;
    }
    if s.selected_fp_enrolled() {
        // Dropping the handle releases our claim; pam_fprintd reclaims it.
        s.fp_block = None;
    } else {
        if s.fp_block.is_none() {
            s.fp_block = Some(fpblock::claim());
        }
        s.surfaces.show_fp(false, "");
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
    // A chip click landed while the previous request was in flight; greetd
    // can hear a cancel again only now. Restart the conversation for the new
    // user instead of serving the old one's message. Only AuthMessage needs
    // this: Error already recreates for the selected name, and SessionReady
    // means the old user's fingerprint verified — they win.
    if let Some(user) = s.switch_pending.take() {
        if matches!(ev, ipc::Ev::AuthMessage { .. }) {
            s.prompt = None;
            s.canceling = true;
            s.recreate_user = Some(user);
            s.fp_held_once = false;
            s.send(ipc::Req::Cancel);
            return;
        }
    }
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
                    // The selected user's enrollment decides whether we court
                    // the reader at all: an unenrolled user gets no hint and
                    // no wait — ack the finger prompt so pam_unix takes over.
                    let fp_ok = s.selected_fp_enrolled();
                    if fingery {
                        s.surfaces.show_fp(fp_ok, text.trim());
                    } else if matches!(kind, AuthMessageType::Info) {
                        s.surfaces.set_status(&text, StatusKind::Info);
                    } else {
                        s.surfaces.set_status(&text, StatusKind::Error);
                    }
                    if fingery && fp_ok && !s.fp_held_once && s.pending.is_none() {
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
            let env = s.session_env.clone();
            s.send(ipc::Req::Start { cmd, env });
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
