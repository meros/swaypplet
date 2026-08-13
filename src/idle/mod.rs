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
//! release only once the compositor has CONFIRMED the lock (bounded by
//! SLEEP_RELEASE_MAX — logind force-continues at its own inhibitor timeout
//! anyway). after-resume/unlock: re-power outputs + restore brightness.
//!
//! Security invariant (from the old lockScript): never blank outputs unless
//! the session is confirmed locked — blanking without a lock would power off
//! an UNLOCKED desktop. LockerUp is only emitted after the compositor
//! acknowledged the lock (ext-session-lock `locked`, relayed by the child
//! over its stdout — see locker.rs). A merely-running child counts for
//! nothing: the 2026-08-02 incident had suspend freeze the cgroup mid-GTK-
//! startup, so the machine slept unlocked and the desktop was visible for
//! ~5 s after lid-open until the lock request finally landed.
//!
//! Fast user switching adds a second rule: only an idle-triggered lock arms
//! the post-lock blank, deferred (BLANK_DELAY) and skipped when the session
//! has gone inactive by then — a switch-away lock races the VT change, and
//! blanking mid-handover leaves the returning user staring at powered-off
//! outputs. Manual and switch locks keep the lock UI lit; the Reblank tier
//! powers the outputs off after 30 s of no input. Correspondingly,
//! SessionActive(true) on a locked session re-powers the outputs, and
//! SessionActive(false) locks the session behind any departure (including a
//! bare Ctrl+Alt+Fn that skipped the switcher script). The sleep path keeps
//! its immediate blank.
//!
//! The locker child inherits this process's env (SWAYPPLET_LOCK_WALLPAPER,
//! SWAYPPLET_LOCK_WAKE_CMD set by the service unit) plus
//! SWAYPPLET_LOCK_REASON=idle|manual|sleep for future locker-side use.
//!
//! Caffeine (panel tile) still works by stopping the whole service:
//! `systemctl --user stop swaypplet-idle.service`.

mod locker;
mod logind;
mod outputs;
mod wayland;

use outputs::{Outputs, Power};

use std::process::Command;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::presence::{self, Event as PresenceEvent};

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
    /// The compositor confirmed the session lock (the locker child relayed
    /// the ext-session-lock `locked` event). May repeat after a
    /// crash-while-locked relaunch; handlers are idempotent.
    LockerUp,
    /// logind's LockedHint was set at our startup: a lock outlived a service
    /// restart and must be relaunched (the old locker died with the cgroup).
    RecoverLock,
    /// logind session Active property changed (VT switch / fast user
    /// switch). Also sent once at startup with the initial value.
    SessionActive(bool),
    /// The supervisor gave up: clean unlock (rc=0), lock unavailable (rc=2),
    /// or spawn failure (rc=-1). Crash-while-locked relaunches internally and
    /// never reaches here.
    LockerGone {
        rc: i32,
    },
    Fatal(String),
}

/// How long to wait for the locker before releasing the sleep inhibitor
/// anyway. Suspend must not hang on a locker that fails to start.
const SLEEP_RELEASE_MAX: Duration = Duration::from_secs(3);

/// Post-lock blank delay for idle-triggered locks: long enough for a
/// user-switch VT change to land (the blank is then skipped). Manual and
/// switch locks never arm this — the lock UI stays visible until the
/// Reblank idle tier powers the outputs off.
const BLANK_DELAY: Duration = Duration::from_millis(600);

