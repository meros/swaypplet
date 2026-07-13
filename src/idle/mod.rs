//! Idle manager — `swaypplet idle` replaces swayidle.
//!
//! Runs as its own process (a systemd user service in sway-session.target),
//! deliberately NOT inside the panel daemon: the lock lifecycle must not die
//! with a panel crash. Three sources feed one event loop on this thread:
//!
//!   - wayland.rs   ext-idle-notify-v1 timeouts (dim / lock / reblank / suspend)
//!   - logind.rs    PrepareForSleep + sleep delay-inhibitor, session Lock/Unlock
//!   - locker.rs    supervised `swaypplet lock` child (relaunch-while-locked)
//!
//! Behavior ported 1:1 from the old swayidle config (users/modules/swayidle.nix
//! in the nixos repo — see its comments for the incident history behind each
//! rule):
//!
//!   240 s  dim to 10% brightness, restore on resume
//!   300 s  lock the session (supervised locker, then blank outputs)
//!    30 s  re-blank after an input bump *while locked*; resume re-powers
//!  1200 s  suspend, only on battery
//!
//! before-sleep: lock + blank while holding the logind sleep delay-inhibitor,
//! release only once the locker is up (bounded by SLEEP_RELEASE_MAX — logind
//! force-continues at its own inhibitor timeout anyway). after-resume/unlock:
//! re-power outputs + restore brightness.
//!
//! Security invariant (from the old lockScript): never blank outputs unless
//! the locker is confirmed running — blanking without a lock would power off
//! an UNLOCKED desktop. LockerUp is only emitted after the child survived its
//! settle window.
//!
//! The locker child inherits this process's env (SWAYPPLET_LOCK_WALLPAPER,
//! SWAYPPLET_LOCK_WAKE_CMD set by the service unit) plus
//! SWAYPPLET_LOCK_REASON=idle|manual|sleep for future locker-side use.
//!
//! Caffeine (panel tile) still works by stopping the whole service:
//! `systemctl --user stop swaypplet-idle.service`.

mod locker;
mod logind;
mod wayland;

use std::process::Command;
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub use wayland::Timeout;

/// Everything the main loop reacts to, from any of the three sources.
pub enum Ev {
    Idled(Timeout),
    Resumed(Timeout),
    /// PrepareForSleep(start). `true` = about to sleep, `false` = resumed.
    Sleep(bool),
    /// logind session Lock signal (e.g. `loginctl lock-session`).
    LockSignal,
    /// logind session Unlock signal.
    UnlockSignal,
    /// The supervised locker child is up (survived its settle window).
    LockerUp,
    /// The supervisor gave up: clean unlock (rc=0), lock unavailable (rc=2),
    /// or spawn failure (rc=-1). Crash-while-locked relaunches internally and
    /// never reaches here.
    LockerGone { rc: i32 },
    Fatal(String),
}

/// How long to wait for the locker before releasing the sleep inhibitor
/// anyway. Suspend must not hang on a locker that fails to start.
const SLEEP_RELEASE_MAX: Duration = Duration::from_secs(3);

