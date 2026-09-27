//! The avatar component (`ui::avatar`, docs/design-system.md §6): a round
//! profile picture (real image when the host resolves one, a deterministic
//! per-user monogram otherwise) with an optional presence dot for a
//! logged-in user.
//!
//! A subclassed widget snapshots the texture into a `RoundedRect` clip
//! (cover-fit, like `ContentFit::Cover`), so the image crops to a circle
//! without a Cairo round-trip. The monogram is the uppercase initial on the
//! `Overlay`'s own CSS fill: `.ui-avatar` rounds it and a `.cat-n` class picks
//! a categorical colour hashed from the name, because which user this is is
//! identity and identity is what the categorical set is for. The initial is
//! drawn in the widget's CSS colour, so no colour is written down here.
//! Callers mark the current user with `.active` (a ring); the dot is a
//! `.ui-avatar-presence` overlay.

use gtk4::{gdk, glib, graphene, gsk, pango, prelude::*, subclass::prelude::*};

mod imp {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    pub struct AvatarImage {
        pub texture: RefCell<Option<gdk::Texture>>,
        /// Uppercase initial for the monogram fallback.
        pub letter: RefCell<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for AvatarImage {
        const NAME: &'static str = "SwayppletAvatarImage";
        type Type = super::AvatarImage;
        type ParentType = gtk4::Widget;
    }

    impl ObjectImpl for AvatarImage {}

    impl WidgetImpl for AvatarImage {
        fn snapshot(&self, snapshot: &gtk4::Snapshot) {
            let widget = self.obj();
            let w = widget.width() as f32;
            let h = widget.height() as f32;
            let size = w.min(h);
            if size <= 0.0 {
                return;
            }
            let r = size / 2.0;
            let cx = w / 2.0;
            let cy = h / 2.0;
            if let Some(texture) = self.texture.borrow().as_ref() {
                let rect = graphene::Rect::new(cx - r, cy - r, size, size);
                let clip = gsk::RoundedRect::from_rect(rect, r);
                snapshot.push_rounded_clip(&clip);
                // ContentFit::Cover: scale to fill the circle, centre overflow.
                let (tw, th) = (texture.width() as f32, texture.height() as f32);
                if tw > 0.0 && th > 0.0 {
                    let scale = (size / tw).max(size / th);
                    let (dw, dh) = (tw * scale, th * scale);
                    let dst = graphene::Rect::new(cx - dw / 2.0, cy - dh / 2.0, dw, dh);
                    snapshot.append_texture(texture, &dst);
                }
                snapshot.pop();
                return;
            }

            // The monogram, on the fill the CSS paints behind this widget.
            // The family and weight are the widget's own (`.ui-avatar`); the
            // size is the avatar's, because an initial scales with its circle
            // the way a glyph keeps its size.
            let letter = self.letter.borrow();
            if !letter.is_empty() {
                let layout = widget.create_pango_layout(Some(&letter));
                let mut font = widget
                    .pango_context()
                    .font_description()
                    .unwrap_or_default();
                font.set_size((size * 0.42).round() as i32 * pango::SCALE);
                layout.set_font_description(Some(&font));
                let (lw, lh) = layout.pixel_size();
                snapshot.save();
                snapshot.translate(&graphene::Point::new(
                    cx - lw as f32 / 2.0,
                    cy - lh as f32 / 2.0,
                ));
                snapshot.append_layout(&layout, &widget.color());
                snapshot.restore();
            }
        }
    }
}

glib::wrapper! {
    pub struct AvatarImage(ObjectSubclass<imp::AvatarImage>)
        @extends gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

impl AvatarImage {
    fn new(size: i32, texture: Option<gdk::Texture>, letter: String) -> Self {
        let obj: Self = glib::Object::new();
        obj.set_size_request(size, size);
        *obj.imp().texture.borrow_mut() = texture;
        *obj.imp().letter.borrow_mut() = letter;
        obj
    }
}

/// Build a round avatar for `name`; `ui::avatar` is the name to call it by.
/// Uses `icon_path` when it loads, otherwise a deterministic monogram. `size`
/// is the diameter in px; `logged_in` adds the presence dot. The returned
/// widget carries `.ui-avatar`; the caller adds `.active` to ring the current
/// user.
/// The presence dot's diameter, as `.ui-avatar-presence` sets it.
const PRESENCE_DOT: i32 = 10;

pub fn avatar(name: &str, icon_path: Option<&str>, size: i32, logged_in: bool) -> gtk4::Widget {
    let texture = icon_path.and_then(|p| gdk::Texture::from_filename(p).ok());
    let letter = name
        .chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_default();
    let has_picture = texture.is_some();
    let image = AvatarImage::new(size, texture, letter);

    let overlay = gtk4::Overlay::new();
    overlay.add_css_class("ui-avatar");
    // The fill is the monogram's ground; under a picture it would only show
    // at the anti-aliased rim.
    if !has_picture {
        overlay.add_css_class(&format!("cat-{}", category_for(name) + 1));
    }
    overlay.set_size_request(size, size);
    // Never stretched: a row or a box that fills its height would make the
    // overlay taller than wide, and its pill-radius fill and ring with it,
    // an oval around a round picture.
    overlay.set_halign(gtk4::Align::Center);
    overlay.set_valign(gtk4::Align::Center);
    overlay.set_child(Some(&image));

    if logged_in {
        let dot = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        dot.add_css_class("ui-avatar-presence");
        dot.set_halign(gtk4::Align::End);
        dot.set_valign(gtk4::Align::End);
        // On the circle's rim at 45°, not in the square's corner, which is
        // outside the circle: the rim point sits r(1 - 1/√2) in from each
        // edge, and the dot is centred on it.
        let inset = (f64::from(size) / 2.0 * (1.0 - std::f64::consts::FRAC_1_SQRT_2)
            - f64::from(PRESENCE_DOT) / 2.0)
            .round()
            .max(0.0) as i32;
        dot.set_margin_end(inset);
        dot.set_margin_bottom(inset);
        overlay.add_overlay(&dot);
    }

    overlay.upcast()
}

/// Hash a username to one of the six categorical slots, stably.
fn category_for(name: &str) -> u32 {
    let mut h: u32 = 2166136261;
    for b in name.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(16777619);
    }
    // FNV alone parks similar short names next to each other (meros/melvin
    // came out 5° apart on the old hue wheel); an avalanche mix spreads them.
    h ^= h >> 16;
    h = h.wrapping_mul(0x45d9f3b);
    h ^= h >> 16;
    h % 6
}
