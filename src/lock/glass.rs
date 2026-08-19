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
//!
//! Two backends draw that. `glass_gl` runs a real refraction shader in a
//! `GtkGLArea` child; this file's `snapshot` runs the GSK blur it always
//! did. The GL one is preferred and the GSK one is the fallback, chosen per
//! pane and not once per process, because the thing that fails is a GL
//! context and each pane asks for its own. Everything below is written so
//! the fallback is reachable at any point, including after the first frame:
//! `GlGlass::failed()` flipping mid-run hides the GL child and this
//! `snapshot` takes over on the next frame with no other state to unwind.

use gtk4::{gdk, glib, graphene, gsk, prelude::*, subclass::prelude::*};

use super::glass_gl::GlGlass;

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
        /// The GL backend, when it was wanted. `None` means the GSK path was
        /// asked for explicitly; `Some` that owns a failed `GlGlass` means it
        /// was tried and did not come up.
        pub gl: RefCell<Option<GlGlass>>,
        /// A live backdrop, when there is one. Anything implementing
        /// `GdkPaintable` works: a `GtkMediaStream` playing a video, an
        /// animation, a paintable driven by a tick callback. It takes
        /// precedence over `texture`, and its `invalidate-contents` is what
        /// drives the refresh, so a still paintable costs nothing per frame
        /// and a moving one costs exactly one crop per changed frame.
        pub paintable: RefCell<Option<gdk::Paintable>>,
        pub paintable_handler: RefCell<Option<glib::SignalHandlerId>>,
        /// A ramp asked for before the GL backend could paint anything.
        /// Held until its first frame, so the animation is seen rather than
        /// finishing against a widget that is not on screen yet.
        pub pending_ramp: Cell<Option<(f64, f64)>>,
        /// The card. Held separately from `first_child()` because the GL
        /// backend inserts a widget in front of it.
        pub child: RefCell<Option<gtk4::Widget>>,
    }

    impl Default for GlassPane {
        fn default() -> Self {
            GlassPane {
                texture: RefCell::new(None),
                radius: Cell::new(super::BLUR_RADIUS),
                ramp: RefCell::new(None),
                gl: RefCell::new(None),
                paintable: RefCell::new(None),
                paintable_handler: RefCell::new(None),
                pending_ramp: Cell::new(None),
                child: RefCell::new(None),
            }
        }
    }

    impl GlassPane {
        /// Is the GL backend drawing this frame?
        pub fn gl_live(&self) -> bool {
            self.gl.borrow().as_ref().is_some_and(|g| !g.failed())
        }

        /// Is the GL backend up but not yet painting? During this window the
        /// pane draws nothing at all: the GL child is transparent and the
        /// GSK path must stay quiet, or the user sees the blur for a few
        /// frames and then the glass replacing it.
        pub fn gl_warming(&self) -> bool {
            self.gl
                .borrow()
                .as_ref()
                .is_some_and(|g| !g.failed() && !g.drawn())
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for GlassPane {
        const NAME: &'static str = "SwayppletGlassPane";
        type Type = super::GlassPane;
        type ParentType = gtk4::Widget;

        fn class_init(_klass: &mut Self::Class) {
            // No BinLayout: with the GL backend there are two children, and
            // they overlap rather than stack. `measure`/`size_allocate` below
            // do it, and the pane still sizes to the card alone so it keeps
            // hugging it exactly.
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
        fn measure(&self, orientation: gtk4::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            // The card decides the pane's size; the GL child fills whatever
            // that turns out to be and must not influence it.
            match self.child.borrow().as_ref() {
                Some(child) => child.measure(orientation, for_size),
                None => (0, 0, -1, -1),
            }
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            // GTK requires a child be measured since its last allocation, in
            // both orientations, even when the parent already knows the size
            // it means to give it. Skipping this is only a warning today, but
            // the allocation it warns about is undefined.
            let place = |w: &gtk4::Widget| {
                w.measure(gtk4::Orientation::Horizontal, -1);
                w.measure(gtk4::Orientation::Vertical, -1);
                w.allocate(width, height, baseline, None);
            };
            if let Some(g) = self.gl.borrow().as_ref() {
                place(g.widget().upcast_ref());
            }
            if let Some(child) = self.child.borrow().as_ref() {
                place(child);
            }
            // The texture's placement is relative to the root, so it moves
            // whenever the pane does. Recomputed here rather than per frame:
            // this is the only thing that can change it.
            self.obj().push_frame();
        }

        fn snapshot(&self, snapshot: &gtk4::Snapshot) {
            let widget = self.obj();
            let w = widget.width() as f32;
            let h = widget.height() as f32;

            // GL backend live: its child draws the glass, including the
            // rounded corners, so this draws nothing of its own. While it is
            // still warming up that child is transparent, and the GSK branch
            // below stays out of the way rather than showing a blur that
            // would be replaced a frame later.
            if self.gl_live()
                && let Some(g) = self.gl.borrow().as_ref()
            {
                widget.snapshot_child(g.widget(), snapshot);
            } else if let Some(paintable) = self.paintable.borrow().as_ref() {
                // GSK fallback, live source. Same blur and scrim as below,
                // over whatever the paintable is showing this frame.
                let clip =
                    gsk::RoundedRect::from_rect(graphene::Rect::new(0.0, 0.0, w, h), CORNER_RADIUS);
                let radius = self.radius.get();
                snapshot.push_rounded_clip(&clip);
                if radius >= 0.5 {
                    snapshot.push_blur(radius);
                }
                let (iw, ih) = (
                    paintable.intrinsic_width() as f32,
                    paintable.intrinsic_height() as f32,
                );
                let (ox, oy, dw, dh) = if iw >= 1.0 && ih >= 1.0 {
                    super::cover_rect(widget.upcast_ref(), iw, ih).unwrap_or((0.0, 0.0, w, h))
                } else {
                    (0.0, 0.0, w, h)
                };
                snapshot.save();
                snapshot.translate(&graphene::Point::new(ox, oy));
                paintable.snapshot(snapshot.upcast_ref::<gdk::Snapshot>(), dw as f64, dh as f64);
                snapshot.restore();
                if radius >= 0.5 {
                    snapshot.pop();
                }
                snapshot.append_color(&SCRIM, &graphene::Rect::new(0.0, 0.0, w, h));
                snapshot.pop();
            } else if let Some(texture) = self.texture.borrow().as_ref()
                && let Some((ox, oy, dw, dh)) = super::cover_rect(
                    widget.upcast_ref(),
                    texture.width() as f32,
                    texture.height() as f32,
                )
            {
                {
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

            if let Some(child) = self.child.borrow().as_ref() {
                widget.snapshot_child(child, snapshot);
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
        let pane: Self = glib::Object::new();
        if super::glass_gl::enabled() {
            let gl = GlGlass::new();
            // Parented first, so it sits behind the card in child order.
            gl.widget().set_parent(&pane);
            *pane.imp().gl.borrow_mut() = Some(gl);
        }
        pane
    }

    pub fn set_child(&self, child: &impl IsA<gtk4::Widget>) {
        let child = child.clone().upcast::<gtk4::Widget>();
        child.set_parent(self);
        *self.imp().child.borrow_mut() = Some(child);
    }

    /// The backdrop to frost behind the child: whatever is painted underneath
    /// this pane, as a paintable.
    ///
    /// Callers hand over the same source the full-screen picture is showing
    /// and do not have to know how it is drawn. A `GdkTexture` — the decoded
    /// wallpaper — takes the still path, uploaded once and sampled with a
    /// cover mapping. Anything else is live: a video through `GtkMediaFile`,
    /// an animation, a paintable driven by a tick. The distinction is made
    /// here rather than by the caller so that swapping a still wallpaper for
    /// a moving one is a change at one call site and nothing downstream
    /// notices.
    pub fn set_backdrop(&self, source: Option<gdk::Paintable>) {
        match source {
            None => {
                self.set_paintable(None);
                self.set_texture(None);
            }
            Some(p) => match p.downcast::<gdk::Texture>() {
                Ok(texture) => {
                    self.set_paintable(None);
                    self.set_texture(Some(texture));
                }
                Err(paintable) => {
                    self.set_texture(None);
                    self.set_paintable(Some(paintable));
                }
            },
        }
    }

    /// Wallpaper texture to frost behind the child; `None` renders the
    /// child alone (solid-palette fallback).
    pub fn set_texture(&self, texture: Option<gdk::Texture>) {
        *self.imp().texture.borrow_mut() = texture;
        self.push_frame();
        self.queue_draw();
    }

    /// A moving backdrop to frost behind the child, taking precedence over
    /// [`set_texture`](Self::set_texture).
    ///
    /// The pane follows the paintable's `invalidate-contents` rather than
    /// polling it, so a still image costs one crop and a 60 fps video costs
    /// one crop per frame — and in both cases the work is proportional to the
    /// pane, not to the wallpaper: only the region this pane actually covers
    /// is ever rendered or uploaded.
    pub fn set_paintable(&self, paintable: Option<gdk::Paintable>) {
        let imp = self.imp();
        if let (Some(old), Some(id)) = (
            imp.paintable.borrow().as_ref(),
            imp.paintable_handler.borrow_mut().take(),
        ) {
            old.disconnect(id);
        }
        if let Some(p) = paintable.as_ref() {
            let id = p.connect_invalidate_contents({
                let pane = self.downgrade();
                move |_| {
                    if let Some(pane) = pane.upgrade() {
                        pane.push_frame();
                        pane.queue_draw();
                    }
                }
            });
            *imp.paintable_handler.borrow_mut() = Some(id);
        }
        *imp.paintable.borrow_mut() = paintable;
        self.push_frame();
        self.queue_draw();
    }

    /// Give the GL backend the pixels behind this pane and where they sit.
    ///
    /// A still texture goes over whole, with the same ContentFit::Cover rect
    /// the GSK path uses, so both backends sample identical pixels and the
    /// seam at the card's edge cannot show. A live paintable is instead
    /// rendered here, cropped to the pane, and handed over pane-sized: a
    /// 360x146 crop is a few hundred kilobytes however large the source is,
    /// which is what keeps a video backdrop affordable.
    fn push_frame(&self) {
        let imp = self.imp();
        if imp.gl.borrow().is_none() {
            return;
        }
        let scale = self.scale_factor() as f32;
        let (pw, ph) = (self.width() as f32, self.height() as f32);

        if let Some(paintable) = imp.paintable.borrow().clone() {
            if pw < 1.0 || ph < 1.0 {
                return; // No allocation yet; size_allocate calls back.
            }
            let margin = imp.gl.borrow().as_ref().map_or(0.0, |g| g.margin(scale));
            if let Some(frame) = self.crop_paintable(&paintable, pw, ph, scale, margin)
                && let Some(g) = imp.gl.borrow().as_ref()
            {
                // Pane-local and negative at the origin: the crop starts a
                // margin above and left of the pane, which is exactly what
                // the rim reflection needs to reach.
                g.set_frame(
                    Some(frame),
                    [
                        -margin,
                        -margin,
                        pw * scale + 2.0 * margin,
                        ph * scale + 2.0 * margin,
                    ],
                );
            }
            return;
        }

        let texture = imp.texture.borrow().clone();
        let Some(texture) = texture else {
            if let Some(g) = imp.gl.borrow().as_ref() {
                g.set_frame(None, [0.0; 4]);
            }
            return;
        };
        if let Some((ox, oy, dw, dh)) = cover_rect(
            self.upcast_ref(),
            texture.width() as f32,
            texture.height() as f32,
        ) && let Some(g) = imp.gl.borrow().as_ref()
        {
            g.set_frame(
                Some(texture),
                [ox * scale, oy * scale, dw * scale, dh * scale],
            );
        }
    }

    /// Render just the part of `paintable` this pane covers, at device
    /// resolution, using the surface's own renderer.
    fn crop_paintable(
        &self,
        paintable: &gdk::Paintable,
        pw: f32,
        ph: f32,
        scale: f32,
        margin: f32,
    ) -> Option<gdk::Texture> {
        let (iw, ih) = (
            paintable.intrinsic_width() as f32,
            paintable.intrinsic_height() as f32,
        );
        // A paintable with no intrinsic size stretches to whatever it is
        // given, so the cover mapping degenerates to the pane itself.
        let (ox, oy, dw, dh) = if iw >= 1.0 && ih >= 1.0 {
            cover_rect(self.upcast_ref(), iw, ih)?
        } else {
            (0.0, 0.0, pw, ph)
        };

        let snapshot = gtk4::Snapshot::new();
        paintable.snapshot(&snapshot, (dw * scale) as f64, (dh * scale) as f64);
        let node = snapshot.to_node()?;
        let renderer = self.native()?.renderer()?;
        // Negative origin because the pane's top-left sits at -(ox, oy) in the
        // source's own space: this is the crop, and it is why the cost here is
        // set by the pane's size and not the wallpaper's.
        Some(renderer.render_texture(
            &node,
            Some(&graphene::Rect::new(
                -ox * scale - margin,
                -oy * scale - margin,
                pw * scale + 2.0 * margin,
                ph * scale + 2.0 * margin,
            )),
        ))
    }

    /// Animatable blur sigma (0 = crisp). The enter transition ramps this
    /// 0 → [`BLUR_RADIUS`] so the glass materializes with the card.
    pub fn set_blur_radius(&self, radius: f64) {
        self.imp().radius.set(radius);
        // One animation drives both backends: the GSK path reads this as a
        // blur sigma, the GL path as how far along the materialize it is.
        if let Some(g) = self.imp().gl.borrow().as_ref() {
            g.set_ramp((radius / BLUR_RADIUS) as f32);
        }
        self.queue_draw();
    }

    /// Run `f` once the pane is actually painting: immediately on the GSK
    /// path, and on the first frame of glass on the GL one.
    fn when_ready(&self, f: impl Fn() + 'static) {
        match self.imp().gl.borrow().as_ref() {
            Some(g) if !g.failed() => g.connect_ready(f),
            _ => f(),
        }
    }

    /// Ramp the blur sigma from its current value to `target` over `ms`,
    /// eased like every other surface motion (anim.rs). Retargeting cancels
    /// a running ramp; respects reduced motion by jumping.
    pub fn ramp_blur_to(&self, target: f64, ms: f64) {
        // The GL backend cannot show anything until its context is up, its
        // wallpaper is uploaded and the pane has an allocation. A ramp
        // started before that is over by the time it could be seen, and the
        // glass arrives in one step — the pop this defers to avoid.
        if self
            .imp()
            .gl
            .borrow()
            .as_ref()
            .is_some_and(|g| !g.drawn() && !g.failed())
        {
            self.imp().pending_ramp.set(Some((target, ms)));
            self.when_ready({
                let pane = self.downgrade();
                move || {
                    if let Some(pane) = pane.upgrade()
                        && let Some((target, ms)) = pane.imp().pending_ramp.take()
                    {
                        pane.start_ramp(target, ms);
                    }
                }
            });
            return;
        }
        self.start_ramp(target, ms);
    }

    fn start_ramp(&self, target: f64, ms: f64) {
        // Through the one place animation lengths are decided, so
        // SWAYPPLET_ANIM_SCALE stretches every motion together.
        let ms = crate::anim::duration(ms);
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
            pane.set_blur_radius(from + (target - from) * crate::anim::decelerate(t));
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

/// The source rect, in `widget`-local coordinates, that reproduces the base
/// picture's ContentFit::Cover placement across the widget's root. `tw`/`th`
/// are the source's intrinsic size — a decoded wallpaper's, or a live
/// paintable's.
fn cover_rect(widget: &gtk4::Widget, tw: f32, th: f32) -> Option<(f32, f32, f32, f32)> {
    let root = widget.root()?;
    let root_widget = root.upcast_ref::<gtk4::Widget>();
    let bounds = widget.compute_bounds(root_widget)?;
    let (rw, rh) = (root_widget.width() as f32, root_widget.height() as f32);
    if rw <= 0.0 || rh <= 0.0 || tw <= 0.0 || th <= 0.0 {
        return None;
    }
    let scale = (rw / tw).max(rh / th);
    let (dw, dh) = (tw * scale, th * scale);
    let ox = (rw - dw) / 2.0 - bounds.x();
    let oy = (rh - dh) / 2.0 - bounds.y();
    Some((ox, oy, dw, dh))
}
