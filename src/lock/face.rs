//! Face verification for the lock screen, triggered by presence.
//!
//! Reports through the same [`crate::fp::EngineEvent`] channel as the
//! fingerprint worker, so a face match unlocks by the same path a finger does.
//!
//! Presence-triggered rather than continuously polled, because every attempt
//! costs something: the IR emitter lights for the duration of the attempt, and
//! the sensor plus the IPU7 spin up behind it. Polling an empty room would
//! burn the emitter at nobody and hold the IPU7 awake on battery. (It no
//! longer lights the visible camera LED — this runs on the infrared sensor,
//! so an attempt is not announced.)
//!
//! Only a *transition* into presence arms an attempt, never presence that was
//! already there. Locking deliberately while sitting at the machine has to
//! stay locked; if the initial state armed it, a manual lock would unlock
//! itself within seconds. So the working sequence is: walk away (the idle
//! manager locks on absence), come back, face unlocks.
//!
//! Resume from sleep arms an attempt too, and that is not the same rule in a
//! different hat. Closing the lid is the one departure the sensor never sees:
//! the machine locks and suspends with the user still in front of it, so
//! presence reads the same either side of the suspend and no edge is ever
//! produced. Face unlock was therefore dead exactly where it is wanted most --
//! lid open, laptop in your hands -- until you got up and came back. The
//! resume edge is the arrival the sensor could not report.
//!
//! Deliberate locks are unaffected. A resume exists only because the machine
//! slept, which on this host means the lid was shut; pressing the lock key and
//! staying put produces neither a resume nor an arrival, so the screen stays
//! locked until a password, a finger, or actually leaving and returning.
//!
//! A lid *opening* arms an attempt too, and for once that is not the same
//! argument again. Two cases close the lid without a suspend behind it, and
//! in both the resume edge above never comes: logind's 30 s holdoff after a
//! resume, and No Sleep mode (`services::inhibit`), where inhibiting the lid
//! switch is half the point. sway's `bindswitch lid:on` still locks and
//! blanks in both, so without this the machine sits locked with the lid open
//! and the one signal that should unlock it never fires. The lid edge is the
//! arrival neither the sensor nor logind reports.
//!
//! Verification asks faced (nixos repo, pkgs/faced) over its session socket,
//! the one face-verification implementation on the system, so "is this the
//! user" is decided in exactly one place. Not a PAM module, for the same
//! reason fingerprint uses fprintd's bus API: the locker's PAM stack stays
//! password-only.
//!
//! One burst per arrival, and faced decides how long it lasts: it stays open
//! while a face is in view, compares only frontal, still frames, and ends on
//! a match, on no face for 1.5 s, or on too many good frames that did not
//! match. So this worker no longer paces attempts. It starts one the moment
//! an arrival edge lands, and after a burst that ended without a match it
//! starts the next one almost at once, for as long as the arrival window
//! lasts. faced bounds the emitter and the attempts on its own side.
//!
//! Exit codes: 0 match, 11 no match / no face / no good look, 13 every frame
//! too dark, 14 busy or cooling down, 10 no enrolled model, 15 locked out,
//! 1 unavailable. The first four are worth retrying; the rest are permanent
//! for this lock episode, and retrying them would only burn the emitter, so
//! they stop the worker.
//!
//! This runs as the session user and needs nothing but the session socket
//! (group `howdy`). The IR node and the enrolled models are root only: faced
//! reads them, and nothing in the session should.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::fp::EngineEvent;
use crate::services::presence::{self, Event as PresenceEvent, Presence};

/// Resume edges from logind, as a channel this thread can drain on its tick.
///
/// One item per PrepareForSleep(true) -> PrepareForSleep(false) transition.
/// The seed reading is deliberately not an edge: a locker spawned *by* the
/// sleep transition starts with the system already going down, and treating
/// that opening `true` as anything but a starting point would be the same
/// mistake presence made.
fn watch_resume() -> mpsc::Receiver<()> {
    let (tx, rx) = mpsc::channel();
    crate::spawn::spawn_tokio_thread("lock-resume", async move {
        let conn = match zbus::Connection::system().await {
            Ok(c) => c,
            Err(e) => {
                log::warn!("face: no system bus, resume will not arm: {e}");
                return;
            }
        };
        let (sleep_tx, mut sleep_rx) = tokio::sync::watch::channel(false);
        tokio::spawn(crate::fp::watch_sleep(conn, sleep_tx));
        let mut asleep = *sleep_rx.borrow_and_update();
        while sleep_rx.changed().await.is_ok() {
            let now = *sleep_rx.borrow_and_update();
            // Only the wake half. Going *into* suspend must not arm anything:
            // the camera would open on a machine that is powering down.
            if asleep && !now && tx.send(()).is_err() {
                return;
            }
            asleep = now;
        }
    });
    rx
}

