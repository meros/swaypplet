//! Shared motion scale for all animated surfaces.
//!
//! The scale, the curves and the reasoning are in `docs/MOTION.md`. This
//! module implements the Rust half; `data/style.css` declares the CSS half,
//! and the two evaluate the same curves so a card driven from here and a
//! label driven from a stylesheet move together.
//!
//! Three curves, from Material 3's standard set: [`standard`] between two
//! on-screen states, [`decelerate`] for anything arriving, [`accelerate`] for
//! anything leaving. Durations: [`EXIT_MS`] and [`ENTER_MS`] (= [`MOVE_MS`]);
//! the micro, emphasis and dwell tiers are CSS-only and live in the
//! stylesheet.
//!
//! # Motion on glass: split the pane from the content
//!
//! swayfx composites layer_effects blur at full strength behind any surface
//! pixel with alpha > 0 — scenefx's `blur_ignore_transparent` is a stencil
//! that discards only pixels with alpha exactly 0 (tex.frag:
//! `if (discard_transparent && gl_FragColor.a == 0.0) discard;`), and blur
//! strength never scales with layer-surface alpha (verified against swayfx
//! 0.5.3; `layer_effects` has no radius/strength either, so there is no
//! compositor-side ramp to drive). The frost is binary. What made the old
//! single crossfade read as a flash was the frost arriving *without its
//! tint*: for most of a 300 ms fade the card was a fully frosted, slightly
//! darkened rectangle carrying almost none of its own color.
//!
//! The recipe, per Apple's material rule (never alpha-fade the effect view;
//! fade what sits on it): the *pane* — the widget whose CSS carries the
//! frosted card background — steps straight to full tint so tint and frost
//! land in the same frame, while the *content* on it fades over the full
//! [`ENTER_MS`]/[`EXIT_MS`]. Exits mirror it: content fades first, and the
//! pane drops from full tint to exactly 0.0 (the stencil threshold) in one
//! frame at the end. Ramping the pane across that boundary is what produced
//! a darkened square, because every intermediate alpha is fully frosted and
//! only partly tinted; [`glass_channel`] carries the argument. [`Reveal`]
//! packages this; surfaces with a directional entrance add a short
//! [`SLIDE_PX`] settle on top. The lock screen's client-side glass
//! (lock/glass.rs) is the one place a true sigma ramp is possible, and does
//! that instead.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4::{glib, graphene, prelude::*, subclass::prelude::*};
use gtk4_layer_shell::LayerShell;

/// Anything leaving. Shorter than an entrance on purpose — waiting for a
/// thing to go is dead time, where an entrance is the thing being waited for.
pub const EXIT_MS: f64 = 200.0;
/// Anything arriving, and anything reflowing between two on-screen states.
pub const ENTER_MS: f64 = 300.0;
pub const MOVE_MS: f64 = 300.0;
// The scale's other three tiers — micro (150ms), emphasis (400ms) and dwell
// (500ms) — have no Rust consumer. Micro is below the threshold where a
// hand-driven tick is worth its wakeups, and the other two are only ever
// keyframes: emphasis lost its last caller when the field's glyph marks went,
// and the tier lives on in data/style.css where the verdict animations are.
// They are declared there rather than sitting in this file as dead constants
// that look authoritative and govern nothing.

/// Travel distance of the settle that accompanies a fade on surfaces with a
/// directional entrance. Short by design — the fade carries the transition,
/// the slide only gives it a direction.
pub const SLIDE_PX: f64 = 24.0;

