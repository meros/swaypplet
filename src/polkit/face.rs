//! Face authentication, on the card that was already asking.
//!
//! `sudo` and `pkexec` are two spellings of one question — may this run as
//! root? — so they get one answer surface. Before this, face elevation drew a
//! card of its own, which meant `pkexec` stacked two prompts with two Cancel
//! buttons and no indication which one the user was answering. The confirm
//! agent now lives in the polkit agent process and drives the polkit card.
//!
//! Two shapes, one card:
//!
//!   * **pkexec** — polkit is already showing the card, because pam_face runs
//!     inside the PAM conversation polkit-agent-helper-1 is having. The face
//!     pill and the camera cue attach to the card that is up.
//!   * **sudo** — nothing else is asking, so the card is synthesised from the
//!     peer fields faced supplies. Same card, same wording, same cue.
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
//! PAM is serial. While pam_face waits, the stack has not reached fprintd or
//! the password, so a user who has already decided to type would otherwise
//! watch a camera they are not looking at hold up their prompt. The first
//! keystroke answers `deny`, pam_face returns PAM_AUTHINFO_UNAVAIL, and the
//! stack moves on immediately. Refusing your own face check costs nothing:
//! the password was always going to be accepted.

use std::cell::RefCell;
use std::rc::Rc;

use crate::face::{self, Request, Stage};

use super::dialog::StatusKind;
use super::{pop_queue, PolkitState};

/// A face confirm currently on screen.
pub(super) struct FaceSession {
    pub(super) id: String,
    /// The face has matched and a press now authorises. Until then a press
    /// means nothing, so it cannot be armed by anything but the daemon.
    pub(super) armed: bool,
    /// We drew the card ourselves (`sudo`), rather than attaching to a polkit
    /// session already showing one (`pkexec`).
    pub(super) standalone: bool,
}

/// Strip the store path off a word without hiding what it is.
///
/// `/nix/store/<hash>-sudo-1.9.17p2/bin/sudo` is true and useless: sixty
/// characters of hash in a prompt whose whole job is letting someone
/// recognise, at a glance, the thing they just asked for. Everything on this
/// system is a store path, so showing them distinguishes nothing.
fn prettify(word: &str) -> String {
    match word.rsplit_once('/') {
        Some((head, tail)) if head.starts_with("/nix/store/") && !tail.is_empty() => {
            tail.to_string()
        }
        _ => word.to_string(),
    }
}

/// Shorten a command line for display, keeping both ends.
///
/// The tail is what matters (`systemctl restart foo`) and so is the leading
/// binary, so the middle is what gets elided.
pub(super) fn summarise(cmdline: &str, exe: &str) -> String {
    let raw = if cmdline.trim().is_empty() { exe } else { cmdline };
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
        Stage::Cancel => {
            // The daemon gave up. Drop the request without answering: it is
            // already gone on the other side, and a late reply would apply to
            // nothing. Guarded by id so a cancel for a superseded request
            // cannot tear down the one currently on screen.
            let matches = matches!(
                state.borrow().face.as_ref(),
                Some(f) if f.id == req.id
            );
            if matches {
                clear(state);
            }
        }
    }
}

fn announce(state: &Rc<RefCell<PolkitState>>, req: Request) {
    // A face confirm already up means faced started a second attempt without
    // the first being answered. Refuse the old one rather than leaving it to
    // time out; the user is about to be shown the new one and must not be
    // answering a question that has scrolled away.
    if let Some(old) = state.borrow_mut().face.take() {
        face::reply(&old.id, false);
    }

    let attached = state.borrow().active.is_some();
    let dialog = state.borrow().dialog.clone();
    let command = summarise(&req.peer_cmdline, &req.peer_exe);

    if !attached {
        let s_allow = state.clone();
        let s_cancel = state.clone();
        dialog.present_elevate(
            &command,
            Box::new(move |_| {
                answer(&s_allow, true);
            }),
            Box::new(move || {
                answer(&s_cancel, false);
            }),
        );
    }

    dialog.show_face(true, "looking", "Looking for you");
    dialog.set_status("", StatusKind::Info);
    // The cue says what to do; the card says what is happening. The cue is
    // read peripherally, on the way to the lens, and "looking for you" is not
    // an instruction.
    state.borrow().cue.set(true, "looking", "Look at the camera");

    state.borrow_mut().face = Some(FaceSession {
        id: req.id,
        armed: false,
        standalone: !attached,
    });
}

