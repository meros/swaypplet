//! The lock screen's half of the compositor cross-fade.
//!
//! swayfx' session-lock backdrop is a black rect under the lock surface. When
//! `lock_fade on` has been armed it is created one code value from
//! transparent and steps to fully opaque the moment this client's
//! `wp_alpha_modifier_v1` multiplier reaches 1.0, which is also when the
//! compositor sends `locked` (patches/swayfx-lock-crossfade.patch). So the
//! whole visual transition is one number: the surface multiplier, ramped
//! here. The composite is `(1-m)·desktop + m·lock`, weights summing to one at
//! every value of m, which is why the backdrop must not ramp alongside it.
//!
//! Two rules make this safe rather than merely pretty.
//!
//! **Never commit.** A lock surface may not be committed without a buffer,
//! and may not be committed with a buffer whose size disagrees with the last
//! acked configure; both are protocol errors that kill the locker and paint
//! the screen red. So every value is set as pending state and a GTK frame is
//! requested; GTK's own commit carries it, at the right size, atomically.
//!
//! **Never trust the frame clock for progress.** The ramp's clock is a plain
//! glib timeout, so an output that is powered off (the lid path stalls GTK's
//! frame callbacks entirely) can stall the *picture* but never the state
//! machine. The exit always reaches its callback.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk4::glib;
use gtk4::prelude::*;

use crate::alpha::SurfaceAlpha;
use crate::anim::{ENTER_MS, EXIT_MS, duration};

/// Ramp tick. Faster than any refresh rate this machine has, so the value
/// GTK picks up when it paints is at most one tick stale.
const TICK: Duration = Duration::from_millis(8);

/// If a window never reports a first paint, start the ramp anyway. Shorter
/// than the compositor's 250 ms first-frame deadline so the client wins.
const FIRST_PAINT_FALLBACK: Duration = Duration::from_millis(150);

/// If a requested frame never arrives (outputs off), stop waiting for it.
const PAINT_FALLBACK: Duration = Duration::from_millis(100);

/// The exit parks here before the visible ramp starts. Dropping below 1.0 is
/// what makes the compositor stop culling the desktop
/// (`scene_node_opaque_region` returns nothing for a buffer at opacity < 1),
/// and the desktop's clients need a full `frame_done` → render → commit →
/// vblank round trip to replace the buffers they last drew before the lock.
/// At 0.999 their contribution is 0.26 of one 8-bit code value, so that round
/// trip happens inside frames nobody can see.
const WAKE_ALPHA: f64 = 0.999;
/// `frame_done` is sent at the end of `handle_frame`, so the round trip is
/// two output frames minimum and three with any render time at all.
const WAKE_HOLD_FRAMES: f64 = 4.0;
const WAKE_HOLD_MIN_MS: f64 = 67.0;
const WAKE_HOLD_MAX_MS: f64 = 134.0;

/// The entrance parks at 0 for this long before it starts moving.
///
/// The first frame a lock surface paints is the most expensive one this
/// process ever renders — GSK uploads the wallpaper, rasterises the clock's
/// glyphs and builds the blur pipeline — and the compositor takes on a new
/// full-screen surface on the same beat. So the frames right after it are the
/// ones most likely to arrive late, and stepping the multiplier across them
/// spends the curve's largest deltas exactly where the cadence is worst,
/// which is what the eye reads as stutter. The hold is invisible (the surface
/// is transparent throughout) and costs only T0.
const WARMUP_FRAMES: f64 = 2.0;
const WARMUP_MIN_MS: f64 = 24.0;
const WARMUP_MAX_MS: f64 = 40.0;

/// A callback that runs at most once, boxed so it can be taken out of a cell.
type OnceCb = Box<dyn FnOnce()>;

struct Entry {
    window: gtk4::Window,
    alpha: SurfaceAlpha,
}

pub struct LockFade {
    enabled: Cell<bool>,
    value: Cell<f64>,
    surfaces: RefCell<Vec<Entry>>,
    ramp: RefCell<Option<glib::SourceId>>,
    entered: Cell<bool>,
    settled: Cell<bool>,
    on_settled: RefCell<Vec<OnceCb>>,
    pending_paints: Cell<usize>,
    armed: Cell<bool>,
    ticker: RefCell<Option<Rc<dyn Fn()>>>,
}

