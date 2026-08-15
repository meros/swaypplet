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
//! locked until a password, a finger, or actually leaving and returning. The
//! one gap is logind's 30 s holdoff after a resume, inside which a lid close
//! does not suspend and so a lid open does not resume; that window falls back
//! to presence, as it did before.
//!
//! Verification shells out to `howdy-verify <user>` (nixos repo,
//! pkgs/howdy-verify), the one face-verification implementation on the system —
//! the same binary PAM uses for sudo and pkexec, so "is this the user" is
//! decided in exactly one place. It reads howdy's config, model format and
//! dlib data, but replaces howdy's compare.py, which is a script rather than a
//! library and whose only route into PAM synthesises an Enter keypress through
//! uinput. Not a PAM module here either, for the same reason fingerprint uses
//! fprintd's bus API: the locker's PAM stack stays password-only.
//!
//! Exit codes (inherited from compare.py, so the contract predates the
//! rewrite): 0 match, 11 no match before its own timeout, 13 every frame too
//! dark, 10 no enrolled model, 1 detector or camera init failed, 12 no
//! username given. The first three are worth retrying; the rest are permanent
//! for this session, and retrying them would just burn the emitter, so they
//! stop the worker.
//!
//! This runs as the session user, so it needs read access to the models
//! (/var/lib/howdy/models) and the camera node, both group `howdy` — the IR
//! node is deliberately not group `video`, because whoever can open the
//! loopback's OUTPUT side can inject frames, and an injected frame is a face
//! unlock bypass needing neither a photo nor physical access. Group changes
//! need a fresh login before the locker inherits them.

use std::sync::mpsc;
use std::thread::sleep;
use std::time::{Duration, Instant};

use crate::fp::EngineEvent;
use crate::presence::{self, Event as PresenceEvent, Presence};

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

/// Tick for this engine's own deadlines. Presence itself is pushed from
/// whoever owns the sensor (see `crate::presence`), so this no longer sets
/// a sampling rate — it only decides how soon after arrival an attempt
/// starts, and how finely `RETRY_AFTER` is honoured.
const POLL: Duration = Duration::from_millis(250);

/// Gap between attempts while presence holds.
///
/// Was 6 s, sized around a verifier that spent 2.097 s on interpreter and
/// dlib startup before looking at a frame: retrying sooner mostly bought more
/// startup. That cost is now zero, an attempt is about 0.6 s warm, and the
/// daemon enforces its own 1.5 s cooldown, so the gap only has to be long
/// enough for a person to react to "didn't recognise you" and adjust.
const RETRY_AFTER: Duration = Duration::from_millis(2500);

/// Attempts per arrival before falling silent until the next one.
///
/// Four rather than three, because each one now costs a fraction of what it
/// did and the whole sequence finishes in about 12 s instead of 21 s. Beyond
/// that the light is wrong or it is not you, and the password field is right
/// there.
const MAX_ATTEMPTS: u32 = 4;

enum Attempt {
    Match,
    Retryable(Option<String>),
    Fatal(String),
}

/// Spawn the face worker. The thread is detached and ends on a match, on a
/// fatal error, or when the lock UI drops the receiver.
pub fn start(user: String) -> mpsc::Receiver<EngineEvent> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || run(&user, &tx));
    rx
}