/// Report what the camera is doing, in the lock screen's vocabulary.
///
/// Same three states, same wording, same ring. Someone who has learned to read
/// the pill while unlocking should not have to learn a second dialect to read
/// it while authorising `sudo`.
fn progress(state: &Rc<RefCell<PolkitState>>, req: Request) {
    // Only for the request currently on screen, and never after it has armed:
    // a late frame state must not overwrite "Recognised you" and un-explain a
    // button that is already asking to be pressed.
    let live = matches!(
        state.borrow().face.as_ref(),
        Some(f) if f.id == req.id && !f.armed
    );
    if !live {
        return;
    }
    let (ring, text) = match req.state.as_str() {
        "looking" => ("looking", "Looking for you"),
        // Never phrased as the user's fault. The emitter or the relay is
        // wrong, and telling somebody to move their face sends them after the
        // wrong thing entirely.
        "dark" => ("dark", "Too dark to see"),
        "face" => ("found", "Hold still"),
        _ => return,
    };
    let dialog = state.borrow().dialog.clone();
    dialog.show_face(true, ring, text);
    state.borrow().cue.set(true, ring, text);
}

fn confirm(state: &Rc<RefCell<PolkitState>>, req: Request) {
    {
        let mut s = state.borrow_mut();
        match s.face.as_mut() {
            // Only the request we announced may arm. Anything else is a reply
            // to a question the user was never shown.
            Some(f) if f.id == req.id => f.armed = true,
            _ => return,
        }
    }
    let dialog = state.borrow().dialog.clone();
    dialog.show_face(true, "ok", "Recognised you");
    dialog.set_status("Press Allow to authorise", StatusKind::Info);
    dialog.arm_allow();
    state.borrow().cue.set(true, "ok", "Recognised you");
}

/// Answer the pending confirm, if there is one and it is answerable.
///
/// Returns true when it consumed the action, so the caller knows not to also
/// treat the press as a password submit.
pub(super) fn answer(state: &Rc<RefCell<PolkitState>>, allow: bool) -> bool {
    let Some(session) = state.borrow_mut().face.take() else {
        return false;
    };
    if allow && !session.armed {
        // Pressed before the face matched. Put it back and keep waiting: the
        // user agreed to something that has not happened yet.
        state.borrow_mut().face = Some(session);
        return false;
    }
    face::reply(&session.id, allow);
    finish(state, session.standalone);
    true
}

/// Give up on a face check the user has stopped waiting for.
///
/// Only ever a deny, and only while unarmed — once the face has matched, the
/// press is the user's to make and nothing here may make it for them.
pub(super) fn abandon(state: &Rc<RefCell<PolkitState>>) {
    let unarmed = matches!(state.borrow().face.as_ref(), Some(f) if !f.armed);
    if !unarmed {
        return;
    }
    let Some(session) = state.borrow_mut().face.take() else {
        return;
    };
    face::reply(&session.id, false);
    finish(state, session.standalone);
}

/// Drop the surface without answering. Only for a request the daemon has
/// already withdrawn.
fn clear(state: &Rc<RefCell<PolkitState>>) {
    let Some(session) = state.borrow_mut().face.take() else {
        return;
    };
    finish(state, session.standalone);
}

fn finish(state: &Rc<RefCell<PolkitState>>, standalone: bool) {
    let dialog = state.borrow().dialog.clone();
    dialog.show_face(false, "", "");
    state.borrow().cue.set(false, "", "");
    if standalone {
        // Our card, so ours to take down — and a polkit request may have been
        // queued behind it while it was up.
        dialog.hide();
        pop_queue(state);
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
        assert!(out.ends_with("systemctl restart foo"), "lost the tail: {out}");
    }

    #[test]
    fn an_empty_cmdline_falls_back_to_the_binary() {
        assert_eq!(
            summarise("   ", "/run/wrappers/bin/pkexec"),
            "/run/wrappers/bin/pkexec"
        );
    }
}
