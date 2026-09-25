//! The elevation agent: one card for `pkexec`, `sudo` and face elevation.
//!
//! Run via `swaypplet polkit-agent`. It owns its own GApplication so it
//! coexists with the main `swaypplet` panel process, and it owns every
//! surface that can authorise something as root, because one process drawing
//! one card is the whole point: two processes meant two cards with two Cancel
//! buttons and no way to tell which one a press answered.
//!
//! ## Three ways in, one card
//!
//! ```text
//!   polkit  ── BeginAuthentication ─┐
//!   pam_race ── begin/channel/end ──┼──►  one Session  ──►  one PolkitDialog
//!   faced   ── announce/confirm ────┘        (card + faced confirm)
//! ```
//!
//! * **pkexec / a policy action.** polkit routes the PAM conversation through
//!   `polkit-agent-helper-1`, a setuid binary we spawn and speak a line
//!   protocol to (`helper.rs`). The password the user types goes down the
//!   helper's stdin; fingerprint and face ride the same card.
//! * **`sudo` in a terminal.** There is no polkit and no helper: `sudo` runs
//!   pam_race itself. pam_race reports to us over a socket (`race.rs`) so the
//!   terminal gets the same card `pkexec` has — a password typed on it goes
//!   back down the socket, and pam_race treats it exactly like one typed at
//!   the terminal. The two prompts race; either finishes the login.
//! * **The face check.** faced opens the camera for either of the above and
//!   asks this process to collect the confirm press (`face.rs`), because a
//!   PAM prompt is answerable by a pipe and the press must come from
//!   somewhere a pipe cannot reach.
//!
//! ## Joined on the caller's pid
//!
//! All three name the same process — the one running pam_race, which is
//! `sudo` itself or the helper we spawned. polkit does not carry that pid,
//! but we know the helper's pid because we started it; pam_race's `begin`
//! and faced's `announce` both carry it. So a session is keyed by that pid,
//! and a `begin` or an `announce` that names a session already on screen
//! attaches to it rather than drawing a second card. A face check that names
//! no session (a legacy or direct caller) draws its own, as it always did.
//!
//! ## What each source is trusted with
//!
//! The helper is setuid root and does the PAM comparison; we never touch
//! libpam. pam_race connects from root (`race.rs` checks the peer), reports
//! which methods are live and carries a password, never a decision. faced
//! reports the camera and collects the press. The password is verified by the
//! PAM stack in every case, never here.

pub(crate) mod agent;
mod cue;
pub(crate) mod dialog;
mod face;
mod helper;
pub(crate) mod race;
mod session;

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::os::fd::RawFd;
use std::rc::Rc;
use std::time::Duration;

use gio::prelude::*;
use gtk4::Application;
use tokio::sync::oneshot;

use crate::settings::store::{self, Elevate};
use crate::theme;

use agent::{AgentEvent, AuthOutcome, AuthRequest, ResolvedIdentity};
use cue::Cue;
use dialog::{Callbacks, Card, Methods, PolkitDialog, StatusKind};
use face::FaceSession;
use helper::{Helper, HelperEvent};

const APP_ID: &str = "dev.swaypplet.polkit";

/// Shared handle to the fd-watcher's `SourceId`. The watcher closure holds
/// one clone and the orchestrator holds another; whichever disposes of the
/// source calls `cancel_fd_source`, which claims ownership and removes it
/// once. Two paths racing `SourceId::remove` aborted the process through
/// glib's non-unwinding trampoline.
type SourceHandle = Rc<Cell<Option<glib::SourceId>>>;

fn cancel_fd_source(handle: &SourceHandle) {
    if let Some(id) = handle.take() {
        crate::spawn::remove_source(id);
    }
}

/// Where a session's password goes, and what draws its card.
enum Backing {
    /// pkexec or a policy action. The helper runs the PAM conversation; the
    /// password goes down its stdin, and a polkit `reply` is owed when the
    /// request resolves. A pam_race `link` may attach too (pam_race runs
    /// inside the helper's stack), used only for the camera gate and the
    /// channel states — never for the password, which the helper owns.
    Polkit {
        helper: Option<Helper>,
        fd_source: Option<SourceHandle>,
        reply: Option<oneshot::Sender<AuthOutcome>>,
        cookie: String,
        identities: Vec<ResolvedIdentity>,
        selected_uid: u32,
    },
    /// `sudo` in a terminal. pam_race is the only conversation; the password
    /// goes back down its socket, and there is no reply to send — the login
    /// completes on the terminal that ran `sudo`.
    Sudo,
    /// A face check that named no session: faced opened the camera with no
    /// pam_race link we could find. The card is ours to draw and drop, and
    /// carries the press and nothing else.
    FaceOnly,
}

