//! Pick a region: a frozen screen you drag a rectangle on.
//!
//! This is the half of a screenshot that `slurp` used to be, and it does one
//! thing slurp cannot. Slurp draws on the live screen, so what you select and
//! what gets captured are two different moments — a notification arriving
//! between them lands in the file. Here the capture happens *first*, the
//! selector shows that frozen image, and the region is a crop of the bytes you
//! were looking at when you dragged.
//!
//! Freezing is also what makes a colour picker possible without a second tool:
//! the pixel under the pointer is already in hand (`super::capture::Image`),
//! so `hyprpicker` has nothing left to do.
//!
//! One surface per output, each showing its own capture. The surface is the
//! only one in the shell that must *not* be frosted — the whole point is
//! seeing the screen — so `swaypplet-screenshot` is deliberately absent from
//! the compositor's `layer_effects` list rather than disabled at map.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::gdk;
use gtk4::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

use crate::layer_shell::{self, LayerShellConfig};

use super::capture::Image;

/// A chosen rectangle, in the buffer pixels of the output it was drawn on.
pub struct Selection {
    /// Which output it came from. Unused by the current callers and kept
    /// because a multi-output shot has to be able to say where it was taken.
    #[allow(dead_code)]
    pub output: String,
    pub image: Image,
}

/// A drag in progress: the two corners, in widget coordinates.
type Rect = (f64, f64, f64, f64);

/// The one-shot answer the selector was opened for.
type Done = Box<dyn FnOnce(Option<Selection>)>;

/// What the pointer is being used for.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Drag out a rectangle; a click without a drag takes the whole output.
    Region,
    /// One click reports the colour under the pointer instead of a crop.
    Pick,
}

/// The frozen screen of one output, with the surface showing it.
struct Sheet {
    output: String,
    image: Image,
    window: gtk4::Window,
    area: gtk4::DrawingArea,
    /// Drag rectangle in widget (logical) coordinates, `None` until a drag
    /// starts. Shared with the draw function.
    rect: Rc<RefCell<Option<Rect>>>,
}

impl Sheet {
    /// Buffer pixels per logical pixel, for this output's capture and surface.
    fn ratio(&self) -> (f64, f64) {
        ratio(
            (f64::from(self.image.width), f64::from(self.image.height)),
            f64::from(self.area.width()),
            f64::from(self.area.height()),
        )
    }
}

/// Everything the selector needs to answer once and then get out of the way.
struct Session {
    sheets: Vec<Sheet>,
    /// Dropped after it fires, so a second Escape cannot answer twice.
    done: RefCell<Option<Done>>,
}

/// Freeze every output and let the owner drag a region on one of them.
///
/// `done` runs exactly once: with the crop, or with `None` if the selection
/// was cancelled or ended up empty. Captures happen on worker threads, so the
/// call returns immediately and the surfaces appear when the pixels land.
pub fn region(app: &gtk4::Application, mode: Mode, done: impl FnOnce(Option<Selection>) + 'static) {
    let monitors = monitors();
    if monitors.is_empty() {
        done(None);
        return;
    }

    // Capture every output before showing anything: a surface that mapped
    // while its neighbour was still being copied would appear in the
    // neighbour's screenshot.
    let pending = Rc::new(RefCell::new(Vec::new()));
    let remaining = Rc::new(std::cell::Cell::new(monitors.len()));
    let done = Rc::new(RefCell::new(Some(Box::new(done) as Box<dyn FnOnce(_)>)));

    for name in monitors {
        let pending = pending.clone();
        let remaining = remaining.clone();
        let done = done.clone();
        let app = app.clone();
        let for_thread = name.clone();

        crate::spawn::spawn_work(
            move || super::capture::output(&for_thread),
            move |result| {
                match result {
                    Ok(image) => pending.borrow_mut().push((name, image)),
                    Err(e) => log::warn!("screenshot: {e}"),
                }
                remaining.set(remaining.get() - 1);
                if remaining.get() > 0 {
                    return;
                }
                let captured = std::mem::take(&mut *pending.borrow_mut());
                let Some(done) = done.borrow_mut().take() else {
                    return;
                };
                if captured.is_empty() {
                    done(None);
                    return;
                }
                present(&app, mode, captured, done);
            },
        );
    }
}

/// Connector name of every monitor GDK knows about.
///
/// No scale factor travels with it. `gdk::Monitor::scale_factor()` is an
/// integer, so an output running Sway's fractional 1.5 answers 2; the number
/// a crop needs is measured later instead, by `Sheet::ratio`.
fn monitors() -> Vec<String> {
    let Some(display) = gdk::Display::default() else {
        return Vec::new();
    };
    display
        .monitors()
        .iter::<gdk::Monitor>()
        .filter_map(Result::ok)
        .filter_map(|m| m.connector().map(|c| c.to_string()))
        .collect()
}

