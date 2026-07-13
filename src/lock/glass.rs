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

/// GSK gaussian blur radius, tuned to read like the swayfx layer blur.
const BLUR_RADIUS: f64 = 28.0;
/// Matches `.lock-card` border-radius in style.css.
const CORNER_RADIUS: f32 = 18.0;
/// Matches `.lock-scrim` (alpha(black, 0.45)) so glass shows the dimmed
/// wallpaper, not a brighter raw patch.
const SCRIM: gdk::RGBA = gdk::RGBA::new(0.0, 0.0, 0.0, 0.45);

mod imp {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    pub struct GlassPane {
        pub texture: RefCell<Option<gdk::Texture>>,
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
                    snapshot.push_rounded_clip(&clip);
                    snapshot.push_blur(BLUR_RADIUS);
                    snapshot.append_texture(texture, &graphene::Rect::new(ox, oy, dw, dh));
                    snapshot.pop();
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
        @extends gtk4::Widget;
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
