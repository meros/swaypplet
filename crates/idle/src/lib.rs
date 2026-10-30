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
//! and one set of rules decides what each event means (policy.rs, a reducer
//! the run loop below feeds and whose effects it carries out).
//!
//! Behavior ported 1:1 from the old swayidle config (users/modules/swayidle.nix
//! in the nixos repo — see its comments for the incident history behind each
//! rule). The durations are the defaults; the settings pane's Idle & Lock tab
//! overrides them (`swaypplet_model::store::Idle`):
//!
//!   240 s  dim to 10% brightness, restore on resume
//!   300 s  lock the session (the screen stays LIT; locking does not blank)
//!    15 m  power outputs off after this much idle time *while locked*;
//!          any input disarms it and re-powers
//!     5 m  the same, on battery, when shorter (`Idle::blank_after_on`)
//!  1200 s  suspend, only on battery
//!
//! The pane is another process, so an edit reaches here as a file: an
//! inotify watch on `~/.config/swaypplet/settings.json` (`swaypplet_base::watch`)
//! sends `Ev::SettingsChanged`, and the loop reloads and hands the wayland
//! thread new timeouts to re-arm with. Zero on any tier is "never". The loop
//! sleeps until the next event or its nearest deadline; nothing ticks.
//!
//! The night window (Idle & Lock tab, `Idle::resolve`) is a second, shorter
//! set of dim / lock / screen-off tiers for a time range, typically the
//! evening. The same once-a-second check resolves it, so the window opening
//! and an edit in the pane arrive by one path and re-arm by one comparison.
//! Suspend is not part of the window: it is battery-only, and cutting an
//! overnight job short is worse than a late suspend. Crossing INTO the
//! window while already idle past its shorter lock tier locks at once, which
//! is `ext-idle-notify` behaving correctly — the seat has been idle longer
//! than the timeout being armed.
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
//! Inhibitors (panel tiles, `swaypplet_system::inhibit`) are two standing switches, and
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
pub(crate) mod logind;
mod outputs;
mod policy;
mod wayland;

use outputs::Outputs;
use policy::{Effect, Input, State, World, step};

use std::process::Command;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use swaypplet_model::{store, store::Settings};
use swaypplet_system::power::{POWER_SUPPLY, Supplies};
use swaypplet_system::{presence, presence::Event as PresenceEvent};

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
    /// A presence event is queued; the top of the loop drains it.
    Wake,
    Fatal(String),
    /// The settings file was written (`swaypplet_base::watch`, inotify).
    SettingsChanged,
}

/// The instant the next minute of the day starts, when the night window may
/// open or close. The settings file itself arrives as an event.
fn next_minute() -> Instant {
    Instant::now() + swaypplet_base::time::until_next_minute(swaypplet_base::time::now())
}

/// The machine as the rules (`policy`) read it.
struct Machine;

impl World for Machine {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn on_battery(&self) -> bool {
        on_battery()
    }
    fn on_ac(&self) -> bool {
        on_ac()
    }
    fn no_lock(&self) -> bool {
        swaypplet_system::inhibit::NoLock.armed()
    }
    fn no_sleep(&self) -> bool {
        swaypplet_system::inhibit::NoSleep.armed()
    }
    fn idle_inhibited(&self) -> bool {
        swaypplet_system::inhibit::idle_inhibited()
    }
}

