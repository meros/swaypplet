//! `swaypplet greet` — greetd greeter reusing the lock-screen UI.
//!
//! Runs as the greeter user inside a minimal cage/sway session; the wrapper
//! config execs `swaypplet greet` and tears the compositor down when it
//! returns. All password auth flows through greetd's PAM conversation —
//! only greetd can start the session, and only its own PAM success lets it.
//!
//! Fingerprint is out-of-band (see `crate::fp_agent`): greetd's PAM stack is
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
//! Selecting a user who already has a live session skips greetd entirely
//! and jumps to that session via the host switcher — going through greetd
//! would start a second session, and the session command's "activate the
//! existing session and exit" fallback makes greetd respawn the greeter,
//! which used to bounce the VT straight back here.
//!
//! Env: SWAYPPLET_GREET_USER (prefilled username), SWAYPPLET_GREET_USERS
//! (comma-separated users shown as clickable chips; first one is the
//! default unless SWAYPPLET_GREET_USER says otherwise), SWAYPPLET_GREET_CMD
//! (session command, default "sway" — runs as whichever user authenticated,
//! so a dispatcher script can pick a per-user session), SWAYPPLET_GREET_ENV
//! (whitespace-separated KEY=VALUE pairs put into the session's PAM
//! environment via greetd — e.g. XDG_SESSION_TYPE=wayland so logind
//! registers a graphical session, which GNOME requires; values cannot
//! contain whitespace), SWAYPPLET_LOCK_WALLPAPER (shared with the lock UI),
//! SWAYPPLET_FP_SOCK (fp-agent socket override),
//! SWAYPPLET_SWITCH_USER_CMD (host switcher, also feeds the chip list).

mod agent;
mod ipc;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use gtk4::glib;
use gtk4::prelude::*;
use greetd_ipc::AuthMessageType;

use crate::fp_agent::{Cmd as FpCmd, Ev as FpEv};
use crate::lock::ui::{StatusKind, SurfaceSet, UserChip};
use crate::switch_user;

const EXIT_STARTED: i32 = 0;
const EXIT_ERROR: i32 = 1;

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
    /// Secret/Visible question outstanding — Enter answers it directly.
    prompt_open: bool,
    /// A request is in flight; greetd hears nothing new until it replies.
    /// With the stack password-only these windows are milliseconds, never a
    /// fingerprint wait.
    blocked: bool,
    /// cancel_session sent — drop conversation events until Canceled.
    canceling: bool,
    pending: Option<Pending>,
    recreate_user: Option<String>,
    /// Chip click that landed while a request was in flight — the
    /// conversation restarts for this user once greetd replies.
    switch_pending: Option<String>,
    /// Per-user fprintd enrollment from `--list`. Governs whether the
    /// selected user gets the reader armed for them. Empty (no `--list`) →
    /// assume enrolled and let the fp-agent's own enrollment check decide.
    enrolled: HashMap<String, bool>,
    /// Users with a live graphical session from `--list` — their chips jump
    /// to the session instead of starting a greetd conversation.
    logged_in: HashMap<String, bool>,
    switch_cmd: Option<String>,
    fp_tx: tokio::sync::mpsc::UnboundedSender<FpCmd>,
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
    /// `true`; the fp-agent double-checks enrollment anyway.
    fn fp_enrolled(&self, user: &str) -> bool {
        self.enrolled.get(user).copied().unwrap_or(true)
    }

    /// Point the fp-agent at the currently selected user (or stand it down
    /// for an unenrolled one). The pill hides until the agent reports Ready
    /// for the new target.
    fn retarget_fp(&mut self) {
        match self.username() {
            Some(user) if self.fp_enrolled(&user) => {
                self.surfaces.show_fp(false, "");
                let _ = self.fp_tx.send(FpCmd::Verify { user });
            }
            _ => {
                let _ = self.fp_tx.send(FpCmd::Stop);
                self.surfaces.show_fp(false, "");
            }
        }
    }
}