/// Evaluate a CSS `cubic-bezier(x1, y1, x2, y2)` at `t`.
///
/// The curve is parametric — the CSS control points describe x and y
/// separately — so getting y for a given time means solving x(u) = t for the
/// parameter u first, then evaluating y(u). Newton-Raphson converges in a
/// handful of steps here because x is monotonic for any legal CSS curve, with
/// a bisection fallback for the flat-derivative case (`decelerate` starts with
/// x' = 0, which is exactly where Newton stalls).
///
/// This exists so the Rust-driven surfaces and the stylesheet share curves
/// rather than approximate each other. `ease_out_cubic` used to stand in for
/// all of them, including exits, which meant everything left the screen
/// decelerating — a thing going away should gather speed, not hesitate.
fn cubic_bezier(x1: f64, y1: f64, x2: f64, y2: f64, t: f64) -> f64 {
    if t <= 0.0 {
        return 0.0;
    }
    if t >= 1.0 {
        return 1.0;
    }
    // B(u) for a unit cubic with P0 = (0,0) and P3 = (1,1).
    let curve = |a: f64, b: f64, u: f64| {
        let v = 1.0 - u;
        3.0 * v * v * u * a + 3.0 * v * u * u * b + u * u * u
    };
    let slope = |a: f64, b: f64, u: f64| {
        let v = 1.0 - u;
        3.0 * v * v * (a) + 6.0 * v * u * (b - a) + 3.0 * u * u * (1.0 - b)
    };

    let mut u = t;
    for _ in 0..8 {
        let dx = slope(x1, x2, u);
        if dx.abs() < 1e-6 {
            break;
        }
        let err = curve(x1, x2, u) - t;
        if err.abs() < 1e-7 {
            return curve(y1, y2, u);
        }
        u -= err / dx;
    }
    // Newton bailed: bisect, which cannot fail on a monotonic x.
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    u = t;
    for _ in 0..24 {
        let x = curve(x1, x2, u);
        if (x - t).abs() < 1e-7 {
            break;
        }
        if x > t {
            hi = u;
        } else {
            lo = u;
        }
        u = (lo + hi) / 2.0;
    }
    curve(y1, y2, u)
}

/// Between two on-screen states: reflow, colour, a value settling.
pub fn standard(t: f64) -> f64 {
    cubic_bezier(0.2, 0.0, 0.0, 1.0, t)
}

/// Arriving. Starts at speed and settles.
pub fn decelerate(t: f64) -> f64 {
    cubic_bezier(0.0, 0.0, 0.0, 1.0, t)
}

/// Leaving. Gathers speed and goes.
pub fn accelerate(t: f64) -> f64 {
    cubic_bezier(0.3, 0.0, 1.0, 1.0, t)
}

/// The one place an animation length is decided, so reduced motion and the
/// debug stretch below apply to every surface without each one remembering
/// to ask.
///
/// Reduced motion collapses to a single frame rather than zero, because the
/// state flow (exit removal, unmap, frost teardown) rides the same tick as
/// the motion and still has to run.
pub fn duration(ms: f64) -> f64 {
    if !animations_enabled() {
        return 1.0;
    }
    ms * debug_scale()
}

/// `SWAYPPLET_ANIM_SCALE` stretches every animation by this factor. A fade
/// that is right at 200 ms is very hard to judge and nearly impossible to
/// capture; at 20x it is four seconds and every frame can be looked at. Read
/// once, because it is a debugging switch and not a live setting.
fn debug_scale() -> f64 {
    static SCALE: std::sync::OnceLock<f64> = std::sync::OnceLock::new();
    *SCALE.get_or_init(|| {
        std::env::var("SWAYPPLET_ANIM_SCALE")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|v| v.is_finite() && *v > 0.0)
            .unwrap_or(1.0)
    })
}

/// GTK's own reduced-motion switch (gtk-enable-animations, which the a11y
/// settings and `GTK_DEBUG=no-animations` both drive). GTK CSS has no
/// `prefers-reduced-motion` media query, so every animation entry point
/// checks here and jumps to the end state instead.
pub fn animations_enabled() -> bool {
    gtk4::Settings::default().is_none_or(|s| s.is_gtk_enable_animations())
}