pub fn run() -> ! {
    let (tx, rx) = mpsc::channel::<Ev>();
    // Display changes go through here, never inline. `swaymsg output * power
    // on` takes 782-820 ms to return, and this loop must stay responsive
    // across exactly that window: the presence edge that lights the screen is
    // the same edge that starts a face attempt.
    let outputs = Outputs::start();
    wayland::start(tx.clone());
    let logind = logind::start(tx.clone());

    let mut locker_active = false;
    // True only between the compositor's lock confirmation (LockerUp) and
    // LockerGone. locker_active alone means "a launch is in flight".
    let mut locker_confirmed = false;
    // Deadline for releasing the sleep inhibitor while a locker launch is in
    // flight. Some(_) means PrepareForSleep(true) arrived and we still hold
    // the inhibitor.
    let mut sleep_release: Option<Instant> = None;
    // Deadline for the deferred post-lock blank (see BLANK_DELAY).
    let mut pending_blank: Option<Instant> = None;
    // Why the current locker was started ("idle" | "manual" | "sleep" |
    // "switch"); decides whether LockerUp blanks the outputs.
    let mut lock_reason: &'static str = "manual";
    // Mirrors the logind session Active property.
    let mut session_active = true;

    // This process owns the sensor for the whole session and publishes it on
    // the session bus; the bar and the lock screen's face engine listen there
    // rather than reading the device themselves. Reads cost 250-400 ms each
    // and the hardware serves about three a second, so three independent
    // readers meant all three queued. See `crate::presence`.
    //
    // None on hardware without the sensor, which keeps the timer-only
    // behaviour intact.
    let presence = presence::serve();
    // Mirrors the sensor, for the tiers that ask whether anyone is here.
    // None until the first reading lands.
    let mut present: Option<bool> = None;

    let release_inhibitor = |sleep_release: &mut Option<Instant>| {
        if sleep_release.take().is_some() {
            logind.release_sleep_inhibitor();
        }
    };

    log::info!("idle: manager started");
    loop {
        // Transitions arrive from the owner thread, which samples on its own
        // cadence; this drain rides the 250 ms tick below, so that tick is
        // the only latency between the sensor changing and us acting on it.
        let mut now_present = None;
        if let Some(events) = &presence {
            while let Ok(event) = events.try_recv() {
                match event {
                    PresenceEvent::Changed(state) => {
                        // Only a change in the settled state is an edge to act
                        // on; the owner sends its opening reading too.
                        if present != Some(state) {
                            present = Some(state);
                            now_present = Some(state);
                        }
                    }
                    // Display only; the bar is what reads it.
                    PresenceEvent::Attention(_) => {}
                }
            }
        }

        if let Some(now_present) = now_present {
            if now_present {
                log::info!("presence: user back — powering outputs on");
                // The mirror of the blank invariant. Blanking is dangerous
                // without a confirmed lock, so it is guarded; powering on
                // never is, so this is unconditional. Without it, returning to
                // a blanked session leaves you facing a dark screen until you
                // touch something — and the locker is by then already
                // attempting face unlock behind it, so the machine looks dead
                // at exactly the moment it is working.
                //
                // Outputs only, no brightness restore. If the dim tier had
                // faded the screen it stays faded, which is still legible, and
                // the existing input-driven Resumed(Dim) path is what should
                // own brightness — presence is not input, and overriding a
                // deliberate brightness setting from a walk-past would be
                // worse than a dim lock screen.
                //
                // This does not reset the idle tiers: the reblank timer keeps
                // running, so presence that turns out to be someone walking
                // past re-blanks on its own rather than holding the panel lit.
                outputs.power("presence.back", Power::On);
            } else {
                log::info!("presence: user gone — locking");
                ensure_locked(
                    &tx,
                    &mut locker_active,
                    &mut locker_confirmed,
                    &mut lock_reason,
                    "presence",
                );
            }
        }

        let ev = rx.recv_timeout(Duration::from_millis(250));

        // Deadlines fire on every pass, event traffic or not.
        if sleep_release.is_some_and(|d| Instant::now() >= d) {
            log::warn!(
                "before-sleep: locker not up after {SLEEP_RELEASE_MAX:?} — releasing inhibitor anyway"
            );
            release_inhibitor(&mut sleep_release);
        }
        if pending_blank.is_some_and(|d| Instant::now() >= d) {
            pending_blank = None;
            // Gates checked at fire time: the lock must still be confirmed
            // (security invariant; a crash relaunch keeps the compositor
            // lock held, so confirmed stays true through the gap) and the
            // session still on the seat — a switch-away lock skips the
            // blank entirely.
            if locker_confirmed && session_active {
                outputs.power("lock.blank", Power::Off);
            } else {
                log::info!("lock.blank: skipped (session inactive or locker gone)");
            }
        }

        let ev = match ev {
            Ok(ev) => ev,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                log::error!("idle: all event sources gone");
                std::process::exit(1);
            }
        };

        match ev {
            Ev::Idled(Timeout::Dim) => {
                outputs.brightness("idle.dim-240s", 10);
            }
            Ev::Resumed(Timeout::Dim) => {
                outputs.brightness("idle.dim-240s.resume", 100);
            }

            Ev::Idled(Timeout::Lock) => {
                // Sitting still is not being away. While the sensor sees
                // someone the 300 s tier is suppressed and the absence path
                // owns locking instead; nothing re-arms this tier until input
                // resumes, which is the intent.
                if present == Some(true) {
                    log::info!("idle.lock-300s: skip (present)");
                } else {
                    log::info!("idle.lock-300s: fire");
                    ensure_locked(
                        &tx,
                        &mut locker_active,
                        &mut locker_confirmed,
                        &mut lock_reason,
                        "idle",
                    );
                }
            }
            Ev::Resumed(Timeout::Lock) => {}

            Ev::Idled(Timeout::Reblank) => {
                // Only blank a locked session that is still on the seat. On an
                // inactive VT (fast user switch) our idle timers keep firing;
                // blanking then would fight the SessionActive(true) re-power.
                if locker_confirmed && session_active {
                    outputs.power("idle.reblank-30s", Power::Off);
                } else {
                    log::info!(
                        "idle.reblank-30s: skip (locker_confirmed={locker_confirmed} session_active={session_active})"
                    );
                }
            }
            // Unconditional (matches old config): if the locker died, a gated
            // resume would leave the panel stuck off on next input.
            Ev::Resumed(Timeout::Reblank) => {
                outputs.power("idle.reblank-30s.resume", Power::On);
            }

            Ev::Idled(Timeout::Suspend) => {
                // Never suspend from an inactive session: our idle timers keep
                // advancing after a VT switch, so an unguarded suspend would
                // sleep the whole machine out from under the user who is
                // actively on another VT.
                if !session_active {
                    log::info!("idle.suspend-1200s: session inactive — skip");
                } else if on_ac() {
                    log::info!("idle.suspend-1200s: on AC — skip");
                } else {
                    run_cmd("idle.suspend-1200s", "systemctl", &["suspend"]);
                }
            }
            Ev::Resumed(Timeout::Suspend) => {}

            Ev::Sleep(true) => {
                log::info!("before-sleep: fire");
                if locker_confirmed {
                    // Already locked — nothing to wait for.
                    sleep_release = Some(Instant::now());
                    release_inhibitor(&mut sleep_release);
                } else {
                    // No lock, or a launch is in flight but the compositor
                    // hasn't confirmed it yet: hold the inhibitor until
                    // LockerUp. Releasing on a mere spawn is how a suspend
                    // freeze once caught the locker mid-startup and slept
                    // the machine unlocked.
                    ensure_locked(
                        &tx,
                        &mut locker_active,
                        &mut locker_confirmed,
                        &mut lock_reason,
                        "sleep",
                    );
                    sleep_release = Some(Instant::now() + SLEEP_RELEASE_MAX);
                }
            }
            Ev::Sleep(false) => {
                // logind.rs re-takes the inhibitor itself on resume.
                outputs.power_brightness("after-resume", Power::On, 100);
            }

            Ev::LockSignal => {
                log::info!("lock: signal");
                ensure_locked(
                    &tx,
                    &mut locker_active,
                    &mut locker_confirmed,
                    &mut lock_reason,
                    "manual",
                );
            }
            Ev::UnlockSignal => {
                log::info!("unlock: signal");
                outputs.power_brightness("unlock", Power::On, 100);
            }

            Ev::RecoverLock => {
                // Re-power outputs first: the dead locker may have left them
                // blanked, and the new lock surface needs a lit output to be
                // seen. Then lock with a reason that keeps the UI visible.
                outputs.power_brightness("recover", Power::On, 100);
                ensure_locked(
                    &tx,
                    &mut locker_active,
                    &mut locker_confirmed,
                    &mut lock_reason,
                    "recover",
                );
            }
            Ev::LockerUp => {
                locker_confirmed = true;
                if sleep_release.is_some() {
                    // Suspend path: blank now — the machine is about to
                    // sleep and the inhibitor must not wait on a timer.
                    log::info!("lock: locker up — blanking outputs (sleep)");
                    outputs.power("lock.blank", Power::Off);
                } else if matches!(lock_reason, "idle" | "presence") {
                    log::info!("lock: locker up — blank in {BLANK_DELAY:?}");
                    pending_blank = Some(Instant::now() + BLANK_DELAY);
                } else {
                    // Manual/switch/recover lock: leave the lock UI visible;
                    // the Reblank idle tier powers the outputs off later.
                    log::info!("lock: locker up — no auto-blank ({lock_reason})");
                }
                // Record the lock in logind so a service restart can recover
                // it (see Ev::RecoverLock).
                logind.set_locked_hint(true);
                release_inhibitor(&mut sleep_release);
            }
            Ev::SessionActive(false) => {
                session_active = false;
                // Leaving the seat (fast user switch, bare VT change):
                // lock the abandoned session behind us. Idempotent when the
                // switcher script already sent Lock.
                ensure_locked(
                    &tx,
                    &mut locker_active,
                    &mut locker_confirmed,
                    &mut lock_reason,
                    "switch",
                );
            }
            Ev::SessionActive(true) => {
                session_active = true;
                if locker_active {
                    // Returning to a locked session: light the outputs so
                    // the lock screen shows instead of a dead panel.
                    outputs.power_brightness("switch.return", Power::On, 100);
                }
            }
            Ev::LockerGone { rc } => {
                locker_active = false;
                locker_confirmed = false;
                // Locked-or-bailed either way: don't hold suspend for this.
                release_inhibitor(&mut sleep_release);
                match rc {
                    0 => {
                        log::info!("lock: clean unlock");
                        logind.set_locked_hint(false);
                        outputs.power_brightness("unlock", Power::On, 100);
                    }
                    2 => {
                        log::warn!("lock: lock unavailable (rc=2) — another locker or no protocol")
                    }
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

fn ensure_locked(
    tx: &mpsc::Sender<Ev>,
    locker_active: &mut bool,
    locker_confirmed: &mut bool,
    lock_reason: &mut &'static str,
    reason: &'static str,
) {
    if *locker_active {
        log::info!("lock: locker already running — skip ({reason})");
        return;
    }
    *locker_active = true;
    *locker_confirmed = false;
    *lock_reason = reason;
    locker::start(tx.clone(), reason);
}

/// Run a short external command, journal-logging fire + exit in the same
/// `<scope>: <event>` format the old swayidle lifecycle scripts used.
/// swaymsg gets `-q` so its JSON reply doesn't pollute the journal.
fn run_cmd(scope: &str, cmd: &str, args: &[&str]) {
    run_cmd_timed(scope, cmd, args, Duration::ZERO);
}

/// As `run_cmd`, plus how long the request sat in the display worker's queue
/// before it ran.
///
/// The two numbers are logged separately on purpose. Queue wait is this
/// process's latency and command duration is the compositor's, and the display
/// power-on budget in docs/face-unlock-architecture.md §7.4 branches on which
/// of the two dominates. One combined number cannot answer that.
pub(super) fn run_cmd_timed(scope: &str, cmd: &str, args: &[&str], waited: Duration) {
    log::info!("{scope}: fire — {cmd} {}", args.join(" "));
    let mut command = Command::new(cmd);
    if cmd == "swaymsg" {
        command.arg("-q");
    }
    let started = Instant::now();
    let status = command.args(args).status();
    let took = started.elapsed().as_millis();
    let queued = waited.as_millis();
    match status {
        Ok(st) if st.success() => log::info!("{scope}: ok in {took}ms (queued {queued}ms)"),
        Ok(st) => log::warn!("{scope}: {cmd} exited {st} after {took}ms (queued {queued}ms)"),
        Err(e) => log::warn!("{scope}: {cmd} failed to spawn: {e}"),
    }
}

/// True when any power supply reports online=1 (AC adapter present).
/// Mirrors the old `grep -q 1 /sys/class/power_supply/*/online`.
fn on_ac() -> bool {
    let Ok(entries) = std::fs::read_dir("/sys/class/power_supply") else {
        return false;
    };
    entries
        .flatten()
        .any(|e| std::fs::read_to_string(e.path().join("online")).is_ok_and(|s| s.trim() == "1"))
}
