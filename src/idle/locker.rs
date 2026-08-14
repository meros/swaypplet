//! Supervised locker child. Port of the old lockerSupervised shell script:
//! with ext-session-lock, a lock client that dies WITHOUT unlocking leaves
//! the compositor holding the lock with no surface (solid red, wedged), so a
//! crashed locker is always relaunched — that never reveals an unlocked
//! desktop, unlike NOT relaunching.
//!
//! `swaypplet lock` exit codes: 0 only on a real unlock; 2 when the lock
//! could not be acquired at all (relaunching would spin — bail); anything
//! else is a crash while locked (relaunch after re-powering outputs so the
//! new lock surface can commit; the lock-idle deadline blanks it later).
//!
//! Readiness: the child prints "LOCKED" on stdout once the compositor
//! confirms the lock; only that emits LockerUp. Time-based settle windows
//! are not a lock — suspend can freeze the cgroup mid-startup.
//!
//! No flock: idempotency is the main loop's `locker_active` flag — all lock
//! triggers (idle tier, logind Lock signal, before-sleep) land on the same
//! thread.

use std::io::{BufRead, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::Sender;
use std::time::Duration;

use super::Ev;

/// A locker spawned early, warmed, and parked on stdin waiting to be told to
/// lock.
///
/// The first GTK window a process presents costs ~880 ms, and for a locker
/// spawned at lock time that window is the lock screen (measured;
/// swaypplet docs/LOCK_TRANSITION_WIP.md). Paying it while nothing is waiting
/// is the difference between a lock screen that appears in a second and one
/// that appears in a frame.
static ARMED: std::sync::Mutex<Option<Child>> = std::sync::Mutex::new(None);

/// Spawn the next locker now and let it warm up. Idempotent: an armed child
/// that is still alive is left alone.
pub fn prewarm() {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            log::warn!("lock: prewarm skipped, current_exe failed: {e}");
            return;
        }
    };
    let mut slot = match ARMED.lock() {
        Ok(g) => g,
        Err(e) => e.into_inner(),
    };
    if let Some(child) = slot.as_mut()
        && matches!(child.try_wait(), Ok(None))
    {
        return;
    }
    match Command::new(&exe)
        .arg("lock")
        .env("SWAYPPLET_LOCK_WAIT", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
    {
        Ok(child) => {
            log::info!("lock: locker pre-warming (pid {})", child.id());
            *slot = Some(child);
        }
        // Non-fatal in every case: a lock with no armed child spawns one cold,
        // which is exactly the old behaviour.
        Err(e) => log::warn!("lock: prewarm spawn failed: {e}"),
    }
}

/// Take the armed child if there is one and it is still alive.
fn take_armed() -> Option<Child> {
    let mut slot = match ARMED.lock() {
        Ok(g) => g,
        Err(e) => e.into_inner(),
    };
    let mut child = slot.take()?;
    match child.try_wait() {
        // Parked and healthy.
        Ok(None) => Some(child),
        // Died while parked. Nothing to salvage; the caller spawns cold.
        Ok(Some(status)) => {
            log::warn!("lock: pre-warmed locker died while parked ({status})");
            None
        }
        Err(e) => {
            log::warn!("lock: could not check pre-warmed locker: {e}");
            None
        }
    }
}

pub fn start(ev: Sender<Ev>, reason: &'static str) {
    std::thread::Builder::new()
        .name("idle-locker".into())
        .spawn(move || supervise(ev, reason))
        .expect("spawn idle-locker thread");
}