/// Toggle a namespace's compositor frost at runtime, so a frosted surface
/// can be hidden without a black frame: mapping or unmapping a BLURRED
/// surface makes swayfx re-initialize blur state, and that transition frame
/// renders black. Disabling the blur node first is unconditional — swayfx
/// skips the node entirely, no stencil, nothing drawn. So blur is show/hide
/// state: enabled right after map, disabled before unmap.
///
/// Glass has to be named alongside blur. The two effects share one scene
/// node, which swayfx keeps live while `blur_enabled ||
/// liquid_glass_enabled` (`arrange_surface` and
/// `configure_layer_shell_surface` in the nixos repo's
/// patches/swayfx-liquid-glass.patch), and the sway config gives every
/// swaypplet namespace `liquid_glass enable` and no `blur enable` at all.
/// `blur disable` on its own therefore turns nothing off, the node survives
/// the unmap, and the black transition frame is back.
///
/// Leaving the surface mapped instead is not an alternative, and the reason
/// is not the frost. A mapped surface with nothing left to draw stops
/// committing, so the compositor goes on showing the last buffer it was
/// sent — the content mid-fade. Disabling the effects does not touch that,
/// because it is the client's own pixels. Hiding means unmapping.
///
/// One command carries both effects. `layer_criteria_parse` splits the
/// effects string on `;` and `,`, and sway's own command splitter is
/// quote-aware (`argsep` tracks `"`), so the semicolon stays inside the
/// quoted argument: one IPC round trip and one acknowledgment for `then` to
/// sequence on. It is also the safer shape on a compositor without the
/// patch, where `liquid_glass` is not a known effect. `layer_criteria_add`
/// deletes the namespace's existing criteria before parsing the new cmdlist
/// and a parse failure destroys the replacement, so the namespace is left
/// with no effects at all; sending the two separately would instead land
/// `blur enable` and then strip the blur_ignore_transparent it needs, which
/// is the screen-sized slab and the crash below. sway answers CMD_SUCCESS
/// either way (`cmd_layer_effects` ignores a NULL criteria), so the failure
/// shows up in its log rather than in the reply.
///
/// Only ever `blur`/`liquid_glass` `enable|disable`, NEVER `reset`. sway
/// merges a runtime `layer_effects` into the namespace's existing criteria
/// (layer_criteria.c `layer_criteria_add` clones the old effects first), so
/// this keeps the blur_ignore_transparent, corner_radius and `liquid_glass_*`
/// numbers the sway config set. `reset` clears blur_ignore_transparent, which
/// sends sway into `wlr_scene_blur_set_transparency_mask_source(node, NULL)`;
/// that dereferences the NULL source at scenefx wlr_scene.c:1153 and takes
/// the whole compositor down (scenefx 0.4.1-git/37ccd72, swayfx 0.5.3-git).
///
/// `then` runs on the GTK thread once sway has acknowledged the change,
/// which hide uses to sequence the unmap after the frost is gone — and on
/// failure too, so a wedged socket can't strand a surface mapped forever.
/// One command means one reply and one `then`, on either path.
pub fn set_layer_blur(namespace: Option<glib::GString>, on: bool, then: impl FnOnce() + 'static) {
    let Some(ns) = namespace else {
        then();
        return;
    };
    let effects = if on {
        "blur enable; liquid_glass enable"
    } else {
        "blur disable; liquid_glass disable"
    };
    crate::sway_ipc::run_command_then(&format!("layer_effects \"{ns}\" \"{effects}\""), then);
}

/// The glass-pane opacity channel for a fade between `from` and `to` at
/// linear progress `t`.
///
/// Zero is the only boundary the compositor cannot follow. swayfx frosts
/// every pixel with alpha above zero at full strength and offers no ramp, so
/// a pane at 0.3 is not a third-frosted card: it is a *fully* frosted
/// rectangle wearing a third of its tint, which reads as a darkened square
/// with nothing in it. Crossing that boundary is therefore a step, taken in
/// a single frame, so the tint arrives and leaves with the frost.
///
/// A 90 ms ramp used to cross it instead, and those ~5 frames at each end
/// were the darkened square. Between two frosted states there is no such
/// mismatch — both are fully frosted — so an ordinary eased ramp is right,
/// which is what the collapsed stack's dimming wants.
pub fn glass_channel(from: f64, to: f64, t: f64) -> f64 {
    if from == 0.0 {
        return if t > 0.0 { to } else { from };
    }
    if to == 0.0 {
        return if t >= 1.0 { to } else { from };
    }
    from + (to - from) * standard(t)
}