/// How long after an arrival this worker keeps starting bursts.
///
/// Replaces a fixed four attempts with a 2.5 s gap between them. The gap was
/// there to pace a verifier that judged a fixed 3 s window; faced now keeps a
/// burst open while a face is in view and ends it itself, so the only thing
/// left to bound here is how long one arrival keeps the camera in play. Past
/// this the person is not looking, or it is not them, and the password field
/// is right there. faced's duty window bounds the emitter regardless.
const ARRIVAL_WINDOW: Duration = Duration::from_secs(15);

/// Gaps before the next burst, by how the last one ended. Short: nothing is
/// waited out here any more. A face that was there but never good, or nobody
/// in frame, starts again at once; "didn't recognise you" gets long enough to
/// be read; a busy daemon gets the floor faced enforces anyway.
const AGAIN_SOON: Duration = Duration::from_millis(100);
const AGAIN_AFTER_NO_MATCH: Duration = Duration::from_millis(800);
const AGAIN_AFTER_FAULT: Duration = Duration::from_millis(1000);

enum Attempt {
    Match,
    Retryable { hint: Option<String>, after: Duration },
    /// Locked out: stop for this arrival, try again on the next one.
    StandDown(String),
    Fatal(String),
}

/// Everything that can arm or disarm an attempt, on one channel, so the
/// worker blocks on it instead of polling three sources on a tick. The tick
/// was 250 ms, which was up to 250 ms between the lid opening and the camera
/// being asked.
enum Wake {
    Presence(PresenceEvent),
    Resume,
    Lid(bool),
}

fn wakes() -> mpsc::Receiver<Wake> {
    let (tx, rx) = mpsc::channel();
    // The idle manager owns the device and publishes it; this listens. Doing
    // its own reads here put a third 4 Hz reader on hardware that serves
    // about three reads a second, and every reader queued behind the others.
    let events = presence::subscribe();
    let resumes = watch_resume();
    let lid = crate::services::lid::watch();
    {
        let tx = tx.clone();
        std::thread::spawn(move || {
            while let Ok(ev) = events.recv_blocking() {
                if tx.send(Wake::Presence(ev)).is_err() {
                    return;
                }
            }
        });
    }
    {
        let tx = tx.clone();
        std::thread::spawn(move || {
            while resumes.recv().is_ok() {
                if tx.send(Wake::Resume).is_err() {
                    return;
                }
            }
        });
    }
    std::thread::spawn(move || {
        while let Ok(open) = lid.recv() {
            if tx.send(Wake::Lid(open)).is_err() {
                return;
            }
        }
    });
    rx
}

/// Spawn the face worker. The thread is detached and ends on a match, on a
/// fatal error, or when the lock UI drops the receiver.
pub fn start(user: String) -> async_channel::Receiver<EngineEvent> {
    let (tx, rx) = async_channel::unbounded();
    std::thread::spawn(move || run(&user, &tx));
    rx
}

