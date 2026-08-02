//! Supervised locker child. Port of the old lockerSupervised shell script:
//! with ext-session-lock, a lock client that dies WITHOUT unlocking leaves
//! the compositor holding the lock with no surface (solid red, wedged), so a
//! crashed locker is always relaunched — that never reveals an unlocked
//! desktop, unlike NOT relaunching.
//!
//! `swaypplet lock` exit codes: 0 only on a real unlock; 2 when the lock
//! could not be acquired at all (relaunching would spin — bail); anything
//! else is a crash while locked (relaunch after re-powering outputs so the
//! new lock surface can commit; the 30 s reblank tier re-blanks if idle).
//!
//! Readiness: the child prints "LOCKED" on stdout once the compositor
//! confirms the lock; only that emits LockerUp. Time-based settle windows
//! are not a lock — suspend can freeze the cgroup mid-startup.
//!
//! No flock: idempotency is the main loop's `locker_active` flag — all lock
//! triggers (idle tier, logind Lock signal, before-sleep) land on the same
//! thread.

use std::io::BufRead;
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;
use std::time::Duration;

use super::Ev;

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

    loop {
        let mut child = match Command::new(&exe)
            .arg("lock")
            .env("SWAYPPLET_LOCK_REASON", reason)
            .stdout(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                log::error!("lock: failed to spawn locker: {e}");
                let _ = ev.send(Ev::LockerGone { rc: -1 });
                return;
            }
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