// ── Reveal: the shared show/hide transition ────────────────────────────

struct RevealInner {
    window: gtk4::Window,
    pane: gtk4::Widget,
    content: RefCell<Option<gtk4::Widget>>,
    slide: RefCell<Option<(SlideBin, f64)>>,
    shown: Cell<bool>,
    tick: RefCell<Option<gtk4::TickCallbackId>>,
    /// The compositor's own alpha for this surface, when it offers one. Bound
    /// on the first show, because the surface does not exist before that.
    alpha: RefCell<Option<crate::alpha::SurfaceAlpha>>,
    /// Run once the hide has finished and the surface is unmapped. A caller
    /// that owns the surface (one card of a stack, say) drops it here.
    on_hidden: RefCell<Option<Box<dyn Fn()>>>,
}

/// One show/hide transition for every glass surface (motion on glass, see
/// the module header): the pane's tint rides [`glass_channel`], the content
/// fades eased over the full duration, an optional [`SlideBin`] adds the
/// directional settle, and the window unmaps only once the exit finishes.
/// Respects reduced motion ([`animations_enabled`]) by jumping.
#[derive(Clone)]
pub struct Reveal {
    inner: Rc<RevealInner>,
}

impl Reveal {
    /// `pane` is the widget whose CSS carries the frosted card background —
    /// its alpha is what swayfx stencils blur against, so [`Reveal`] owns
    /// its opacity outright and lands it on exactly 0.0 when hidden.
    pub fn new(window: &gtk4::Window, pane: &impl IsA<gtk4::Widget>) -> Self {
        Reveal {
            inner: Rc::new(RevealInner {
                window: window.clone(),
                pane: pane.clone().upcast(),
                content: RefCell::new(None),
                slide: RefCell::new(None),
                shown: Cell::new(false),
                tick: RefCell::new(None),
                alpha: RefCell::new(None),
                on_hidden: RefCell::new(None),
            }),
        }
    }

    /// Everything drawn on the glass; fades over the full enter/exit
    /// duration. Without one, pane and content fade as a single channel.
    pub fn content(self, content: &impl IsA<gtk4::Widget>) -> Self {
        *self.inner.content.borrow_mut() = Some(content.clone().upcast());
        self
    }

    /// Point at the content after the fact, for a surface whose contents are
    /// rebuilt in place.
    pub fn set_content(&self, content: &impl IsA<gtk4::Widget>) {
        *self.inner.content.borrow_mut() = Some(content.clone().upcast());
    }

    /// Drive the material outright, for a gesture that is following a finger
    /// rather than playing a transition. Cancels any transition in flight.
    pub fn set_alpha(&self, a: f64) {
        self.cancel_tick();
        self.set_material_alpha(a);
        if let Some(c) = &*self.inner.content.borrow() {
            c.set_opacity(if self.inner.alpha.borrow().is_some() {
                1.0
            } else {
                a
            });
        }
    }

    /// Give up the compositor alpha handle while the surface it is bound to
    /// still exists.
    ///
    /// `wp_alpha_modifier_surface_v1` must be destroyed *before* its
    /// `wl_surface`: once the surface is gone, every request on the handle,
    /// the destructor included, is a fatal protocol error (see
    /// [`crate::alpha`]). Anyone about to destroy the window calls this
    /// first.
    pub fn release_alpha(&self) {
        drop(self.inner.alpha.borrow_mut().take());
    }

    /// Pair the fade with a settle: `bin` translates from `px` below its
    /// resting spot to 0 on show and back on hide.
    pub fn slide(self, bin: &SlideBin, px: f64) -> Self {
        *self.inner.slide.borrow_mut() = Some((bin.clone(), px));
        self
    }

    /// Called once the exit has finished and the surface is unmapped. Only
    /// one hook; setting it again replaces the previous one.
    pub fn connect_hidden(&self, f: impl Fn() + 'static) {
        *self.inner.on_hidden.borrow_mut() = Some(Box::new(f));
    }