fn present(app: &gtk4::Application, mode: Mode, captured: Vec<(String, Image)>, done: Done) {
    static CONFIG: LayerShellConfig = LayerShellConfig {
        // Absent from the compositor's layer_effects list on purpose: this is
        // the one surface that must show the screen, not frost it.
        namespace: "swaypplet-screenshot",
        layer: Layer::Overlay,
        exclusive: false,
        default_width: None,
        default_height: None,
        anchors: &[
            (Edge::Top, true),
            (Edge::Bottom, true),
            (Edge::Left, true),
            (Edge::Right, true),
        ],
        margins: &[],
        // Escape has to reach the selector even though the panel below it may
        // want the keyboard.
        keyboard_mode: KeyboardMode::Exclusive,
    };

    let display = gdk::Display::default();
    let mut sheets = Vec::new();

    for (output, image) in captured {
        let monitor = display.as_ref().and_then(|d| {
            d.monitors()
                .iter::<gdk::Monitor>()
                .filter_map(Result::ok)
                .find(|m| m.connector().is_some_and(|c| c == output))
        });
        let window = layer_shell::create_layer_window_on(app, &CONFIG, monitor.as_ref());
        window.set_decorated(false);
        // Ignore everyone else's exclusive zone. Anchored to all four edges
        // with the default zone of 0, the surface is shrunk by the bar's
        // reserved strip, which leaves the bar unselectable and un-dimmed
        // above a selector claiming to cover the screen.
        window.set_exclusive_zone(-1);
        // Fully opaque: the frozen screen is the background, and the near-unity
        // opacity the other surfaces need for compositor blending would show
        // the live screen through the still one.
        window.set_opacity(1.0);

        let texture = texture_for(&image);
        let picture = gtk4::Picture::for_paintable(&texture);
        picture.set_content_fit(gtk4::ContentFit::Fill);

        // Both expands: an overlay child is given its natural size, and a
        // DrawingArea's natural size is nothing at all — which is a dimming
        // layer that silently covers zero pixels.
        let area = gtk4::DrawingArea::builder()
            .hexpand(true)
            .vexpand(true)
            .build();
        area.set_cursor(gdk::Cursor::from_name("crosshair", None).as_ref());

        let overlay = gtk4::Overlay::new();
        overlay.set_child(Some(&picture));
        overlay.add_overlay(&area);
        window.set_child(Some(&overlay));

        sheets.push(Sheet {
            output,
            image,
            window,
            area,
            rect: Rc::new(RefCell::new(None)),
        });
    }

    let session = Rc::new(Session {
        sheets,
        done: RefCell::new(Some(done)),
    });

    for index in 0..session.sheets.len() {
        wire(&session, index, mode);
    }
    for sheet in &session.sheets {
        seed_rect(sheet);
        sheet.window.present();
    }
}

/// Dev hook: `SWPP_SELECT_RECT=x,y,w,h` starts the selector with a rectangle
/// already drawn, so the headless harness can screenshot the selection chrome
/// it has no pointer to drag out. Ignored when unset, which is always in a
/// real session.
fn seed_rect(sheet: &Sheet) {
    let Ok(spec) = std::env::var("SWPP_SELECT_RECT") else {
        return;
    };
    let parts: Vec<f64> = spec
        .split(',')
        .filter_map(|n| n.trim().parse().ok())
        .collect();
    if let [x, y, w, h] = parts[..] {
        *sheet.rect.borrow_mut() = Some((x, y, x + w, y + h));
        sheet.area.queue_draw();
    }
}

fn texture_for(image: &Image) -> gdk::MemoryTexture {
    gdk::MemoryTexture::new(
        image.width as i32,
        image.height as i32,
        gdk::MemoryFormat::R8g8b8a8,
        &glib::Bytes::from(&image.pixels),
        (image.width * 4) as usize,
    )
}

