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

use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread::sleep;
use std::time::{Duration, Instant};

use crate::fp::EngineEvent;
use crate::presence::Presence;

/// Fallback when the unit does not set `SWAYPPLET_FACE_COMPARE`. The locker
/// inherits a curated PATH from swaypplet-idle.service, so in practice the
/// absolute path from the environment is what runs.
const COMPARE: &str = "howdy-verify";

fn compare_command() -> String {
    std::env::var("SWAYPPLET_FACE_COMPARE").unwrap_or_else(|_| COMPARE.to_string())
}

/// Presence poll cadence. The sensor samples at 10 Hz; this only decides how
/// soon after arrival the first attempt starts.
const POLL: Duration = Duration::from_millis(250);

/// Debounce into "gone". Shorter than the idle manager's, because here it only
/// stops attempts rather than locking anything.
const GONE_AFTER: Duration = Duration::from_secs(3);

/// Debounce into "back". An attempt takes seconds anyway.
const BACK_AFTER: Duration = Duration::from_secs(1);

/// Gap between attempts while presence holds.
const RETRY_AFTER: Duration = Duration::from_secs(6);

/// Attempts per arrival before falling silent until the next one. Three failed
/// tries means the light is wrong or it is not you; the password field is
/// right there.
const MAX_ATTEMPTS: u32 = 3;

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
    let Some(mut presence) = Presence::detect() else {
        let _ = tx.send(EngineEvent::Unavailable(
            "no presence sensor — face unlock idle".to_string(),
        ));
        return;
    };

    // Some(deadline) means an attempt is armed for that instant.
    let mut armed: Option<Instant> = None;
    let mut attempts = 0u32;
    let compare = compare_command();

    loop {
        sleep(POLL);

        match presence.poll(GONE_AFTER, BACK_AFTER) {
            Some(true) => {
                log::info!("face: presence returned — arming");
                attempts = 0;
                armed = Some(Instant::now());
            }
            Some(false) => {
                log::info!("face: presence gone — standing down");
                armed = None;
            }
            None => {}
        }

        let Some(due) = armed else { continue };
        if Instant::now() < due {
            continue;
        }

        match attempt(&compare, user) {
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
                    log::info!("face: {attempts} attempts without a match — waiting for next arrival");
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

fn attempt(compare: &str, user: &str) -> Attempt {
    let status = Command::new(compare)
        .arg(user)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    match status {
        Ok(status) => match status.code() {
            Some(0) => Attempt::Match,
            Some(11) => Attempt::Retryable(None),
            Some(13) => Attempt::Retryable(Some("too dark for face unlock".to_string())),
            Some(10) => Attempt::Fatal("no enrolled face model".to_string()),
            Some(12) => Attempt::Fatal("compare rejected the username".to_string()),
            Some(code) => Attempt::Fatal(format!("{compare} exited {code}")),
            None => Attempt::Fatal(format!("{compare} killed by a signal")),
        },
        Err(e) => Attempt::Fatal(format!("{compare}: {e}")),
    }
}
