//! Shared motion scale for all animated surfaces.
//!
//! One easing (cubic ease-out) and three durations, so every surface moves
//! the same way:
//! - micro interactions (hover/press color changes): 150ms, CSS only
//! - structural reveal/collapse: 200ms (GtkRevealer `transition_duration`)
//! - enter/move: [`ENTER_MS`]/[`MOVE_MS`]; exit: [`EXIT_MS`]
//!
//! The CSS side of this scale is documented in `data/style.css`.
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
//! frosted card background — snaps to full tint within [`GLASS_MS`] so
//! tint and frost land in the same beat, while the *content* on it fades
//! over the full [`ENTER_MS`]/[`EXIT_MS`]. Exits mirror it: content fades
//! first, the pane drops in the last [`GLASS_MS`] and must land on exactly
//! 0.0 (the stencil threshold). [`Reveal`] packages this; surfaces with a
//! directional entrance add a short [`SLIDE_PX`] settle on top. The lock
//! screen's client-side glass (lock/glass.rs) is the one place a true
//! sigma ramp is possible, and does that instead.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4::{glib, graphene, prelude::*, subclass::prelude::*};

pub const ENTER_MS: f64 = 300.0;
pub const MOVE_MS: f64 = 300.0;
pub const EXIT_MS: f64 = 200.0;

/// How fast the pane's own tint arrives (and, on exit, leaves). Short by
/// design: the compositor frost is behind every alpha > 0 pixel from the
/// first frame, so the tint has to land with it or the card reads as an
/// untinted frosted rectangle.
pub const GLASS_MS: f64 = 90.0;

/// Travel distance of the settle that accompanies a fade on surfaces with a
/// directional entrance. Short by design — the fade carries the transition,
/// the slide only gives it a direction.
pub const SLIDE_PX: f64 = 24.0;

pub fn ease_out_cubic(t: f64) -> f64 {
    1.0 - (1.0 - t).powi(3)
}

/// GTK's own reduced-motion switch (gtk-enable-animations, which the a11y
/// settings and `GTK_DEBUG=no-animations` both drive). GTK CSS has no
/// `prefers-reduced-motion` media query, so every animation entry point
/// checks here and jumps to the end state instead.
pub fn animations_enabled() -> bool {
    gtk4::Settings::default().is_none_or(|s| s.is_gtk_enable_animations())
}

/// The glass-pane opacity channel for a fade between `from` and `to` at
/// linear progress `t` of a `total_ms` animation. Rising fades arrive
/// within [`GLASS_MS`]; falling fades hold until the last [`GLASS_MS`] and
/// land exactly on `to` (which must be 0.0 when hiding — swayfx stencils
/// blur at alpha exactly zero).
pub fn glass_channel(from: f64, to: f64, t: f64, total_ms: f64) -> f64 {
    if to >= from {
        let g = (t * total_ms / GLASS_MS).min(1.0);
        from + (to - from) * g
    } else {
        let g = ((1.0 - t) * total_ms / GLASS_MS).min(1.0);
        to + (from - to) * g
    }
}

// ── Reveal: the shared show/hide transition ────────────────────────────

