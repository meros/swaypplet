//! Live pictures of workspaces, for the bar's peek and for pins.
//!
//! A picture is composed, not captured whole. Every window on the workspace
//! gets a [`LivePicture`] at the spot `scene.rs` computed, scaled into the
//! tile's fixed preview box, on a plain ground. Frames from `live.rs` land on
//! those pictures by window identifier, so each window updates on its own and
//! an idle one costs nothing after its first frame.
//!
//! Until a window's first frame arrives, its spot shows the app icon on a
//! plain panel. That is also what stays when a capture never arrives, so a
//! slow client degrades the tile and does not blank it.
//!

use std::collections::HashMap;

use gtk4::prelude::*;
use gtk4::{gdk, glib};

use super::live::Frame;
use super::scene::{self, Scene};

/// Every picture a window's frames land on, by window identifier. The
/// bar's peek and a pinned mirror each keep one, fed by their own
/// `live::Stream`.
#[derive(Default)]
pub struct Live {
    pictures: HashMap<String, Vec<LivePicture>>,
}

/// Most windows kept in [`LAST`]; past it the cache starts over, which only
/// costs a grey box until each window next draws.
const LAST_MAX: usize = 64;

thread_local! {
    /// The last picture of every window any surface has shown, by window.
    ///
    /// The compositor sends a frame only when a window has damage, so a
    /// capture started again for an idle window - a launcher row rebuilt on
    /// the next keystroke, a pin that followed focus to another output, the
    /// peek opened a second time - gets no first frame, and its picture
    /// stayed the grey placeholder until the window next drew. A new picture
    /// starts from this instead. Thumbnails, so the memory is small.
    static LAST: std::cell::RefCell<HashMap<String, gdk::Texture>> =
        std::cell::RefCell::new(HashMap::new());
}

impl Live {
    /// Every window identifier drawn, for the capture to ask for.
    pub fn window_ids(&self) -> Vec<String> {
        self.pictures.keys().cloned().collect()
    }

    /// Register a picture for a window's frames, drawn by the caller. It
    /// starts from the window's last picture, when one was ever shown.
    pub fn add(&mut self, id: String, picture: LivePicture) {
        if let Some(texture) = LAST.with(|l| l.borrow().get(&id).cloned()) {
            picture.set_texture(texture);
            // Not parented yet: the caller appends it after this returns.
            let weak = picture.downgrade();
            glib::idle_add_local_once(move || {
                if let Some(parent) = weak.upgrade().and_then(|p| p.parent()) {
                    parent.add_css_class("live");
                }
            });
        }
        self.pictures.entry(id).or_default().push(picture);
    }

    /// Put a frame on every picture of its window, and hand back the
    /// texture it became, for a caller that keeps the last one.
    pub fn frame(&self, frame: Frame) -> Option<gdk::Texture> {
        if !self.pictures.contains_key(&frame.id) {
            return None;
        }
        let texture = texture(frame.width, frame.height, frame.pixels);
        self.show(&frame.id, &texture);
        LAST.with(|l| {
            let mut last = l.borrow_mut();
            if last.len() >= LAST_MAX && !last.contains_key(&frame.id) {
                last.clear();
            }
            last.insert(frame.id.clone(), texture.clone());
        });
        Some(texture)
    }

    /// Put a texture on every picture of a window.
    pub fn show(&self, id: &str, texture: &gdk::Texture) {
        let Some(pics) = self.pictures.get(id) else {
            return;
        };
        for pic in pics {
            pic.set_texture(texture.clone());
            // The icon under it stops being visible once there are pixels;
            // the class lets the stylesheet drop the placeholder's panel too.
            if let Some(parent) = pic.parent() {
                parent.add_css_class("live");
            }
        }
    }
}

/// Premultiplied BGRA, tightly packed, as `live::Frame` carries it.
pub fn texture(width: u32, height: u32, pixels: Vec<u8>) -> gdk::Texture {
    let bytes = glib::Bytes::from_owned(pixels);
    gdk::MemoryTexture::new(
        width as i32,
        height as i32,
        gdk::MemoryFormat::B8g8r8a8Premultiplied,
        &bytes,
        (width * 4) as usize,
    )
    .upcast()
}

/// A live picture of a workspace in a box of `w` by `h`: every window at
/// its place (`scene::fit`), each registered in `live` for its frames. An
/// empty or vanished workspace says so.
pub fn preview(scene: Option<&Scene>, w: i32, h: i32, live: &mut Live) -> gtk4::Widget {
    // No clip anywhere in a picture. Under the switcher's 3D transform, GTK
    // draws a clipped node offscreen first, at a scale it estimates from
    // the transform, and for a place turned left that estimate is low: its
    // pictures came out blurred (carousel.rs, `placed`). So the box is a
    // plain `Fixed`, and every window is kept inside it by arithmetic
    // instead: `scene::fit` never crops, and the rectangle is clamped here
    // against rounding and the 4 px floor, so the `Fixed` never asks for
    // more than `w` by `h`.
    let fixed = gtk4::Fixed::new();
    fixed.add_css_class("jump-preview");
    fixed.set_size_request(w, h);
    fixed.set_halign(gtk4::Align::Center);
    fixed.set_valign(gtk4::Align::Start);

    match scene {
        Some(scene) if !scene.windows.is_empty() => {
            let (s, dx, dy) = scene::fit(scene.width, scene.height, w, h);
            for win in &scene.windows {
                let (x, y, ww, wh) = place_in(
                    (
                        dx + f64::from(win.x) * s,
                        dy + f64::from(win.y) * s,
                        f64::from(win.w) * s,
                        f64::from(win.h) * s,
                    ),
                    w,
                    h,
                );
                let (slot, pic) = window_slot(&win.app, ww, wh);
                fixed.put(&slot, f64::from(x), f64::from(y));
                if let Some(id) = &win.id {
                    live.add(id.clone(), pic);
                }
            }
        }
        _ => {
            let empty = gtk4::Label::new(Some("empty"));
            empty.add_css_class("jump-empty");
            empty.set_size_request(w, h);
            fixed.put(&empty, 0.0, 0.0);
        }
    }
    fixed.upcast()
}

