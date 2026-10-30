//! The greeter's side of the greetd conversation, as a pure state machine.
//!
//! greetd's protocol is strictly synchronous: one request on the wire, then
//! nothing until its reply. Everything a person can do at the card (type,
//! press Enter, pick another user, touch the reader) arrives on its own
//! schedule, so the rules for when each of those may reach greetd live here,
//! where they can be tested, rather than in the GTK closures that used to
//! hold them. [`Conversation::on`] takes one input and returns what the
//! surfaces, greetd and the fp-agent have to be told, in order.

use std::collections::HashMap;

use greetd_ipc::AuthMessageType;
use swaypplet_fp::agent::{Cmd as FpCmd, Ev as FpEv};
use swaypplet_ui::auth_field::Tone;

use crate::ipc;

/// Something that happened at the card, on greetd's socket or at the reader.
pub enum Input {
    /// The window is up: open the conversation for the prefilled user and
    /// arm the reader for them.
    Start,
    /// The full picker data landed (`switch_user::list`).
    Roster {
        enrolled: HashMap<String, bool>,
        logged_in: HashMap<String, bool>,
    },
    /// Enter in the password field (or a fingerprint token standing in for
    /// it). Empty is a bare Enter.
    Submit(String),
    /// A user was picked under the card.
    SwitchUser(String),
    Greetd(ipc::Ev),
    Fp(FpEv),
}

/// What [`Conversation::on`] asks of the world.
#[derive(PartialEq)]
pub enum Effect {
    Greetd(ipc::Req),
    Fp(FpCmd),
    Status(String, Tone),
    Verifying(bool),
    /// The card is now for this user.
    Username(String),
    FpArmed(bool),
    FpHint(String),
    Shake,
    FlashSuccess,
    /// greetd started the session: exit so it can hand over.
    Started,
    /// greetd's socket failed: exit with an error.
    Fatal(String),
}

struct Pending {
    user: String,
    password: String,
}

pub struct Conversation {
    /// The user the card is for.
    selected: String,
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
    /// Users with a live graphical session from `--list`: signing in as one
    /// resumes that session (the dispatcher's `switch-user --resume`).
    logged_in: HashMap<String, bool>,
}

impl Conversation {
    pub fn new(selected: String, session_cmd: String, session_env: Vec<String>) -> Self {
        Self {
            selected,
            session_cmd,
            session_env,
            session_user: None,
            prompt_open: false,
            blocked: false,
            canceling: false,
            pending: None,
            recreate_user: None,
            switch_pending: None,
            enrolled: HashMap::new(),
            logged_in: HashMap::new(),
        }
    }

    pub fn on(&mut self, input: Input) -> Vec<Effect> {
        let mut out = Vec::new();
        match input {
            Input::Start => {
                if let Some(user) = self.username() {
                    self.session_user = Some(user.clone());
                    self.send(ipc::Req::Create { username: user }, &mut out);
                }
                self.retarget_fp(&mut out);
            }
            Input::Roster {
                enrolled,
                logged_in,
            } => {
                self.enrolled = enrolled;
                self.logged_in = logged_in;
                self.retarget_fp(&mut out);
            }
            Input::Submit(password) => self.submit(password, &mut out),
            Input::SwitchUser(user) => self.switch_user(user, &mut out),
            Input::Greetd(ev) => self.greetd(ev, &mut out),
            Input::Fp(ev) => self.fp(ev, &mut out),
        }
        out
    }

    /// The user the card is for, if it names one.
    pub fn username(&self) -> Option<String> {
        Some(self.selected.clone()).filter(|u| !u.is_empty())
    }

    fn send(&mut self, req: ipc::Req, out: &mut Vec<Effect>) {
        self.blocked = true;
        out.push(Effect::Greetd(req));
    }

    /// Whether `user` has an enrolled fingerprint. Unknown (no `--list`) →
    /// `true`; the fp-agent double-checks enrollment anyway.
    fn fp_enrolled(&self, user: &str) -> bool {
        self.enrolled.get(user).copied().unwrap_or(true)
    }