struct RevealInner {
    window: gtk4::Window,
    pane: gtk4::Widget,
    content: RefCell<Option<gtk4::Widget>>,
    slide: RefCell<Option<(SlideBin, f64)>>,
    shown: Cell<bool>,
    tick: RefCell<Option<gtk4::TickCallbackId>>,
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
            }),
        }
    }

    /// Everything drawn on the glass; fades over the full enter/exit
    /// duration. Without one, pane and content fade as a single channel.
    pub fn content(self, content: &impl IsA<gtk4::Widget>) -> Self {
        *self.inner.content.borrow_mut() = Some(content.clone().upcast());
        self
    }

    /// Pair the fade with a settle: `bin` translates from `px` below its
    /// resting spot to 0 on show and back on hide.
    pub fn slide(self, bin: &SlideBin, px: f64) -> Self {
        *self.inner.slide.borrow_mut() = Some((bin.clone(), px));
        self
    }

    /// Logical state: what the surface is transitioning toward.
    pub fn is_shown(&self) -> bool {
        self.inner.shown.get()
    }

    pub fn show(&self) {
        let inner = &self.inner;
        inner.shown.set(true);
        let was_visible = inner.window.is_visible();
        inner.window.set_visible(true);
        if !animations_enabled() {
            self.cancel_tick();
            inner.pane.set_opacity(1.0);
            if let Some(c) = &*inner.content.borrow() {
                c.set_opacity(1.0);
            }
            if let Some((bin, _)) = &*inner.slide.borrow() {
                bin.jump_to(0.0);
            }
            return;
        }
        // A fresh map starts from the fully hidden pose — instant-hide
        // paths (window unmapped without hide()) would otherwise skip the
        // enter transition entirely.
        if !was_visible {
            inner.pane.set_opacity(0.0);
            if let Some(c) = &*inner.content.borrow() {
                c.set_opacity(0.0);
            }
            if let Some((bin, px)) = &*inner.slide.borrow() {
                bin.jump_to(*px);
            }
        }
        if let Some((bin, _)) = &*inner.slide.borrow() {
            bin.slide_to(0.0, ENTER_MS);
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
            bin.slide_to(*px, EXIT_MS);
        }
        self.animate(false);
    }

    fn cancel_tick(&self) {
        if let Some(id) = self.inner.tick.take() {
            id.remove();
        }
    }

    fn finish_hide(&self) {
        let inner = &self.inner;
        inner.pane.set_opacity(0.0);
        if let Some(c) = &*inner.content.borrow() {
            c.set_opacity(0.0);
        }
        if let Some((bin, px)) = &*inner.slide.borrow() {
            bin.jump_to(*px);
        }
        inner.window.set_visible(false);
    }

    /// Drive pane + content from their *current* opacities toward the
    /// target, so retriggers mid-transition reverse smoothly.
    fn animate(&self, entering: bool) {
        self.cancel_tick();
        let inner = &self.inner;
        let pane_from = inner.pane.opacity();
        let content_from = inner.content.borrow().as_ref().map(|c| c.opacity());
        let target = if entering { 1.0 } else { 0.0 };
        let total = if entering { ENTER_MS } else { EXIT_MS };
        let start = glib::monotonic_time();
        let this = self.clone();
        let id = inner.pane.add_tick_callback(move |_, _| {
            let t = (((glib::monotonic_time() - start) as f64 / 1000.0) / total).clamp(0.0, 1.0);
            this.inner
                .pane
                .set_opacity(glass_channel(pane_from, target, t, total));
            if let (Some(from), Some(c)) = (content_from, this.inner.content.borrow().as_ref()) {
                c.set_opacity(from + (target - from) * ease_out_cubic(t));
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

    #[derive(Default)]
    pub struct SlideBin {
        pub dy: Cell<f64>,
        pub tick: RefCell<Option<gtk4::TickCallbackId>>,
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
            snapshot.translate(&graphene::Point::new(0.0, self.dy.get() as f32));
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
    /// Bin that renders its child vertically offset by an animatable `dy`,
    /// without touching layout — measure and allocation pass through
    /// unchanged, only the render nodes move. Anything translated past the
    /// surface edge is clipped by the fixed-size layer surface, so the
    /// child can start below its resting position (positive `dy`) and
    /// settle up into place.
    pub struct SlideBin(ObjectSubclass<imp::SlideBin>)
        @extends gtk4::Widget;
}

impl SlideBin {
    pub fn new() -> Self {
        glib::Object::new()
    }

    pub fn set_child(&self, child: &impl IsA<gtk4::Widget>) {
        child.set_parent(self);
    }

    /// Set the offset immediately, cancelling any running settle.
    pub fn jump_to(&self, dy: f64) {
        if let Some(id) = self.imp().tick.take() {
            id.remove();
        }
        self.imp().dy.set(dy);
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
        let from = imp.dy.get();
        if from == target {
            return;
        }
        let start = glib::monotonic_time();
        let id = self.add_tick_callback(move |bin, _| {
            let t = (((glib::monotonic_time() - start) as f64 / 1000.0) / ms).clamp(0.0, 1.0);
            bin.imp().dy.set(from + (target - from) * ease_out_cubic(t));
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
