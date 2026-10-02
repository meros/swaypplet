//! The black curtain the animated wallpaper fades behind on battery
//! (`services::wallpaper_battery`'s `black`).
//!
//! The fade used to be the player's own: contrast and saturation eased over
//! mpv's IPC. mpv applies those only when it renders a video frame, which is
//! 15 a second at the wallpaper's half speed and fewer as it slows to a
//! stop, so the fade stepped with the video however often it was written.
//! A fade needs a new value on every refresh of the output, and the one
//! clock that ticks at that rate on this side of the socket is GTK's.
//!
//! So the fade is a layer surface of its own: one per output, plain black,
//! on the bottom layer with mpvpaper (`-l bottom`), mapped after it and so
//! stacked above it, and below every window. It covers the whole output,
//! bar included, takes no keyboard and has an empty input region, so the
//! pointer reaches the desktop and the windows through it. Its opacity is
//! eased on each output's own frame clock from one start time on the
//! monotonic clock, so a 60 Hz panel and a 144 Hz monitor draw the same
//! curve, each at its own rate. Look → Motion and reduced motion reach it
//! through `anim::duration`: with motion off it cuts.
//!
//! Fully transparent, it is unmapped: nothing of it is on screen, and
//! nothing ticks, in normal operation. It is lifted as soon as the picture
//! under it is black (mpvpaper drawing black with no video track), so it is
//! only ever up during a transition.
//!
//! The worker thread drives it ([`Cmd`]) and is told when the curtain is up
//! is on screen; everything here runs on the GTK thread.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use gtk4::{gdk, glib, graphene};
use gtk4_layer_shell::Layer;

use crate::shell::{Namespace, PerMonitor, Surface};

/// The curtain's level when it is up: black to the eye (the beach loop
/// under it measures a mean of 0.7 of 255, against 0 for black), and not
/// opaque to the compositor. A sheet that draws alpha 255 hides what is
/// under it from sway, which then stops sending it frame callbacks:
/// mpvpaper cannot draw the black that goes under the curtain, and the
/// stale video frame it drew last shows for one refresh when the curtain
/// lifts (measured in a nested sway: one frame at grey 148 in the middle of
/// black at 0.999, none at 0.996 or 0.9). The window keeps the layer
/// surfaces' near-unity opacity for the same reason (no `opaque()`).
const UP: f64 = 0.996;

/// What the worker asks of the curtain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmd {
    /// Fade up ([`UP`]) from wherever it is; say when it gets there.
    In,
    /// Up at once (the picture under it is black already); say when it
    /// is on screen.
    Cover,
    /// Fade away from wherever it is.
    Out,
    /// Gone at once: the picture under it is black.
    Lift,
}

/// One leg of the curtain's opacity on the monotonic clock (µs), eased
/// both ends.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Run {
    from: f64,
    to: f64,
    t0: i64,
    span: i64,
}

impl Run {
    const CLEAR: Run = Run {
        from: 0.0,
        to: 0.0,
        t0: 0,
        span: 0,
    };

    /// From where `self` is at `now` to `to`, taking `full` µs for the whole
    /// way and its share of that for less: a fade turned around midway goes
    /// back the distance it came, at the same pace.
    fn toward(&self, now: i64, to: f64, full: i64) -> Run {
        let from = self.at(now);
        Run {
            from,
            to,
            t0: now,
            span: (full as f64 * (to - from).abs()).round() as i64,
        }
    }

    fn at(&self, now: i64) -> f64 {
        if self.ended(now) {
            return self.to;
        }
        let t = ((now - self.t0) as f64 / self.span as f64).clamp(0.0, 1.0);
        self.from + (self.to - self.from) * t * t * (3.0 - 2.0 * t)
    }

    fn ended(&self, now: i64) -> bool {
        self.span <= 0 || now >= self.t0 + self.span
    }

    /// Whether anything of it shows, now or on the way.
    fn shows(&self) -> bool {
        self.from > 0.0 || self.to > 0.0
    }
}

/// The curtain on every output.
#[derive(Clone)]
pub struct Curtain(Rc<Inner>);