/// One elevation on screen. At most one is active; the rest wait in `queue`.
struct Session {
    backing: Backing,
    /// pam_race's report channel, once it has attached. Present for `Sudo`
    /// from the start and for `Polkit` once pam_race connects; carries the
    /// password for `Sudo` and nothing but liveness for `Polkit`.
    link: Option<race::Link>,
    /// The process running pam_race — `sudo` or the helper — that a `begin`
    /// and an `announce` are matched on. `None` until known.
    race_pid: Option<u32>,
    /// What the card says is accepting input right now. The caption's resting
    /// sentence is this and nothing else.
    methods: Methods,
    /// A password submitted before the conversation was ready for it. The
    /// card shows the entry from the first frame, so a fast user can answer
    /// early; the keystroke is held here and flushed into the next prompt
    /// rather than dropped.
    buffered_password: Option<String>,
    /// True between a prompt and the answer to it, on the helper path.
    waiting_password: bool,
    /// A face confirm riding on this card, from faced. Independent of the
    /// backing: for `Sudo` and `Polkit` it rides a live request; for
    /// `FaceOnly` it is the whole reason the card exists.
    face: Option<FaceSession>,
    /// Resolved to a terminal outcome once, so a late second signal (a
    /// biometric win followed by `granted`, a helper SUCCESS followed by a
    /// hangup) cannot approve or deny twice.
    settled: bool,
}

impl Session {
    fn cookie(&self) -> Option<&str> {
        match &self.backing {
            Backing::Polkit { cookie, .. } => Some(cookie),
            _ => None,
        }
    }
}

struct PolkitState {
    dialog: Rc<PolkitDialog>,
    /// The look-at-the-camera pill, on its own unblurred surface. See cue.rs.
    cue: Cue,
    active: Option<Session>,
    queue: VecDeque<Pending>,
}

/// A request that arrived while the card was busy.
enum Pending {
    Polkit {
        request: AuthRequest,
        reply: oneshot::Sender<AuthOutcome>,
    },
    Sudo {
        begin: race::Begin,
        link: race::Link,
    },
}

/// The elevation settings in force, read per request so a change in the
/// settings pane lands on the next `sudo` without a restart.
fn elevate_settings() -> Elevate {
    store::current().elevate()
}

pub fn run() {
    // The settings live copy, so `elevate_settings` reads the user's choices
    // and follows the file for the life of the process.
    store::init();
    store::watch();

    let app = Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::FLAGS_NONE)
        .build();

    let state: Rc<RefCell<Option<Rc<RefCell<PolkitState>>>>> = Rc::new(RefCell::new(None));

    let state_startup = state.clone();
    app.connect_startup(move |app| {
        theme::load_css();

        let dialog = PolkitDialog::new(app);
        // After the dialog on purpose: sway stacks surfaces within a layer in
        // creation order, and the cue has to sit above the card's backdrop.
        let cue = Cue::new(app);
        let inner = Rc::new(RefCell::new(PolkitState {
            dialog,
            cue,
            active: None,
            queue: VecDeque::new(),
        }));
        *state_startup.borrow_mut() = Some(inner.clone());

        // The faced confirm agent, for face-authenticated sudo and pkexec.
        face::register(&inner);

        // pam_race's report socket, for terminal `sudo` and the channel
        // states of `pkexec`. Absence is not fatal: without it, `sudo` keeps
        // its terminal prompt and `pkexec` keeps its helper-only card.
        {
            let (race_tx, race_rx) = async_channel::unbounded::<race::Message>();
            if race::listen(race_tx) {
                let inner = inner.clone();
                glib::spawn_future_local(async move {
                    while let Ok(msg) = race_rx.recv().await {
                        handle_race(&inner, msg);
                    }
                });
            }
        }

        // The polkit agent thread's events, awaited on the GTK main loop:
        // the process sleeps until polkit calls.
        let agent_rx = agent::start();
        let inner_for_events = inner.clone();
        glib::spawn_future_local(async move {
            while let Ok(event) = agent_rx.recv().await {
                handle_agent_event(&inner_for_events, event);
            }
        });
    });

    app.connect_activate(|_app| log::info!("swaypplet elevation agent ready"));
    app.connect_shutdown(|_| log::info!("swaypplet elevation agent shutting down"));

    // Empty argv: run() would treat the subcommand word as a file to open.
    app.run_with_args::<&str>(&[]);
}

