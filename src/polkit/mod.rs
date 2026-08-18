//! Standalone polkit authentication agent.
//!
//! Run via `swaypplet polkit-agent`. Owns its own GApplication so it
//! coexists peacefully with the main `swaypplet` panel process.
//!
//! ## Architecture
//!
//! ```text
//!     polkit                                                user
//!       │                                                    │
//!       │ BeginAuthentication                                │
//!       ▼                                                    │
//!  ┌─────────────┐  AgentEvent::Begin   ┌──────────────────┐ │
//!  │ zbus thread │ ───────────────────► │ GTK main thread  │ │
//!  │ (tokio)     │                      │  - PolkitDialog  │◄┘
//!  └─────────────┘  oneshot reply       │  - Helper sub-   │
//!         ▲ ─────────────────────────── │    process       │
//!         │                             │  - fd watcher    │
//!         │                             └──────────────────┘
//! ```
//!
//! polkit-agent-helper-1 is the trusted SUID-root binary that performs
//! the actual PAM conversation. We spawn it, parse its line-protocol
//! stdout, and feed it user input. Fingerprint, password, hardware
//! tokens — everything routes through whatever PAM stack the host
//! configures.

pub(crate) mod agent;
mod cue;
pub(crate) mod dialog;
mod face;
mod helper;
mod session;

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::os::fd::RawFd;
use std::rc::Rc;
use std::time::Duration;

use gio::prelude::*;
use gtk4::Application;
use tokio::sync::oneshot;

use crate::theme;

use agent::{AgentEvent, AuthOutcome, AuthRequest};
use cue::Cue;
use dialog::{PolkitDialog, StatusKind};
use face::FaceSession;
use helper::{Helper, HelperEvent};

const APP_ID: &str = "dev.swaypplet.polkit";

/// Shared handle to the fd-watcher's `SourceId`. The watcher closure
/// holds one clone and the orchestrator holds another (via
/// `ActiveSession::fd_source`). Whichever side wants to dispose of the
/// source calls `cancel_fd_source`, which atomically claims ownership
/// and routes through `SourceId::remove()` exactly once — the other
/// side sees `None` and does nothing. This prevents the non-unwinding
/// panic from glib when two paths race to remove the same source.
type SourceHandle = Rc<Cell<Option<glib::SourceId>>>;

fn cancel_fd_source(handle: &SourceHandle) {
    if let Some(id) = handle.take() {
        crate::spawn::remove_source(id);
    }
}

struct ActiveSession {
    request: AuthRequest,
    helper: Option<Helper>,
    fd_source: Option<SourceHandle>,
    reply: Option<oneshot::Sender<AuthOutcome>>,
    selected_uid: u32,
    /// True after PAM_PROMPT_ECHO_OFF/ON until the user submits a response.
    waiting_password: bool,
    /// A password submitted before PAM asked for one.
    ///
    /// The card shows the entry from the first prompt onward, so a fast user
    /// can submit before the conversation is ready for it. pam_race made that
    /// window small — it prompts within about 200 ms of the stack starting,
    /// rather than after a camera burst and a reader timeout — but small is
    /// not zero, and dropping that keystroke silently is the one thing worse
    /// than not showing the entry at all. It is held here and flushed into
    /// the next prompt.
    buffered_password: Option<String>,
}

impl ActiveSession {
    fn finish(&mut self, outcome: AuthOutcome) {
        if let Some(handle) = self.fd_source.take() {
            cancel_fd_source(&handle);
        }
        // Drop helper (sends SIGKILL via Drop) before resolving reply.
        self.helper.take();
        if let Some(reply) = self.reply.take() {
            let _ = reply.send(outcome);
        }
    }
}

struct PolkitState {
    dialog: Rc<PolkitDialog>,
    /// The look-at-the-camera pill, on its own unblurred surface. See cue.rs.
    cue: Cue,
    active: Option<ActiveSession>,
    queue: VecDeque<PendingRequest>,
    /// A face confirm on screen, from faced. Independent of `active`: for
    /// pkexec it rides on a live polkit session, for sudo there is none.
    face: Option<FaceSession>,
}

struct PendingRequest {
    request: AuthRequest,
    reply: oneshot::Sender<AuthOutcome>,
}