/// Hook up drawing, dragging, and the keys that end the session.
fn wire(session: &Rc<Session>, index: usize, mode: Mode) {
    let sheet = &session.sheets[index];

    // ── Drawing ──
    let rect = sheet.rect.clone();
    let buffer = (f64::from(sheet.image.width), f64::from(sheet.image.height));
    sheet.area.set_draw_func(move |_, cr, w, h| {
        let (w, h) = (f64::from(w), f64::from(h));
        // Dim everything, then clear the selection back to fully transparent
        // so the frozen screen below shows through at full brightness.
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.42);
        let _ = cr.paint();

        let Some((x, y, rw, rh)) = normalized(&rect.borrow(), w, h) else {
            return;
        };
        cr.set_operator(cairo::Operator::Clear);
        cr.rectangle(x, y, rw, rh);
        let _ = cr.fill();

        cr.set_operator(cairo::Operator::Over);
        // Gruvbox aqua, the shell's accent — same colour the bar uses for
        // "this is the thing you are acting on".
        cr.set_source_rgb(0.408, 0.616, 0.416);
        cr.set_line_width(1.0);
        cr.rectangle(x + 0.5, y + 0.5, rw - 1.0, rh - 1.0);
        let _ = cr.stroke();

        draw_size_chip(cr, ratio(buffer, w, h), x, y, rw, rh, h);
    });

    // ── Dragging ──
    let drag = gtk4::GestureDrag::new();
    let start = Rc::new(std::cell::Cell::new((0.0, 0.0)));

    let rect = sheet.rect.clone();
    let area = sheet.area.clone();
    let start_c = start.clone();
    drag.connect_drag_begin(move |_, x, y| {
        start_c.set((x, y));
        *rect.borrow_mut() = Some((x, y, x, y));
        area.queue_draw();
    });

    let rect = sheet.rect.clone();
    let area = sheet.area.clone();
    let start_c = start.clone();
    drag.connect_drag_update(move |_, dx, dy| {
        let (sx, sy) = start_c.get();
        *rect.borrow_mut() = Some((sx, sy, sx + dx, sy + dy));
        area.queue_draw();
    });

    let session_c = session.clone();
    let start_c = start.clone();
    drag.connect_drag_end(move |_, dx, dy| {
        let (sx, sy) = start_c.get();
        session_c.finish(index, mode, (sx, sy), (sx + dx, sy + dy));
    });
    sheet.area.add_controller(drag);

    // ── Keys ──
    let keys = gtk4::EventControllerKey::new();
    let session_c = session.clone();
    keys.connect_key_pressed(move |_, key, _, _| {
        match key {
            gdk::Key::Escape => session_c.cancel(),
            // Enter with nothing dragged takes the whole output, which is the
            // fastest path to "the screen as it is".
            gdk::Key::Return | gdk::Key::KP_Enter => session_c.whole(index),
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    });
    sheet.window.add_controller(keys);
}

/// A drag rectangle as origin + size, dropped if it is too small to mean a
/// deliberate region.
fn normalized(rect: &Option<Rect>, max_w: f64, max_h: f64) -> Option<Rect> {
    let (x0, y0, x1, y1) = (*rect)?;
    let x = x0.min(x1).clamp(0.0, max_w);
    let y = y0.min(y1).clamp(0.0, max_h);
    let w = (x1 - x0).abs().min(max_w - x);
    let h = (y1 - y0).abs().min(max_h - y);
    (w >= 1.0 && h >= 1.0).then_some((x, y, w, h))
}

/// The pixel count, printed just outside the rectangle so it never covers the
/// thing being measured — and inside it when the rectangle is at the top edge.
fn draw_size_chip(
    cr: &cairo::Context,
    (sx, sy): (f64, f64),
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    max_h: f64,
) {
    let text = format!("{} × {}", (w * sx) as i64, (h * sy) as i64);
    cr.select_font_face(
        "monospace",
        cairo::FontSlant::Normal,
        cairo::FontWeight::Normal,
    );
    cr.set_font_size(12.0);
    let Ok(extents) = cr.text_extents(&text) else {
        return;
    };

    let pad = 5.0;
    let cw = extents.width() + pad * 2.0;
    let ch = 20.0;
    let cx = x;
    let cy = if y >= ch + 4.0 {
        y - ch - 4.0
    } else if y + h + ch + 4.0 <= max_h {
        y + h + 4.0
    } else {
        y + 4.0
    };

    cr.set_source_rgba(0.114, 0.106, 0.102, 0.92);
    cr.rectangle(cx, cy, cw, ch);
    let _ = cr.fill();
    cr.set_source_rgb(0.922, 0.859, 0.698);
    cr.move_to(cx + pad, cy + ch - 6.0);
    let _ = cr.show_text(&text);
}

impl Session {
    /// Tear every surface down before answering: the caller may put something
    /// on screen, and it must not land on top of a selector still mapped.
    fn answer(&self, selection: Option<Selection>) {
        let Some(done) = self.done.borrow_mut().take() else {
            return;
        };
        for sheet in &self.sheets {
            sheet.window.set_visible(false);
            sheet.window.close();
        }
        done(selection);
    }

    fn cancel(&self) {
        self.answer(None);
    }

    /// The whole output, uncropped.
    fn whole(&self, index: usize) {
        let sheet = &self.sheets[index];
        self.answer(Some(Selection {
            output: sheet.output.clone(),
            image: sheet.image.clone(),
        }));
    }