struct Inner {
    sheets: Rc<PerMonitor<Sheet>>,
    run: Cell<Run>,
    /// The current run has reached its end and been put down.
    landed: Cell<bool>,
    /// Called once the current run lands.
    done: RefCell<Option<Box<dyn FnOnce()>>>,
    /// Lands the run if no frame clock does: an output that is off ticks
    /// nothing, and the worker must not wait on it.
    guard: RefCell<Option<glib::SourceId>>,
    /// The whole fade in ms, before `anim::duration`, which is applied per
    /// fade so a Motion change lands on the next one.
    full_ms: f64,
}

/// One output's curtain.
struct Sheet {
    surface: Surface,
    black: Black,
    /// A tick callback is registered on the window.
    ticking: Rc<Cell<bool>>,
}

impl Curtain {
    /// One sheet per output, following hotplug, none of them mapped.
    /// `full_ms` is the length of a fade the whole way.
    pub fn new(app: &gtk4::Application, full_ms: f64) -> Curtain {
        let inner = Rc::new(Inner {
            sheets: PerMonitor::new(),
            run: Cell::new(Run::CLEAR),
            landed: Cell::new(true),
            done: RefCell::new(None),
            guard: RefCell::new(None),
            full_ms,
        });
        let weak = Rc::downgrade(&inner);
        let app = app.clone();
        inner.sheets.watch(move |monitor| {
            let sheet = Sheet::new(&app, monitor);
            // A monitor plugged in mid-fade joins it where it is.
            if let Some(inner) = weak.upgrade()
                && inner.run.get().shows()
                && !(inner.landed.get() && inner.run.get().to <= 0.0)
            {
                inner.show(&sheet, glib::monotonic_time());
            }
            sheet
        });
        Curtain(inner)
    }

    /// Carry out what the worker asked; `done` runs once the curtain is
    /// where it was asked to be (`In`, `Cover`), and right away for the
    /// others.
    pub fn apply(&self, cmd: Cmd, done: impl FnOnce() + 'static) {
        let inner = &self.0;
        let now = glib::monotonic_time();
        let full = (crate::anim::duration(inner.full_ms) * 1000.0).round() as i64;
        let run = match cmd {
            Cmd::In => inner.run.get().toward(now, UP, full),
            Cmd::Out => inner.run.get().toward(now, 0.0, full),
            Cmd::Cover => Run::CLEAR.toward(now, UP, 0),
            Cmd::Lift => Run::CLEAR,
        };
        log::debug!(
            "curtain: {cmd:?} from {:.3} over {} ms",
            run.from,
            run.span / 1000
        );
        let done: Box<dyn FnOnce()> = match cmd {
            Cmd::In | Cmd::Cover => Box::new(done),
            Cmd::Out | Cmd::Lift => {
                done();
                Box::new(|| {})
            }
        };
        Inner::start(inner, run, now, done);
    }
}

impl Inner {
    fn start(this: &Rc<Inner>, run: Run, now: i64, done: Box<dyn FnOnce()>) {
        if let Some(id) = this.guard.borrow_mut().take() {
            crate::spawn::remove_source(id);
        }
        this.run.set(run);
        this.landed.set(false);
        // A run cut off by this one never finishes: its waiter is the
        // worker's previous transition, which has moved on.
        *this.done.borrow_mut() = Some(done);
        if this.sheets.is_empty() || !run.shows() {
            this.land();
            return;
        }
        this.sheets.for_each(|sheet| this.show(sheet, now));
        let weak = Rc::downgrade(this);
        let after = Duration::from_micros(run.span.max(0) as u64) + Duration::from_millis(250);
        *this.guard.borrow_mut() = Some(glib::timeout_add_local_once(after, move || {
            if let Some(inner) = weak.upgrade() {
                inner.guard.borrow_mut().take();
                inner.land();
            }
        }));
    }

    /// Map `sheet` at the level of `now` and keep it following the run.
    fn show(self: &Rc<Self>, sheet: &Sheet, now: i64) {
        sheet.black.set_opacity(self.run.get().at(now));
        if !sheet.surface.window().is_visible() {
            sheet.surface.show();
        }
        if sheet.ticking.replace(true) {
            return;
        }
        let weak: Weak<Inner> = Rc::downgrade(self);
        let black = sheet.black.clone();
        let ticking = sheet.ticking.clone();
        sheet.surface.window().add_tick_callback(move |_, clock| {
            let Some(inner) = weak.upgrade() else {
                ticking.set(false);
                return glib::ControlFlow::Break;
            };
            let now = clock.frame_time();
            let run = inner.run.get();
            black.set_opacity(run.at(now));
            if !run.ended(now) {
                return glib::ControlFlow::Continue;
            }
            ticking.set(false);
            inner.land();
            glib::ControlFlow::Break
        });
    }