    /// Logical state: what the surface is transitioning toward.
    pub fn is_shown(&self) -> bool {
        self.inner.shown.get()
    }

    pub fn show(&self) {
        let inner = &self.inner;
        inner.shown.set(true);
        let was_visible = inner.window.is_visible();
        // Map first, frost second: the surface maps effect-less (nothing
        // for swayfx to initialize, so no black frame), and the enable
        // lands a frame or two into the fade — inside the GLASS_MS tint
        // lead (see set_layer_blur).
        inner.window.set_visible(true);
        // Rebind when the handle has nothing left to speak to. GDK builds a
        // new `wl_surface` for a window it re-realizes, and an alpha handle
        // is bound to the surface it was made for, not to the window — so a
        // surface that came back needs a new one or the material never fades
        // again (and, before `is_valid`, took the process down instead).
        let rebind = inner
            .alpha
            .borrow()
            .as_ref()
            .is_none_or(|alpha| !alpha.is_valid());
        if rebind {
            *inner.alpha.borrow_mut() = crate::alpha::SurfaceAlpha::attach(&inner.window);
        }
        if inner.window.is_layer_window() {
            set_layer_blur(inner.window.namespace(), true, || {});
        }
        if !animations_enabled() {
            self.cancel_tick();
            if let Some(a) = &*inner.alpha.borrow() {
                a.set(1.0);
            }
            inner.pane.set_opacity(1.0);
            if let Some(c) = &*inner.content.borrow() {
                c.set_opacity(1.0);
            }
            if let Some((bin, _)) = &*inner.slide.borrow() {
                bin.jump_to(0.0);
            }
            return;
        }
        // Start from the fully hidden pose on a fresh map OR a parked
        // surface (pane at exactly 0) — instant-hide paths (window unmapped
        // without hide()) would otherwise skip the enter transition.
        if !was_visible || self.material_alpha() == 0.0 {
            self.set_material_alpha(0.0);
            if let Some(c) = &*inner.content.borrow() {
                c.set_opacity(if inner.alpha.borrow().is_some() {
                    1.0
                } else {
                    0.0
                });
            }
            if let Some((bin, px)) = &*inner.slide.borrow() {
                bin.jump_to(*px);
            }
        }
        if let Some((bin, _)) = &*inner.slide.borrow() {
            bin.slide_to(0.0, duration(ENTER_MS));
        }
        self.animate(true);
    }

    pub fn hide(&self) {
        let inner = &self.inner;
        inner.shown.set(false);
        if !animations_enabled() || !inner.window.is_visible() {
            self.cancel_tick();
            self.finish_hide();
            return;
        }
        if let Some((bin, px)) = &*inner.slide.borrow() {
            bin.slide_to(*px, duration(EXIT_MS));
        }
        self.animate(false);
    }

    /// The material's current opacity: the compositor's number when it owns
    /// it, the pane widget's alpha otherwise.
    fn material_alpha(&self) -> f64 {
        match &*self.inner.alpha.borrow() {
            Some(a) => a.get(),
            None => self.inner.pane.opacity(),
        }
    }

    /// Drive the material. With `wp_alpha_modifier_v1` this is one number for
    /// pane, frost and content together, which is what makes the frost fade
    /// at all; the pane widget then stays fully opaque and the content stops
    /// needing a fade of its own. Without it, the old split stands: the pane
    /// carries the tint and the content fades on top of it.
    fn set_material_alpha(&self, a: f64) {
        match &*self.inner.alpha.borrow() {
            Some(m) => {
                m.set(a);
                self.inner.pane.set_opacity(1.0);
                // The multiplier is surface state, so it lands on the next
                // commit and only a redraw produces one.
                self.inner.pane.queue_draw();
            }
            None => self.inner.pane.set_opacity(a),
        }
    }

    fn cancel_tick(&self) {
        if let Some(id) = self.inner.tick.take() {
            id.remove();
        }
    }