    /// Whether `user` already has a live graphical session. Unknown (no
    /// `--list` yet) → `false`; only the wording depends on it, since the
    /// dispatcher decides for itself.
    fn has_session(&self, user: &str) -> bool {
        self.logged_in.get(user).copied().unwrap_or(false)
    }

    /// Point the fp-agent at the currently selected user (or stand it down
    /// for an unenrolled one). The pill hides until the agent reports Ready
    /// for the new target. A signed-in user gets it too: the finger resumes
    /// their session the way the password does.
    fn retarget_fp(&mut self, out: &mut Vec<Effect>) {
        match self.username() {
            Some(user) if self.fp_enrolled(&user) => {
                out.push(Effect::FpArmed(false));
                out.push(Effect::Fp(FpCmd::Verify { user }));
            }
            _ => {
                out.push(Effect::Fp(FpCmd::Stop));
                out.push(Effect::FpArmed(false));
            }
        }
    }

    /// Cancel the current greetd session and recreate it for `user`. With the
    /// PAM stack password-only the conversation always rests at an answerable
    /// prompt, so the cancel is acknowledged immediately.
    fn restart_conversation(&mut self, user: String, out: &mut Vec<Effect>) {
        self.prompt_open = false;
        self.canceling = true;
        self.recreate_user = Some(user);
        self.send(ipc::Req::Cancel, out);
    }

    fn submit(&mut self, password: String, out: &mut Vec<Effect>) {
        if password.is_empty() && !self.prompt_open {
            return;
        }
        let Some(user) = self.username() else {
            out.push(Effect::Status("Enter a username".into(), Tone::Error));
            return;
        };

        if self.canceling {
            // Restart already in flight — this secret rides along.
            self.pending = Some(Pending {
                user: user.clone(),
                password,
            });
            self.recreate_user = Some(user);
            return;
        }

        let same_user = self.session_user.as_deref() == Some(user.as_str());
        if self.prompt_open && same_user {
            self.prompt_open = false;
            out.push(Effect::Status(String::new(), Tone::Info));
            out.push(Effect::Verifying(true));
            self.send(ipc::Req::Respond(Some(password)), out);
            return;
        }

        // No prompt open (conversation still being created) or the
        // conversation belongs to another user: park the secret; it
        // auto-submits at the next matching prompt.
        self.pending = Some(Pending {
            user: user.clone(),
            password,
        });
        out.push(Effect::Status(String::new(), Tone::Info));
        out.push(Effect::Verifying(true));
        if !self.blocked {
            self.restart_conversation(user, out);
        }
    }

    /// A user was picked under the card: the greetd conversation restarts for
    /// them and the reader is retargeted, signed in or not.
    fn switch_user(&mut self, user: String, out: &mut Vec<Effect>) {
        let already_current = self.session_user.as_deref() == Some(user.as_str())
            && self.username().as_deref() == Some(user.as_str());
        if already_current {
            return;
        }
        self.selected = user.clone();
        out.push(Effect::Username(user.clone()));
        self.pending = None;
        out.push(Effect::Status(String::new(), Tone::Info));
        out.push(Effect::Verifying(false));
        self.retarget_fp(out);
        if self.canceling {
            // A restart is mid-flight; just retarget the recreate.
            self.recreate_user = Some(user);
            return;
        }
        if self.blocked {
            // greetd can't hear a cancel until the in-flight request resolves
            // (milliseconds). The restart runs the moment greetd replies
            // (`switch_pending` in `greetd`).
            self.switch_pending = Some(user);
            return;
        }
        self.restart_conversation(user, out);
    }