fn run(user: &str, tx: &async_channel::Sender<EngineEvent>) {
    // No sensor is not nothing to do. Presence is one of the two things that
    // arm an attempt and resume is the other, and resume comes from logind, so
    // a machine without the sensor still face-unlocks when the lid opens. It
    // just never does so from someone walking up to it. Say that once and
    // carry on rather than ending the worker, which is what used to happen and
    // what would now silently take the lid with it.
    if Presence::detect().is_none() {
        let _ = tx.send_blocking(EngineEvent::Unavailable(
            "no presence sensor — face unlock waits for a resume".to_string(),
        ));
    }

    // Register as the lock agent and hold the connection for the whole lock
    // episode. faced treats the connection itself as the registration, so
    // this must outlive every attempt; dropping it deregisters, and faced
    // reads that as the end of the episode (it resets the unlock lockout).
    //
    // This is what lets the daemon refuse unlock verification when no lock
    // screen is actually up, rather than trusting that whoever asked had a
    // good reason. Held in a binding rather than discarded: `let _ = ...`
    // would drop it immediately and deregister on the spot.
    //
    // Checked again before every attempt: a faced restart drops it while this
    // lock screen lives on, and faced then refuses every unlock until the
    // next lock (2026-10-25, a password typed after an `nx`).
    let mut lock_agent = register_lock_agent();

    let wakes = wakes();
    // None until the first reading lands, so the opening state is a starting
    // point rather than an arrival — arming on it would run face unlock
    // against whoever locked the machine a moment ago.
    let mut present: Option<bool> = None;

    // Some(deadline) means an attempt is armed for that instant.
    let mut armed: Option<Instant> = None;
    // The arrival this run of bursts belongs to: when, and what it was.
    let mut arrival: Option<(Instant, &'static str)> = None;
    // Transient Unavailable results, reset on each fresh arrival. See attempt().
    let mut transient = 0u32;

    loop {
        // Block until something happens or the armed attempt is due. Nothing
        // here ticks: an arrival starts the camera as soon as it is read.
        let wait = armed.map_or(Duration::from_secs(3600), |due| {
            due.saturating_duration_since(Instant::now())
        });
        let mut queued = Vec::new();
        match wakes.recv_timeout(wait) {
            Ok(w) => queued.push(w),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            // Every source has gone. Nothing can arm any more; sleep out an
            // armed attempt rather than spin.
            Err(mpsc::RecvTimeoutError::Disconnected) => std::thread::sleep(wait),
        }
        queued.extend(wakes.try_iter());

        // Collapse everything queued into one edge; only a change in settled
        // state is an arrival or a departure.
        let mut edge = None;
        let mut woke = None;
        for w in queued {
            match w {
                Wake::Presence(PresenceEvent::Changed(state)) if present != Some(state) => {
                    let first = present.is_none();
                    present = Some(state);
                    if !first {
                        edge = Some(state);
                    }
                }
                Wake::Presence(_) => {}
                Wake::Resume => woke = Some("resumed from sleep"),
                Wake::Lid(true) => woke = Some("lid opened"),
                Wake::Lid(false) => {}
            }
        }

        match edge {
            Some(true) => {
                log::info!("face: presence returned — arming");
                transient = 0;
                armed = Some(Instant::now());
                arrival = Some((Instant::now(), "presence returned"));
            }
            Some(false) => {
                log::info!("face: presence gone — standing down");
                armed = None;
            }
            None => {}
        }

        // A wake outranks whatever presence last said. The lid was shut and
        // is now open, which is an arrival however still the sensor reads, and
        // it resets the arrival window for the same reason an arrival does:
        // this is a fresh approach to the machine, not the tail of the last
        // one. Collapsed with presence above so a lid open that also resumes
        // arms once, not twice.
        if let Some(why) = woke {
            log::info!("face: {why} — arming");
            transient = 0;
            armed = Some(Instant::now());
            arrival = Some((Instant::now(), why));
        }

        let Some(due) = armed else { continue };
        if Instant::now() < due {
            continue;
        }

        if !lock_agent
            .as_ref()
            .is_some_and(crate::face::still_registered)
        {
            log::info!("face: lock agent registration lost — registering again");
            lock_agent = register_lock_agent();
        }

        match attempt(tx, &mut transient) {
            Attempt::Match => {
                // End to end, for the journal: the arrival edge as this
                // process read it, to the match. faced logs its own split.
                if let Some((at, why)) = arrival {
                    log::info!(
                        "face: match — unlocking, {} ms after {why}",
                        at.elapsed().as_millis()
                    );
                }
                let _ = tx.send_blocking(EngineEvent::Match(user.to_string()));
                return;
            }
            Attempt::Retryable { hint, after } => {
                if let Some(hint) = hint {
                    log::info!("face: {hint}");
                    if tx.send_blocking(EngineEvent::Hint(hint)).is_err() {
                        return;
                    }
                }
                let spent = arrival.map_or(ARRIVAL_WINDOW, |(at, _)| at.elapsed());
                if spent >= ARRIVAL_WINDOW {
                    log::info!(
                        "face: no match in {} s — waiting for next arrival",
                        ARRIVAL_WINDOW.as_secs()
                    );
                    armed = None;
                } else {
                    armed = Some(Instant::now() + after);
                }
            }
            Attempt::StandDown(why) => {
                log::info!("face: {why} — waiting for next arrival");
                if tx.send_blocking(EngineEvent::Hint(why)).is_err() {
                    return;
                }
                armed = None;
            }
            Attempt::Fatal(why) => {
                log::info!("face: disabled — {why}");
                let _ = tx.send_blocking(EngineEvent::Unavailable(why));
                return;
            }
        }
    }
}

/// Run one verification against faced, forwarding progress to the UI.
///
/// Holds a socket for the length of the burst rather than forking a process,
/// so progress and hints arrive while the burst runs.
/// Register with faced as the lock agent. Not fatal when it fails: faced only
/// enforces the registration when configured to, and face unlock working
/// without the extra guard is better than not working at all.
fn register_lock_agent() -> Option<std::os::unix::net::UnixStream> {
    crate::face::register_lock()
        .map_err(|e| log::warn!("face: could not register as lock agent: {e}"))
        .ok()
}

fn attempt(tx: &async_channel::Sender<EngineEvent>, transient: &mut u32) -> Attempt {
    let verdict = crate::face::verify(|state| {
        let _ = tx.send_blocking(EngineEvent::Progress(state));
    });

    let verdict = match verdict {
        Ok(v) => v,
        // The daemon is unreachable. Retryable once: faced is socket
        // activated and may legitimately be restarting underneath us.
        Err(e) => {
            *transient += 1;
            return if *transient <= 1 {
                Attempt::Retryable { hint: None, after: AGAIN_AFTER_FAULT }
            } else {
                Attempt::Fatal(format!("faced unreachable: {e}"))
            };
        }
    };

    map_exit(verdict.exit, verdict.saw_face, &verdict.outcome, transient)
}

/// The exit-code contract, as documented in docs/face-unlock-architecture.md
/// §4.2 in the nixos repo. faced is the authority; this is the reader.
///
/// The default arm is `Retryable`, deliberately. It used to be `Fatal`, which
/// meant any code this match did not know about ended the worker thread and
/// disabled face unlock for the whole lock episode. A single transient stall
/// then looked identical to a broken install, and adding any new code on the
/// verifier side would silently disable the feature on an older locker.
fn map_exit(exit: i32, saw_face: bool, outcome: &str, transient: &mut u32) -> Attempt {
    match exit {
        0 => Attempt::Match,
        // no_match | deadline | no_face. Which one decides the wording, and
        // the daemon is the only thing that knows: "didn't recognise you",
        // "look at the screen" and nothing at all send the user to different
        // actions, so this must not guess.
        11 => match outcome {
            "no_match" => Attempt::Retryable {
                hint: Some("Didn't recognise you".to_string()),
                after: AGAIN_AFTER_NO_MATCH,
            },
            // A face, but never frontal and still enough to compare. The
            // burst already said so live; start the next one at once.
            "deadline" if saw_face => Attempt::Retryable {
                hint: None,
                after: AGAIN_SOON,
            },
            // Nobody in frame for the no-face window. Silent: "Looking for
            // you" is already on screen, and a failure flashed every couple
            // of seconds at someone reading a notification is noise.
            _ => Attempt::Retryable { hint: None, after: AGAIN_SOON },
        },
        // Never the user's fault, and never the room's: the camera carries
        // its own illuminator, so nothing lit means the emitter or the relay
        // is wrong. Asking someone to reposition their face, or to turn a
        // light on, would be misleading.
        13 => Attempt::Retryable {
            hint: Some("No infrared light".to_string()),
            after: AGAIN_AFTER_FAULT,
        },
        // Busy or cooling down: faced's own floor, a fraction of a second.
        14 => Attempt::Retryable { hint: None, after: AGAIN_SOON },
        1 => {
            *transient += 1;
            if *transient <= 1 {
                Attempt::Retryable { hint: None, after: AGAIN_AFTER_FAULT }
            } else {
                Attempt::Fatal("face unlock unavailable".to_string())
            }
        }
        10 => Attempt::Fatal("no enrolled face model".to_string()),
        12 => Attempt::Fatal("compare rejected the username".to_string()),
        // Locked out. The first lockout is timed and the next arrival may
        // find it over; the second holds until a password. Either way a
        // retry is refused before the camera opens, so stand down rather than
        // end the worker.
        15 => Attempt::StandDown("Too many tries — use your password".to_string()),
        16 => Attempt::Fatal("face unlock refused".to_string()),
        17 => Attempt::Fatal("face unlock declined".to_string()),
        code => Attempt::Retryable {
            hint: Some(format!("face unlock returned {code}")),
            after: AGAIN_AFTER_FAULT,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_burst_that_never_got_a_good_look_retries_at_once_and_silently() {
        let mut t = 0;
        assert!(matches!(
            map_exit(11, true, "deadline", &mut t),
            Attempt::Retryable { hint: None, after } if after == AGAIN_SOON
        ));
        assert!(matches!(
            map_exit(11, false, "no_face", &mut t),
            Attempt::Retryable { hint: None, after } if after == AGAIN_SOON
        ));
    }

    #[test]
    fn a_real_no_match_says_so_and_waits_long_enough_to_read() {
        let mut t = 0;
        match map_exit(11, true, "no_match", &mut t) {
            Attempt::Retryable { hint: Some(h), after } => {
                assert_eq!(h, "Didn't recognise you");
                assert_eq!(after, AGAIN_AFTER_NO_MATCH);
            }
            _ => panic!("no_match must be retryable with a hint"),
        }
    }

    #[test]
    fn a_lockout_stands_down_for_this_arrival_rather_than_ending_the_worker() {
        let mut t = 0;
        assert!(matches!(map_exit(15, false, "LockedOut", &mut t), Attempt::StandDown(_)));
    }
}