impl LockFade {
    /// Decide once, before any surface exists.
    ///
    /// `SWAYPPLET_LOCK_FADE=0` is set by the supervisor on the paths where a
    /// fade is wrong (see locker.rs); reduced motion skips it because
    /// collapsing a fade to `duration()`'s 1 ms would present exactly one
    /// see-through frame of an already-locked session; and the `lock_fade`
    /// IPC both tells the compositor to start the backdrop see-through and
    /// acts as the version gate — swaypplet is rebuilt and restarted in place
    /// while swayfx only changes on session restart, so an unpatched
    /// compositor answering "Unknown/invalid command" is the normal state for
    /// hours after a rebuild and must degrade to a hard cut, not to a lock
    /// screen fading up out of black.
    pub fn new() -> Rc<Self> {
        let enabled = !crate::lock::fade_suppressed()
            && crate::anim::animations_enabled()
            && crate::alpha::preload()
            && arm_compositor();
        if !enabled {
            log::info!("lock: cross-fade off, cutting to the lock screen");
        }
        Rc::new(LockFade {
            enabled: Cell::new(enabled),
            value: Cell::new(if enabled { 0.0 } else { 1.0 }),
            surfaces: RefCell::new(Vec::new()),
            ramp: RefCell::new(None),
            entered: Cell::new(false),
            settled: Cell::new(false),
            on_settled: RefCell::new(Vec::new()),
            pending_paints: Cell::new(0),
            armed: Cell::new(false),
            ticker: RefCell::new(None),
        })
    }

    pub fn enabled(&self) -> bool {
        self.enabled.get()
    }

    /// The thing that makes each ramp value actually reach the compositor.
    ///
    /// `queue_draw` asks GTK for a frame; GTK gives one only when the render
    /// node tree differs from the last, and a lock screen at rest is
    /// pixel-identical frame to frame. So the surfaces hand over a way to
    /// change one pixel (`SurfaceSet::pulse`), and every broadcast calls it
    /// before asking for the frame that carries the multiplier.
    pub fn set_ticker(&self, f: Rc<dyn Fn()>) {
        *self.ticker.borrow_mut() = Some(f);
    }

    /// Hook one lock window. Must run before `assign_window_to_monitor`,
    /// which presents (and therefore realizes) the window itself.
    pub fn bind(self: &Rc<Self>, window: &gtk4::Window) {
        if !self.enabled.get() {
            return;
        }
        let fade = self.clone();
        window.connect_realize(move |w| fade.attach(w));
        let fade = self.clone();
        window.connect_unrealize(move |w| fade.release(w));

        // A monitor hotplugged after the ramp started attaches at the current
        // value and never gates anything.
        if self.entered.get() {
            return;
        }
        self.pending_paints.set(self.pending_paints.get() + 1);
        let fade = self.clone();
        window.connect_map(move |w| fade.watch_first_paint(w));
    }

    /// `::realize` is the earliest hook there is: the `wl_surface` exists and
    /// no role object does (gtk4-session-lock creates the lock surface during
    /// `present`, i.e. at map). The multiplier set here is pending compositor
    /// state and promotes atomically with GTK's first buffer, so there is no
    /// window in which an opaque frame can slip out ahead of it.
    fn attach(&self, window: &gtk4::Window) {
        if !self.enabled.get() {
            return;
        }
        // wp_alpha_modifier_v1.get_surface on a surface that already has one
        // is the fatal `already_constructed` error.
        if self.surfaces.borrow().iter().any(|e| &e.window == window) {
            return;
        }
        let Some(alpha) = SurfaceAlpha::attach(window) else {
            // preload() said the global was there; if the per-surface bind
            // still failed, nobody fades, or the outputs would disagree.
            self.enabled.set(false);
            self.broadcast(1.0);
            self.settle();
            return;
        };
        alpha.set_pending(self.value.get());
        self.surfaces.borrow_mut().push(Entry {
            window: window.clone(),
            alpha,
        });
    }

    /// `::unrealize` is RUN_LAST, so this runs before `gtk_window_unrealize`
    /// destroys the `wl_surface`. Dropping the handle any later sends
    /// `wp_alpha_modifier_surface_v1.destroy` on a freed proxy.
    fn release(&self, window: &gtk4::Window) {
        self.surfaces.borrow_mut().retain(|e| &e.window != window);
    }

    /// `::map` is emitted before `get_lock_surface`, before the initial
    /// configure roundtrip and before GSK's first render, so a ramp started
    /// there lands its first *visible* value mid-curve — on a cold locker
    /// process, at 0.7 or higher. `after-paint` fires once GDK has attached
    /// and committed a buffer, which is the first multiplier the compositor
    /// can act on, and it is always exactly 0.
    fn watch_first_paint(self: &Rc<Self>, window: &gtk4::Window) {
        let Some(clock) = window.frame_clock() else {
            self.first_paint_done();
            return;
        };
        let fade = self.clone();
        let id: Rc<RefCell<Option<glib::SignalHandlerId>>> = Rc::new(RefCell::new(None));
        let id2 = id.clone();
        *id.borrow_mut() = Some(clock.connect_after_paint(move |c| {
            if let Some(h) = id2.borrow_mut().take() {
                c.disconnect(h);
            }
            fade.first_paint_done();
        }));
    }