pub fn run() -> ! {
    let (tx, rx) = mpsc::channel::<Ev>();
    // Display changes go through here, never inline. `swaymsg output * power
    // on` takes 782-820 ms to return, and this loop must stay responsive
    // across exactly that window: the presence edge that lights the screen is
    // the same edge that starts a face attempt.
    let outputs = Outputs::start();
    // The timers, from the settings file, and the handle to re-arm them.
    // `saved` is the file; `cfg` is what is in force this minute, which is a
    // different set inside the night window (`Idle::resolve`). Everything
    // in `policy` reads `cfg` and never learns that a window exists.
    let saved = Settings::load().idle();
    let cfg = saved.resolve(store::local_minute_of_day());
    {
        let tx = tx.clone();
        swaypplet_base::watch::files_on_thread(vec![store::path()], move || {
            let _ = tx.send(Ev::SettingsChanged);
        });
    }
    let timeouts = wayland::start(tx.clone(), wayland::Timeouts::from(&cfg));
    let logind = logind::start(tx.clone());

    // Warm the next locker now. The first GTK window a process presents costs
    // ~880 ms and for a locker spawned at lock time that window IS the lock
    // screen, which is why locking used to take about a second to show
    // anything (swaypplet docs/LOCK_TRANSITION.md). Paying it here, while
    // the session is unlocked and nobody is waiting, makes it free.
    locker::prewarm();

    let mut state = State::new(saved, cfg, next_minute(), on_battery());
    let world = Machine;

    // This process owns the sensor for the whole session and publishes it on
    // the session bus; the bar and the lock screen's face engine listen there
    // rather than reading the device themselves. Reads cost 250-400 ms each
    // and the hardware serves about three a second, so three independent
    // readers meant all three queued. See `swaypplet_system::presence`.
    //
    // None on hardware without the sensor, which keeps the timer-only
    // behaviour intact.
    //
    // The loop sleeps on `rx`, not on a tick, so a presence edge has to wake
    // it. Without the forwarder below it was drained only when some other
    // event happened to arrive: on 2026-10-25 a return was acted on 3 s late,
    // and only because the face unlock woke the loop. On a blanked lock
    // screen that is a walk-up to a dark panel while face unlock runs
    // behind it.
    let presence = presence::serve().map(|events| {
        let (queue, drained) = mpsc::channel();
        let wake = tx.clone();
        std::thread::Builder::new()
            .name("presence-wake".into())
            .spawn(move || {
                while let Ok(event) = events.recv_blocking() {
                    if queue.send(event).is_err() || wake.send(Ev::Wake).is_err() {
                        return;
                    }
                }
            })
            .expect("spawn presence-wake");
        drained
    });

    let apply = |effects: Vec<Effect>| {
        for effect in effects {
            match effect {
                Effect::Power(scope, power) => outputs.power(scope, power),
                Effect::PowerBrightness(scope, power, level) => {
                    outputs.power_brightness(scope, power, level)
                }
                Effect::Brightness(scope, level) => outputs.brightness(scope, level),
                Effect::StartLocker(reason) => locker::start(tx.clone(), reason),
                Effect::Prewarm => locker::prewarm(),
                Effect::ReleaseSleepInhibitor => logind.release_sleep_inhibitor(),
                Effect::SetLockedHint(locked) => logind.set_locked_hint(locked),
                Effect::Retime(cfg) => {
                    if timeouts.send(wayland::Timeouts::from(&cfg)).is_err() {
                        log::error!("idle: wayland thread gone; timers not re-armed");
                    }
                }
                Effect::Suspend => run_cmd("idle.suspend", "systemctl", &["suspend"]),
                Effect::Exit => std::process::exit(1),
            }
        }
    };

    log::info!("idle: manager started");
    loop {
        // Transitions arrive from the owner thread, which samples on its own
        // cadence; each one also sends `Ev::Wake`, so the loop is here within
        // microseconds of the sensor changing.
        if let Some(events) = &presence {
            let batch: Vec<PresenceEvent> = events.try_iter().collect();
            if !batch.is_empty() {
                apply(step(&mut state, Input::Presence(batch), &world));
            }
        }

        let now = Instant::now();
        if state.settings_due(now) {
            let next_check = (now >= state.next_settings_check).then(next_minute);
            let reload = state.settings_changed.then(|| Settings::load().idle());
            let minute = store::local_minute_of_day();
            apply(step(
                &mut state,
                Input::Settings {
                    reload,
                    minute,
                    next_check,
                },
                &world,
            ));
        }

        // Asleep until the next event or the nearest deadline: no tick.
        let ev = rx.recv_timeout(state.deadline().saturating_duration_since(Instant::now()));

        apply(step(&mut state, Input::Deadlines, &world));

        let ev = match ev {
            Ok(ev) => ev,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                log::error!("idle: all event sources gone");
                std::process::exit(1);
            }
        };
        apply(step(&mut state, Input::Event(ev), &world));
    }
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
pub(crate) fn run_cmd_timed(scope: &str, cmd: &str, args: &[&str], waited: Duration) {
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

/// True when the machine has a battery and nothing is charging it. The
/// battery screen-off tier must not reach a machine that has no battery
/// (`Supplies::on_battery`).
fn on_battery() -> bool {
    supplies().on_battery()
}

/// True when any power supply reports online=1 (AC adapter present).
/// Mirrors the old `grep -q 1 /sys/class/power_supply/*/online`.
fn on_ac() -> bool {
    supplies().on_ac()
}

/// The supplies now; none when the directory cannot be read.
fn supplies() -> Supplies {
    Supplies::scan(std::path::Path::new(POWER_SUPPLY)).unwrap_or_default()
}