pub fn run() {
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
            face: None,
        }));
        *state_startup.borrow_mut() = Some(inner.clone());

        // The confirm agent for face-authenticated sudo and pkexec lives
        // here rather than in the panel, so one process owns every surface
        // that can authorise something. Two processes meant two cards.
        face::register(&inner);

        // Start zbus agent thread, then poll its event channel from the
        // GTK main loop.
        let agent_rx = agent::start();
        let inner_for_poll = inner.clone();
        glib::timeout_add_local(Duration::from_millis(40), move || {
            while let Ok(event) = agent_rx.try_recv() {
                handle_agent_event(&inner_for_poll, event);
            }
            glib::ControlFlow::Continue
        });
    });

    app.connect_activate(|_app| {
        log::info!("swaypplet polkit agent ready");
    });

    app.connect_shutdown(|_| {
        log::info!("swaypplet polkit agent shutting down");
    });

    // Empty argv: run() would parse std::env::args and treat the
    // `polkit-agent` subcommand word as a file to open, which FLAGS_NONE
    // rejects ("This application can not open files.", exit status 1).
    app.run_with_args::<&str>(&[]);
}

// ────────────────────────────────────────────────────────────────────────
// Event dispatch
// ────────────────────────────────────────────────────────────────────────

fn handle_agent_event(state: &Rc<RefCell<PolkitState>>, event: AgentEvent) {
    match event {
        AgentEvent::Begin { request, reply } => {
            // A standalone face card owns the dialog just as a session does;
            // presenting over it would swap the card under the user's hands
            // while a press was pending on it.
            let busy = {
                let s = state.borrow();
                s.active.is_some() || s.face.as_ref().is_some_and(|f| f.standalone)
            };
            if busy {
                state
                    .borrow_mut()
                    .queue
                    .push_back(PendingRequest { request, reply });
            } else {
                start_session(state, request, reply);
            }
        }
        AgentEvent::Cancel { cookie } => {
            handle_agent_cancel(state, &cookie);
        }
    }
}

fn handle_agent_cancel(state: &Rc<RefCell<PolkitState>>, cookie: &str) {
    // Cancel the active session if it matches…
    let cancel_active = matches!(
        state.borrow().active.as_ref(),
        Some(s) if s.request.cookie == cookie
    );
    if cancel_active {
        end_session(state, AuthOutcome::Cancelled);
        return;
    }
    // …otherwise drop it from the queue.
    let mut s = state.borrow_mut();
    s.queue.retain(|p| p.request.cookie != cookie);
}

fn start_session(
    state: &Rc<RefCell<PolkitState>>,
    request: AuthRequest,
    reply: oneshot::Sender<AuthOutcome>,
) {
    let selected_uid = request.identities[0].uid;
    let initial_username = request.identities[0].username.clone();

    {
        let mut s = state.borrow_mut();
        s.active = Some(ActiveSession {
            request: request.clone(),
            helper: None,
            fd_source: None,
            reply: Some(reply),
            selected_uid,
            waiting_password: false,
            buffered_password: None,
        });
    }

    // Hand the dialog three closures bound to this Rc — they call back
    // into the orchestrator on user actions.
    let dialog = state.borrow().dialog.clone();
    let s_pwd = state.clone();
    let on_password = Box::new(move |pwd: String| handle_user_password(&s_pwd, pwd));
    let s_cancel = state.clone();
    let on_cancel = Box::new(move || end_session(&s_cancel, AuthOutcome::Cancelled));
    let s_ident = state.clone();
    let on_identity = Box::new(move |uid: u32| handle_identity_change(&s_ident, uid));
    // Typing is a decision: it abandons a face check still waiting on the
    // camera so PAM can reach the password without waiting out the burst.
    let s_typing = state.clone();
    let on_typing = Box::new(move || face::abandon(&s_typing));

    dialog.present(&request, on_password, on_cancel, on_identity, on_typing);

    spawn_helper(state, &initial_username);
}