pub fn run() -> ! {
    let (tx, rx) = mpsc::channel::<Ev>();
    wayland::start(tx.clone());
    let logind = logind::start(tx.clone());

    let mut locker_active = false;
    // Deadline for releasing the sleep inhibitor while a locker launch is in
    // flight. Some(_) means PrepareForSleep(true) arrived and we still hold
    // the inhibitor.
    let mut sleep_release: Option<Instant> = None;

    let release_inhibitor = |sleep_release: &mut Option<Instant>| {
        if sleep_release.take().is_some() {
            logind.release_sleep_inhibitor();
        }
    };

    log::info!("idle: manager started");
    loop {
        let ev = match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(ev) => ev,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if sleep_release.is_some_and(|d| Instant::now() >= d) {
                    log::warn!("before-sleep: locker not up after {SLEEP_RELEASE_MAX:?} — releasing inhibitor anyway");
                    release_inhibitor(&mut sleep_release);
                }
                continue;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                log::error!("idle: all event sources gone");
                std::process::exit(1);
            }
        };

        match ev {
            Ev::Idled(Timeout::Dim) => {
                run_cmd("idle.dim-240s", "brightnessctl", &["set", "10%", "-n"]);
            }
            Ev::Resumed(Timeout::Dim) => {
                run_cmd("idle.dim-240s.resume", "brightnessctl", &["set", "100%", "-n"]);
            }

            Ev::Idled(Timeout::Lock) => {
                log::info!("idle.lock-300s: fire");
                ensure_locked(&tx, &mut locker_active, "idle");
            }
            Ev::Resumed(Timeout::Lock) => {}

            Ev::Idled(Timeout::Reblank) => {
                // Only blank when locked: the gate is the security invariant.
                if locker_active {
                    run_cmd("idle.reblank-30s", "swaymsg", &["output", "*", "power", "off"]);
                }
            }
            // Unconditional (matches old config): if the locker died, a gated
            // resume would leave the panel stuck off on next input.
            Ev::Resumed(Timeout::Reblank) => {
                run_cmd("idle.reblank-30s.resume", "swaymsg", &["output", "*", "power", "on"]);
            }

            Ev::Idled(Timeout::Suspend) => {
                if on_ac() {
                    log::info!("idle.suspend-1200s: on AC — skip");
                } else {
                    run_cmd("idle.suspend-1200s", "systemctl", &["suspend"]);
                }
            }
            Ev::Resumed(Timeout::Suspend) => {}

            Ev::Sleep(true) => {
                log::info!("before-sleep: fire");
                if locker_active {
                    // Already locked — nothing to wait for.
                    sleep_release = Some(Instant::now());
                    release_inhibitor(&mut sleep_release);
                } else {
                    ensure_locked(&tx, &mut locker_active, "sleep");
                    sleep_release = Some(Instant::now() + SLEEP_RELEASE_MAX);
                }
            }
            Ev::Sleep(false) => {
                // logind.rs re-takes the inhibitor itself on resume.
                run_cmd("after-resume", "swaymsg", &["output", "*", "power", "on"]);
                run_cmd("after-resume", "brightnessctl", &["set", "100%", "-n"]);
            }

            Ev::LockSignal => {
                log::info!("lock: signal");
                ensure_locked(&tx, &mut locker_active, "manual");
            }
            Ev::UnlockSignal => {
                log::info!("unlock: signal");
                run_cmd("unlock", "swaymsg", &["output", "*", "power", "on"]);
                run_cmd("unlock", "brightnessctl", &["set", "100%", "-n"]);
            }

            Ev::LockerUp => {
                log::info!("lock: locker up — blanking outputs");
                run_cmd("lock.blank", "swaymsg", &["output", "*", "power", "off"]);
                release_inhibitor(&mut sleep_release);
            }
            Ev::LockerGone { rc } => {
                locker_active = false;
                // Locked-or-bailed either way: don't hold suspend for this.
                release_inhibitor(&mut sleep_release);
                match rc {
                    0 => {
                        log::info!("lock: clean unlock");
                        run_cmd("unlock", "swaymsg", &["output", "*", "power", "on"]);
                        run_cmd("unlock", "brightnessctl", &["set", "100%", "-n"]);
                    }
                    2 => log::warn!("lock: lock unavailable (rc=2) — another locker or no protocol"),
                    _ => log::error!("lock: locker never started (rc={rc}) — NOT blanking"),
                }
            }

            Ev::Fatal(msg) => {
                log::error!("idle: fatal: {msg}");
                std::process::exit(1);
            }
        }
    }
}

fn ensure_locked(tx: &mpsc::Sender<Ev>, locker_active: &mut bool, reason: &'static str) {
    if *locker_active {
        log::info!("lock: locker already running — skip ({reason})");
        return;
    }
    *locker_active = true;
    locker::start(tx.clone(), reason);
}

/// Run a short external command, journal-logging fire + exit in the same
/// `<scope>: <event>` format the old swayidle lifecycle scripts used.
/// swaymsg gets `-q` so its JSON reply doesn't pollute the journal.
fn run_cmd(scope: &str, cmd: &str, args: &[&str]) {
    log::info!("{scope}: fire — {cmd} {}", args.join(" "));
    let mut command = Command::new(cmd);
    if cmd == "swaymsg" {
        command.arg("-q");
    }
    match command.args(args).status() {
        Ok(st) if st.success() => log::info!("{scope}: ok"),
        Ok(st) => log::warn!("{scope}: {cmd} exited {st}"),
        Err(e) => log::warn!("{scope}: {cmd} failed to spawn: {e}"),
    }
}

/// True when any power supply reports online=1 (AC adapter present).
/// Mirrors the old `grep -q 1 /sys/class/power_supply/*/online`.
fn on_ac() -> bool {
    let Ok(entries) = std::fs::read_dir("/sys/class/power_supply") else {
        return false;
    };
    entries.flatten().any(|e| {
        std::fs::read_to_string(e.path().join("online"))
            .is_ok_and(|s| s.trim() == "1")
    })
}