    /// Hide fully: opacities land on exactly 0.0 and the surface UNMAPS, so
    /// interaction with whatever is behind resumes unconditionally. The
    /// frost is dropped first and the unmap waits for sway's acknowledgment
    /// — unmapping a blurred surface flashes a black frame, and a mapped
    /// alpha-0 one keeps a stale frost, so effects-off-then-unmap is the only
    /// clean exit (see [`set_layer_blur`]). A show() racing the
    /// acknowledgment wins: the unmap is skipped.
    fn finish_hide(&self) {
        let inner = &self.inner;
        self.set_material_alpha(0.0);
        if let Some(c) = &*inner.content.borrow() {
            c.set_opacity(0.0);
        }
        if let Some((bin, px)) = &*inner.slide.borrow() {
            bin.jump_to(*px);
        }
        if !inner.window.is_visible() {
            self.hidden();
            return;
        }
        if inner.window.is_layer_window() {
            let this = self.clone();
            set_layer_blur(inner.window.namespace(), false, move || {
                if this.inner.shown.get() {
                    // A show() beat the reply here; its own enable may have
                    // reached sway before this disable, so restate it.
                    set_layer_blur(this.inner.window.namespace(), true, || {});
                } else {
                    this.inner.window.set_visible(false);
                    this.hidden();
                }
            });
        } else {
            inner.window.set_visible(false);
            self.hidden();
        }
    }

    /// Announce the surface is gone. Taken rather than borrowed, so a hook
    /// that drops the Reveal it belongs to cannot re-enter it.
    fn hidden(&self) {
        let hook = self.inner.on_hidden.borrow_mut().take();
        if let Some(f) = hook {
            f();
        }
    }