fn spawn_helper(state: &Rc<RefCell<PolkitState>>, username: &str) {
    let cookie = match state.borrow().active.as_ref() {
        Some(s) => s.request.cookie.clone(),
        None => return,
    };

    log::info!("polkit: spawning helper for user {username}");
    match Helper::spawn(username, &cookie) {
        Ok(helper) => {
            let fd = helper.stdout_raw_fd();
            install_fd_watch(state, fd);
            if let Some(active) = state.borrow_mut().active.as_mut() {
                active.helper = Some(helper);
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
            // Give the user a moment to read the error before the modal
            // disappears. Guarded by cookie so that if this session ends
            // meanwhile (cancel) and a queued one becomes active, the timer
            // doesn't error out the wrong session.
            let s = state.clone();
            glib::timeout_add_local_once(Duration::from_secs(3), move || {
                let same_session =
                    s.borrow().active.as_ref().map(|a| &a.request.cookie) == Some(&cookie);
                if same_session {
                    end_session(&s, AuthOutcome::Error("polkit helper unavailable".into()));
                }
            });
        }
    }
}

fn install_fd_watch(state: &Rc<RefCell<PolkitState>>, fd: RawFd) {
    let state_weak = Rc::downgrade(state);
    // Shared slot for the source id. The closure and the orchestrator
    // both hold a clone; whichever calls `.take()` first owns the remove.
    let handle: SourceHandle = Rc::new(Cell::new(None));
    let handle_cb = handle.clone();
    let source = crate::glib_unix::fd_add_local(
        fd,
        glib::IOCondition::IN | glib::IOCondition::HUP | glib::IOCondition::ERR,
        move |_fd, _cond| {
            let Some(state) = state_weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let drained = drain_helper(&state);
            if drained {
                glib::ControlFlow::Continue
            } else {
                // Claim the SourceId so no other path tries to remove
                // it later. glib will auto-remove on Break.
                handle_cb.take();
                glib::ControlFlow::Break
            }
        },
    );
    handle.set(Some(source));
    if let Some(active) = state.borrow_mut().active.as_mut() {
        active.fd_source = Some(handle);
    }
}

/// Drain everything the helper has buffered and dispatch the resulting
/// events. Returns `false` if the source should be removed (because the
/// session ended or the helper disappeared).
fn drain_helper(state: &Rc<RefCell<PolkitState>>) -> bool {
    let (events, eof) = {
        let mut s = state.borrow_mut();
        let Some(active) = s.active.as_mut() else {
            return false;
        };
        let Some(helper) = active.helper.as_mut() else {
            return false;
        };
        helper.read_events()
    };

    for event in events {
        if !apply_helper_event(state, event) {
            return false;
        }
    }

    if eof {
        // Helper closed without SUCCESS/FAILURE — treat as failure.
        let still_active = state.borrow().active.is_some();
        if still_active {
            log::warn!("polkit helper exited unexpectedly");
            let dialog = state.borrow().dialog.clone();
            dialog.set_status("Authentication helper exited", StatusKind::Error);
            dialog.shake();
            // Respawn for retry.
            let username = state
                .borrow()
                .active
                .as_ref()
                .and_then(|a| {
                    a.request
                        .identities
                        .iter()
                        .find(|i| i.uid == a.selected_uid)
                        .map(|i| i.username.clone())
                })
                .unwrap_or_default();
            if !username.is_empty() {
                // Tear down current helper before respawn. We're running
                // inside the watcher callback and will return
                // ControlFlow::Break below, which auto-removes the
                // source — so we just claim ownership of the SourceId
                // (via cancel_fd_source's take) to prevent any later
                // path from trying to remove it.
                if let Some(active) = state.borrow_mut().active.as_mut() {
                    if let Some(handle) = active.fd_source.take() {
                        // Just drop the SourceId; glib will remove it
                        // via the pending Break return from this callback.
                        handle.take();
                    }
                    active.helper.take();
                }
                spawn_helper(state, &username);
            } else {
                // No identity to respawn for — resolve the request instead
                // of leaving a dead dialog that can never complete.
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

/// Apply a single helper event. Returns `false` to signal the watcher
/// should stop (session ended).
fn apply_helper_event(state: &Rc<RefCell<PolkitState>>, event: HelperEvent) -> bool {
    let dialog = state.borrow().dialog.clone();
    match event {
        HelperEvent::PromptEchoOff(prompt) | HelperEvent::PromptEchoOn(prompt) => {
            if helper::is_fingerprint_hint(&prompt) {
                dialog.show_fingerprint(true, "Touch fingerprint reader");
                dialog.set_status("", StatusKind::Info);
            } else {
                dialog.show_fingerprint(false, "");
                dialog.set_password_prompt(&prompt);
                dialog.set_status("", StatusKind::Info);
            }
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
            // The user answered before PAM asked. Send it now rather than
            // making them type it twice.
            if let Some(password) = buffered {
                handle_user_password(state, password);
            }
            true
        }
        HelperEvent::Info(msg) => {
            if helper::is_fingerprint_hint(&msg) {
                let label = humanise_fingerprint(&msg);
                dialog.show_fingerprint(true, &label);
                dialog.set_status("", StatusKind::Info);
            } else {
                dialog.set_status(&msg, StatusKind::Info);
            }
            true
        }
        HelperEvent::Error(msg) => {
            dialog.set_verifying(false);
            dialog.set_status(&msg, StatusKind::Error);
            dialog.shake();
            true
        }
        HelperEvent::Success => {
            // The pills stay. Hiding them here collapsed their row at the same
            // instant the status line appeared, so the card resized twice in
            // opposite directions while the user was still looking at it.
            dialog.set_status("Authorised", StatusKind::Success);
            dialog.flash_success();
            // Brief celebratory hold before dismissing. Guarded by cookie:
            // the user can cancel during the hold, which pops a queued
            // request into `active` — ending *that* session with Success
            // here would approve an unrelated action.
            let s = state.clone();
            let cookie = state
                .borrow()
                .active
                .as_ref()
                .map(|a| a.request.cookie.clone());
            glib::timeout_add_local_once(Duration::from_millis(450), move || {
                let same_session =
                    s.borrow().active.as_ref().map(|a| &a.request.cookie) == cookie.as_ref();
                if same_session {
                    end_session(&s, AuthOutcome::Success);
                }
            });
            false
        }
        HelperEvent::Failure => {
            dialog.set_verifying(false);
            dialog.set_status("Authentication failed", StatusKind::Error);
            dialog.shake();
            // The helper exits after FAILURE. The eof branch in
            // drain_helper will respawn it for retry.
            true
        }
    }
}

// ────────────────────────────────────────────────────────────────────────
// User actions
// ────────────────────────────────────────────────────────────────────────

fn handle_user_password(state: &Rc<RefCell<PolkitState>>, password: String) {
    // An empty submit while a face is armed is the Allow press, not a blank
    // password. A non-empty one is a password, and the user typing it has
    // already abandoned the face check via on_typing.
    if password.is_empty() && face::answer(state, true) {
        return;
    }
    let mut s = state.borrow_mut();
    let Some(active) = s.active.as_mut() else {
        return;
    };
    let Some(helper) = active.helper.as_mut() else {
        return;
    };
    if !active.waiting_password {
        // The helper is not ready for input yet. Hold the response rather
        // than dropping it; the next prompt flushes it.
        active.buffered_password = Some(password);
        drop(s);
        state.borrow().dialog.set_verifying(true);
        return;
    }
    if let Err(e) = helper.send_response(&password) {
        log::error!("polkit: failed to send password to helper: {e}");
    }
    active.waiting_password = false;
    // Grey the card out while the helper runs PAM; any next helper event
    // (prompt, error, failure) clears it. Success closes the dialog anyway.
    let dialog = s.dialog.clone();
    drop(s);
    dialog.set_verifying(true);
}

fn handle_identity_change(state: &Rc<RefCell<PolkitState>>, uid: u32) {
    let username = {
        let s = state.borrow();
        let active = match s.active.as_ref() {
            Some(a) => a,
            None => return,
        };
        if active.selected_uid == uid {
            return;
        }
        active
            .request
            .identities
            .iter()
            .find(|i| i.uid == uid)
            .map(|i| i.username.clone())
    };
    let Some(username) = username else { return };

    // Tear down current helper, restart for the new identity.
    {
        let mut s = state.borrow_mut();
        if let Some(active) = s.active.as_mut() {
            if let Some(handle) = active.fd_source.take() {
                cancel_fd_source(&handle);
            }
            active.helper.take();
            active.selected_uid = uid;
        }
    }
    spawn_helper(state, &username);
}

fn end_session(state: &Rc<RefCell<PolkitState>>, outcome: AuthOutcome) {
    // Any face check attached to this session dies with it, armed or not. It
    // was asked on behalf of a PAM conversation that is over, so a deny costs
    // nothing, and leaving it pending holds the camera and the infrared
    // emitter open until faced times its own confirm window out.
    face::answer(state, false);
    let dialog = state.borrow().dialog.clone();
    {
        let mut s = state.borrow_mut();
        s.face = None;
        if let Some(mut active) = s.active.take() {
            active.finish(outcome);
        }
    }
    dialog.show_face(false, "", "");
    state.borrow().cue.set(false, "", "");
    dialog.hide();

    pop_queue(state);
}

/// Start the next queued request, if the card is free.
fn pop_queue(state: &Rc<RefCell<PolkitState>>) {
    let free = {
        let s = state.borrow();
        s.active.is_none() && !s.face.as_ref().is_some_and(|f| f.standalone)
    };
    if !free {
        return;
    }
    let next = state.borrow_mut().queue.pop_front();
    if let Some(next) = next {
        start_session(state, next.request, next.reply);
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
