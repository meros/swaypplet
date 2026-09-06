//! Face authentication, on the card that was already asking.
//!
//! `sudo` and `pkexec` are two spellings of one question — may this run as
//! root? — so they get one answer surface. faced opens the camera inside
//! pam_race's conversation and asks this process, the confirm agent, to
//! collect the press. This module attaches that check to the session already
//! on the card (`mod.rs` keys sessions by the pid of the process running
//! pam_race, which faced names as the peer), reports the camera through the
//! cue and the caption, arms the Allow button on a match, and answers faced.
//!
//! The card is the session's, not the face's. When the camera gives up the
//! caption says so and the resting sentence drops the camera from the list
//! of live methods; the reader and the password are still racing and the
//! card stays up for them. The old face-only card that vanished the moment
//! the burst ended is now the fallback for a face check that names no
//! session at all.
//!
//! Why the press stays explicit
//! ----------------------------
//! A match says the right person is in front of the camera. It does not say
//! they asked for this: a face is presented by walking into a room. The
//! button is the part a pipe cannot forge — `yes | sudo -S id` answers a PAM
//! prompt and cannot answer this — so it is never skipped and never
//! auto-submitted by some other method succeeding.
//!
//! Why typing abandons the check
//! -----------------------------
//! A keystroke says which method this person chose, and a camera that keeps
//! looking after the answer is elsewhere is a camera running for nothing. So
//! the first keystroke answers `deny`, faced cancels the burst, and the
//! emitter goes dark. Refusing your own face check costs nothing: the
//! password was always going to be accepted. It is a setting
//! (`elevate.typing_abandons_face`) because some people type while they wait.

use std::cell::RefCell;
use std::rc::Rc;

use crate::face::{self, Progress, Request, Stage};

use super::dialog::StatusKind;
use super::{Backing, PolkitState, Session, elevate_settings, end_session, pop_queue};

/// A face confirm currently on screen.
pub(super) struct FaceSession {
    /// faced's id for the attempt.
    pub(super) id: String,
    /// The face has matched and a press now authorises. Until then a press
    /// means nothing, so it cannot be armed by anything but the daemon.
    pub(super) armed: bool,
}

impl FaceSession {
    fn announced(&self) -> bool {
        !self.id.is_empty()
    }
}

/// Strip the store path off a word without hiding what it is.
///
/// `/nix/store/<hash>-sudo-1.9.17p2/bin/sudo` is true and useless: sixty
/// characters of hash in a prompt whose whole job is letting someone
/// recognise, at a glance, the thing they just asked for.
fn prettify(word: &str) -> String {
    match word.rsplit_once('/') {
        Some((head, tail)) if head.starts_with("/nix/store/") && !tail.is_empty() => {
            tail.to_string()
        }
        _ => word.to_string(),
    }
}

/// Shorten a command line for display, keeping both ends.
pub(super) fn summarise(cmdline: &str, exe: &str) -> String {
    let raw = if cmdline.trim().is_empty() {
        exe
    } else {
        cmdline
    };
    let joined: Vec<String> = raw.split_whitespace().map(prettify).collect();
    let text = joined.join(" ");
    let text = text.trim();
    if text.chars().count() <= 72 {
        return text.to_string();
    }
    let head: String = text.chars().take(40).collect();
    let tail: String = text
        .chars()
        .skip(text.chars().count().saturating_sub(28))
        .collect();
    format!("{head}…{tail}")
}

/// Start the confirm agent and route its requests to the card.
pub(super) fn register(state: &Rc<RefCell<PolkitState>>) {
    let (tx, rx) = async_channel::unbounded::<Request>();
    face::start(tx);

    let state = state.clone();
    glib::spawn_future_local(async move {
        while let Ok(req) = rx.recv().await {
            on_request(&state, req);
        }
    });
}

fn on_request(state: &Rc<RefCell<PolkitState>>, req: Request) {
    match req.stage {
        Stage::Announce => announce(state, req),
        Stage::Progress => progress(state, req),
        Stage::Confirm => confirm(state, req),
        Stage::Cancel => cancel(state, req),
    }
}

