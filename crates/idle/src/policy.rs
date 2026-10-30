//! The idle manager's rules, as a reducer: [`step`] takes the state and one
//! thing that happened, and returns what the outputs, logind, the locker and
//! the wayland thread have to be told. Nothing in here touches a device, a
//! socket or the clock; [`World`] is the one window onto the machine, so the
//! security invariant in the crate docs (never blank an unlocked session) is
//! something a test can hold the rules to.
//!
//! The run loop in `lib.rs` is the only caller. It keeps the order a pass has
//! always had: drain presence, run the settings check, sleep until the next
//! event or deadline, fire the deadlines, then handle the event.

use std::time::{Duration, Instant};

use swaypplet_base::lock_proto::{EXIT_UNAVAILABLE, EXIT_UNLOCKED};
use swaypplet_model::store::Idle;
use swaypplet_system::presence::Event as PresenceEvent;

use crate::Ev;
use crate::outputs::Power;
use crate::wayland::Timeout;

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
pub(crate) const SLEEP_RELEASE_MAX: Duration = Duration::from_secs(4);

/// What the rules read from the machine, at the moment they read it.
pub(crate) trait World {
    fn now(&self) -> Instant;
    /// A battery exists and nothing is charging it.
    fn on_battery(&self) -> bool;
    /// Any supply reports online=1.
    fn on_ac(&self) -> bool;
    /// The No Lock switch is armed.
    fn no_lock(&self) -> bool;
    /// The No Sleep switch is armed.
    fn no_sleep(&self) -> bool;
    /// A compositor idle inhibitor is up.
    fn idle_inhibited(&self) -> bool;
}