    /// Drive pane + content from their *current* opacities toward the
    /// target, so retriggers mid-transition reverse smoothly.
    fn animate(&self, entering: bool) {
        self.cancel_tick();
        let inner = &self.inner;
        let compositor_owns = inner.alpha.borrow().is_some();
        let pane_from = self.material_alpha();
        let content_from = inner.content.borrow().as_ref().map(|c| c.opacity());
        let target = if entering { 1.0 } else { 0.0 };
        let total = duration(if entering { ENTER_MS } else { EXIT_MS });
        // Arriving decelerates into place; leaving gathers speed and goes.
        // One curve for both directions made every surface hesitate on its
        // way out. See docs/MOTION.md.
        let ease: fn(f64) -> f64 = if entering { decelerate } else { accelerate };
        let start = glib::monotonic_time();
        let this = self.clone();
        let id = inner.pane.add_tick_callback(move |_, _| {
            let t = (((glib::monotonic_time() - start) as f64 / 1000.0) / total).clamp(0.0, 1.0);
            if compositor_owns {
                // Material and content are one object now, eased together.
                this.set_material_alpha(pane_from + (target - pane_from) * ease(t));
            } else {
                this.set_material_alpha(glass_channel(pane_from, target, t));
                if let (Some(from), Some(c)) = (content_from, this.inner.content.borrow().as_ref())
                {
                    c.set_opacity(from + (target - from) * ease(t));
                }
            }
            if t >= 1.0 {
                this.inner.tick.take();
                if !entering {
                    this.finish_hide();
                }
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
        *inner.tick.borrow_mut() = Some(id);
    }
}

mod imp {
    use super::*;
    use std::cell::{Cell, RefCell};

    pub struct SlideBin {
        pub d: Cell<f64>,
        pub horizontal: Cell<bool>,
        pub scale: Cell<f64>,
        pub tick: RefCell<Option<gtk4::TickCallbackId>>,
    }

    impl Default for SlideBin {
        fn default() -> Self {
            Self {
                d: Cell::new(0.0),
                horizontal: Cell::new(false),
                scale: Cell::new(1.0),
                tick: RefCell::new(None),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SlideBin {
        const NAME: &'static str = "SwayppletSlideBin";
        type Type = super::SlideBin;
        type ParentType = gtk4::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk4::BinLayout>();
        }
    }

    impl ObjectImpl for SlideBin {
        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for SlideBin {
        fn snapshot(&self, snapshot: &gtk4::Snapshot) {
            let d = self.d.get() as f32;
            let offset = if self.horizontal.get() {
                graphene::Point::new(d, 0.0)
            } else {
                graphene::Point::new(0.0, d)
            };
            snapshot.translate(&offset);
            let scale = self.scale.get();
            if scale != 1.0 {
                // Shrink toward the top edge of the trailing side, which is
                // where a right-anchored card is pinned.
                let w = self.obj().width() as f32;
                snapshot.translate(&graphene::Point::new(w, 0.0));
                snapshot.scale(scale as f32, scale as f32);
                snapshot.translate(&graphene::Point::new(-w, 0.0));
            }
            let widget = self.obj();
            let mut child = widget.first_child();
            while let Some(c) = child {
                widget.snapshot_child(&c, snapshot);
                child = c.next_sibling();
            }
        }
    }
}

glib::wrapper! {
    /// Bin that renders its child offset by an animatable distance, and
    /// optionally scaled, without touching layout — measure and allocation
    /// pass through unchanged, only the render nodes move. Anything
    /// translated past the surface edge is clipped by the fixed-size layer
    /// surface, so the child can start off its resting position and settle
    /// into place.
    ///
    /// One axis each, so two of them nest cleanly: a card's slot in a stack
    /// is vertical placement, and its entrance is a horizontal settle, and
    /// neither animation can then tread on the other's tick.
    pub struct SlideBin(ObjectSubclass<imp::SlideBin>)
        @extends gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

impl SlideBin {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Offset along x instead of y.
    pub fn horizontal() -> Self {
        let bin: Self = glib::Object::new();
        bin.imp().horizontal.set(true);
        bin
    }

    pub fn set_child(&self, child: &impl IsA<gtk4::Widget>) {
        child.set_parent(self);
    }

    /// Set the offset immediately, cancelling any running settle.
    pub fn jump_to(&self, dy: f64) {
        if let Some(id) = self.imp().tick.take() {
            id.remove();
        }
        self.imp().d.set(dy);
        self.queue_draw();
    }

    /// Render at `scale`, pinned to the trailing edge. Layout is untouched,
    /// so this costs nothing and cannot move the surface.
    pub fn set_scale(&self, scale: f64) {
        if self.imp().scale.get() == scale {
            return;
        }
        self.imp().scale.set(scale);
        self.queue_draw();
    }

    /// Animate the offset to `target` over `ms`, eased like every other
    /// surface motion. Retargeting mid-flight continues from the current
    /// offset.
    pub fn slide_to(&self, target: f64, ms: f64) {
        let imp = self.imp();
        if let Some(id) = imp.tick.take() {
            id.remove();
        }
        let from = imp.d.get();
        if from == target {
            return;
        }
        let start = glib::monotonic_time();
        let id = self.add_tick_callback(move |bin, _| {
            let t = (((glib::monotonic_time() - start) as f64 / 1000.0) / ms).clamp(0.0, 1.0);
            bin.imp().d.set(from + (target - from) * standard(t));
            bin.queue_draw();
            if t >= 1.0 {
                bin.imp().tick.take();
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
        imp.tick.replace(Some(id));
    }
}

impl Default for SlideBin {
    fn default() -> Self {
        Self::new()
    }
}

/// Onset nudge travel/duration (board bays, battery critical).
const NUDGE_PX: f64 = -3.0;
const NUDGE_MS: f64 = 100.0;

/// One onset transient, then static rest (vision P2). `slide_to` has no
/// completion callback, so the return leg rides a timeout. Reduced motion
/// skips it entirely — the state classes carry the change.
pub fn nudge(slide: &SlideBin) {
    if !animations_enabled() {
        return;
    }
    let onset = duration(NUDGE_MS);
    slide.slide_to(NUDGE_PX, onset);
    let slide = slide.clone();
    glib::timeout_add_local_once(std::time::Duration::from_millis(onset as u64), move || {
        slide.slide_to(0.0, duration(EXIT_MS));
    });
}