/// Does the card on screen belong to the process faced names?
fn matches_active(state: &Rc<RefCell<PolkitState>>, pid: u32) -> bool {
    state
        .borrow()
        .active
        .as_ref()
        .and_then(|a| a.race_pid)
        .is_some_and(|p| p != 0 && p == pid)
}

fn announce(state: &Rc<RefCell<PolkitState>>, req: Request) {
    // A confirm already announced means faced started a second attempt with
    // the first unanswered. Refuse the old one rather than let it time out.
    {
        let mut s = state.borrow_mut();
        if let Some(active) = s.active.as_mut()
            && let Some(old) = active.face.as_ref()
            && old.announced()
        {
            face::reply(&old.id, false);
            active.face = None;
        }
    }

    let dialog = state.borrow().dialog.clone();
    let command = summarise(&req.peer_cmdline, &req.peer_exe);

    if matches_active(state, req.peer_pid) {
        // Rides the card already up, whichever backing drew it.
        let mut s = state.borrow_mut();
        if let Some(active) = s.active.as_mut() {
            active.face = Some(FaceSession {
                id: req.id.clone(),
                armed: false,
            });
            active.methods.face = true;
        }
    } else if state.borrow().active.is_some() {
        // Some other card is up; there is nowhere to put this press. Decline
        // now, which also stops the burst, rather than answer a question the
        // user was never shown.
        log::info!(
            "face: declining a confirm for pid {} while another card is up",
            req.peer_pid
        );
        face::reply(&req.id, false);
        return;
    } else {
        // Nothing asked through pam_race: a direct caller. The old
        // face-only card, ours to draw and drop.
        let mut s = state.borrow_mut();
        s.active = Some(Session {
            backing: Backing::FaceOnly,
            link: None,
            race_pid: Some(req.peer_pid),
            methods: super::dialog::Methods {
                face: true,
                fp: false,
                password: false,
            },
            buffered_password: None,
            waiting_password: false,
            face: Some(FaceSession {
                id: req.id.clone(),
                armed: false,
            }),
            settled: false,
        });
        drop(s);
        let card = super::dialog::Card {
            title: "Administrator access",
            message: "A program is asking to run as root.",
            icon_name: "",
            action_id: "",
            command: Some(command.as_str()),
            identities: &[],
            details: format!("Command: {command}"),
            password: false,
        };
        dialog.present(&card, super::callbacks(state, false));
    }

    let methods = state.borrow().active.as_ref().map(|a| a.methods);
    if let Some(m) = methods {
        dialog.set_methods(m);
    }
    report(state, "looking", "Looking for you", "Look at the camera");
}

/// Say what the camera is doing: on the cue by the lens when it is on, else
/// on the card's caption. The cue's wording is the instruction, the card's
/// is the report, so a user reading peripherally is told what to do.
fn report(state: &Rc<RefCell<PolkitState>>, ring: &str, card_text: &str, cue_text: &str) {
    let dialog = state.borrow().dialog.clone();
    if elevate_settings().cue {
        state.borrow().cue.set(true, ring, cue_text);
    } else {
        dialog.hint(card_text);
    }
}

fn progress(state: &Rc<RefCell<PolkitState>>, req: Request) {
    let live = matches!(
        state.borrow().active.as_ref().and_then(|a| a.face.as_ref()),
        Some(f) if f.id == req.id && !f.armed
    );
    if !live {
        return;
    }
    let Some(progress) = Progress::parse(&req.state) else {
        return;
    };
    report(state, progress.ring(), progress.text(), progress.text());
}

fn confirm(state: &Rc<RefCell<PolkitState>>, req: Request) {
    {
        let mut s = state.borrow_mut();
        match s.active.as_mut().and_then(|a| a.face.as_mut()) {
            Some(f) if f.id == req.id => f.armed = true,
            _ => return,
        }
    }
    let dialog = state.borrow().dialog.clone();
    dialog.set_status(
        "Recognised you — press Allow to authorise",
        StatusKind::Info,
    );
    dialog.arm_allow();
    state.borrow().cue.set(true, "ok", "Recognised you");
}