// ────────────────────────────────────────────────────────────────────────
// Presenting the card
// ────────────────────────────────────────────────────────────────────────

/// The callbacks that route every user action on the card back into the
/// orchestrator. `identity` is a no-op for a card with no identity picker.
fn callbacks(state: &Rc<RefCell<PolkitState>>, identity: bool) -> Callbacks {
    let s_submit = state.clone();
    let s_cancel = state.clone();
    let s_ident = state.clone();
    let s_typing = state.clone();
    Callbacks {
        on_submit: Rc::new(move |text| handle_submit(&s_submit, text)),
        on_cancel: Rc::new(move || cancel_active(&s_cancel)),
        on_identity: if identity {
            Rc::new(move |uid| handle_identity_change(&s_ident, uid))
        } else {
            Rc::new(|_| {})
        },
        on_typing: Rc::new(move || face::abandon(&s_typing)),
    }
}

// ────────────────────────────────────────────────────────────────────────
// polkit events
// ────────────────────────────────────────────────────────────────────────

fn handle_agent_event(state: &Rc<RefCell<PolkitState>>, event: AgentEvent) {
    match event {
        AgentEvent::Begin { request, reply } => {
            if is_busy(state) {
                state
                    .borrow_mut()
                    .queue
                    .push_back(Pending::Polkit { request, reply });
            } else {
                start_polkit(state, request, reply);
            }
        }
        AgentEvent::Cancel { cookie } => handle_agent_cancel(state, &cookie),
    }
}

fn is_busy(state: &Rc<RefCell<PolkitState>>) -> bool {
    state.borrow().active.is_some()
}

fn handle_agent_cancel(state: &Rc<RefCell<PolkitState>>, cookie: &str) {
    let is_active = state
        .borrow()
        .active
        .as_ref()
        .and_then(Session::cookie)
        .is_some_and(|c| c == cookie);
    if is_active {
        end_session(state, AuthOutcome::Cancelled);
        return;
    }
    state.borrow_mut().queue.retain(|p| match p {
        Pending::Polkit { request, .. } => request.cookie != cookie,
        Pending::Sudo { .. } => true,
    });
}

fn start_polkit(
    state: &Rc<RefCell<PolkitState>>,
    request: AuthRequest,
    reply: oneshot::Sender<AuthOutcome>,
) {
    let selected_uid = request.identities[0].uid;
    let username = request.identities[0].username.clone();

    {
        let mut s = state.borrow_mut();
        s.active = Some(Session {
            backing: Backing::Polkit {
                helper: None,
                fd_source: None,
                reply: Some(reply),
                cookie: request.cookie.clone(),
                identities: request.identities.clone(),
                selected_uid,
            },
            link: None,
            race_pid: None,
            methods: Methods {
                password: true,
                ..Methods::default()
            },
            buffered_password: None,
            waiting_password: false,
            face: None,
            settled: false,
        });
    }

    // Draw the card from the request, before any late affordance can arrive.
    let dialog = state.borrow().dialog.clone();
    let card = Card {
        title: "Authentication Required",
        message: if request.message.is_empty() {
            "An action requires authorization."
        } else {
            request.message.as_str()
        },
        icon_name: request.icon_name.as_str(),
        action_id: request.action_id.as_str(),
        command: None,
        identities: &request.identities,
        details: dialog::format_details(&request),
        password: true,
    };
    dialog.present(&card, callbacks(state, request.identities.len() > 1));
    dialog.set_methods(Methods {
        password: true,
        ..Methods::default()
    });

    spawn_helper(state, &username);
}

