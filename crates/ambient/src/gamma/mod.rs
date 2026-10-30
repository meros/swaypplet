//! The night light, in the panel process (docs/SETTINGS.md, `night_light`).
//!
//! Replaces gammastep, which computed the sun a second time from the same
//! coordinates the theme reads. Now one sun (`theme::sun`) drives both the
//! automatic mode and the colour temperature, so the screen warms and the
//! theme darkens against the same horizon.
//!
//! # When it wakes
//!
//! Nothing ticks at a steady temperature. With `schedule: sun` a timer asks
//! the sun once a minute; outside twilight the answer does not move and
//! nothing else happens. When the target moves, the shown temperature ramps
//! to it: over a minute for the sun (the next minute's step picks up where
//! this one ends, so twilight is one continuous slide), over a second for a
//! change in the settings, over two at startup. A ramp steps only as often
//! as it moves half a mired, and never faster than 10 Hz, so a twilight
//! minute is a handful of table writes and a slider drag is ten a second.

mod color;
mod wayland;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use swaypplet_model::{
    store,
    store::{NightLight, NightSchedule},
};

/// The ramp for a change made in the settings or the panel.
const MANUAL: Duration = Duration::from_secs(1);
/// The ramp at startup, from the compositor's own tables.
const STARTUP: Duration = Duration::from_secs(2);
/// The sun is asked this often, and each answer ramps over the same time.
const SUN_PERIOD: Duration = Duration::from_secs(60);
/// A ramp writes a table when it has moved this far, in mired.
const STEP_MIRED: f64 = 0.5;
/// And never more often than this.
const MIN_STEP: Duration = Duration::from_millis(100);

/// Mirror of `night_light.enabled` for the tile's state reader, which runs
/// off the GTK thread where the settings do not live.
static ENABLED: AtomicBool = AtomicBool::new(false);

/// Whether the compositor gave the night light its tables.
pub fn available() -> bool {
    wayland::available()
}

/// Whether the night light is switched on. Any thread.
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Switch it on or off, from any thread: the tile's action runs on a
/// worker, and the settings live on the GTK thread.
pub fn set_enabled(on: bool) {
    glib::MainContext::default().invoke(move || {
        store::edit::<NightLight>(|n| n.enabled = on);
    });
}

struct Ramp {
    from: f64,
    to: f64,
    start: Instant,
    duration: Duration,
}

impl Ramp {
    /// Where the ramp is `elapsed` after its start, in mired, and whether
    /// it has arrived. Eased at both ends, so a change starts and settles
    /// softly.
    fn at(&self, elapsed: Duration) -> (f64, bool) {
        let t = (elapsed.as_secs_f64() / self.duration.as_secs_f64()).min(1.0);
        let e = t * t * (3.0 - 2.0 * t);
        (self.from + (self.to - self.from) * e, t >= 1.0)
    }
}

/// How often a ramp over `mired` in `duration` steps: as often as it moves
/// [`STEP_MIRED`], never faster than [`MIN_STEP`] and never slower than the
/// whole ramp. A slow sun minute ticks every few seconds, a manual change
/// every 100 ms.
fn tick_every(mired: f64, duration: Duration) -> Duration {
    let steps = (mired.abs() / STEP_MIRED).max(1.0);
    duration
        .div_f64(steps)
        .clamp(MIN_STEP, duration.max(MIN_STEP))
}

struct Engine {
    gamma: wayland::Gamma,
    /// What is on screen, in kelvin.
    shown: Cell<f64>,
    /// Where the running ramp started and ends, in mired.
    ramp: RefCell<Option<Ramp>>,
    tick: RefCell<Option<glib::SourceId>>,
    minute: RefCell<Option<glib::SourceId>>,
    settings: Cell<NightLight>,
}

impl Engine {
    fn target(&self) -> f64 {
        let settings = self.settings.get();
        let elevation = match settings.schedule {
            NightSchedule::Sun => swaypplet_ui::theme::sun::daylight_now(),
            NightSchedule::Always => None,
        };
        color::target_kelvin(settings, elevation)
    }