fn run(user: &str, tx: &mpsc::Sender<EngineEvent>) {
    // No sensor is not nothing to do. Presence is one of the two things that
    // arm an attempt and resume is the other, and resume comes from logind, so
    // a machine without the sensor still face-unlocks when the lid opens. It
    // just never does so from someone walking up to it. Say that once and
    // carry on rather than ending the worker, which is what used to happen and
    // what would now silently take the lid with it.
    if Presence::detect().is_none() {
        let _ = tx.send(EngineEvent::Unavailable(
            "no presence sensor — face unlock waits for a resume".to_string(),
        ));
    }

    // Register as the lock agent and hold the connection for the whole lock
    // episode. faced treats the connection itself as the registration, so
    // this must outlive every attempt; dropping it deregisters.
    //
    // This is what lets the daemon refuse unlock verification when no lock
    // screen is actually up, rather than trusting that whoever asked had a
    // good reason. Held in a binding rather than discarded: `let _ = ...`
    // would drop it immediately and deregister on the spot.
    let _lock_agent = match crate::face::register_lock() {
        Ok(stream) => Some(stream),
        Err(e) => {
            // Not fatal. The daemon only enforces the requirement when
            // configured to, and face unlock working without the extra guard
            // is better than not working at all.
            log::warn!("face: could not register as lock agent: {e}");
            None
        }
    };

    // The idle manager owns the device and publishes it; this listens. Doing
    // its own reads here put a third 4 Hz reader on hardware that serves
    // about three reads a second, and every reader queued behind the others.
    let events = presence::subscribe();
    let resumes = watch_resume();
    // None until the first reading lands, so the opening state is a starting
    // point rather than an arrival — arming on it would run face unlock
    // against whoever locked the machine a moment ago.
    let mut present: Option<bool> = None;

    // Some(deadline) means an attempt is armed for that instant.
    let mut armed: Option<Instant> = None;
    let mut attempts = 0u32;
    // Transient Unavailable results, reset on each fresh arrival. See attempt().
    let mut transient = 0u32;

    loop {
        sleep(POLL);

        // Collapse everything queued since the last tick into one edge; only
        // a change in settled state is an arrival or a departure.
        let mut edge = None;
        while let Ok(event) = events.try_recv() {
            if let PresenceEvent::Changed(state) = event
                && present != Some(state)
            {
                let first = present.is_none();
                present = Some(state);
                if !first {
                    edge = Some(state);
                }
            }
        }

        match edge {
            Some(true) => {
                log::info!("face: presence returned — arming");
                attempts = 0;
                transient = 0;
                armed = Some(Instant::now());
            }
            Some(false) => {
                log::info!("face: presence gone — standing down");
                armed = None;
            }
            None => {}
        }

        // A resume outranks whatever presence last said. The lid was shut and
        // is now open, which is an arrival however still the sensor reads, and
        // it resets the attempt budget for the same reason an arrival does:
        // this is a fresh approach to the machine, not the tail of the last
        // one. Drained after presence so the two cannot arm twice for one
        // return.
        let mut woke = false;
        while resumes.try_recv().is_ok() {
            woke = true;
        }
        if woke {
            log::info!("face: resumed from sleep — arming");
            attempts = 0;
            transient = 0;
            armed = Some(Instant::now());
        }

        let Some(due) = armed else { continue };
        if Instant::now() < due {
            continue;
        }

        match attempt(tx, &mut transient) {
            Attempt::Match => {
                log::info!("face: match — unlocking");
                let _ = tx.send(EngineEvent::Match(user.to_string()));
                return;
            }
            Attempt::Retryable(hint) => {
                attempts += 1;
                if let Some(hint) = hint {
                    log::info!("face: {hint}");
                    if tx.send(EngineEvent::Hint(hint)).is_err() {
                        return;
                    }
                }
                if attempts >= MAX_ATTEMPTS {
                    log::info!(
                        "face: {attempts} attempts without a match — waiting for next arrival"
                    );
                    armed = None;
                } else {
                    armed = Some(Instant::now() + RETRY_AFTER);
                }
            }
            Attempt::Fatal(why) => {
                log::info!("face: disabled — {why}");
                let _ = tx.send(EngineEvent::Unavailable(why));
                return;
            }
        }
    }
}

/// The exit-code contract, as documented in docs/face-unlock-architecture.md
/// §4.2 in the nixos repo. The verifier is the authority; this is the reader.
///
/// The default arm is `Retryable`, deliberately. It used to be `Fatal`, which
/// meant any code this match did not know about ended the worker thread and
/// disabled face unlock for the whole lock episode. Two things made that bad:
/// a single transient stall looked identical to a broken install, and adding
/// any new code on the verifier side would silently disable the feature on an
/// older locker. Unknown means unknown, and the safe reading of unknown is
/// "try again"; the attempt counter already bounds how long that goes on.
/// Run one verification against faced, forwarding progress to the UI.
///
/// Holds a socket for the length of the burst rather than forking a process.
/// The old path forked `howdy-verify`, which paid 2.1 s of interpreter and
/// dlib startup before looking at a frame and could report nothing until it
/// exited. The exit-code contract is unchanged; it now arrives over the wire
/// instead of through a process status.
fn attempt(tx: &mpsc::Sender<EngineEvent>, transient: &mut u32) -> Attempt {
    let verdict = crate::face::verify(|state| {
        let _ = tx.send(EngineEvent::Progress(state));
    });

    let verdict = match verdict {
        Ok(v) => v,
        // The daemon is unreachable. Retryable once: faced is socket
        // activated and may legitimately be restarting underneath us.
        Err(e) => {
            *transient += 1;
            return if *transient <= 1 {
                Attempt::Retryable(None)
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
        // no_match | no_face | deadline. Which of the three decides the
        // wording, and the daemon is the only thing that knows: "didn't
        // recognise you" and "didn't see you" send the user to completely
        // different actions, so this must not guess.
        11 => Attempt::Retryable(Some(if saw_face || outcome == "no_match" {
            "Didn't recognise you".to_string()
        } else {
            "Didn't see you".to_string()
        })),
        // Never the user's fault, and never the room's: the camera carries
        // its own illuminator, so nothing lit means the emitter or the relay
        // is wrong. Asking someone to reposition their face, or to turn a
        // light on, would be misleading.
        13 => Attempt::Retryable(Some("No infrared light".to_string())),
        14 => Attempt::Retryable(Some("Face unlock busy".to_string())),
        1 => {
            *transient += 1;
            if *transient <= 1 {
                Attempt::Retryable(None)
            } else {
                Attempt::Fatal("face unlock unavailable".to_string())
            }
        }
        10 => Attempt::Fatal("no enrolled face model".to_string()),
        12 => Attempt::Fatal("compare rejected the username".to_string()),
        15 => Attempt::Fatal("too many failed face attempts".to_string()),
        16 => Attempt::Fatal("face unlock refused".to_string()),
        17 => Attempt::Fatal("face unlock declined".to_string()),
        code => Attempt::Retryable(Some(format!("face unlock returned {code}"))),
    }
}