fn spawn_helper(state: &Rc<RefCell<PolkitState>>, username: &str) {
    let cookie = match state.borrow().active.as_ref().and_then(Session::cookie) {
        Some(c) => c.to_string(),
        None => return,
    };

    log::info!("polkit: spawning helper for user {username}");
    match Helper::spawn(username, &cookie) {
        Ok(helper) => {
            let fd = helper.stdout_raw_fd();
            let pid = helper.pid();
            install_fd_watch(state, fd);
            let mut s = state.borrow_mut();
            if let Some(active) = s.active.as_mut() {
                active.race_pid = Some(pid);
                if let Backing::Polkit { helper: h, .. } = &mut active.backing {
                    *h = Some(helper);
                }
                active.waiting_password = false;
            }
        }
        Err(e) => {
            log::error!("polkit: failed to spawn helper: {e}");
            let dialog = state.borrow().dialog.clone();
            dialog.set_status(
                &format!("Failed to spawn polkit helper: {e}"),
                StatusKind::Error,
            );
            dialog.lock_inputs();
            let s = state.clone();
            glib::timeout_add_local_once(Duration::from_secs(3), move || {
                let same = s
                    .borrow()
                    .active
                    .as_ref()
                    .and_then(Session::cookie)
                    .map(str::to_string)
                    == Some(cookie.clone());
                if same {
                    end_session(&s, AuthOutcome::Error("polkit helper unavailable".into()));
                }
            });
        }
    }
}

fn install_fd_watch(state: &Rc<RefCell<PolkitState>>, fd: RawFd) {
    let state_weak = Rc::downgrade(state);
    let handle: SourceHandle = Rc::new(Cell::new(None));
    let handle_cb = handle.clone();
    let source = crate::glib_unix::fd_add_local(
        fd,
        glib::IOCondition::IN | glib::IOCondition::HUP | glib::IOCondition::ERR,
        move |_fd, _cond| {
            let Some(state) = state_weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if drain_helper(&state) {
                glib::ControlFlow::Continue
            } else {
                handle_cb.take();
                glib::ControlFlow::Break
            }
        },
    );
    handle.set(Some(source));
    if let Some(Backing::Polkit { fd_source, .. }) =
        state.borrow_mut().active.as_mut().map(|a| &mut a.backing)
    {
        *fd_source = Some(handle);
    }
}

fn drain_helper(state: &Rc<RefCell<PolkitState>>) -> bool {
    let (events, eof) = {
        let mut s = state.borrow_mut();
        let Some(active) = s.active.as_mut() else {
            return false;
        };
        let Backing::Polkit {
            helper: Some(h), ..
        } = &mut active.backing
        else {
            return false;
        };
        h.read_events()
    };

    for event in events {
        if !apply_helper_event(state, event) {
            return false;
        }
    }

    if eof {
        let still = state.borrow().active.is_some();
        if still {
            log::warn!("polkit helper exited unexpectedly");
            let dialog = state.borrow().dialog.clone();
            dialog.set_status("Authentication helper exited", StatusKind::Error);
            dialog.shake();
            let username = helper_username(state);
            if !username.is_empty() {
                // We are inside the watcher and will return Break, which
                // auto-removes the source; just claim the SourceId so nothing
                // else removes it, then respawn.
                if let Some(Backing::Polkit {
                    fd_source, helper, ..
                }) = state.borrow_mut().active.as_mut().map(|a| &mut a.backing)
                {
                    if let Some(handle) = fd_source.take() {
                        handle.take();
                    }
                    helper.take();
                }
                spawn_helper(state, &username);
            } else {
                end_session(
                    state,
                    AuthOutcome::Error("authentication helper exited".into()),
                );
            }
        }
        return false;
    }
    true
}

fn helper_username(state: &Rc<RefCell<PolkitState>>) -> String {
    let s = state.borrow();
    let Some(active) = s.active.as_ref() else {
        return String::new();
    };
    let Backing::Polkit {
        identities,
        selected_uid,
        ..
    } = &active.backing
    else {
        return String::new();
    };
    identities
        .iter()
        .find(|i| i.uid == *selected_uid)
        .map(|i| i.username.clone())
        .unwrap_or_default()
}