pub fn run() -> ! {
    if let Err(e) = gtk4::init() {
        eprintln!("swaypplet greet: GTK init failed: {e}");
        std::process::exit(EXIT_ERROR);
    }
    crate::theme::load_css();

    // Start from the cheap SWAYPPLET_GREET_USERS name list so the window can
    // present without blocking on the host `--list` (two busctl calls per
    // user; fprintd may be cold at boot). The richer `--list` data — avatars,
    // presence, fingerprint enrollment — is fetched off-thread below and
    // upgrades the chips once it lands.
    let users: Vec<String> = std::env::var("SWAYPPLET_GREET_USERS")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|u| !u.is_empty())
        .map(str::to_string)
        .collect();
    let chips = users.iter().map(|u| UserChip::plain(u)).collect::<Vec<_>>();
    let (enrolled, logged_in): (HashMap<String, bool>, HashMap<String, bool>) =
        (HashMap::new(), HashMap::new());
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
    let (fp_tx, fp_rx) = agent::start();
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
        prompt_open: false,
        blocked: false,
        canceling: false,
        pending: None,
        recreate_user: None,
        switch_pending: None,
        enrolled,
        logged_in,
        switch_cmd: switch_user::cmd(),
        fp_tx,
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

    // Kick off the conversation for the prefilled user right away and arm
    // the reader for them — the common case fingerprints straight in
    // without touching the keyboard.
    {
        let mut s = st.borrow_mut();
        if let Some(user) = s.username() {
            s.session_user = Some(user.clone());
            s.send(ipc::Req::Create { username: user });
        }
        s.retarget_fp();
    }

    // Upgrade the env-name chips to the host `--list` data (avatars, presence,
    // fingerprint enrollment) off the main thread — the window is already up.
    // Until this lands, unknown enrollment is treated as enrolled (the reader
    // arms) and unknown presence as logged-out (a chip click starts a greetd
    // conversation rather than jumping); both self-correct once it resolves.
    {
        let st = st.clone();
        let surfaces = surfaces.clone();
        crate::spawn::spawn_work(switch_user::list, move |list| {
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
            {
                let mut s = st.borrow_mut();
                s.enrolled = list.iter().map(|u| (u.user.clone(), u.fingerprint)).collect();
                s.logged_in = list.iter().map(|u| (u.user.clone(), u.logged_in)).collect();
            }
            surfaces.set_greet_chips(&chips);
            st.borrow_mut().retarget_fp();
        });
    }

    {
        let st = st.clone();
        let ml = main_loop.clone();
        let code = exit_code.clone();
        glib::timeout_add_local(Duration::from_millis(40), move || {
            while let Ok(ev) = rx.try_recv() {
                handle_event(&st, ev, &ml, &code);
            }
            while let Ok(ev) = fp_rx.try_recv() {
                handle_fp_event(&st, ev);
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
    if password.is_empty() && !s.prompt_open {
        return;
    }
    let Some(user) = s.username() else {
        s.surfaces.set_status("Enter a username", StatusKind::Error);
        return;
    };

    if s.canceling {
        // Restart already in flight — this secret rides along.
        s.pending = Some(Pending { user: user.clone(), password });
        s.recreate_user = Some(user);
        return;
    }

    let same_user = s.session_user.as_deref() == Some(user.as_str());
    if s.prompt_open && same_user {
        s.prompt_open = false;
        s.surfaces.set_status("", StatusKind::Info);
        s.surfaces.set_verifying(true);
        s.send(ipc::Req::Respond(Some(password)));
        return;
    }

    // No prompt open (conversation still being created) or the conversation
    // belongs to another user: park the secret; it auto-submits at the next
    // matching prompt.
    s.pending = Some(Pending {
        user: user.clone(),
        password,
    });
    s.surfaces.set_status("", StatusKind::Info);
    s.surfaces.set_verifying(true);
    if !s.blocked {
        restart_conversation(&mut s, user);
    }
}

/// A user chip was clicked. A user with a live session gets jumped to
/// directly (their lock screen still guards it); anyone else gets the
/// greetd conversation restarted for them and the reader retargeted.
fn switch_user(st: &Rc<RefCell<State>>, user: String) {
    let mut s = st.borrow_mut();
    if s.logged_in.get(&user).copied().unwrap_or(false) {
        if let Some(cmd) = s.switch_cmd.clone() {
            s.surfaces.set_status("Switching…", StatusKind::Info);
            switch_user::switch_to(&cmd, &user);
            return;
        }
    }
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
    s.retarget_fp();
    if s.canceling {
        // A restart is mid-flight; just retarget the recreate.
        s.recreate_user = Some(user);
        return;
    }
    if s.blocked {
        // greetd can't hear a cancel until the in-flight request resolves
        // (milliseconds). The restart runs the moment greetd replies
        // (`switch_pending` in `handle_event`).
        s.switch_pending = Some(user);
        return;
    }
    restart_conversation(&mut s, user);
}

/// Cancel the current greetd session and recreate it for `user`. With the
/// PAM stack password-only the conversation always rests at an answerable
/// prompt, so the cancel is acknowledged immediately.
fn restart_conversation(s: &mut State, user: String) {
    s.prompt_open = false;
    s.canceling = true;
    s.recreate_user = Some(user);
    s.send(ipc::Req::Cancel);
}

fn handle_fp_event(st: &Rc<RefCell<State>>, ev: FpEv) {
    match ev {
        FpEv::Ready => {
            st.borrow()
                .surfaces
                .show_fp(true, "Touch the fingerprint reader");
        }
        FpEv::Hint { msg } => {
            st.borrow().surfaces.show_fp(true, &msg);
        }
        FpEv::Unavailable { msg } => {
            log::info!("fingerprint unavailable: {msg}");
            st.borrow().surfaces.show_fp(false, "");
        }
        FpEv::Match { user, token } => {
            // The agent verified `user`'s finger on the reader. Guard
            // against a stale match racing a chip switch: only submit if
            // that user is still the one selected on screen.
            let selected = st.borrow().username();
            if selected.as_deref() != Some(user.as_str()) {
                log::info!("discarding fingerprint match for deselected {user}");
                return;
            }
            st.borrow().surfaces.show_fp(true, "Fingerprint OK");
            submit(st, token);
        }
    }
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
    // user instead of serving the old one's message. Error needs no special
    // case (it already recreates for the selected name). SessionReady here
    // means the OLD user's auth completed after someone else was selected —
    // the UI already shows the new user, so starting the old session would
    // open A's desktop under B's name. The person at the keyboard wins; the
    // completed auth is discarded.
    if let Some(user) = s.switch_pending.take() {
        if matches!(ev, ipc::Ev::AuthMessage { .. } | ipc::Ev::SessionReady) {
            s.prompt_open = false;
            s.canceling = true;
            s.recreate_user = Some(user);
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
                            restart_conversation(&mut s, user);
                        }
                        None => {
                            s.prompt_open = true;
                            s.surfaces.set_verifying(false);
                        }
                    }
                }
                AuthMessageType::Info | AuthMessageType::Error => {
                    // Rare with a password-only stack (expired account and
                    // the like) — show and ack immediately so the
                    // conversation keeps moving.
                    if matches!(kind, AuthMessageType::Info) {
                        s.surfaces.set_status(&text, StatusKind::Info);
                    } else {
                        s.surfaces.set_status(&text, StatusKind::Error);
                    }
                    s.send(ipc::Req::Respond(None));
                }
            }
        }
        ipc::Ev::SessionReady => {
            // Stand the reader down before handover so the session's own
            // locker (or the next greeter) can claim it cleanly.
            let _ = s.fp_tx.send(FpCmd::Stop);
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
            s.session_user = None;
            s.prompt_open = false;
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
