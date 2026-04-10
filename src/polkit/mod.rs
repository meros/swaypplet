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

mod agent;
mod dialog;
mod helper;
mod session;

use std::cell::RefCell;
use std::collections::VecDeque;
use std::os::fd::RawFd;
use std::rc::Rc;
use std::time::Duration;

use gio::prelude::*;
use gtk4::Application;
use tokio::sync::oneshot;

use crate::theme;

use agent::{AgentEvent, AuthOutcome, AuthRequest};
use dialog::{PolkitDialog, StatusKind};
use helper::{Helper, HelperEvent};

const APP_ID: &str = "dev.swaypplet.polkit";

struct ActiveSession {
    request: AuthRequest,
    helper: Option<Helper>,
    fd_source: Option<glib::SourceId>,
    reply: Option<oneshot::Sender<AuthOutcome>>,
    selected_uid: u32,
    /// True after PAM_PROMPT_ECHO_OFF/ON until the user submits a response.
    waiting_password: bool,
}

impl ActiveSession {
    fn finish(&mut self, outcome: AuthOutcome) {
        if let Some(source) = self.fd_source.take() {
            source.remove();
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
    active: Option<ActiveSession>,
    queue: VecDeque<PendingRequest>,
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
        let inner = Rc::new(RefCell::new(PolkitState {
            dialog,
            active: None,
            queue: VecDeque::new(),
        }));
        *state_startup.borrow_mut() = Some(inner.clone());

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

    // Force activation even with no command-line args.
    app.run();
}

// ────────────────────────────────────────────────────────────────────────
// Event dispatch
// ────────────────────────────────────────────────────────────────────────

fn handle_agent_event(state: &Rc<RefCell<PolkitState>>, event: AgentEvent) {
    match event {
        AgentEvent::Begin { request, reply } => {
            let has_active = state.borrow().active.is_some();
            if has_active {
                state.borrow_mut().queue.push_back(PendingRequest { request, reply });
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

    dialog.present(&request, on_password, on_cancel, on_identity);

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
            // disappears.
            let s = state.clone();
            glib::timeout_add_local_once(Duration::from_secs(3), move || {
                end_session(&s, AuthOutcome::Error("polkit helper unavailable".into()));
            });
        }
    }
}

fn install_fd_watch(state: &Rc<RefCell<PolkitState>>, fd: RawFd) {
    let state_weak = Rc::downgrade(state);
    let source = glib::unix_fd_add_local(
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
                glib::ControlFlow::Break
            }
        },
    );
    if let Some(active) = state.borrow_mut().active.as_mut() {
        active.fd_source = Some(source);
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
                // Tear down current helper before respawn.
                if let Some(active) = state.borrow_mut().active.as_mut() {
                    if let Some(src) = active.fd_source.take() {
                        src.remove();
                    }
                    active.helper.take();
                }
                spawn_helper(state, &username);
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
            if let Some(active) = state.borrow_mut().active.as_mut() {
                active.waiting_password = true;
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
            dialog.set_status(&msg, StatusKind::Error);
            dialog.shake();
            true
        }
        HelperEvent::Success => {
            dialog.show_fingerprint(false, "");
            dialog.set_status("Authenticated", StatusKind::Success);
            dialog.flash_success();
            // Brief celebratory hold before dismissing.
            let s = state.clone();
            glib::timeout_add_local_once(Duration::from_millis(450), move || {
                end_session(&s, AuthOutcome::Success);
            });
            false
        }
        HelperEvent::Failure => {
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
    let mut s = state.borrow_mut();
    let Some(active) = s.active.as_mut() else { return };
    let Some(helper) = active.helper.as_mut() else { return };
    if !active.waiting_password {
        // Helper isn't ready for input yet — ignore stray submits.
        return;
    }
    if let Err(e) = helper.send_response(&password) {
        log::error!("polkit: failed to send password to helper: {e}");
    }
    active.waiting_password = false;
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
            if let Some(src) = active.fd_source.take() {
                src.remove();
            }
            active.helper.take();
            active.selected_uid = uid;
        }
    }
    spawn_helper(state, &username);
}

fn end_session(state: &Rc<RefCell<PolkitState>>, outcome: AuthOutcome) {
    let dialog = state.borrow().dialog.clone();
    {
        let mut s = state.borrow_mut();
        if let Some(mut active) = s.active.take() {
            active.finish(outcome);
        }
    }
    dialog.hide();

    // Pop the next queued request, if any.
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