fn apply_helper_event(state: &Rc<RefCell<PolkitState>>, event: HelperEvent) -> bool {
    let dialog = state.borrow().dialog.clone();
    match event {
        HelperEvent::PromptEchoOff(prompt) | HelperEvent::PromptEchoOn(prompt) => {
            dialog.set_password_prompt(&prompt);
            dialog.set_verifying(false);
            let buffered = {
                let mut s = state.borrow_mut();
                match s.active.as_mut() {
                    Some(active) => {
                        active.waiting_password = true;
                        active.buffered_password.take()
                    }
                    None => None,
                }
            };
            if let Some(password) = buffered {
                handle_submit(state, password);
            }
            true
        }
        HelperEvent::Info(msg) => {
            if helper::is_fingerprint_hint(&msg) {
                set_channel(state, race::Channel::Fp, true);
                dialog.hint(&humanise_fingerprint(&msg));
            } else {
                dialog.set_status(&msg, StatusKind::Info);
            }
            true
        }
        HelperEvent::Error(msg) => {
            dialog.reject(&msg);
            true
        }
        HelperEvent::Success => {
            settle_success(state);
            false
        }
        HelperEvent::Failure => {
            dialog.reject("Authentication failed");
            true
        }
    }
}

// ────────────────────────────────────────────────────────────────────────
// pam_race events
// ────────────────────────────────────────────────────────────────────────

fn handle_race(state: &Rc<RefCell<PolkitState>>, msg: race::Message) {
    match msg.event {
        race::Event::Begin(begin) => handle_race_begin(state, msg.link, begin),
        race::Event::Channel { channel, live } => {
            if link_is_active(state, &msg.link) {
                set_channel(state, channel, live);
            }
        }
        race::Event::End { won } => {
            if link_is_active(state, &msg.link) {
                handle_race_end(state, won);
            }
        }
        race::Event::Granted => {
            if link_is_active(state, &msg.link) {
                settle_success(state);
            }
        }
        race::Event::Hangup => {
            if link_is_active(state, &msg.link) {
                if !state.borrow().active.as_ref().is_some_and(|a| a.settled) {
                    end_session(state, AuthOutcome::Cancelled);
                }
            } else {
                drop_queued_link(state, &msg.link);
            }
        }
    }
}

fn handle_race_begin(state: &Rc<RefCell<PolkitState>>, link: race::Link, begin: race::Begin) {
    let want = elevate_settings();
    log::info!(
        "race: begin for {} from pid {} ({}), reader {}",
        begin.user,
        begin.pid,
        begin.exe,
        if begin.fp { "started" } else { "absent" }
    );

    // A `begin` for the helper already on screen is the pkexec path: pam_race
    // runs inside our helper. Attach the link for the camera gate and the
    // channel states; the helper keeps the password.
    let attach = state
        .borrow()
        .active
        .as_ref()
        .and_then(|a| a.race_pid)
        .is_some_and(|pid| pid == begin.pid);
    if attach {
        // The same link beginning again is `sudo` retrying after a wrong
        // password: the stack rejected the token, and pam_race is asking
        // once more on the same handle. Say so, and start the channels over.
        let retry = state
            .borrow()
            .active
            .as_ref()
            .and_then(|a| a.link.as_ref().map(race::Link::id))
            == Some(link.id());
        let dialog = state.borrow().dialog.clone();
        if let Some(active) = state.borrow_mut().active.as_mut() {
            active.link = Some(link.clone());
            active.methods.fp = false;
            active.methods.face = false;
            active.face = None;
        }
        if retry {
            dialog.reject("Wrong password");
            let methods = state.borrow().active.as_ref().map(|a| a.methods);
            if let Some(m) = methods {
                dialog.set_methods(m);
            }
        }
        link.card(true, want.face);
        return;
    }

    // Otherwise this is a terminal `sudo`. Decline the card when it is turned
    // off (pam_race keeps the terminal prompt), queue it when the card is
    // busy, else draw it.
    if !want.terminal_card {
        link.card(false, want.face);
        link.close();
        return;
    }
    if is_busy(state) {
        state
            .borrow_mut()
            .queue
            .push_back(Pending::Sudo { begin, link });
        return;
    }
    start_sudo(state, begin, link, want);
}