    fn fp(&mut self, ev: FpEv, out: &mut Vec<Effect>) {
        match ev {
            FpEv::Ready => out.push(Effect::FpArmed(true)),
            FpEv::Hint { msg } => out.push(Effect::FpHint(msg)),
            FpEv::Unavailable { msg } => {
                log::info!("fingerprint unavailable: {msg}");
                out.push(Effect::FpArmed(false));
            }
            FpEv::Match { user, token } => {
                // The agent verified `user`'s finger on the reader. Guard
                // against a stale match racing a chip switch: only submit if
                // that user is still the one selected on screen.
                if self.username().as_deref() != Some(user.as_str()) {
                    log::info!("discarding fingerprint match for deselected {user}");
                    return;
                }
                out.push(Effect::FpHint("Fingerprint OK".into()));
                self.submit(token, out);
            }
        }
    }

    fn greetd(&mut self, ev: ipc::Ev, out: &mut Vec<Effect>) {
        self.blocked = false;
        // A chip click landed while the previous request was in flight;
        // greetd can hear a cancel again only now. Restart the conversation
        // for the new user instead of serving the old one's message. Error
        // needs no special case (it already recreates for the selected
        // name). SessionReady here means the OLD user's auth completed after
        // someone else was selected — the UI already shows the new user, so
        // starting the old session would open A's desktop under B's name.
        // The person at the keyboard wins; the completed auth is discarded.
        if let Some(user) = self.switch_pending.take()
            && matches!(ev, ipc::Ev::AuthMessage { .. } | ipc::Ev::SessionReady)
        {
            self.restart_conversation(user, out);
            return;
        }
        match ev {
            ipc::Ev::AuthMessage { kind, text } => {
                if self.canceling {
                    return; // stale message from the dying session
                }
                match kind {
                    AuthMessageType::Secret | AuthMessageType::Visible => {
                        if matches!(kind, AuthMessageType::Visible) {
                            out.push(Effect::Status(text, Tone::Info));
                        }
                        match self.pending.take() {
                            Some(p) if self.session_user.as_deref() == Some(p.user.as_str()) => {
                                self.send(ipc::Req::Respond(Some(p.password)), out);
                            }
                            Some(p) => {
                                // Username changed while waiting — start over.
                                let user = p.user.clone();
                                self.pending = Some(p);
                                self.restart_conversation(user, out);
                            }
                            None => {
                                self.prompt_open = true;
                                out.push(Effect::Verifying(false));
                            }
                        }
                    }
                    AuthMessageType::Info | AuthMessageType::Error => {
                        // Rare with a password-only stack (expired account
                        // and the like) — show and ack immediately so the
                        // conversation keeps moving.
                        let tone = if matches!(kind, AuthMessageType::Info) {
                            Tone::Info
                        } else {
                            Tone::Error
                        };
                        out.push(Effect::Status(text, tone));
                        self.send(ipc::Req::Respond(None), out);
                    }
                }
            }
            ipc::Ev::SessionReady => {
                // Stand the reader down before handover so the session's own
                // locker (or the next greeter) can claim it cleanly.
                out.push(Effect::Fp(FpCmd::Stop));
                out.push(Effect::FlashSuccess);
                if self
                    .session_user
                    .as_deref()
                    .is_some_and(|u| self.has_session(u))
                {
                    out.push(Effect::Status("Resuming\u{2026}".into(), Tone::Info));
                }
                let cmd = self.session_cmd.clone();
                let env = self.session_env.clone();
                self.send(ipc::Req::Start { cmd, env }, out);
            }
            ipc::Ev::SessionStarted => out.push(Effect::Started),
            ipc::Ev::Canceled => {
                self.canceling = false;
                if let Some(user) = self.recreate_user.take() {
                    self.session_user = Some(user.clone());
                    self.send(ipc::Req::Create { username: user }, out);
                }
            }
            ipc::Ev::Error { auth, text } => {
                if self.canceling {
                    return; // error from the dying session; Canceled follows
                }
                // The session is gone either way; reset and start a fresh one.
                self.session_user = None;
                self.prompt_open = false;
                self.pending = None;
                out.push(Effect::Verifying(false));
                if auth {
                    log::info!("auth failed: {text}");
                    // Same words the lock screen uses. Neither surface names
                    // its own mode — "Login failed" here and "Wrong password"
                    // there told the same person two different stories about
                    // the same mistyped key.
                    out.push(Effect::Status("Wrong password".into(), Tone::Error));
                    out.push(Effect::Shake);
                } else {
                    log::warn!("greetd error: {text}");
                    out.push(Effect::Status(text, Tone::Error));
                }
                if let Some(user) = self.username() {
                    self.session_user = Some(user.clone());
                    self.send(ipc::Req::Create { username: user }, out);
                }
                // Re-send the fp target: a fingerprint match parks the engine
                // until the next command, so a matched-but-rejected token (TTL
                // expired, Respond raced a chip switch) would otherwise leave
                // the pill frozen on "Fingerprint OK" with a dead reader. The
                // engine answers every re-send with its current state.
                self.retarget_fp(out);
            }
            ipc::Ev::Fatal(text) => out.push(Effect::Fatal(text)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn greeter(user: &str) -> Conversation {
        Conversation::new(user.into(), "sway".into(), Vec::new())
    }

    fn secret() -> Input {
        Input::Greetd(ipc::Ev::AuthMessage {
            kind: AuthMessageType::Secret,
            text: "Password:".into(),
        })
    }

    fn sent(effects: &[Effect]) -> Vec<&ipc::Req> {
        effects
            .iter()
            .filter_map(|e| match e {
                Effect::Greetd(r) => Some(r),
                _ => None,
            })
            .collect()
    }

    fn create(user: &str) -> ipc::Req {
        ipc::Req::Create {
            username: user.into(),
        }
    }

    fn respond(password: &str) -> ipc::Req {
        ipc::Req::Respond(Some(password.into()))
    }

    /// A greeter for `user` sitting at greetd's password prompt.
    fn at_prompt(user: &str) -> Conversation {
        let mut c = greeter(user);
        assert!(sent(&c.on(Input::Start)) == [&create(user)]);
        assert!(sent(&c.on(secret())).is_empty());
        c
    }

    #[test]
    fn the_prefilled_user_gets_a_conversation_and_the_reader() {
        let mut c = greeter("meros");
        let out = c.on(Input::Start);
        assert!(sent(&out) == [&create("meros")]);
        assert!(out.contains(&Effect::Fp(FpCmd::Verify {
            user: "meros".into()
        })));
    }

    #[test]
    fn enter_at_the_prompt_answers_it() {
        let mut c = at_prompt("meros");
        assert!(sent(&c.on(Input::Submit("hunter2".into()))) == [&respond("hunter2")]);
    }

    #[test]
    fn a_bare_enter_before_any_prompt_does_nothing() {
        let mut c = greeter("meros");
        c.on(Input::Start);
        assert!(c.on(Input::Submit(String::new())).is_empty());
    }

    #[test]
    fn a_password_typed_before_the_prompt_waits_for_it() {
        let mut c = greeter("meros");
        c.on(Input::Start);
        // Create is still in flight: nothing reaches greetd yet.
        assert!(sent(&c.on(Input::Submit("hunter2".into()))).is_empty());
        assert!(sent(&c.on(secret())) == [&respond("hunter2")]);
    }

    #[test]
    fn a_pick_while_greetd_is_busy_restarts_on_its_reply() {
        let mut c = greeter("meros");
        c.on(Input::Start);
        // Create for meros is in flight; greetd cannot hear a cancel yet.
        let out = c.on(Input::SwitchUser("melvin".into()));
        assert!(sent(&out).is_empty());
        assert!(out.contains(&Effect::Username("melvin".into())));
        // meros's prompt arrives: it is not shown, the conversation restarts.
        assert!(sent(&c.on(secret())) == [&ipc::Req::Cancel]);
        assert!(sent(&c.on(Input::Greetd(ipc::Ev::Canceled))) == [&create("melvin")]);
    }

    #[test]
    fn the_old_users_success_after_a_pick_never_starts_their_session() {
        let mut c = at_prompt("meros");
        c.on(Input::Submit("hunter2".into()));
        // Someone else is picked while meros's password is with PAM.
        c.on(Input::SwitchUser("melvin".into()));
        let out = c.on(Input::Greetd(ipc::Ev::SessionReady));
        assert!(sent(&out) == [&ipc::Req::Cancel]);
        assert!(!out.contains(&Effect::FlashSuccess));
    }

    #[test]
    fn a_secret_typed_mid_restart_rides_along() {
        let mut c = at_prompt("meros");
        c.on(Input::SwitchUser("melvin".into()));
        assert!(sent(&c.on(Input::Submit("melvins".into()))).is_empty());
        assert!(sent(&c.on(Input::Greetd(ipc::Ev::Canceled))) == [&create("melvin")]);
        assert!(sent(&c.on(secret())) == [&respond("melvins")]);
    }

    #[test]
    fn the_dying_sessions_messages_are_dropped() {
        let mut c = at_prompt("meros");
        c.on(Input::SwitchUser("melvin".into()));
        // Cancel is on the wire; a late prompt or error from meros's session
        // must not be answered or shown.
        assert!(c.on(secret()).is_empty());
        let late = Input::Greetd(ipc::Ev::Error {
            auth: true,
            text: "late".into(),
        });
        assert!(c.on(late).is_empty());
    }

    #[test]
    fn a_match_for_a_deselected_user_is_dropped() {
        let mut c = at_prompt("meros");
        c.on(Input::SwitchUser("melvin".into()));
        let stale = Input::Fp(FpEv::Match {
            user: "meros".into(),
            token: "t".into(),
        });
        assert!(c.on(stale).is_empty());
    }

    #[test]
    fn a_match_for_the_selected_user_submits_its_token() {
        let mut c = at_prompt("meros");
        let out = c.on(Input::Fp(FpEv::Match {
            user: "meros".into(),
            token: "tok".into(),
        }));
        assert!(out.contains(&Effect::FpHint("Fingerprint OK".into())));
        assert!(sent(&out) == [&respond("tok")]);
    }

    #[test]
    fn a_wrong_password_says_so_and_starts_over() {
        let mut c = at_prompt("meros");
        c.on(Input::Submit("nope".into()));
        let out = c.on(Input::Greetd(ipc::Ev::Error {
            auth: true,
            text: "pam".into(),
        }));
        assert!(out.contains(&Effect::Status("Wrong password".into(), Tone::Error)));
        assert!(out.contains(&Effect::Shake));
        assert!(sent(&out) == [&create("meros")]);
        // The reader is re-sent its target, so a rejected token cannot leave
        // it parked.
        assert!(out.contains(&Effect::Fp(FpCmd::Verify {
            user: "meros".into()
        })));
    }

    #[test]
    fn an_unenrolled_user_stands_the_reader_down() {
        let mut c = greeter("melvin");
        let out = c.on(Input::Roster {
            enrolled: HashMap::from([("melvin".into(), false)]),
            logged_in: HashMap::new(),
        });
        assert!(out.contains(&Effect::Fp(FpCmd::Stop)));
    }

    #[test]
    fn a_signed_in_user_is_told_their_session_resumes() {
        let mut c = at_prompt("meros");
        c.on(Input::Roster {
            enrolled: HashMap::new(),
            logged_in: HashMap::from([("meros".into(), true)]),
        });
        c.on(Input::Submit("hunter2".into()));
        let out = c.on(Input::Greetd(ipc::Ev::SessionReady));
        assert!(out.contains(&Effect::Status("Resuming\u{2026}".into(), Tone::Info)));
        assert!(out.contains(&Effect::Fp(FpCmd::Stop)));
    }
}