/// What a step asks of the world, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Effect {
    Power(&'static str, Power),
    PowerBrightness(&'static str, Power, u8),
    Brightness(&'static str, u8),
    /// Start a supervised locker for this reason.
    StartLocker(&'static str),
    /// Warm the next locker.
    Prewarm,
    ReleaseSleepInhibitor,
    SetLockedHint(bool),
    /// Re-arm the wayland thread's timers for this tier set.
    Retime(Idle),
    /// `systemctl suspend`.
    Suspend,
    /// A source died: exit(1).
    Exit,
}

/// One thing a pass of the run loop hands the rules.
pub(crate) enum Input {
    /// Every presence event queued since the last pass. Taken as one batch,
    /// because only where the sensor settled is an edge.
    Presence(Vec<PresenceEvent>),
    /// The settings check is due. `reload` is the file, read again when it
    /// was written; `minute` the local minute of day; `next_check` the next
    /// minute boundary when this check was due by the clock.
    Settings {
        reload: Option<Idle>,
        minute: u16,
        next_check: Option<Instant>,
    },
    /// The loop woke: fire whatever deadline has passed.
    Deadlines,
    Event(Ev),
}

pub(crate) struct State {
    /// The tiers as the file has them.
    pub saved: Idle,
    /// The tiers in force this minute: `saved`, or the night window's.
    pub cfg: Idle,
    pub settings_changed: bool,
    pub next_settings_check: Instant,
    pub locker_active: bool,
    /// True only between the compositor's lock confirmation (LockerUp) and
    /// LockerGone. locker_active alone means "a launch is in flight".
    pub locker_confirmed: bool,
    /// Deadline for releasing the sleep inhibitor while a locker launch is in
    /// flight. Some(_) means PrepareForSleep(true) arrived and we still hold
    /// the inhibitor.
    pub sleep_release: Option<Instant>,
    /// When the locked, idle session's outputs should go off. Some(_) only
    /// while locked AND idle; any input clears it. See `blank_deadline`.
    pub blank_at: Option<Instant>,
    /// The power source as last read, for the blank tier. Read again at every
    /// arm and on the minute check rather than watched: a minute's lag on an
    /// unplug costs at most a minute of lit lock screen.
    pub battery: bool,
    /// Why the current locker was started ("idle" | "manual" | "sleep" |
    /// "switch"); decides whether LockerUp blanks the outputs.
    pub lock_reason: &'static str,
    /// Mirrors the logind session Active property.
    pub session_active: bool,
    /// True only while the dim tier's 10% is on the screen, so the resume
    /// restores brightness it actually took away.
    pub dimmed: bool,
    /// Mirrors the sensor, for the tiers that ask whether anyone is here.
    /// None until the first reading lands.
    pub present: Option<bool>,
}

impl State {
    pub fn new(saved: Idle, cfg: Idle, next_settings_check: Instant, battery: bool) -> Self {
        Self {
            saved,
            cfg,
            settings_changed: false,
            next_settings_check,
            locker_active: false,
            locker_confirmed: false,
            sleep_release: None,
            blank_at: None,
            battery,
            lock_reason: "manual",
            session_active: true,
            dimmed: false,
            present: None,
        }
    }

    /// Whether the settings check runs this pass.
    pub fn settings_due(&self, now: Instant) -> bool {
        self.settings_changed || now >= self.next_settings_check
    }

    /// The nearest deadline the loop has to wake for.
    pub fn deadline(&self) -> Instant {
        [
            Some(self.next_settings_check),
            self.sleep_release,
            self.blank_at,
        ]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(self.next_settings_check)
    }
}

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
///
/// On battery the tier is the shorter of the two settings
/// (`Idle::blank_after_on`). The power source is read by the caller at arm
/// time and again on the minute check, so unplugging while locked shortens a
/// countdown already running.
fn blank_deadline(cfg: &Idle, on_battery: bool, now: Instant) -> Option<Instant> {
    let secs = cfg.blank_after_on(on_battery);
    (secs > 0).then(|| now + Duration::from_secs(u64::from(secs)))
}

/// The blank tier as the log lines say it: `300s (on battery)`, `900s`, or
/// the setting's never.
fn blank_label(cfg: &Idle, on_battery: bool) -> String {
    match cfg.blank_after_on(on_battery) {
        0 => "never (setting)".to_string(),
        secs if on_battery && secs != cfg.blank_after_s => format!("{secs}s (on battery)"),
        secs => format!("{secs}s"),
    }
}

pub(crate) fn step(s: &mut State, input: Input, w: &impl World) -> Vec<Effect> {
    let mut out = Vec::new();
    match input {
        Input::Presence(events) => presence(s, events, w, &mut out),
        Input::Settings {
            reload,
            minute,
            next_check,
        } => settings(s, reload, minute, next_check, w, &mut out),
        Input::Deadlines => deadlines(s, w, &mut out),
        Input::Event(ev) => event(s, ev, w, &mut out),
    }
    out
}

fn release_inhibitor(s: &mut State, out: &mut Vec<Effect>) {
    if s.sleep_release.take().is_some() {
        out.push(Effect::ReleaseSleepInhibitor);
    }
}

fn ensure_locked(s: &mut State, reason: &'static str, out: &mut Vec<Effect>) {
    if s.locker_active {
        log::info!("lock: locker already running — skip ({reason})");
        return;
    }
    s.locker_active = true;
    s.locker_confirmed = false;
    s.lock_reason = reason;
    out.push(Effect::StartLocker(reason));
}

fn presence(s: &mut State, events: Vec<PresenceEvent>, w: &impl World, out: &mut Vec<Effect>) {
    let mut now_present = None;
    for event in events {
        match event {
            PresenceEvent::Changed(state) => {
                // Only a change in the settled state is an edge to act on;
                // the owner sends its opening reading too.
                if s.present != Some(state) {
                    s.present = Some(state);
                    now_present = Some(state);
                }
            }
            // Display only; the bar is what reads it.
            PresenceEvent::Attention(_) => {}
        }
    }
    let Some(now_present) = now_present else {
        return;
    };
    if now_present {
        log::info!("presence: user back — powering outputs on");
        // The mirror of the blank invariant. Blanking is dangerous without a
        // confirmed lock, so it is guarded; powering on never is, so this is
        // unconditional. Without it, returning to a blanked session leaves
        // you facing a dark screen until you touch something — and the
        // locker is by then already attempting face unlock behind it, so the
        // machine looks dead at exactly the moment it is working.
        //
        // Outputs only, no brightness restore. If the dim tier had faded the
        // screen it stays faded, which is still legible, and the existing
        // input-driven Resumed(Dim) path is what should own brightness —
        // presence is not input, and overriding a deliberate brightness
        // setting from a walk-past would be worse than a dim lock screen.
        //
        // Presence is not input, so it does not *disarm* the blank deadline
        // — but it does light the screen, so it must arm one. Without this, a
        // walk-past after the deadline had already fired would power the
        // panel on with nothing left to turn it off again, and the lock
        // screen would sit lit until someone touched a key.
        if s.locker_confirmed {
            s.battery = w.on_battery();
            s.blank_at = blank_deadline(&s.cfg, s.battery, w.now());
        }
        out.push(Effect::Power("presence.back", Power::On));
    } else if !s.cfg.walk_away_lock {
        log::info!("presence: user gone — walk-away lock off (setting), not locking");
    } else if w.no_lock() {
        log::info!("presence: user gone — No Lock armed, not locking");
    } else if w.idle_inhibited() {
        // Absence is the one tier the compositor cannot suppress for us. The
        // timeout tiers ride ext-idle-notify, which honours idle inhibitors
        // on its own (wayland.rs), so a video player or a hand-set
        // `inhibit_idle` already holds them off; walking away is not idle,
        // so this path has to ask. One IPC round trip, on an edge rather
        // than on the tick.
        log::info!("presence: user gone — idle inhibited, not locking");
    } else {
        log::info!("presence: user gone — locking");
        ensure_locked(s, "presence", out);
    }
}

/// The settings file (an event) and the clock (each minute boundary). An
/// edit is reloaded whole, and the night window is resolved on every check,
/// so one comparison covers both causes: an edit in the pane, and the window
/// opening or closing. The blank duration and the dim level are read from
/// `cfg` at fire time, so they need no re-arm at all.
fn settings(
    s: &mut State,
    reload: Option<Idle>,
    minute: u16,
    next_check: Option<Instant>,
    w: &impl World,
    out: &mut Vec<Effect>,
) {
    if let Some(next) = next_check {
        s.next_settings_check = next;
    }
    let reloaded = std::mem::take(&mut s.settings_changed);
    if let Some(saved) = reload {
        s.saved = saved;
    }
    let fresh = s.saved.resolve(minute);
    let was_battery = std::mem::replace(&mut s.battery, w.on_battery());
    if s.battery != was_battery {
        log::info!(
            "idle: {} — screen off {}",
            if s.battery { "on battery" } else { "on AC" },
            blank_label(&fresh, s.battery)
        );
    }
    let tier_changed = fresh != s.cfg || s.battery != was_battery;
    if fresh != s.cfg {
        log::info!(
            "idle: {} — dim {}s to {}%, lock {}s, blank {}s, suspend {}s (0 is never)",
            if reloaded {
                "settings changed"
            } else if s.saved.in_night(minute) {
                "night window open"
            } else {
                "night window closed"
            },
            fresh.dim_after_s,
            fresh.dim_level,
            fresh.lock_after_s,
            fresh.blank_after_s,
            fresh.suspend_after_s
        );
        s.cfg = fresh;
        out.push(Effect::Retime(s.cfg));
    }
    // A blank already counting down was armed against the old tier, or the
    // old power source. Shorten it to the new one, never extend it: the
    // window's job, and the battery tier's, is to put the screen out sooner,
    // and a locked screen that stays lit past the new deadline because the
    // window opened one second too late is the bug this avoids. Plugging in
    // therefore leaves a shorter countdown alone. `None` from the new tier is
    // "never", which disarms.
    if tier_changed && let Some(at) = s.blank_at {
        s.blank_at = blank_deadline(&s.cfg, s.battery, w.now()).map(|fresh_at| at.min(fresh_at));
    }
}

/// Deadlines fire on every pass, event traffic or not.
fn deadlines(s: &mut State, w: &impl World, out: &mut Vec<Effect>) {
    if s.sleep_release.is_some_and(|d| w.now() >= d) {
        log::warn!(
            "before-sleep: locker not up after {SLEEP_RELEASE_MAX:?} — releasing inhibitor anyway"
        );
        // Whatever is on the panel is not a locked screen, and with the
        // cross-fade it may be a half-dissolved one. Blank before releasing,
        // so the last frame scanned out — the one still on the panel at
        // resume, before the compositor repaints — is nothing.
        out.push(Effect::Power("before-sleep.timeout", Power::Off));
        release_inhibitor(s, out);
    }
    if s.blank_at.is_some_and(|d| w.now() >= d) {
        s.blank_at = None;
        // Gates checked at fire time, not at arm time: the lock must still be
        // confirmed (the security invariant — never blank an unlocked
        // session; a crash relaunch keeps the compositor lock held, so
        // confirmed stays true through the gap) and the session must still be
        // on the seat, since a switch-away lock leaves our idle timers
        // running on a VT we no longer own.
        if s.locker_confirmed && s.session_active {
            log::info!(
                "lock.blank: {} idle while locked",
                blank_label(&s.cfg, s.battery)
            );
            out.push(Effect::Power("lock.blank", Power::Off));
        } else {
            log::info!("lock.blank: skipped (session inactive or locker gone)");
        }
    }
}

fn event(s: &mut State, ev: Ev, w: &impl World, out: &mut Vec<Effect>) {
    match ev {
        Ev::Idled(Timeout::Dim) => {
            // Dimming is the first step toward the lock, so No Lock owns it:
            // a screen that fades while you read it is the same complaint as
            // one that locks while you read it.
            if w.no_lock() {
                log::info!("idle.dim: skip (No Lock)");
            } else {
                s.dimmed = true;
                log::info!(
                    "idle.dim: {}s idle — {}%",
                    s.cfg.dim_after_s,
                    s.cfg.dim_level
                );
                out.push(Effect::Brightness("idle.dim", s.cfg.dim_level));
            }
        }
        // Restore only what this tier faded. The resume fires whether or not
        // the dim did, and an unconditional 100% would undo a deliberate
        // brightness setting every time input resumed under No Lock.
        Ev::Resumed(Timeout::Dim) => {
            if std::mem::take(&mut s.dimmed) {
                out.push(Effect::Brightness("idle.dim.resume", 100));
            }
        }

        Ev::Idled(Timeout::Lock) => {
            // Sitting still is not being away. While the sensor sees someone
            // the 300 s tier is suppressed and the absence path owns locking
            // instead; nothing re-arms this tier until input resumes, which
            // is the intent.
            if w.no_lock() {
                log::info!("idle.lock: skip (No Lock)");
            } else if s.present == Some(true) {
                log::info!("idle.lock: skip (present)");
            } else {
                log::info!("idle.lock: {}s idle — fire", s.cfg.lock_after_s);
                ensure_locked(s, "idle", out);
            }
        }
        Ev::Resumed(Timeout::Lock) => {}

        // Input stopped while locked: start counting toward the blank. This
        // no longer blanks anything by itself.
        Ev::Idled(Timeout::LockIdle) => {
            if s.locker_confirmed {
                s.battery = w.on_battery();
                s.blank_at = blank_deadline(&s.cfg, s.battery, w.now());
                match s.blank_at {
                    Some(_) => {
                        log::info!("lock.blank: armed for {}", blank_label(&s.cfg, s.battery))
                    }
                    None => log::info!("lock.blank: never (setting)"),
                }
            }
        }
        // Input while locked: the user is here, so cancel the pending blank
        // and light the outputs. Powering on is unconditional (matches the
        // old config): if the locker died, a gated resume would leave the
        // panel stuck off on the next keypress.
        Ev::Resumed(Timeout::LockIdle) => {
            if s.blank_at.take().is_some() {
                log::info!("lock.blank: disarmed (input)");
            }
            out.push(Effect::Power("idle.lock-idle.resume", Power::On));
        }

        Ev::Idled(Timeout::Suspend) => {
            // Never suspend from an inactive session: our idle timers keep
            // advancing after a VT switch, so an unguarded suspend would sleep
            // the whole machine out from under the user who is actively on
            // another VT.
            if !s.session_active {
                log::info!("idle.suspend: session inactive — skip");
            } else if w.on_ac() {
                log::info!("idle.suspend: on AC — skip");
            } else if w.no_sleep() {
                // No Sleep's inhibitor covers logind's *lid* handling, and
                // this tier is not logind's. Without this the second path
                // wins anyway: shut the lid on battery, and twenty minutes
                // later the machine this switch exists to keep awake
                // suspends itself.
                log::info!("idle.suspend: No Sleep armed — skip");
            } else {
                log::info!("idle.suspend: {}s idle on battery", s.cfg.suspend_after_s);
                out.push(Effect::Suspend);
            }
        }
        Ev::Resumed(Timeout::Suspend) => {}

        Ev::Sleep(true) => {
            log::info!("before-sleep: fire");
            if s.locker_confirmed {
                // Already locked — nothing to wait for.
                s.sleep_release = Some(w.now());
                release_inhibitor(s, out);
            } else {
                // No lock, or a launch is in flight but the compositor hasn't
                // confirmed it yet: hold the inhibitor until LockerUp.
                // Releasing on a mere spawn is how a suspend freeze once
                // caught the locker mid-startup and slept the machine
                // unlocked.
                ensure_locked(s, "sleep", out);
                s.sleep_release = Some(w.now() + SLEEP_RELEASE_MAX);
            }
        }
        Ev::Sleep(false) => {
            // logind.rs re-takes the inhibitor itself on resume.
            out.push(Effect::PowerBrightness("after-resume", Power::On, 100));
        }

        Ev::LockSignal => {
            log::info!("lock: signal");
            ensure_locked(s, "manual", out);
        }
        Ev::UnlockSignal => {
            log::info!("unlock: signal");
            s.blank_at = None;
            out.push(Effect::PowerBrightness("unlock", Power::On, 100));
        }

        Ev::RecoverLock => {
            // Re-power outputs first: the dead locker may have left them
            // blanked, and the new lock surface needs a lit output to be
            // seen. Then lock with a reason that keeps the UI visible.
            out.push(Effect::PowerBrightness("recover", Power::On, 100));
            ensure_locked(s, "recover", out);
        }
        Ev::LockerUp => {
            s.locker_confirmed = true;
            if s.sleep_release.is_some() {
                // Suspend path: blank now. The machine is about to sleep and
                // the inhibitor must not wait on a timer. This is the one
                // blank that is not an idle policy.
                log::info!("lock: locker up — blanking outputs (sleep)");
                s.blank_at = None;
                out.push(Effect::Power("lock.blank", Power::Off));
            } else {
                // Every other lock reason, without exception, leaves the
                // screen lit. The lock screen is meant to be seen: it is
                // where face unlock reports what it is doing, and a panel
                // that goes dark a second after locking made that invisible.
                // Blanking waits for real idle time.
                s.battery = w.on_battery();
                s.blank_at = blank_deadline(&s.cfg, s.battery, w.now());
                log::info!(
                    "lock: locker up ({}) — screen stays lit, blank in {}",
                    s.lock_reason,
                    blank_label(&s.cfg, s.battery)
                );
            }
            // Record the lock in logind so a service restart can recover it
            // (see Ev::RecoverLock).
            out.push(Effect::SetLockedHint(true));
            release_inhibitor(s, out);
        }
        Ev::SessionActive(false) => {
            s.session_active = false;
            // Leaving the seat (fast user switch, bare VT change): lock the
            // abandoned session behind us. Idempotent when the switcher
            // script already sent Lock.
            ensure_locked(s, "switch", out);
        }
        Ev::SessionActive(true) => {
            s.session_active = true;
            if s.locker_active {
                // Returning to a locked session: light the outputs so the
                // lock screen shows instead of a dead panel.
                out.push(Effect::PowerBrightness("switch.return", Power::On, 100));
            }
        }
        Ev::LockerGone { rc } => {
            s.locker_active = false;
            s.locker_confirmed = false;
            // The lock episode is over, so the blank deadline goes with it.
            // The fire-time gate would refuse anyway, but leaving a live
            // deadline pointing at an unlocked session is the kind of state
            // that survives one refactor and blanks a desktop after the next.
            s.blank_at = None;
            // Locked-or-bailed either way: don't hold suspend for this.
            release_inhibitor(s, out);
            match rc {
                EXIT_UNLOCKED => {
                    log::info!("lock: clean unlock");
                    out.push(Effect::SetLockedHint(false));
                    out.push(Effect::PowerBrightness("unlock", Power::On, 100));
                }
                EXIT_UNAVAILABLE => {
                    log::warn!("lock: lock unavailable (rc=2) — another locker or no protocol")
                }
                _ => log::error!("lock: locker never started (rc={rc}) — NOT blanking"),
            }
            // Arm the next one straight away, so the warm-up lands now rather
            // than on whoever locks next. rc=2 included: a lock that could
            // not be acquired says nothing about the next one.
            out.push(Effect::Prewarm);
        }

        Ev::Fatal(msg) => {
            log::error!("idle: fatal: {msg}");
            out.push(Effect::Exit);
        }
        // Handled at the top of the next pass, with the clock.
        Ev::SettingsChanged => s.settings_changed = true,
        // The presence queue is drained at the top of the next pass.
        Ev::Wake => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    struct Fake {
        now: Cell<Instant>,
        battery: bool,
    }

    impl Fake {
        fn new() -> Self {
            Self {
                now: Cell::new(Instant::now()),
                battery: false,
            }
        }
        fn advance(&self, by: Duration) {
            self.now.set(self.now.get() + by);
        }
    }

    impl World for Fake {
        fn now(&self) -> Instant {
            self.now.get()
        }
        fn on_battery(&self) -> bool {
            self.battery
        }
        fn on_ac(&self) -> bool {
            !self.battery
        }
        fn no_lock(&self) -> bool {
            false
        }
        fn no_sleep(&self) -> bool {
            false
        }
        fn idle_inhibited(&self) -> bool {
            false
        }
    }

    fn state(w: &Fake) -> State {
        let cfg = Idle::default();
        State::new(cfg, cfg, w.now() + Duration::from_secs(60), false)
    }

    fn blanks(effects: &[Effect]) -> bool {
        effects.iter().any(|e| {
            matches!(
                e,
                Effect::Power(_, Power::Off) | Effect::PowerBrightness(_, Power::Off, _)
            )
        })
    }

    /// Run `inputs` through the rules, firing deadlines after each one the
    /// way the loop does, and say whether anything powered the outputs off
    /// while the lock was not confirmed.
    fn blanked_unlocked(s: &mut State, w: &Fake, inputs: Vec<Input>) -> bool {
        let mut bad = false;
        for input in inputs {
            for i in [input, Input::Deadlines] {
                let confirmed_before = s.locker_confirmed;
                let out = step(s, i, w);
                if blanks(&out) && !(confirmed_before || s.locker_confirmed) {
                    bad = true;
                }
            }
        }
        bad
    }

    #[test]
    fn locking_never_blanks_until_the_compositor_confirms() {
        let w = Fake::new();
        let mut s = state(&w);
        let out = step(&mut s, Input::Event(Ev::Idled(Timeout::Lock)), &w);
        assert_eq!(out, [Effect::StartLocker("idle")]);
        // Input stops and the blank tier passes, with no LockerUp: the
        // locker is still starting, or died before the lock was real.
        step(&mut s, Input::Event(Ev::Idled(Timeout::LockIdle)), &w);
        w.advance(Duration::from_secs(3600));
        assert!(!blanks(&step(&mut s, Input::Deadlines, &w)));
    }

    #[test]
    fn a_confirmed_lock_blanks_once_the_tier_passes() {
        let w = Fake::new();
        let mut s = state(&w);
        step(&mut s, Input::Event(Ev::LockSignal), &w);
        step(&mut s, Input::Event(Ev::LockerUp), &w);
        w.advance(Duration::from_secs(u64::from(s.cfg.blank_after_s)));
        assert_eq!(
            step(&mut s, Input::Deadlines, &w),
            [Effect::Power("lock.blank", Power::Off)]
        );
    }

    #[test]
    fn a_blank_armed_while_locked_dies_with_the_lock() {
        let w = Fake::new();
        let mut s = state(&w);
        step(&mut s, Input::Event(Ev::LockSignal), &w);
        step(&mut s, Input::Event(Ev::LockerUp), &w);
        step(&mut s, Input::Event(Ev::LockerGone { rc: 0 }), &w);
        w.advance(Duration::from_secs(3600));
        assert!(!blanks(&step(&mut s, Input::Deadlines, &w)));
    }

    #[test]
    fn a_crash_relaunch_does_not_reopen_the_blank_gate() {
        let w = Fake::new();
        let mut s = state(&w);
        // A locker that never confirmed, then gave up.
        step(&mut s, Input::Event(Ev::LockSignal), &w);
        step(&mut s, Input::Event(Ev::LockerGone { rc: 1 }), &w);
        step(&mut s, Input::Event(Ev::Idled(Timeout::LockIdle)), &w);
        w.advance(Duration::from_secs(3600));
        assert!(!blanks(&step(&mut s, Input::Deadlines, &w)));
    }

    #[test]
    fn no_idle_path_blanks_an_unlocked_session() {
        let w = Fake::new();
        let mut s = state(&w);
        let walk = vec![
            Input::Presence(vec![PresenceEvent::Changed(true)]),
            Input::Event(Ev::Idled(Timeout::Dim)),
            Input::Event(Ev::Idled(Timeout::LockIdle)),
            Input::Presence(vec![PresenceEvent::Changed(false)]),
            Input::Event(Ev::SessionActive(false)),
            Input::Event(Ev::SessionActive(true)),
            Input::Presence(vec![PresenceEvent::Changed(true)]),
            Input::Event(Ev::Idled(Timeout::Lock)),
            Input::Event(Ev::Idled(Timeout::LockIdle)),
            Input::Event(Ev::UnlockSignal),
            Input::Event(Ev::LockerGone { rc: 0 }),
            Input::Event(Ev::Idled(Timeout::LockIdle)),
        ];
        for input in walk {
            w.advance(Duration::from_secs(1000));
            assert!(!blanked_unlocked(&mut s, &w, vec![input]));
        }
    }

    #[test]
    fn the_sleep_path_blanks_on_confirmation_and_releases() {
        let w = Fake::new();
        let mut s = state(&w);
        let out = step(&mut s, Input::Event(Ev::Sleep(true)), &w);
        assert_eq!(out, [Effect::StartLocker("sleep")]);
        assert!(!blanks(&step(&mut s, Input::Deadlines, &w)));
        let out = step(&mut s, Input::Event(Ev::LockerUp), &w);
        assert_eq!(
            out,
            [
                Effect::Power("lock.blank", Power::Off),
                Effect::SetLockedHint(true),
                Effect::ReleaseSleepInhibitor,
            ]
        );
    }

    #[test]
    fn a_locker_that_misses_the_sleep_deadline_still_lets_the_machine_sleep() {
        // The one blank without a confirmed lock, by design: the machine is
        // going down either way, and the frame left on the panel at resume
        // should be nothing rather than the desktop.
        let w = Fake::new();
        let mut s = state(&w);
        step(&mut s, Input::Event(Ev::Sleep(true)), &w);
        w.advance(SLEEP_RELEASE_MAX);
        assert_eq!(
            step(&mut s, Input::Deadlines, &w),
            [
                Effect::Power("before-sleep.timeout", Power::Off),
                Effect::ReleaseSleepInhibitor,
            ]
        );
    }

    #[test]
    fn a_second_trigger_while_locking_starts_no_second_locker() {
        let w = Fake::new();
        let mut s = state(&w);
        step(&mut s, Input::Event(Ev::LockSignal), &w);
        assert!(step(&mut s, Input::Event(Ev::Idled(Timeout::Lock)), &w).is_empty());
        assert!(step(&mut s, Input::Event(Ev::SessionActive(false)), &w).is_empty());
    }

    #[test]
    fn sitting_still_in_view_does_not_lock() {
        let w = Fake::new();
        let mut s = state(&w);
        step(
            &mut s,
            Input::Presence(vec![PresenceEvent::Changed(true)]),
            &w,
        );
        assert!(step(&mut s, Input::Event(Ev::Idled(Timeout::Lock)), &w).is_empty());
    }

    #[test]
    fn a_window_that_shortens_the_tier_shortens_a_running_countdown() {
        let w = Fake::new();
        let mut s = state(&w);
        step(&mut s, Input::Event(Ev::LockSignal), &w);
        step(&mut s, Input::Event(Ev::LockerUp), &w);
        let armed = s.blank_at.expect("armed on LockerUp");
        let shorter = Idle {
            blank_after_s: 60,
            ..s.saved
        };
        let out = step(
            &mut s,
            Input::Settings {
                reload: Some(shorter),
                minute: 0,
                next_check: None,
            },
            &w,
        );
        assert_eq!(out, [Effect::Retime(shorter)]);
        assert!(s.blank_at.expect("still armed") < armed);
    }
}