fn start_sudo(
    state: &Rc<RefCell<PolkitState>>,
    begin: race::Begin,
    link: race::Link,
    want: Elevate,
) {
    {
        let mut s = state.borrow_mut();
        s.active = Some(Session {
            backing: Backing::Sudo,
            link: Some(link.clone()),
            race_pid: Some(begin.pid),
            methods: Methods {
                password: true,
                fp: false,
                face: false,
            },
            buffered_password: None,
            waiting_password: true,
            face: None,
            settled: false,
        });
    }

    let dialog = state.borrow().dialog.clone();
    let command = face::summarise(&begin.cmdline, &begin.exe);
    let card = Card {
        title: "Administrator access",
        message: "A program is asking to run as root.",
        icon_name: "",
        action_id: "",
        command: Some(command.as_str()),
        identities: &[],
        details: format!("Command: {command}"),
        password: true,
    };
    dialog.present(&card, callbacks(state, false));
    dialog.set_methods(Methods {
        password: true,
        fp: false,
        face: false,
    });
    // Tell pam_race the card is up and whether it may open the camera.
    link.card(true, want.face);
}

fn handle_race_end(state: &Rc<RefCell<PolkitState>>, won: race::Won) {
    match won {
        // A biometric won: pam_race returned success and the stack is
        // accepting the login. `granted` follows for most services, but one
        // that never calls setcred would not send it, so a biometric end is a
        // success in its own right.
        race::Won::Face | race::Won::Fp => settle_success(state),
        // A password was handed on; its verdict is `granted` or the next
        // `begin`. A wrong password ends with neither `won` nor `granted` and
        // reappears as the next `begin`, which the card reads as a retry.
        race::Won::Password | race::Won::None => {}
    }
}

fn link_is_active(state: &Rc<RefCell<PolkitState>>, link: &race::Link) -> bool {
    state
        .borrow()
        .active
        .as_ref()
        .and_then(|a| a.link.as_ref().map(race::Link::id))
        == Some(link.id())
}

fn drop_queued_link(state: &Rc<RefCell<PolkitState>>, link: &race::Link) {
    state.borrow_mut().queue.retain(|p| match p {
        Pending::Sudo { link: l, .. } => l.id() != link.id(),
        Pending::Polkit { .. } => true,
    });
}

/// A method came up or went away. Repaints the caption honestly and, for a
/// reader that just armed, says so.
fn set_channel(state: &Rc<RefCell<PolkitState>>, channel: race::Channel, live: bool) {
    let dialog = state.borrow().dialog.clone();
    let methods = {
        let mut s = state.borrow_mut();
        let Some(active) = s.active.as_mut() else {
            return;
        };
        match channel {
            race::Channel::Fp => active.methods.fp = live,
            race::Channel::Face => active.methods.face = live,
        }
        active.methods
    };
    dialog.set_methods(methods);
}

// ────────────────────────────────────────────────────────────────────────
// User actions
// ────────────────────────────────────────────────────────────────────────

/// The user pressed Enter or the button. Empty text while a face is armed is
/// the Allow press; anything else is a password for whichever backing owns
/// the conversation.
fn handle_submit(state: &Rc<RefCell<PolkitState>>, text: String) {
    if text.is_empty() {
        face::answer(state, true);
        return;
    }

    enum Route {
        Helper,
        Link(race::Link),
        Buffer,
        None,
    }
    let route = {
        let mut s = state.borrow_mut();
        let Some(active) = s.active.as_mut() else {
            return;
        };
        match &active.backing {
            Backing::Polkit {
                helper: Some(_), ..
            } => {
                if active.waiting_password {
                    active.waiting_password = false;
                    Route::Helper
                } else {
                    active.buffered_password = Some(text.clone());
                    Route::Buffer
                }
            }
            Backing::Sudo => active.link.clone().map(Route::Link).unwrap_or(Route::None),
            _ => Route::None,
        }
    };
    let dialog = state.borrow().dialog.clone();
    match route {
        Route::Helper => {
            let mut s = state.borrow_mut();
            if let Some(Backing::Polkit {
                helper: Some(h), ..
            }) = s.active.as_mut().map(|a| &mut a.backing)
                && let Err(e) = h.send_response(&text)
            {
                log::error!("polkit: cannot send password to helper: {e}");
            }
            drop(s);
            dialog.set_verifying(true);
        }
        Route::Link(link) => {
            link.password(&text);
            dialog.set_verifying(true);
        }
        Route::Buffer => dialog.set_verifying(true),
        Route::None => {}
    }
}

