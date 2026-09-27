//! Live pictures of workspaces: the picture itself, for `view.rs` (a pin,
//! the bar's peek) and the pins popover.
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

use super::live::{Crop, Frame, Want};
use super::scene::{self, Scene};

/// Every picture a window's frames land on, by window identifier. Each
/// workspace view keeps one, fed through `feed.rs`.
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

    /// What to capture for every window drawn: each cut to the largest box
    /// any of its pictures is drawn in, in device pixels (its own device
    /// box, or its size at `scale` device pixels to one of GTK's), and to
    /// `crop` of it when given.
    pub fn wants(&self, crop: Option<Crop>, scale: f64) -> Vec<Want> {
        self.pictures
            .iter()
            .map(|(id, pics)| {
                let (w, h) = pics.iter().fold((1, 1), |(w, h), p| {
                    let (pw, ph) = p
                        .device_box()
                        .unwrap_or_else(|| draw_size(p.width_request(), p.height_request(), scale));
                    (w.max(pw), h.max(ph))
                });
                Want {
                    id: id.clone(),
                    crop,
                    size: (w, h),
                }
            })
            .collect()
    }

    /// Device pixels to one of GTK's where the pictures are drawn, from the
    /// first one on a surface; 1 before any is.
    pub fn device_scale(&self) -> f64 {
        self.pictures
            .values()
            .flatten()
            .find(|p| p.native().is_some())
            .map_or(1.0, device_scale)
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
        let (id, texture) = remember(frame)?;
        self.show(&id, &texture);
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

/// A box `w` by `h` in GTK's pixels, in device pixels at `scale`. Rounded
/// up, so a fractional scale never cuts a frame below what it is drawn at.
fn draw_size(w: i32, h: i32, scale: f64) -> (u32, u32) {
    let px = |v: i32| (f64::from(v.max(1)) * scale.max(1.0)).ceil() as u32;
    (px(w), px(h))
}

/// Device pixels to one of GTK's where `widget` is drawn: its surface's
/// scale, fractional when the output is, or the whole-number scale factor
/// before it has a surface.
pub fn device_scale(widget: &impl IsA<gtk4::Widget>) -> f64 {
    widget
        .native()
        .and_then(|n| n.surface())
        .map_or(f64::from(widget.scale_factor()), |s| s.scale())
}

/// A frame as a texture, kept as its window's last picture ([`LAST`]).
///
/// `None` when GTK would not take the frame's dmabuf; live frames are
/// then off for the process (`gpu::refuse`), and the pictures keep what
/// they had.
pub fn remember(frame: Frame) -> Option<(String, gdk::Texture)> {
    if !super::gpu::usable() {
        // Dropped: its output buffer goes back at once.
        return None;
    }
    let texture = match frame.buffer.into_texture() {
        Ok(texture) => texture,
        Err(e) => {
            super::gpu::refuse(&e);
            return None;
        }
    };
    LAST.with(|l| {
        let mut last = l.borrow_mut();
        if last.len() >= LAST_MAX && !last.contains_key(&frame.id) {
            last.clear();
        }
        last.insert(frame.id.clone(), texture.clone());
    });
    Some((frame.id, texture))
}

/// A live picture of a workspace in a box of `w` by `h`: every window at
/// its place (`scene::fit`), each registered in `live` for its frames. An
/// empty or vanished workspace says so.
///
/// Windows are placed on the device grid at `scale`, not on GTK's: a
/// rectangle rounded to whole GTK pixels was up to a device pixel off the
/// window's shape at 2x, and its frame, contained in it, left a sliver of
/// panel at one edge. Each picture carries its box in device pixels, which
/// the capture is cut to and the frame is drawn in.
pub fn preview(scene: Option<&Scene>, w: i32, h: i32, scale: f64, live: &mut Live) -> gtk4::Widget {
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
                let rect = (
                    dx + f64::from(win.x) * s,
                    dy + f64::from(win.y) * s,
                    f64::from(win.w) * s,
                    f64::from(win.h) * s,
                );
                let ((x, y), (lw, lh), (dw, dh)) = on_grid(rect, w, h, scale);
                let (slot, pic) = window_slot(&win.app, lw, lh);
                pic.set_device_box(dw, dh);
                fixed.put(&slot, x, y);
                if let Some(id) = &win.id {
                    live.add(id.clone(), pic);
                }
            }
        }
        _ => {
            let empty = gtk4::Label::new(Some("empty"));
            crate::ui::set_text_style(&empty, crate::ui::Text::Label, crate::ui::Tone::Faint);
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

/// A window's rectangle in a `w` by `h` box, in GTK's pixels, placed on the
/// device grid at `scale`: its corner in GTK's pixels (on a device pixel),
/// its slot in whole GTK pixels, and its box in device pixels.
///
/// The slot is the device box rounded down, so the preview never asks for
/// more than `w` by `h`; the picture draws its whole device box from the
/// slot's corner, past the slot by less than one of GTK's pixels, and
/// nothing clips it (see `preview`).
fn on_grid(
    rect: (f64, f64, f64, f64),
    w: i32,
    h: i32,
    scale: f64,
) -> ((f64, f64), (i32, i32), (u32, u32)) {
    let scale = scale.max(1.0);
    let device = |v: i32| (f64::from(v) * scale).floor() as i32;
    let (x, y, dw, dh) = place_in(
        (
            rect.0 * scale,
            rect.1 * scale,
            rect.2 * scale,
            rect.3 * scale,
        ),
        device(w),
        device(h),
    );
    let slot = |v: i32| ((f64::from(v) / scale).floor() as i32).max(1);
    (
        (f64::from(x) / scale, f64::from(y) / scale),
        (slot(dw), slot(dh)),
        (dw as u32, dh as u32),
    )
}

/// A window's spot: the app icon on a panel, with the live picture over it.
fn window_slot(app: &str, w: i32, h: i32) -> (gtk4::Overlay, LivePicture) {
    let panel = crate::ui::vbox(0);
    crate::ui::placeholder::adopt(&panel);
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
    // The shadow is what lifts a picture off the desktop behind it.
    crate::ui::lifted::adopt(&slot);
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

/// Where a `tw` by `th` frame is drawn in a `w` by `h` picture on a surface
/// at `scale`, and whether it lands one frame pixel to one device pixel.
///
/// The worker cuts a frame to fit the picture in device pixels
/// (`live::out_size`), so it is drawn at its own size: one pixel to one,
/// from a corner on the device grid, with nothing for a filter to blend.
/// Stretched to fill instead, a frame one pixel short of the box was scaled
/// by 491/492 and centred on a half pixel, and the linear filter blurred
/// all of it. A frame larger than the box (a window grown since it was
/// cut) is contained: shown whole, with a sliver of the panel beside it
/// when its aspect differs from sway's rect.
fn placed((w, h): (f32, f32), (tw, th): (f32, f32), scale: f32) -> ((f32, f32, f32, f32), bool) {
    let scale = scale.max(1.0);
    let (nw, nh) = (tw / scale, th / scale);
    let exact = nw <= w + 1e-3 && nh <= h + 1e-3;
    let (dw, dh) = if exact {
        (nw, nh)
    } else {
        let s = (w / tw).min(h / th);
        (tw * s, th * s)
    };
    let snap = |v: f32| (v * scale).round() / scale;
    ((snap((w - dw) / 2.0), snap((h - dh) / 2.0), dw, dh), exact)
}

#[cfg(test)]
mod tests {
    use super::{draw_size, on_grid, place_in, placed};

    #[test]
    fn a_frame_cut_to_the_box_is_drawn_one_pixel_to_one_on_the_grid() {
        // 2x: a 396 by 491 frame in a 198 by 246 picture. Stretched, it was
        // 492 device pixels tall and began half a device pixel down.
        let ((x, y, w, h), exact) = placed((198.0, 246.0), (396.0, 491.0), 2.0);
        assert!(exact);
        assert_eq!((w * 2.0, h * 2.0), (396.0, 491.0));
        assert_eq!(((x * 2.0).fract(), (y * 2.0).fract()), (0.0, 0.0));
        // 1.5x: the corner still falls on a device pixel.
        let ((x, y, _, _), exact) = placed((200.0, 125.0), (297.0, 186.0), 1.5);
        assert!(exact);
        assert!((x * 1.5 - (x * 1.5).round()).abs() < 1e-4);
        assert!((y * 1.5 - (y * 1.5).round()).abs() < 1e-4);
    }

    #[test]
    fn a_frame_larger_than_the_box_is_contained() {
        let ((x, y, w, h), exact) = placed((200.0, 125.0), (800.0, 400.0), 2.0);
        assert!(!exact);
        assert_eq!((w, h), (200.0, 100.0));
        assert_eq!((x, y), (0.0, 12.5));
    }

    #[test]
    fn a_window_is_placed_on_the_device_grid() {
        // At 2x, a window 180.3 by 100.6 at 10.3, 20.2: to the nearest
        // device pixel, where whole GTK pixels were up to one device pixel
        // off each way.
        let ((x, y), slot, device) = on_grid((10.3, 20.2, 180.3, 100.6), 400, 250, 2.0);
        assert_eq!(device, (361, 201));
        assert_eq!((x, y), (10.5, 20.0));
        assert_eq!(slot, (180, 100));
        // At the box's edge the slot, rounded down, stays inside it.
        let ((x, _), (sw, _), (dw, _)) = on_grid((219.7, 0.0, 180.3, 50.0), 400, 250, 2.0);
        assert!(x + f64::from(sw) <= 400.0);
        assert!(x * 2.0 + f64::from(dw) <= 800.0);
        // 1.5x: the corner is on a device pixel.
        let ((x, y), _, _) = on_grid((10.3, 20.2, 100.0, 60.0), 400, 250, 1.5);
        assert_eq!(((x * 1.5).fract(), (y * 1.5).fract()), (0.0, 0.0));
    }

    #[test]
    fn a_draw_box_is_in_device_pixels_rounded_up() {
        assert_eq!(draw_size(200, 125, 2.0), (400, 250));
        assert_eq!(draw_size(201, 125, 1.5), (302, 188));
        // Never below one pixel, never below 1x.
        assert_eq!(draw_size(0, -3, 0.5), (1, 1));
    }

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
        /// The box the frame is drawn in, in device pixels, when the caller
        /// placed the picture on the device grid (`preview`); otherwise the
        /// widget's own size.
        pub device_box: std::cell::Cell<Option<(u32, u32)>>,
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
            let obj = self.obj();
            let scale = super::device_scale(&*obj) as f32;
            let (w, h) = match self.device_box.get() {
                Some((bw, bh)) => (bw as f32 / scale.max(1.0), bh as f32 / scale.max(1.0)),
                None => (obj.width() as f32, obj.height() as f32),
            };
            let (tw, th) = (texture.width() as f32, texture.height() as f32);
            if w <= 0.0 || h <= 0.0 || tw <= 0.0 || th <= 0.0 {
                return;
            }
            let ((x, y, dw, dh), exact) = super::placed((w, h), (tw, th), scale);
            snapshot.append_scaled_texture(
                texture,
                if exact {
                    gsk::ScalingFilter::Nearest
                } else {
                    gsk::ScalingFilter::Linear
                },
                &graphene::Rect::new(x, y, dw, dh),
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

    /// Draw the frame in a box of `w` by `h` device pixels from the
    /// picture's corner, which may reach past its allocation by less than
    /// one of GTK's pixels.
    pub fn set_device_box(&self, w: u32, h: u32) {
        use gtk4::subclass::prelude::ObjectSubclassIsExt;
        self.imp().device_box.set(Some((w.max(1), h.max(1))));
        self.queue_draw();
    }

    /// The box set by [`Self::set_device_box`].
    pub fn device_box(&self) -> Option<(u32, u32)> {
        use gtk4::subclass::prelude::ObjectSubclassIsExt;
        self.imp().device_box.get()
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
