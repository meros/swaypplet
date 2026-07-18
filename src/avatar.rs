//! Shared user-avatar widget: a round profile picture (real image when the
//! host resolves one, a deterministic per-user monogram otherwise) with an
//! optional green presence dot for a logged-in user.
//!
//! Rendering mirrors `lock::glass`: a subclassed widget snapshots the texture
//! into a `RoundedRect` clip (cover-fit, like `ContentFit::Cover`), so the
//! image crops to a circle without a Cairo round-trip. The dummy monogram is
//! a filled hue circle plus the uppercase initial. The returned widget is an
//! `Overlay` carrying `.user-avatar`; callers mark the current user with an
//! `.active` CSS class (accent ring) and the dot is a `.user-dot` overlay.

use gtk4::{gdk, glib, graphene, gsk, pango, prelude::*, subclass::prelude::*};

mod imp {
    use super::*;
    use std::cell::RefCell;

    pub struct AvatarImage {
        pub texture: RefCell<Option<gdk::Texture>>,
        /// Filled circle colour for the monogram fallback (ignored when a
        /// texture is present).
        pub hue: RefCell<gdk::RGBA>,
        /// Uppercase initial for the monogram fallback.
        pub letter: RefCell<String>,
    }

    // Not derived: gdk::RGBA has no Default impl.
    impl Default for AvatarImage {
        fn default() -> Self {
            Self {
                texture: RefCell::new(None),
                hue: RefCell::new(gdk::RGBA::TRANSPARENT),
                letter: RefCell::new(String::new()),
            }
        }
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
            let rect = graphene::Rect::new(cx - r, cy - r, size, size);
            let clip = gsk::RoundedRect::from_rect(rect, r);

            snapshot.push_rounded_clip(&clip);
            if let Some(texture) = self.texture.borrow().as_ref() {
                // ContentFit::Cover: scale to fill the circle, centre overflow.
                let (tw, th) = (texture.width() as f32, texture.height() as f32);
                if tw > 0.0 && th > 0.0 {
                    let scale = (size / tw).max(size / th);
                    let (dw, dh) = (tw * scale, th * scale);
                    let dst = graphene::Rect::new(cx - dw / 2.0, cy - dh / 2.0, dw, dh);
                    snapshot.append_texture(texture, &dst);
                }
            } else {
                snapshot.append_color(&self.hue.borrow(), &rect);
            }
            snapshot.pop();

            // Monogram sits on top of the hue fill (never over an image).
            if self.texture.borrow().is_none() {
                let letter = self.letter.borrow();
                if !letter.is_empty() {
                    let layout = widget.create_pango_layout(Some(&letter));
                    let mut font = pango::FontDescription::new();
                    font.set_weight(pango::Weight::Semibold);
                    font.set_size((size * 0.42).round() as i32 * pango::SCALE);
                    layout.set_font_description(Some(&font));
                    let (lw, lh) = layout.pixel_size();
                    snapshot.save();
                    snapshot.translate(&graphene::Point::new(
                        cx - lw as f32 / 2.0,
                        cy - lh as f32 / 2.0,
                    ));
                    let ink = gdk::RGBA::new(0.97, 0.95, 0.89, 0.96);
                    snapshot.append_layout(&layout, &ink);
                    snapshot.restore();
                }
            }
        }
    }
}

glib::wrapper! {
    pub struct AvatarImage(ObjectSubclass<imp::AvatarImage>) @extends gtk4::Widget;
}

impl AvatarImage {
    fn new(size: i32, texture: Option<gdk::Texture>, hue: gdk::RGBA, letter: String) -> Self {
        let obj: Self = glib::Object::new();
        obj.set_size_request(size, size);
        *obj.imp().texture.borrow_mut() = texture;
        *obj.imp().hue.borrow_mut() = hue;
        *obj.imp().letter.borrow_mut() = letter;
        obj
    }
}

/// Build a round avatar for `name`. Uses `icon_path` when it loads, otherwise a
/// deterministic monogram. `size` is the diameter in px; `logged_in` adds a
/// green presence dot. The returned widget carries `.user-avatar`; the caller
/// adds `.active` to ring the current user.
pub fn avatar(name: &str, icon_path: Option<&str>, size: i32, logged_in: bool) -> gtk4::Widget {
    let texture = icon_path.and_then(|p| gdk::Texture::from_filename(p).ok());
    let letter = name
        .chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_default();
    let image = AvatarImage::new(size, texture, hue_for(name), letter);

    let overlay = gtk4::Overlay::new();
    overlay.add_css_class("user-avatar");
    overlay.set_size_request(size, size);
    overlay.set_child(Some(&image));

    if logged_in {
        let dot = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        dot.add_css_class("user-dot");
        dot.set_halign(gtk4::Align::End);
        dot.set_valign(gtk4::Align::End);
        overlay.add_overlay(&dot);
    }

    overlay.upcast()
}

/// Hash a username to a stable, muted hue (theme-friendly: mid saturation and
/// lightness so any initial stays legible on it).
fn hue_for(name: &str) -> gdk::RGBA {
    let mut h: u32 = 2166136261;
    for b in name.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(16777619);
    }
    // FNV alone parks similar short names next to each other (meros/melvin
    // came out 5° apart); an avalanche mix spreads them across the wheel.
    h ^= h >> 16;
    h = h.wrapping_mul(0x45d9f3b);
    h ^= h >> 16;
    let hue = (h % 360) as f64;
    let (r, g, b) = hsl_to_rgb(hue, 0.32, 0.42);
    gdk::RGBA::new(r as f32, g as f32, b as f32, 1.0)
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (f64, f64, f64) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r1, g1, b1) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    (r1 + m, g1 + m, b1 + m)
}