fn supervise(ev: Sender<Ev>, reason: &'static str) {
    prewarm_fprintd();

    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            log::error!("lock: current_exe failed: {e}");
            let _ = ev.send(Ev::LockerGone { rc: -1 });
            return;
        }
    };

    // The armed child is used once. A relaunch after a crash spawns cold: it
    // is the rare path, and warming there would mean holding a spare process
    // for a case that should not happen.
    let mut armed = take_armed();
    // Suppressed on the sleep path (the deferral would be charged straight to
    // the logind inhibitor window) and on every relaunch (the compositor is
    // already holding an abandoned lock, whose backdrop is opaque and pinned,
    // so a ramp would be ignored anyway).
    let mut relaunch = false;

    loop {
        let fade = !relaunch && reason != "sleep";
        let mut child = match armed.take() {
            Some(mut child) => {
                // The reason cannot ride on the environment of a process that
                // was spawned before anybody knew it.
                let suffix = if fade { "" } else { " nofade" };
                let sent = child
                    .stdin
                    .as_mut()
                    .ok_or_else(|| std::io::Error::other("no stdin pipe"))
                    .and_then(|stdin| {
                        writeln!(stdin, "LOCK {reason}{suffix}").and_then(|()| stdin.flush())
                    });
                match sent {
                    Ok(()) => child,
                    Err(e) => {
                        log::warn!("lock: could not command pre-warmed locker: {e}");
                        let _ = child.kill();
                        let _ = child.wait();
                        continue;
                    }
                }
            }
            None => match Command::new(&exe)
                .arg("lock")
                .env("SWAYPPLET_LOCK_REASON", reason)
                .env("SWAYPPLET_LOCK_FADE", if fade { "1" } else { "0" })
                .stdout(Stdio::piped())
                .spawn()
            {
                Ok(c) => c,
                Err(e) => {
                    log::error!("lock: failed to spawn locker: {e}");
                    let _ = ev.send(Ev::LockerGone { rc: -1 });
                    return;
                }
            },
        };

        // Readiness handshake: the child prints "LOCKED" on stdout once the
        // compositor confirms the lock (ext-session-lock `locked` event).
        // Only that emits LockerUp — a merely-alive child proves nothing (a
        // suspend freeze can land before its lock request is even sent).
        // Reading to EOF keeps the pipe from ever blocking the child.
        if let Some(out) = child.stdout.take() {
            let ev = ev.clone();
            std::thread::Builder::new()
                .name("idle-locker-ready".into())
                .spawn(move || {
                    for line in std::io::BufReader::new(out).lines() {
                        match line {
                            Ok(l) if l.trim() == "LOCKED" => {
                                let _ = ev.send(Ev::LockerUp);
                            }
                            Ok(_) => {}
                            Err(_) => break,
                        }
                    }
                })
                .expect("spawn idle-locker-ready thread");
        }

        let rc = match child.wait() {
            Ok(st) => st.code().unwrap_or(1), // killed by signal = crash
            Err(e) => {
                log::error!("lock: wait failed: {e}");
                1
            }
        };
        match rc {
            0 | 2 => {
                let _ = ev.send(Ev::LockerGone { rc });
                return;
            }
            _ => {
                relaunch = true;
                log::warn!("lock: locker died rc={rc} while locked — re-powering, relaunching");
                let _ = Command::new("swaymsg")
                    .args(["-q", "output", "*", "power", "on"])
                    .status();
                std::thread::sleep(Duration::from_millis(200));
            }
        }
    }
}

/// Pre-warm fprintd so the sensor is bound before the locker asks for it.
/// fprintd is D-Bus activated and exits ~30 s after last use; without this
/// the first fingerprint tap after an idle lock eats a cold-start miss.
fn prewarm_fprintd() {
    let ok = zbus::blocking::Connection::system()
        .and_then(|conn| {
            conn.call_method(
                Some("net.reactivated.Fprint"),
                "/net/reactivated/Fprint/Manager",
                Some("net.reactivated.Fprint.Manager"),
                "GetDefaultDevice",
                &(),
            )
        })
        .is_ok();
    if ok {
        log::info!("lock: fprintd pre-warm OK");
    } else {
        log::warn!("lock: fprintd pre-warm failed (non-fatal)");
    }
}
