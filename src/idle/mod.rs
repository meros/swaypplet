//! Idle manager — `swaypplet idle` replaces swayidle.
//!
//! Runs as its own process (a systemd user service in sway-session.target),
//! deliberately NOT inside the panel daemon: the lock lifecycle must not die
//! with a panel crash. Three sources feed one event loop on this thread:
//!
//!   - wayland.rs   ext-idle-notify-v1 timeouts (dim / lock / lock-idle / suspend)
//!   - logind.rs    PrepareForSleep + sleep delay-inhibitor, session Lock/Unlock
//!   - locker.rs    supervised `swaypplet lock` child (relaunch-while-locked)
//!
//! Behavior ported 1:1 from the old swayidle config (users/modules/swayidle.nix
//! in the nixos repo — see its comments for the incident history behind each
//! rule). The durations are the defaults; the settings pane's Idle & Lock tab
//! overrides them (`settings::store::Idle`):
//!
//!   240 s  dim to 10% brightness, restore on resume
//!   300 s  lock the session (the screen stays LIT; locking does not blank)
//!    15 m  power outputs off after this much idle time *while locked*;
//!          any input disarms it and re-powers
//!  1200 s  suspend, only on battery
//!
//! The pane is another process, so an edit reaches here as a file: this loop
//! stats `~/.config/swaypplet/settings.json` once a second (`SETTINGS_POLL`)
//! and, when its mtime moves, reloads and hands the wayland thread new
//! timeouts to re-arm with. Zero on any tier is "never".
//!
//! Locking and blanking are deliberately unrelated. They used to be welded
//! together in three places, so any lock put the panel out within a second
//! and the lock screen was effectively never visible — including while face
//! unlock was trying to tell the user what it was doing.
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
//! Fast user switching no longer needs a rule of its own. The blank deadline
//! is checked at fire time rather than arm time, so a lock that races a VT
//! change simply finds the session inactive fifteen minutes later and skips;
//! there is nothing to defer and nothing to cancel. That used to need a
//! deferred blank with a skip condition, purely because the blank fired
//! within a second of locking. Correspondingly, SessionActive(true) on a
//! locked session re-powers the outputs, and SessionActive(false) locks the
//! session behind any departure (including a bare Ctrl+Alt+Fn that skipped
//! the switcher script). The sleep path keeps its immediate blank, because
//! that is the machine powering down rather than an idle policy.
//!
//! The locker child inherits this process's env (SWAYPPLET_LOCK_WAKE_CMD set
//! by the service unit) plus SWAYPPLET_LOCK_REASON=idle|manual|sleep for
//! future locker-side use. It used to inherit SWAYPPLET_LOCK_WALLPAPER and a
//! set of SWAYPPLET_GLASS_* material numbers as well; the compositor draws the
//! lock's wallpaper and glass now, and the locker reads neither.
//!
//! Inhibitors (panel tiles, `crate::inhibit`) are two standing switches, and
//! both are read at fire time rather than being cached here. **No Sleep**
//! holds a logind lid-switch inhibitor, which stops logind suspending on a
//! lid close; the suspend tier below reads it too, because "don't sleep with
//! the lid shut" has to mean both paths or it means neither. **No Lock**
//! suppresses the dim tier, the lock tier and the walk-away lock — every way
//! this process locks a session nobody asked it to lock. Neither one touches
//! the paths a user asked for: the before-sleep lock, the Lock signal, the
//! VT-switch lock and an explicit `systemctl suspend` all still run, so
//! neither switch can leave the machine asleep and unlocked.
//!
//! A compositor idle inhibitor (a video player's idle-inhibit-v1, or sway's
//! `inhibit_idle` by hand) suppresses the timeout tiers for free, since
//! `get_idle_notification` honours it (wayland.rs). The absence path is the
//! one tier that has to ask (`idle_inhibited`).

mod locker;
mod logind;
mod outputs;
mod wayland;

use outputs::{Outputs, Power};