/// A window's rectangle in whole pixels, at least 4 by 4 so a tiny float is
/// still a mark, and inside the `w` by `h` box.
fn place_in((x, y, ww, wh): (f64, f64, f64, f64), w: i32, h: i32) -> (i32, i32, i32, i32) {
    let ww = (ww.round() as i32).clamp(4.min(w), w);
    let wh = (wh.round() as i32).clamp(4.min(h), h);
    let x = (x.round() as i32).clamp(0, w - ww);
    let y = (y.round() as i32).clamp(0, h - wh);
    (x, y, ww, wh)
}

/// A window's spot: the app icon on a panel, with the live picture over it.
fn window_slot(app: &str, w: i32, h: i32) -> (gtk4::Overlay, LivePicture) {
    let panel = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    panel.add_css_class("jump-window-panel");
    panel.set_size_request(w, h);
    let icon = gtk4::Image::from_icon_name(&icon_name(app));
    icon.set_pixel_size((w.min(h) / 2).clamp(12, 40));
    icon.set_vexpand(true);
    icon.set_valign(gtk4::Align::Center);
    panel.append(&icon);

    let pic = LivePicture::new();
    pic.set_size_request(w, h);

    // Square, not clipped to rounded corners: see `preview`.
    let slot = gtk4::Overlay::new();
    slot.add_css_class("jump-window");
    slot.set_child(Some(&panel));
    slot.add_overlay(&pic);
    (slot, pic)
}

fn icon_name(app: &str) -> String {
    let lower = app.to_lowercase();
    let known = gdk::Display::default()
        .map(|display| gtk4::IconTheme::for_display(&display).has_icon(&lower))
        .unwrap_or(false);
    if known {
        lower
    } else {
        "application-x-executable".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::place_in;

    #[test]
    fn a_window_is_kept_inside_the_box() {
        // Rounding up at the right edge.
        assert_eq!(
            place_in((219.6, 0.0, 220.6, 100.0), 440, 275),
            (219, 0, 221, 100)
        );
        // The 4 px floor at the bottom edge.
        assert_eq!(place_in((10.0, 274.0, 1.0, 1.0), 440, 275), (10, 271, 4, 4));
        // A window as big as the box.
        assert_eq!(
            place_in((0.0, 0.0, 440.4, 275.4), 440, 275),
            (0, 0, 440, 275)
        );
    }
}

mod picture_imp {
    use std::cell::RefCell;

    use gtk4::prelude::*;
    use gtk4::subclass::prelude::*;
    use gtk4::{gdk, glib, graphene, gsk};

    #[derive(Default)]
    pub struct LivePicture {
        pub texture: RefCell<Option<gdk::Texture>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for LivePicture {
        const NAME: &'static str = "SwayppletLivePicture";
        type Type = super::LivePicture;
        type ParentType = gtk4::Widget;
    }

    impl ObjectImpl for LivePicture {}

    impl WidgetImpl for LivePicture {
        fn snapshot(&self, snapshot: &gtk4::Snapshot) {
            let Some(texture) = &*self.texture.borrow() else {
                return;
            };
            let (w, h) = (self.obj().width() as f32, self.obj().height() as f32);
            let (tw, th) = (texture.width() as f32, texture.height() as f32);
            if w <= 0.0 || h <= 0.0 || tw <= 0.0 || th <= 0.0 {
                return;
            }
            // Contain: a capture whose aspect differs from sway's rect
            // (client-side shadows, a resize in flight) shows whole, with a
            // sliver of the panel beside it, and loses none of its edges.
            let s = (w / tw).min(h / th);
            let (dw, dh) = (tw * s, th * s);
            snapshot.append_scaled_texture(
                texture,
                gsk::ScalingFilter::Linear,
                &graphene::Rect::new((w - dw) / 2.0, (h - dh) / 2.0, dw, dh),
            );
        }
    }
}

glib::wrapper! {
    /// A window's live frame, drawn with linear filtering.
    ///
    /// Not a `GtkPicture`, whose texture node lets GTK pick a mipmap level
    /// from the scale it estimates for the transform above it. Under the
    /// switcher's perspective that estimate is badly low for a place turned
    /// left (0.26 where the place is drawn at about 1: `graphene_matrix_
    /// decompose` on a perspective matrix depends on which way the plane
    /// turns), so its frames were sampled from a quarter-size mip and came
    /// out blurred while the right side stayed sharp. Linear filtering uses
    /// no mipmaps and samples the frame itself. The frames are already cut
    /// to about the size they are drawn at (`live.rs`), so nothing is left
    /// for mipmaps to smooth.
    pub struct LivePicture(ObjectSubclass<picture_imp::LivePicture>)
        @extends gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

impl LivePicture {
    pub fn new() -> Self {
        glib::Object::new()
    }

    pub fn set_texture(&self, texture: gdk::Texture) {
        use gtk4::subclass::prelude::ObjectSubclassIsExt;
        self.imp().texture.replace(Some(texture));
        self.queue_draw();
    }
}

impl Default for LivePicture {
    fn default() -> Self {
        Self::new()
    }
}