fn handle_identity_change(state: &Rc<RefCell<PolkitState>>, uid: u32) {
    let username = {
        let s = state.borrow();
        let Some(active) = s.active.as_ref() else {
            return;
        };
        let Backing::Polkit {
            identities,
            selected_uid,
            ..
        } = &active.backing
        else {
            return;
        };
        if *selected_uid == uid {
            return;
        }
        identities
            .iter()
            .find(|i| i.uid == uid)
            .map(|i| i.username.clone())
    };
    let Some(username) = username else { return };

    {
        let mut s = state.borrow_mut();
        if let Some(Backing::Polkit {
            fd_source,
            helper,
            selected_uid,
            ..
        }) = s.active.as_mut().map(|a| &mut a.backing)
        {
            if let Some(handle) = fd_source.take() {
                cancel_fd_source(&handle);
            }
            helper.take();
            *selected_uid = uid;
        }
    }
    spawn_helper(state, &username);
}

fn cancel_active(state: &Rc<RefCell<PolkitState>>) {
    end_session(state, AuthOutcome::Cancelled);
}

/// Approved. Flash the card, hold it a beat, then resolve. Guarded so a
/// second success signal on the same session does nothing.
fn settle_success(state: &Rc<RefCell<PolkitState>>) {
    {
        let mut s = state.borrow_mut();
        let Some(active) = s.active.as_mut() else {
            return;
        };
        if active.settled {
            return;
        }
        active.settled = true;
    }
    let dialog = state.borrow().dialog.clone();
    dialog.set_status("Authorised", StatusKind::Success);
    dialog.flash_success();
    let s = state.clone();
    glib::timeout_add_local_once(Duration::from_millis(450), move || {
        let same = s.borrow().active.as_ref().is_some_and(|a| a.settled);
        if same {
            end_session(&s, AuthOutcome::Success);
        }
    });
}

/// Take the card down, resolve whatever it owed, and pop the queue.
fn end_session(state: &Rc<RefCell<PolkitState>>, outcome: AuthOutcome) {
    // A face check attached to this card dies with it. Deny costs nothing —
    // the PAM conversation it belonged to is over — and leaving it pending
    // holds the camera open until faced times its own window out.
    face::answer(state, false);

    let dialog = state.borrow().dialog.clone();
    let backing = state.borrow_mut().active.take().map(|a| a.backing);
    if let Some(Backing::Polkit {
        mut fd_source,
        helper,
        mut reply,
        ..
    }) = backing
    {
        if let Some(handle) = fd_source.take() {
            cancel_fd_source(&handle);
        }
        drop(helper); // SIGKILL via Drop
        if let Some(reply) = reply.take() {
            let _ = reply.send(outcome);
        }
    }
    // A `Sudo` or `FaceOnly` link is closed on drop; pam_race's own end/hangup
    // is the authority on the terminal side.

    state.borrow().cue.set(false, "", "");
    dialog.hide();

    pop_queue(state);
}

fn pop_queue(state: &Rc<RefCell<PolkitState>>) {
    if is_busy(state) {
        return;
    }
    let next = state.borrow_mut().queue.pop_front();
    match next {
        Some(Pending::Polkit { request, reply }) => start_polkit(state, request, reply),
        Some(Pending::Sudo { begin, link }) => {
            let want = elevate_settings();
            start_sudo(state, begin, link, want);
        }
        None => {}
    }
}

fn humanise_fingerprint(msg: &str) -> String {
    let lower = msg.to_ascii_lowercase();
    if lower.contains("not centered") || lower.contains("centered") {
        "Centre your finger on the reader".into()
    } else if lower.contains("too short") || lower.contains("swipe") {
        "Swipe again, slower".into()
    } else if lower.contains("remove") {
        "Remove finger and try again".into()
    } else if lower.contains("no match") || lower.contains("not recognised") {
        "Not recognised — try again".into()
    } else if lower.contains("place") || lower.contains("touch") {
        "Touch fingerprint reader".into()
    } else {
        msg.to_string()
    }
}