    /// Put the run down at its end, once: every sheet at its last level,
    /// unmapped if that is clear, and the waiter told.
    fn land(&self) {
        if self.landed.replace(true) {
            return;
        }
        if let Some(id) = self.guard.borrow_mut().take() {
            crate::spawn::remove_source(id);
        }
        let to = self.run.get().to;
        log::debug!("curtain: at {to}");
        self.sheets.for_each(|sheet| {
            sheet.black.set_opacity(to);
            if to <= 0.0 && sheet.surface.window().is_visible() {
                sheet.surface.hide();
            }
        });
        if let Some(done) = self.done.borrow_mut().take() {
            done();
        }
    }
}

impl Sheet {
    fn new(app: &gtk4::Application, monitor: &gdk::Monitor) -> Sheet {
        let surface = Surface::builder(app, Namespace::WallpaperCurtain)
            .monitor(Some(monitor))
            .layer(Layer::Bottom)
            .fill()
            .over_exclusive_zones()
            .no_card()
            .build();
        let black = Black::new();
        black.set_hexpand(true);
        black.set_vexpand(true);
        black.set_can_target(false);
        surface.root().append(&black);
        // Click-through: the region belongs to the GdkSurface, which exists
        // once the window is realized and is sent on the next commit.
        surface.window().connect_map(|window| {
            if let Some(s) = window.surface() {
                s.set_input_region(Some(&gdk::cairo::Region::create()));
            }
        });
        Sheet {
            surface,
            black,
            ticking: Rc::new(Cell::new(false)),
        }
    }
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Black;

    #[glib::object_subclass]
    impl ObjectSubclass for Black {
        const NAME: &'static str = "SwayppletCurtain";
        type Type = super::Black;
        type ParentType = gtk4::Widget;
    }

    impl ObjectImpl for Black {}

    impl WidgetImpl for Black {
        fn snapshot(&self, snapshot: &gtk4::Snapshot) {
            let w = self.obj();
            // A colour node: no texture to upload, whatever the output's
            // size, and the opacity above it is one more node.
            snapshot.append_color(
                &gdk::RGBA::BLACK,
                &graphene::Rect::new(0.0, 0.0, w.width() as f32, w.height() as f32),
            );
        }
    }
}

glib::wrapper! {
    /// Solid black over its whole allocation.
    pub struct Black(ObjectSubclass<imp::Black>)
        @extends gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

impl Black {
    fn new() -> Self {
        glib::Object::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: i64 = 1_500_000;

    #[test]
    fn a_fade_runs_end_to_end_eased_and_every_frame_moves() {
        let run = Run::CLEAR.toward(0, 1.0, FULL);
        assert_eq!(run.span, FULL);
        assert_eq!(run.at(0), 0.0);
        assert_eq!(run.at(FULL), 1.0);
        assert!((run.at(FULL / 2) - 0.5).abs() < 1e-9);
        // A frame every 16.7 ms: each one a new, higher value.
        let mut last = run.at(0);
        for i in 1..=90 {
            let v = run.at(i * 16_667);
            assert!(v > last || v == 1.0, "frame {i} does not move: {v}");
            last = v;
        }
    }

    #[test]
    fn turned_around_it_goes_back_from_where_it_is_for_its_share() {
        let going = Run::CLEAR.toward(0, 1.0, FULL);
        let now = FULL / 2;
        let back = going.toward(now, 0.0, FULL);
        assert!((back.from - 0.5).abs() < 1e-9);
        assert_eq!(back.span, FULL / 2);
        assert_eq!(back.at(now), back.from);
        assert_eq!(back.at(now + back.span), 0.0);
        // Already there: nothing to run.
        let there = Run::CLEAR.toward(0, 1.0, 0).toward(10, 1.0, FULL);
        assert_eq!(there.span, 0);
        assert!(there.ended(10));
    }

    #[test]
    fn clear_shows_nothing_and_a_cut_is_at_its_end_at_once() {
        assert!(!Run::CLEAR.shows());
        let cover = Run::CLEAR.toward(5, 1.0, 0);
        assert!(cover.shows() && cover.ended(5));
        assert_eq!(cover.at(5), 1.0);
    }
}