    /// All monitors leave 0 together. GTK's `::map` order and per-window GSK
    /// render times differ (4K external vs 1080p internal), and a shared ramp
    /// clock started on the first one makes the last one step in mid-curve.
    fn first_paint_done(self: &Rc<Self>) {
        let left = self.pending_paints.get().saturating_sub(1);
        self.pending_paints.set(left);
        if left == 0 {
            self.begin_enter();
        }
    }

    /// Call once, right after `instance.lock()`. A window that never paints
    /// must not hold the fade past the compositor's own deadline.
    pub fn arm_fallback(self: &Rc<Self>) {
        if !self.enabled.get() || self.armed.replace(true) {
            return;
        }
        let fade = self.clone();
        glib::timeout_add_local_once(FIRST_PAINT_FALLBACK, move || {
            if !fade.entered.get() {
                log::info!("lock: no first paint in {FIRST_PAINT_FALLBACK:?}, starting the fade");
                fade.pending_paints.set(0);
                fade.begin_enter();
            }
        });
    }

    /// Run `cb` once the entrance is over, i.e. once the surface is opaque
    /// and a layout change can no longer be seen through it.
    ///
    /// Everything that arrives from a worker while the lock screen is
    /// see-through has to come through here. A card that grows mid-ramp is a
    /// jump the fade cannot hide, and the work behind it (a chip rebuild, an
    /// avatar decode) lands on the frames with the least slack, so a late
    /// answer costs both the layout and the cadence.
    pub fn on_settled(self: &Rc<Self>, cb: impl FnOnce() + 'static) {
        if !self.enabled.get() || self.settled.get() {
            cb();
            return;
        }
        self.on_settled.borrow_mut().push(Box::new(cb));
    }

    /// Idempotent, and never skipped: the ramp runs on a wall clock and
    /// `arm_fallback` guarantees it starts, so anything queued here runs even
    /// on the paths where no frame is ever painted.
    fn settle(&self) {
        if self.settled.replace(true) {
            return;
        }
        let queued: Vec<OnceCb> = self.on_settled.borrow_mut().drain(..).collect();
        for cb in queued {
            cb();
        }
    }

    fn broadcast(&self, value: f64) {
        let value = value.clamp(0.0, 1.0);
        self.value.set(value);
        // Before the multiplier, so the pixel is already dirty when GTK
        // decides whether this frame is worth drawing.
        let ticker = self.ticker.borrow().clone();
        if let Some(tick) = ticker {
            tick();
        }
        for e in self.surfaces.borrow().iter() {
            e.alpha.set_pending(value);
            // The multiplier only reaches the compositor on this surface's
            // next commit, and GTK only commits when it paints. Ask for a
            // frame instead of committing ourselves — see the module docs.
            e.window.queue_draw();
        }
    }

    fn cancel(&self) {
        if let Some(id) = self.ramp.borrow_mut().take() {
            id.remove();
        }
    }

    /// Run `cb` after the next frame any lock window paints, so the value
    /// just broadcast is known to have been committed. Falls back on a wall
    /// clock, because a powered-off output produces no frames at all.
    fn after_paint_once(self: &Rc<Self>, cb: impl FnOnce() + 'static) {
        let done = Rc::new(Cell::new(false));
        let cell: Rc<RefCell<Option<OnceCb>>> = Rc::new(RefCell::new(Some(Box::new(cb))));

        let fire = {
            let done = done.clone();
            let cell = cell.clone();
            move || {
                if done.replace(true) {
                    return;
                }
                if let Some(cb) = cell.borrow_mut().take() {
                    cb();
                }
            }
        };

        let clock = self
            .surfaces
            .borrow()
            .first()
            .and_then(|e| e.window.frame_clock());
        if let Some(clock) = clock {
            let fire = fire.clone();
            let id: Rc<RefCell<Option<glib::SignalHandlerId>>> = Rc::new(RefCell::new(None));
            let id2 = id.clone();
            *id.borrow_mut() = Some(clock.connect_after_paint(move |c| {
                if let Some(h) = id2.borrow_mut().take() {
                    c.disconnect(h);
                }
                fire();
            }));
        }
        glib::timeout_add_local_once(PAINT_FALLBACK, fire);
    }