    /// Resolve a finished drag into a crop.
    ///
    /// A click that never moved is not an empty selection but a request for
    /// the whole output — the same gesture slurp treats as a cancel, which is
    /// the wrong default for a button labelled "screenshot".
    fn finish(&self, index: usize, mode: Mode, from: (f64, f64), to: (f64, f64)) {
        let sheet = &self.sheets[index];

        if mode == Mode::Pick {
            // A one-pixel crop carries the colour, so the picker and the
            // region share one result type; the caller reads `pixel(0, 0)`.
            let (px, py) = to_buffer(from, sheet.ratio());
            let image = sheet.image.crop(px as i32, py as i32, 1, 1);
            self.answer((!image.pixels.is_empty()).then(|| Selection {
                output: sheet.output.clone(),
                image,
            }));
            return;
        }

        let rect = Some((from.0, from.1, to.0, to.1));
        let (w, h) = (
            f64::from(sheet.area.width()),
            f64::from(sheet.area.height()),
        );
        match normalized(&rect, w, h) {
            Some((x, y, rw, rh)) => {
                let (sx, sy) = sheet.ratio();
                let (bx, by) = to_buffer((x, y), (sx, sy));
                let image = sheet.image.crop(
                    bx as i32,
                    by as i32,
                    (rw * sx).round() as u32,
                    (rh * sy).round() as u32,
                );
                if image.pixels.is_empty() {
                    self.answer(None);
                } else {
                    self.answer(Some(Selection {
                        output: sheet.output.clone(),
                        image,
                    }));
                }
            }
            None => self.whole(index),
        }
    }
}

/// Logical widget coordinates to the buffer pixels the capture is in.
fn to_buffer((x, y): (f64, f64), (sx, sy): (f64, f64)) -> (u32, u32) {
    (
        (x * sx).round().max(0.0) as u32,
        (y * sy).round().max(0.0) as u32,
    )
}

/// Buffer pixels per logical pixel, measured from the capture against the
/// surface showing it.
///
/// Asking GDK gives the wrong answer on a fractionally scaled output:
/// `scale_factor()` is an integer, so Sway's 1.5 comes back as 2 and every
/// crop lands 4/3 too large, drifting further from the drag the nearer the
/// pointer gets to the far edge. A 3840×2160 capture on a 2560×1440 surface
/// says 1.5 without anyone having to ask.
///
/// Per axis, because a capture whose aspect ratio disagrees with its surface
/// should skew the crop the same way the `ContentFit::Fill` picture skews the
/// image the user is dragging on.
fn ratio((bw, bh): (f64, f64), w: f64, h: f64) -> (f64, f64) {
    let sx = if w > 0.0 { bw / w } else { 1.0 };
    let sy = if h > 0.0 { bh / h } else { 1.0 };
    (sx, sy)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drag_normalizes_whichever_way_it_was_drawn() {
        let up_left = normalized(&Some((90.0, 80.0, 10.0, 20.0)), 100.0, 100.0);
        let down_right = normalized(&Some((10.0, 20.0, 90.0, 80.0)), 100.0, 100.0);
        assert_eq!(up_left, Some((10.0, 20.0, 80.0, 60.0)));
        assert_eq!(up_left, down_right);
    }

    #[test]
    fn a_click_without_a_drag_is_not_a_rectangle() {
        assert_eq!(
            normalized(&Some((10.0, 10.0, 10.0, 10.0)), 100.0, 100.0),
            None
        );
    }

    #[test]
    fn a_drag_off_the_edge_stays_on_the_output() {
        let r = normalized(&Some((50.0, 50.0, 300.0, 300.0)), 100.0, 100.0);
        assert_eq!(r, Some((50.0, 50.0, 50.0, 50.0)));
    }

    #[test]
    fn logical_coordinates_scale_up_to_buffer_pixels() {
        assert_eq!(to_buffer((10.0, 20.0), (2.0, 2.0)), (20, 40));
        assert_eq!(to_buffer((10.0, 20.0), (1.0, 1.0)), (10, 20));
    }

    #[test]
    fn a_fractional_output_measures_its_own_ratio() {
        // 3840×2160 captured, shown on a 2560×1440 surface: Sway scale 1.5,
        // which gdk::Monitor::scale_factor() would have called 2.
        assert_eq!(ratio((3840.0, 2160.0), 2560.0, 1440.0), (1.5, 1.5));
        assert_eq!(to_buffer((1000.0, 500.0), (1.5, 1.5)), (1500, 750));
    }

    #[test]
    fn an_integer_output_is_unchanged() {
        assert_eq!(ratio((2880.0, 1800.0), 1440.0, 900.0), (2.0, 2.0));
    }

    #[test]
    fn an_unmapped_surface_does_not_divide_by_zero() {
        assert_eq!(ratio((3840.0, 2160.0), 0.0, 0.0), (1.0, 1.0));
    }
}
