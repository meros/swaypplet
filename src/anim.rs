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
//! # Motion on glass: fade, plus a short settle
//!
//! swayfx composites layer_effects blur at full strength behind any surface
//! pixel with alpha > 0 — scenefx's `blur_ignore_transparent` is a stencil
//! that discards only pixels with alpha exactly 0 (tex.frag:
//! `if (discard_transparent && gl_FragColor.a == 0.0) discard;`), and blur
//! strength never scales with layer-surface alpha. A fade therefore shows
//! the frost at full strength before the content is legible: the glass
//! materializes first, then the content resolves on it. That is the chosen
//! look. The OSD enters and exits as a pure crossfade; surfaces with a
//! directional entrance (start menu, notification cards) pair the fade with
//! a short [`SLIDE_PX`] settle instead of a full-height/width wipe.

use gtk4::{glib, graphene, prelude::*, subclass::prelude::*};

pub const ENTER_MS: f64 = 300.0;
pub const MOVE_MS: f64 = 300.0;
pub const EXIT_MS: f64 = 200.0;

/// Travel distance of the settle that accompanies a fade on surfaces with a
/// directional entrance. Short by design — the fade carries the transition,
/// the slide only gives it a direction.
pub const SLIDE_PX: f64 = 24.0;

pub fn ease_out_cubic(t: f64) -> f64 {
    1.0 - (1.0 - t).powi(3)
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
