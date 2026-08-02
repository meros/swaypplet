//! Frosted-glass pane for the lock/greeter card.
//!
//! swayfx layer_effects blur covers neither ext-session-lock surfaces nor a
//! surface's own content, so the lock screen fakes its glass in GTK: the
//! full-screen background stays a crisp `Picture`, and this widget re-draws
//! the same wallpaper texture — blurred and scrim-dimmed — clipped to the
//! card's rounded bounds. The card's translucent `@surface` fill then tints
//! it exactly like the layer-shell glass surfaces.
//!
//! The pane samples the texture with the same ContentFit::Cover mapping the
//! base picture uses (scale to fill the root, center the overflow), offset
//! by its own position, so the blurred region lines up pixel-perfect with
//! the crisp image around it.

use gtk4::{gdk, glib, graphene, gsk, prelude::*, subclass::prelude::*};

/// GSK gaussian blur radius at rest, tuned to read like the swayfx layer
/// blur. Unlike the layer-shell surfaces (whose compositor frost is binary,
/// see anim.rs), this client-side blur has a real sigma to animate: the
/// enter transition ramps it 0 → this while the card fades in — the one
/// surface in swaypplet that can do the full iOS-style materialize.
pub const BLUR_RADIUS: f64 = 28.0;
/// Matches `.lock-card` border-radius in style.css.
const CORNER_RADIUS: f32 = 18.0;
/// Matches `.lock-scrim` (alpha(black, 0.45)) so glass shows the dimmed
/// wallpaper, not a brighter raw patch.
const SCRIM: gdk::RGBA = gdk::RGBA::new(0.0, 0.0, 0.0, 0.45);

mod imp {
    use super::*;
    use std::cell::{Cell, RefCell};

    pub struct GlassPane {
        pub texture: RefCell<Option<gdk::Texture>>,
        pub radius: Cell<f64>,
        pub ramp: RefCell<Option<gtk4::TickCallbackId>>,
    }

    impl Default for GlassPane {
        fn default() -> Self {
            GlassPane {
                texture: RefCell::new(None),
                radius: Cell::new(super::BLUR_RADIUS),
                ramp: RefCell::new(None),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for GlassPane {
        const NAME: &'static str = "SwayppletGlassPane";
        type Type = super::GlassPane;
        type ParentType = gtk4::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk4::BinLayout>();
        }
    }

    impl ObjectImpl for GlassPane {
        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for GlassPane {
        fn snapshot(&self, snapshot: &gtk4::Snapshot) {
            let widget = self.obj();
            let w = widget.width() as f32;
            let h = widget.height() as f32;

            if let Some(texture) = self.texture.borrow().as_ref() {
                if let Some((ox, oy, dw, dh)) = super::cover_rect(widget.upcast_ref(), texture) {
                    let clip = gsk::RoundedRect::from_rect(
                        graphene::Rect::new(0.0, 0.0, w, h),
                        CORNER_RADIUS,
                    );
                    let radius = self.radius.get();
                    snapshot.push_rounded_clip(&clip);
                    // Below ~0.5 the blur is invisible; skipping the node
                    // keeps the ramp's first frames free.
                    if radius >= 0.5 {
                        snapshot.push_blur(radius);
                    }
                    snapshot.append_texture(texture, &graphene::Rect::new(ox, oy, dw, dh));
                    if radius >= 0.5 {
                        snapshot.pop();
                    }
                    snapshot.append_color(&SCRIM, &graphene::Rect::new(0.0, 0.0, w, h));
                    snapshot.pop();
                }
            }

            let mut child = widget.first_child();
            while let Some(c) = child {
                widget.snapshot_child(&c, snapshot);
                child = c.next_sibling();
            }
        }
    }
}

glib::wrapper! {
    pub struct GlassPane(ObjectSubclass<imp::GlassPane>)
        @extends gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

impl GlassPane {
    pub fn new() -> Self {
        glib::Object::new()
    }

    pub fn set_child(&self, child: &impl IsA<gtk4::Widget>) {
        child.set_parent(self);
    }

    /// Wallpaper texture to frost behind the child; `None` renders the
    /// child alone (solid-palette fallback).
    pub fn set_texture(&self, texture: Option<gdk::Texture>) {
        *self.imp().texture.borrow_mut() = texture;
        self.queue_draw();
    }

    /// Animatable blur sigma (0 = crisp). The enter transition ramps this
    /// 0 → [`BLUR_RADIUS`] so the glass materializes with the card.
    pub fn set_blur_radius(&self, radius: f64) {
        self.imp().radius.set(radius);
        self.queue_draw();
    }

    /// Ramp the blur sigma from its current value to `target` over `ms`,
    /// eased like every other surface motion (anim.rs). Retargeting cancels
    /// a running ramp; respects reduced motion by jumping.
    pub fn ramp_blur_to(&self, target: f64, ms: f64) {
        if let Some(id) = self.imp().ramp.take() {
            id.remove();
        }
        if !crate::anim::animations_enabled() {
            self.set_blur_radius(target);
            return;
        }
        let from = self.imp().radius.get();
        if from == target {
            return;
        }
        let start = glib::monotonic_time();
        let id = self.add_tick_callback(move |pane, _| {
            let t = (((glib::monotonic_time() - start) as f64 / 1000.0) / ms).clamp(0.0, 1.0);
            pane.set_blur_radius(from + (target - from) * crate::anim::ease_out_cubic(t));
            if t >= 1.0 {
                pane.imp().ramp.take();
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
        *self.imp().ramp.borrow_mut() = Some(id);
    }
}

impl Default for GlassPane {
    fn default() -> Self {
        Self::new()
    }
}

/// The texture rect, in `widget`-local coordinates, that reproduces the base
/// picture's ContentFit::Cover placement across the widget's root.
fn cover_rect(widget: &gtk4::Widget, texture: &gdk::Texture) -> Option<(f32, f32, f32, f32)> {
    let root = widget.root()?;
    let root_widget = root.upcast_ref::<gtk4::Widget>();
    let bounds = widget.compute_bounds(root_widget)?;
    let (rw, rh) = (root_widget.width() as f32, root_widget.height() as f32);
    let (tw, th) = (texture.width() as f32, texture.height() as f32);
    if rw <= 0.0 || rh <= 0.0 || tw <= 0.0 || th <= 0.0 {
        return None;
    }
    let scale = (rw / tw).max(rh / th);
    let (dw, dh) = (tw * scale, th * scale);
    let ox = (rw - dw) / 2.0 - bounds.x();
    let oy = (rh - dh) / 2.0 - bounds.y();
    Some((ox, oy, dw, dh))
}