use std::process::Command;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::presence::{self, Event as PresenceEvent};
use crate::settings::store::{self, Idle, Settings};

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
///
/// 4 s rather than 3: the compositor now defers `locked` until the lock
/// screen is fully opaque and that frame has been presented, which adds the
/// entrance to the path this deadline bounds. The sleep path suppresses the
/// cross-fade entirely (locker.rs), so in practice it pays none of that, but
/// the deadline has to survive the case where it does. logind's
/// InhibitDelayMaxSec is 5 s, so this still leaves room for the release round
/// trip.
const SLEEP_RELEASE_MAX: Duration = Duration::from_secs(4);

/// How often the settings file is stat'd for a change. One `stat` a second
/// is nothing; the latency it sets is how long an edit in the pane takes to
/// reach the timers.
const SETTINGS_POLL: Duration = Duration::from_secs(1);

/// When the locked, idle session's outputs should go off, counted from now.
///
/// The blank deadline is the ONLY thing that blanks the screen. Blanking used
/// to be tangled into three separate paths: a 600 ms deferred blank after any
/// idle or presence lock, a 30 s re-blank tier after input while locked, and
/// the pre-suspend blank. Two of those meant that locking for any reason, or
/// glancing at the machine and looking away, put the panel out within a
/// second, and the lock screen was effectively never visible.
///
/// Now: lock and blank are unrelated. Locking leaves the screen lit. The
/// outputs go off only after `blank_after_s` of continuous idle time *while
/// locked*, and any input resets it. The suspend path still blanks, because
/// that is the machine powering down rather than an idle policy. `None` is
/// the setting's "never": the lock screen stays lit until something else
/// turns it off.
fn blank_deadline(cfg: &Idle) -> Option<Instant> {
    (cfg.blank_after_s > 0)
        .then(|| Instant::now() + Duration::from_secs(u64::from(cfg.blank_after_s)))
}