    /// Ramp from what is shown to the target over `duration`.
    fn retarget(self: &Rc<Self>, duration: Duration) {
        let (from, to) = (color::mired(self.shown.get()), color::mired(self.target()));
        if let Some(id) = self.tick.borrow_mut().take() {
            swaypplet_base::spawn::remove_source(id);
        }
        if (to - from).abs() < STEP_MIRED / 2.0 {
            *self.ramp.borrow_mut() = None;
            return;
        }
        *self.ramp.borrow_mut() = Some(Ramp {
            from,
            to,
            start: Instant::now(),
            duration,
        });
        let every = tick_every(to - from, duration);
        let this = self.clone();
        let id = glib::timeout_add_local(every, move || this.step());
        *self.tick.borrow_mut() = Some(id);
    }

    fn step(&self) -> glib::ControlFlow {
        let (m, done) = {
            let ramp = self.ramp.borrow();
            let Some(r) = ramp.as_ref() else {
                return glib::ControlFlow::Break;
            };
            r.at(r.start.elapsed())
        };
        self.show(1e6 / m);
        if done {
            *self.ramp.borrow_mut() = None;
            self.tick.borrow_mut().take();
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    }

    fn show(&self, kelvin: f64) {
        log::debug!("night light: {kelvin:.0} K");
        self.shown.set(kelvin);
        self.gamma.set(kelvin);
    }

    /// The minute timer runs only while the sun decides.
    fn follow_sun(self: &Rc<Self>) {
        let wanted = {
            let s = self.settings.get();
            s.enabled && s.schedule == NightSchedule::Sun
        };
        let running = self.minute.borrow().is_some();
        if wanted == running {
            return;
        }
        if let Some(id) = self.minute.borrow_mut().take() {
            swaypplet_base::spawn::remove_source(id);
        }
        if wanted {
            let this = self.clone();
            let id = glib::timeout_add_local(SUN_PERIOD, move || {
                this.retarget(SUN_PERIOD);
                glib::ControlFlow::Continue
            });
            *self.minute.borrow_mut() = Some(id);
        }
    }
}

/// Take the gamma tables and follow the settings and the sun for as long as
/// the process lives. The panel calls this; nothing else does, because the
/// compositor gives each output's tables to one client.
pub fn follow_settings() {
    let settings = store::current().night_light();
    ENABLED.store(settings.enabled, Ordering::Relaxed);
    let Some(gamma) = wayland::Gamma::start() else {
        return;
    };
    let engine = Rc::new(Engine {
        gamma,
        shown: Cell::new(f64::from(NightLight::DAY_K)),
        ramp: RefCell::new(None),
        tick: RefCell::new(None),
        minute: RefCell::new(None),
        settings: Cell::new(settings),
    });
    engine.retarget(STARTUP);
    engine.follow_sun();
    store::observe_section(
        |s| s.night_light(),
        move |now| {
            ENABLED.store(now.enabled, Ordering::Relaxed);
            engine.settings.set(now);
            engine.retarget(MANUAL);
            engine.follow_sun();
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(from: f64, to: f64, secs: u64) -> Ramp {
        Ramp {
            from,
            to,
            start: Instant::now(),
            duration: Duration::from_secs(secs),
        }
    }

    #[test]
    fn a_ramp_eases_from_start_to_end_and_then_holds() {
        let r = ramp(200.0, 300.0, 2);
        assert_eq!(r.at(Duration::ZERO), (200.0, false));
        assert_eq!(r.at(Duration::from_secs(1)), (250.0, false), "halfway");
        assert_eq!(r.at(Duration::from_secs(2)), (300.0, true));
        assert_eq!(r.at(Duration::from_secs(9)), (300.0, true));
        // Eased: the first tenth covers less than a tenth of the way.
        let (early, _) = r.at(Duration::from_millis(200));
        assert!(early > 200.0 && early < 210.0, "{early}");
    }

    #[test]
    fn a_ramp_ticks_once_per_half_mired_within_its_bounds() {
        // A manual change over a second: never faster than 10 Hz.
        assert_eq!(tick_every(150.0, MANUAL), MIN_STEP);
        // A slow sun minute moving 6 mired: twelve steps, every 5 s.
        assert_eq!(tick_every(-6.0, SUN_PERIOD), Duration::from_secs(5));
        // Less than a step is one step, over the whole ramp.
        assert_eq!(tick_every(0.1, SUN_PERIOD), SUN_PERIOD);
        // A ramp shorter than the floor still ticks at the floor.
        assert_eq!(tick_every(0.1, Duration::from_millis(20)), MIN_STEP);
    }
}