/// The attempt ended on faced's side, by any route. Drop the camera from the
/// card's live methods and say why, without taking the card down: the reader
/// and the password are still racing. Only a card that existed for the face
/// alone goes with it.
fn cancel(state: &Rc<RefCell<PolkitState>>, req: Request) {
    let mine = matches!(
        state.borrow().active.as_ref().and_then(|a| a.face.as_ref()),
        Some(f) if f.id == req.id
    );
    if !mine {
        return;
    }
    let (face_only, methods) = {
        let mut s = state.borrow_mut();
        let Some(active) = s.active.as_mut() else {
            return;
        };
        active.face = None;
        active.methods.face = false;
        (matches!(active.backing, Backing::FaceOnly), active.methods)
    };
    let dialog = state.borrow().dialog.clone();
    state.borrow().cue.set(false, "", "");
    dialog.set_methods(methods);

    // Name what happened, in the lock screen's words, for the outcomes the
    // user can act on. A match, a preemption (they typed) and a decline
    // (they cancelled) need no words: the user already knows.
    let hint = match req.outcome.as_str() {
        "no_face" | "deadline" => "Didn't see you",
        "no_match" => "Didn't recognise you",
        "too_dark" => "No infrared light",
        _ => "",
    };
    if !hint.is_empty() {
        dialog.hint(hint);
    }

    if face_only {
        end_session(state, super::AuthOutcome::Cancelled);
        pop_queue(state);
    }
}

/// Answer the pending confirm, if there is one and it is answerable.
///
/// Returns true when it consumed the action, so the caller knows not to also
/// treat the press as a password submit.
pub(super) fn answer(state: &Rc<RefCell<PolkitState>>, allow: bool) -> bool {
    let (id, armed) = {
        let s = state.borrow();
        match s.active.as_ref().and_then(|a| a.face.as_ref()) {
            Some(f) if f.announced() => (f.id.clone(), f.armed),
            _ => return false,
        }
    };
    if allow && !armed {
        // Pressed before the face matched. Keep waiting: the user agreed to
        // something that has not happened yet.
        return false;
    }
    face::reply(&id, allow);
    // faced's cancel follows and clears the card's face state; nothing more
    // to do here, and for an allow the win arrives through pam_race.
    true
}

/// Give up on a face check the user has stopped waiting for. Only ever a
/// deny, only while unarmed, and only when the setting says typing means
/// that — once the face has matched, the press is the user's to make.
pub(super) fn abandon(state: &Rc<RefCell<PolkitState>>) {
    if !elevate_settings().typing_abandons_face {
        return;
    }
    let unarmed = matches!(
        state.borrow().active.as_ref().and_then(|a| a.face.as_ref()),
        Some(f) if f.announced() && !f.armed
    );
    if unarmed {
        answer(state, false);
    }
}

#[cfg(test)]
mod tests {
    use super::summarise;

    #[test]
    fn a_short_command_is_shown_whole() {
        assert_eq!(
            summarise("sudo systemctl restart foo", "/run/wrappers/bin/sudo"),
            "sudo systemctl restart foo"
        );
    }

    #[test]
    fn the_store_path_is_stripped_but_the_binary_survives() {
        assert_eq!(
            summarise("", "/nix/store/abc123-sudo-1.9.17p2/bin/sudo"),
            "sudo"
        );
    }

    #[test]
    fn a_long_command_keeps_both_ends() {
        let long = format!("sudo sh -c '{}' && systemctl restart foo", "x".repeat(90));
        let out = summarise(&long, "");
        assert!(out.contains('…'), "expected elision, got {out}");
        assert!(out.starts_with("sudo sh -c"), "lost the head: {out}");
        assert!(
            out.ends_with("systemctl restart foo"),
            "lost the tail: {out}"
        );
    }

    #[test]
    fn an_empty_cmdline_falls_back_to_the_binary() {
        assert_eq!(
            summarise("   ", "/run/wrappers/bin/pkexec"),
            "/run/wrappers/bin/pkexec"
        );
    }
}