pub fn run() -> ! {
    let (tx, rx) = mpsc::channel::<Ev>();
    // Display changes go through here, never inline. `swaymsg output * power
    // on` takes 782-820 ms to return, and this loop must stay responsive
    // across exactly that window: the presence edge that lights the screen is
    // the same edge that starts a face attempt.
    let outputs = Outputs::start();
    // The timers, from the settings file, and the handle to re-arm them.
    let mut cfg = Settings::load().idle();
    let mut settings_file = store::Watch::new();
    let mut next_settings_check = Instant::now() + SETTINGS_POLL;
    let timeouts = wayland::start(tx.clone(), wayland::Timeouts::from(&cfg));
    let logind = logind::start(tx.clone());

    // Warm the next locker now. The first GTK window a process presents costs
    // ~880 ms and for a locker spawned at lock time that window IS the lock
    // screen, which is why locking used to take about a second to show
    // anything (swaypplet docs/LOCK_TRANSITION_WIP.md). Paying it here, while
    // the session is unlocked and nobody is waiting, makes it free.
    locker::prewarm();

    let mut locker_active = false;
    // True only between the compositor's lock confirmation (LockerUp) and
    // LockerGone. locker_active alone means "a launch is in flight".
    let mut locker_confirmed = false;
    // Deadline for releasing the sleep inhibitor while a locker launch is in
    // flight. Some(_) means PrepareForSleep(true) arrived and we still hold
    // the inhibitor.
    let mut sleep_release: Option<Instant> = None;
    // When the locked, idle session's outputs should go off. Some(_) only
    // while locked AND idle; any input clears it. See `blank_deadline`.
    let mut blank_at: Option<Instant> = None;
    // Why the current locker was started ("idle" | "manual" | "sleep" |
    // "switch"); decides whether LockerUp blanks the outputs.
    let mut lock_reason: &'static str = "manual";
    // Mirrors the logind session Active property.
    let mut session_active = true;
    // True only while the dim tier's 10% is on the screen, so the resume
    // restores brightness it actually took away.
    let mut dimmed = false;

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
                // Presence is not input, so it does not *disarm* the blank
                // deadline — but it does light the screen, so it must arm
                // one. Without this, a walk-past after the deadline had
                // already fired would power the panel on with nothing left
                // to turn it off again, and the lock screen would sit lit
                // until someone touched a key.
                if locker_confirmed {
                    blank_at = blank_deadline(&cfg);
                }
                outputs.power("presence.back", Power::On);
            } else if !cfg.walk_away_lock {
                log::info!("presence: user gone — walk-away lock off (setting), not locking");
            } else if crate::inhibit::NoLock.armed() {
                log::info!("presence: user gone — No Lock armed, not locking");
            } else if crate::inhibit::idle_inhibited() {
                // Absence is the one tier the compositor cannot suppress for
                // us. The timeout tiers ride ext-idle-notify, which honours
                // idle inhibitors on its own (wayland.rs), so a video player
                // or a hand-set `inhibit_idle` already holds them off; walking away is not
                // idle, so this path has to ask. One IPC round trip, on an
                // edge rather than on the tick.
                log::info!("presence: user gone — idle inhibited, not locking");
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

        // The settings file, once a second. A moved mtime is reloaded whole;
        // only a change in the idle section is worth a log line and a
        // re-arm. The blank duration and the dim level are read from `cfg`
        // at fire time, so they need no re-arm at all.
        if Instant::now() >= next_settings_check {
            next_settings_check = Instant::now() + SETTINGS_POLL;
            if settings_file.changed() {
                let fresh = Settings::load().idle();
                if fresh != cfg {
                    log::info!(
                        "idle: settings changed — dim {}s to {}%, lock {}s, blank {}s, suspend {}s (0 is never)",
                        fresh.dim_after_s,
                        fresh.dim_level,
                        fresh.lock_after_s,
                        fresh.blank_after_s,
                        fresh.suspend_after_s
                    );
                    cfg = fresh;
                    if timeouts.send(wayland::Timeouts::from(&cfg)).is_err() {
                        log::error!("idle: wayland thread gone; timers not re-armed");
                    }
                }
            }
        }

        let ev = rx.recv_timeout(Duration::from_millis(250));

        // Deadlines fire on every pass, event traffic or not.
        if sleep_release.is_some_and(|d| Instant::now() >= d) {
            log::warn!(
                "before-sleep: locker not up after {SLEEP_RELEASE_MAX:?} — releasing inhibitor anyway"
            );
            // Whatever is on the panel is not a locked screen, and with the
            // cross-fade it may be a half-dissolved one. Blank before
            // releasing, so the last frame scanned out — the one still on the
            // panel at resume, before the compositor repaints — is nothing.
            outputs.power("before-sleep.timeout", Power::Off);
            release_inhibitor(&mut sleep_release);
        }
        if blank_at.is_some_and(|d| Instant::now() >= d) {
            blank_at = None;
            // Gates checked at fire time, not at arm time: the lock must
            // still be confirmed (the security invariant — never blank an
            // unlocked session; a crash relaunch keeps the compositor lock
            // held, so confirmed stays true through the gap) and the session
            // must still be on the seat, since a switch-away lock leaves our
            // idle timers running on a VT we no longer own.
            if locker_confirmed && session_active {
                log::info!("lock.blank: {}s idle while locked", cfg.blank_after_s);
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
                // Dimming is the first step toward the lock, so No Lock owns
                // it: a screen that fades while you read it is the same
                // complaint as one that locks while you read it.
                if crate::inhibit::NoLock.armed() {
                    log::info!("idle.dim: skip (No Lock)");
                } else {
                    dimmed = true;
                    log::info!("idle.dim: {}s idle — {}%", cfg.dim_after_s, cfg.dim_level);
                    outputs.brightness("idle.dim", cfg.dim_level);
                }
            }
            // Restore only what this tier faded. The resume fires whether or
            // not the dim did, and an unconditional 100% would undo a
            // deliberate brightness setting every time input resumed under
            // No Lock.
            Ev::Resumed(Timeout::Dim) => {
                if std::mem::take(&mut dimmed) {
                    outputs.brightness("idle.dim.resume", 100);
                }
            }

            Ev::Idled(Timeout::Lock) => {
                // Sitting still is not being away. While the sensor sees
                // someone the 300 s tier is suppressed and the absence path
                // owns locking instead; nothing re-arms this tier until input
                // resumes, which is the intent.
                if crate::inhibit::NoLock.armed() {
                    log::info!("idle.lock: skip (No Lock)");
                } else if present == Some(true) {
                    log::info!("idle.lock: skip (present)");
                } else {
                    log::info!("idle.lock: {}s idle — fire", cfg.lock_after_s);
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

            // Input stopped while locked: start counting toward the blank.
            // This no longer blanks anything by itself.
            Ev::Idled(Timeout::LockIdle) => {
                if locker_confirmed {
                    blank_at = blank_deadline(&cfg);
                    match blank_at {
                        Some(_) => log::info!("lock.blank: armed for {}s", cfg.blank_after_s),
                        None => log::info!("lock.blank: never (setting)"),
                    }
                }
            }
            // Input while locked: the user is here, so cancel the pending
            // blank and light the outputs. Powering on is unconditional
            // (matches the old config): if the locker died, a gated resume
            // would leave the panel stuck off on the next keypress.
            Ev::Resumed(Timeout::LockIdle) => {
                if blank_at.take().is_some() {
                    log::info!("lock.blank: disarmed (input)");
                }
                outputs.power("idle.lock-idle.resume", Power::On);
            }

            Ev::Idled(Timeout::Suspend) => {
                // Never suspend from an inactive session: our idle timers keep
                // advancing after a VT switch, so an unguarded suspend would
                // sleep the whole machine out from under the user who is
                // actively on another VT.
                if !session_active {
                    log::info!("idle.suspend: session inactive — skip");
                } else if on_ac() {
                    log::info!("idle.suspend: on AC — skip");
                } else if crate::inhibit::NoSleep.armed() {
                    // No Sleep's inhibitor covers logind's *lid* handling,
                    // and this tier is not logind's. Without this the second
                    // path wins anyway: shut the lid on battery, and twenty
                    // minutes later the machine this switch exists to keep
                    // awake suspends itself.
                    log::info!("idle.suspend: No Sleep armed — skip");
                } else {
                    log::info!("idle.suspend: {}s idle on battery", cfg.suspend_after_s);
                    run_cmd("idle.suspend", "systemctl", &["suspend"]);
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
                blank_at = None;
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
                    // Suspend path: blank now. The machine is about to sleep
                    // and the inhibitor must not wait on a timer. This is the
                    // one blank that is not an idle policy.
                    log::info!("lock: locker up — blanking outputs (sleep)");
                    blank_at = None;
                    outputs.power("lock.blank", Power::Off);
                } else {
                    // Every other lock reason, without exception, leaves the
                    // screen lit. The lock screen is meant to be seen: it is
                    // where face unlock reports what it is doing, and a panel
                    // that goes dark a second after locking made that
                    // invisible. Blanking waits for real idle time.
                    blank_at = blank_deadline(&cfg);
                    log::info!(
                        "lock: locker up ({lock_reason}) — screen stays lit, blank in {}",
                        match blank_at {
                            Some(_) => format!("{}s", cfg.blank_after_s),
                            None => "never (setting)".to_string(),
                        }
                    );
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
                // The lock episode is over, so the blank deadline goes with
                // it. The fire-time gate would refuse anyway, but leaving a
                // live deadline pointing at an unlocked session is the kind
                // of state that survives one refactor and blanks a desktop
                // after the next.
                blank_at = None;
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
                // Arm the next one straight away, so the warm-up lands now
                // rather than on whoever locks next. rc=2 included: a lock
                // that could not be acquired says nothing about the next one.
                locker::prewarm();
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
