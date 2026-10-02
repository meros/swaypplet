//! Pause the animated wallpaper on battery (`look.pause_wallpaper_on_battery`).
//!
//! The wallpaper video is mpvpaper's, a systemd user service this process
//! does not own. The NixOS side starts it with mpv's JSON IPC on
//! `$XDG_RUNTIME_DIR/mpvpaper.sock`, and this service keeps one property on
//! it: `pause` is true exactly while the switch is on and the machine runs
//! on battery. mpvpaper's own `-p` auto-pause leaves a paused player alone,
//! so a pause set here holds.
//!
//! Three things re-assert the state: a battery change (`services::battery`,
//! UPower), a settings change, and a 30-second tick while a pause is wanted.
//! The tick is for a restarted mpvpaper, which comes up playing on a new
//! socket; nothing announces it, and polling only while paused keeps a
//! plugged-in machine free of the wake-up.
//!
//! Resuming is only ever undoing this service's own pause. A player paused
//! by anything else stays paused, and a fresh player (a new socket inode)
//! was never paused here, so going back to AC leaves it alone. The one
//! case this cannot see is a panel restart mid-pause: the new process did
//! not pause that player and so will not resume it, until mpvpaper restarts
//! or the next unplug and replug.
//!
//! The socket writes run on a worker thread: the GTK thread only sends it
//! the wanted state. A missing socket is the normal state of a session
//! without the animated wallpaper and is logged at debug only.

use std::cell::RefCell;
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use crate::services::battery;
use crate::services::power::ChargeState;

/// How often the pause is re-asserted while it is wanted, for a player
/// that restarted on a new socket.
const TICK_S: u32 = 30;

/// Whether the player should be paused: the switch is on and the battery
/// is discharging. Plugged in at a charge threshold (`Idle`), full, or a
/// state the firmware does not name is not "on battery".
fn wanted(setting: bool, state: Option<ChargeState>) -> bool {
    setting && state == Some(ChargeState::Discharging)
}

/// What to send the player, and whether this service holds a pause once it
/// lands.
///
/// `held` is "the last pause this service sent stuck", and `fresh` is a
/// player this service has not spoken to (a new socket, or the first one
/// after a failure). A fresh player is playing as far as this service
/// knows, so it gets the pause again when one is wanted and is never sent a
/// resume, which would be undoing a pause it did not make.
fn decide(want: bool, held: bool, fresh: bool) -> (Option<bool>, bool) {
    let held = held && !fresh;
    match (want, held) {
        (true, false) => (Some(true), true),
        (false, true) => (Some(false), false),
        _ => (None, held),
    }
}

/// The socket's identity, so a restarted player on the same path reads as
/// a new one.
fn socket_id(path: &Path) -> Option<(u64, u64)> {
    std::fs::metadata(path).ok().map(|m| (m.dev(), m.ino()))
}

fn send(path: &Path, pause: bool) -> std::io::Result<()> {
    let mut stream = UnixStream::connect(path)?;
    stream.set_write_timeout(Some(Duration::from_secs(1)))?;
    // mpv answers on the same socket; nothing here needs the answer, and
    // dropping the stream is a clean disconnect on its side.
    writeln!(stream, r#"{{"command":["set_property","pause",{pause}]}}"#)
}

/// The worker: one wanted state in, at most one command out.
fn worker(path: PathBuf, rx: mpsc::Receiver<bool>) {
    let mut player: Option<(u64, u64)> = None;
    let mut held = false;
    for want in rx {
        if !want && !held {
            continue;
        }
        let Some(id) = socket_id(&path) else {
            log::debug!("wallpaper-pause: no player at {}", path.display());
            player = None;
            continue;
        };
        let fresh = player != Some(id);
        player = Some(id);
        let (command, after) = decide(want, held, fresh);
        let Some(pause) = command else {
            held = after;
            continue;
        };
        match send(&path, pause) {
            Ok(()) => {
                let what = if pause {
                    "paused (on battery)"
                } else {
                    "resumed"
                };
                log::info!("wallpaper-pause: {what}");
                held = after;
            }
            Err(e) => {
                log::debug!("wallpaper-pause: {}: {e}", path.display());
                // Speak to whatever is there next time as a new player.
                player = None;
            }
        }
    }
}

/// Start following, from the panel process. Does nothing on a machine
/// without a battery, where the switch can never be wanted.
pub fn follow() {
    if !battery::start() {
        return;
    }
    let path = glib::user_runtime_dir().join("mpvpaper.sock");
    let (tx, rx) = mpsc::channel::<bool>();
    let spawned = std::thread::Builder::new()
        .name("wallpaper-pause".into())
        .spawn(move || worker(path, rx));
    if let Err(e) = spawned {
        log::warn!("wallpaper-pause: failed to spawn thread: {e}");
        return;
    }

    let want_now = || {
        let setting = crate::settings::store::with(|s| s.look().pause_wallpaper_on_battery);
        wanted(setting, battery::current().map(|b| b.state))
    };
    let tick: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();
    let apply = Rc::new(move || {
        let want = want_now();
        let _ = tx.send(want);
        let mut t = tick.borrow_mut();
        match (want, t.is_some()) {
            (true, false) => {
                let tx = tx.clone();
                *t = Some(glib::timeout_add_seconds_local(TICK_S, move || {
                    let _ = tx.send(want_now());
                    glib::ControlFlow::Continue
                }));
            }
            (false, true) => {
                if let Some(id) = t.take() {
                    crate::spawn::remove_source(id);
                }
            }
            _ => {}
        }
    });
    apply();
    {
        let apply = apply.clone();
        battery::observe(move || apply());
    }
    crate::settings::store::observe(move || apply());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_discharging_battery_with_the_switch_on_wants_a_pause() {
        assert!(wanted(true, Some(ChargeState::Discharging)));
        assert!(!wanted(false, Some(ChargeState::Discharging)));
        for plugged in [
            ChargeState::Charging,
            ChargeState::Full,
            ChargeState::Idle,
            ChargeState::Unknown,
        ] {
            assert!(!wanted(true, Some(plugged)), "{plugged:?}");
        }
        assert!(!wanted(true, None));
    }

    #[test]
    fn it_pauses_once_and_resumes_only_its_own_pause() {
        // Unplugged: pause, and hold it.
        assert_eq!(decide(true, false, false), (Some(true), true));
        // The tick while held, same player: nothing to say.
        assert_eq!(decide(true, true, false), (None, true));
        // Plugged in: undo our pause.
        assert_eq!(decide(false, true, false), (Some(false), false));
        // Nothing held and nothing wanted: never send a resume.
        assert_eq!(decide(false, false, false), (None, false));
        assert_eq!(decide(false, false, true), (None, false));
    }

    #[test]
    fn a_restarted_player_is_paused_again_and_never_resumed() {
        // mpvpaper restarted while on battery: it is playing, pause it.
        assert_eq!(decide(true, true, true), (Some(true), true));
        // Restarted after we paused the old one, now on AC: the new one
        // was never paused here, so leave it and drop the hold.
        assert_eq!(decide(false, true, true), (None, false));
    }
}