    /// Idempotent; the first caller wins.
    pub fn begin_enter(self: &Rc<Self>) {
        if !self.enabled.get() || self.entered.replace(true) {
            return;
        }
        let hold = self.frames_ms(WARMUP_FRAMES, WARMUP_MIN_MS, WARMUP_MAX_MS);
        let span = duration(ENTER_MS);
        let start = Instant::now();
        let cadence = RefCell::new(Some(Cadence::start(self)));
        let this = self.clone();
        let id = glib::timeout_add_local(TICK, move || {
            let ms = start.elapsed().as_secs_f64() * 1000.0;
            if ms < hold {
                return glib::ControlFlow::Continue;
            }
            let t = ((ms - hold) / span).min(1.0);
            // Smoothstep, for the same reason the exit uses it rather than an
            // ease: an ease-out spends its biggest step on the very first
            // frame of the ramp, which on the way in is the frame right
            // behind the surface's cold first paint. Smoothstep(1.0) is
            // exactly 1.0, which is what promotes the backdrop to CONFIRMED
            // and releases `locked`.
            this.broadcast(t * t * (3.0 - 2.0 * t));
            if t >= 1.0 {
                *this.ramp.borrow_mut() = None;
                if let Some(c) = cadence.borrow_mut().take() {
                    c.finish("enter");
                }
                this.settle();
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
        *self.ramp.borrow_mut() = Some(id);
    }

    /// Cut to full opacity now.
    ///
    /// Authentication can land before the compositor has confirmed the lock,
    /// and `gtk_session_lock_instance_unlock` is a silent no-op until it has.
    /// `locked` follows the first commit at 1.0, so this turns a ~150 ms
    /// stall behind a finished success flash into about two frames.
    pub fn rush_to_opaque(self: &Rc<Self>) {
        if !self.enabled.get() {
            return;
        }
        self.cancel();
        self.entered.set(true);
        self.broadcast(1.0);
        self.settle();
    }

    /// Ramp out, then run `then` (always `instance.unlock()`).
    pub fn exit(self: &Rc<Self>, then: impl FnOnce() + 'static) {
        if !self.enabled.get() {
            then();
            return;
        }
        self.cancel();
        // Exactly 1.0, and *committed*, before anything else. A backdrop the
        // compositor pinned opaque (fade deadline, abandoned-lock relaunch)
        // hands itself back only on a commit at 1.0, and while it is pinned
        // the multiplier is ignored, so without this the fade-out is silently
        // a hard cut. Pixel-wise a no-op: the surface was already opaque.
        // It has to be its own commit — a second pending value in the same
        // frame would overwrite it before GTK ever sent it.
        self.broadcast(1.0);
        let this = self.clone();
        self.after_paint_once(move || this.start_exit_ramp(then));
    }

    fn start_exit_ramp(self: &Rc<Self>, then: impl FnOnce() + 'static) {
        self.broadcast(WAKE_ALPHA);

        let hold = self.frames_ms(WAKE_HOLD_FRAMES, WAKE_HOLD_MIN_MS, WAKE_HOLD_MAX_MS);
        let span = duration(EXIT_MS);
        let start = Instant::now();
        let cadence = RefCell::new(Some(Cadence::start(self)));
        let this = self.clone();
        let done: RefCell<Option<Box<dyn FnOnce()>>> = RefCell::new(Some(Box::new(then)));

        let id = glib::timeout_add_local(TICK, move || {
            let ms = start.elapsed().as_secs_f64() * 1000.0;
            if ms < hold {
                return glib::ControlFlow::Continue;
            }
            let t = ((ms - hold) / span).min(1.0);
            // Smoothstep on the *desktop's* weight rather than ease-out on
            // the lock screen's. Ease-out spends its biggest step first,
            // which is exactly the frame where a slow desktop client is still
            // holding the buffer it drew before the lock.
            let desktop = t * t * (3.0 - 2.0 * t);
            this.broadcast(WAKE_ALPHA * (1.0 - desktop));
            if t >= 1.0 {
                this.broadcast(0.0);
                *this.ramp.borrow_mut() = None;
                if let Some(c) = cadence.borrow_mut().take() {
                    c.finish("exit");
                }
                let cb = done.borrow_mut().take();
                // One more frame, so 0.0 rides a real commit. At 0.0 with a
                // see-through backdrop, destroying the lock surfaces is then
                // a pixel-exact no-op.
                this.after_paint_once(move || {
                    if let Some(cb) = cb {
                        cb();
                    }
                });
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
        *self.ramp.borrow_mut() = Some(id);
    }

    /// `frames` refresh periods of the slowest attached monitor, clamped.
    /// Not routed through `anim::duration`: these are vblank counts, not
    /// animations, and they do not stretch with the debug scale.
    fn frames_ms(&self, frames: f64, min: f64, max: f64) -> f64 {
        let mut slowest_mhz = 60_000;
        if let Some(display) = gdk4::Display::default() {
            let monitors = display.monitors();
            for i in 0..monitors.n_items() {
                if let Some(m) = monitors.item(i).and_downcast::<gdk4::Monitor>() {
                    let r = m.refresh_rate();
                    if r > 0 && r < slowest_mhz {
                        slowest_mhz = r;
                    }
                }
            }
        }
        let frame_ms = 1_000_000.0 / f64::from(slowest_mhz);
        (frames * frame_ms).clamp(min, max)
    }
}

/// Frame-clock cadence over one ramp, reported as a single line when it ends.
///
/// One summary rather than a line per frame: enough to tell a dropped frame
/// from an even ramp, cheap enough to leave on.
///
/// Read it for what it is. `after-paint` marks a frame-clock cycle, and GTK
/// runs the cycle whether or not it ends up drawing anything — an entrance
/// that logged 20 frames at a 16 ms median had committed 11 of them, because
/// nothing on screen had changed (`SurfaceSet::pulse`). With the commit pixel
/// dirtied on every broadcast the two are the same number again, which is the
/// only condition under which this line describes what the eye sees.
struct Cadence {
    clock: Option<gdk4::FrameClock>,
    handler: Option<glib::SignalHandlerId>,
    gaps: Rc<RefCell<Vec<f64>>>,
}

impl Cadence {
    fn start(fade: &LockFade) -> Cadence {
        let gaps: Rc<RefCell<Vec<f64>>> = Rc::new(RefCell::new(Vec::new()));
        let clock = fade
            .surfaces
            .borrow()
            .first()
            .and_then(|e| e.window.frame_clock());
        let handler = clock.as_ref().map(|clock| {
            let gaps = gaps.clone();
            let last = Cell::new(Instant::now());
            clock.connect_after_paint(move |_| {
                let now = Instant::now();
                gaps.borrow_mut()
                    .push(now.duration_since(last.get()).as_secs_f64() * 1000.0);
                last.set(now);
            })
        });
        Cadence {
            clock,
            handler,
            gaps,
        }
    }

    fn finish(mut self, what: &str) {
        if let (Some(clock), Some(id)) = (self.clock.take(), self.handler.take()) {
            clock.disconnect(id);
        }
        let mut gaps = self.gaps.borrow().clone();
        if gaps.is_empty() {
            log::info!("lock: {what} fade painted no frames");
            return;
        }
        // The first interval runs from the ramp being armed to the first
        // frame after it, which on the way in spans the warm-up hold. It is
        // not a paint interval and would be reported as the worst one.
        if gaps.len() > 1 {
            gaps.remove(0);
        }
        gaps.sort_by(f64::total_cmp);
        let median = gaps[gaps.len() / 2];
        let worst = gaps[gaps.len() - 1];
        log::info!(
            "lock: {what} fade {} frames, median {median:.0} ms, worst {worst:.0} ms",
            gaps.len()
        );
    }
}

/// Tell the compositor to start this lock's backdrops see-through, and how
/// long to wait for us before giving up.
///
/// One-shot and expiring on the compositor side, so a locker that arms and
/// then dies cannot leave the next one see-through. The deadline scales with
/// `SWAYPPLET_ANIM_SCALE`, which is what makes
/// `SWAYPPLET_ANIM_SCALE=20 swaypplet lock` a usable way to look at frames —
/// the compositor stretches with the client, no second variable and no
/// session restart.
fn arm_compositor() -> bool {
    let deadline = (duration(ENTER_MS) * 3.0 + 400.0).clamp(800.0, 60_000.0) as i64;
    let result = swayipc::Connection::new()
        .and_then(|mut c| c.run_command(format!("lock_fade on {deadline}")));
    match result {
        Ok(outcome) => {
            let ok = !outcome.is_empty() && outcome.iter().all(|r| r.is_ok());
            if !ok {
                for r in &outcome {
                    if let Err(e) = r {
                        log::info!("lock: lock_fade rejected: {e}");
                    }
                }
                log::info!("lock: compositor has no lock_fade; cutting");
            }
            ok
        }
        Err(e) => {
            log::info!("lock: sway IPC unavailable ({e}); cutting");
            false
        }
    }
}
