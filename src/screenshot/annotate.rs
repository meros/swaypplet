//! Draw on a capture before it leaves.
//!
//! Four tools, chosen by what a screenshot is actually for: a box and an arrow
//! to say *look here*, a pen for everything those two are too rigid for, and a
//! pixelate to say *not that*. Redaction earns its place over the prettier
//! options — a screenshot of a terminal or a browser is the most common way a
//! token or an address gets shared by accident, and the moment to catch it is
//! while looking at the picture.
//!
//! Strokes are kept as a list, not baked into the pixels, so undo is dropping
//! the last one and the export is a replay. The canvas draws at whatever size
//! the window is; the export replays at the capture's own resolution, so
//! annotating a 2880-wide shot in a 1400-wide window still writes 2880 pixels.
//!
//! The capture is a texture, uploaded once and scaled by the GPU; only the
//! strokes are drawn with cairo, on a transparent layer over it. The canvas
//! used to rebuild the capture as a cairo surface on every draw, which is
//! every pointer motion while drawing: 20 MB made and scaled in software per
//! event at 2x.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gtk4::gdk;
use gtk4::prelude::*;

use super::capture::Image;

/// Colours a mark can be, in the order they appear in the palette: red,
/// yellow, green, blue and the text colour.
///
/// From the tokens, like the rest of the shell, and the readable tones of
/// them (`--danger`, `--cat-n`, `--fg`), not the fills: a mark has to
/// survive being drawn on top of an arbitrary screenshot, which the muted
/// fills do not reliably do. Read once per editor, in the mode it opened in.
fn palette() -> [(f64, f64, f64); 5] {
    let paint = crate::theme::paint();
    let [_, yellow, blue, _, green, _] = paint.categorical;
    [paint.status.danger, yellow, green, blue, paint.fg].map(|c| (c.0, c.1, c.2))
}

const STROKE_WIDTH: f64 = 3.0;

/// How coarse a pixelated block is, as a fraction of the shorter edge — so a
/// redaction stays unreadable whether it covers a word or a window.
const PIXELATE_DIVISOR: u32 = 90;
const PIXELATE_MIN: u32 = 6;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Pen,
    Highlight,
    Box_,
    Arrow,
    Pixelate,
}

/// One mark, in the capture's own pixel coordinates so the export needs no
/// transform of its own.
#[derive(Clone)]
struct Stroke {
    tool: Tool,
    colour: (f64, f64, f64),
    /// Freehand path for `Pen` and `Highlight`; first and last point define the rest.
    points: Vec<(f64, f64)>,
    /// A pixelate's block averages, by block, so each is read from the
    /// capture once: a drag only adds the blocks at its growing edge.
    blocks: Blocks,
}

/// Block rectangle (x, y, right, bottom) to its average colour.
type BlockMap = HashMap<(u32, u32, u32, u32), (f64, f64, f64)>;
type Blocks = Rc<RefCell<BlockMap>>;

struct Editor {
    image: Image,
    strokes: RefCell<Vec<Stroke>>,
    /// The stroke being dragged, drawn but not yet committed.
    live: RefCell<Option<Stroke>>,
    tool: std::cell::Cell<Tool>,
    colour: std::cell::Cell<usize>,
    palette: [(f64, f64, f64); 5],
    /// The strokes' layer, over the capture's picture.
    area: gtk4::DrawingArea,
    window: gtk4::Window,
}

/// Open the editor on a capture. `done` gets the annotated image when the
/// owner keeps it, and nothing when they close the window.
pub fn open(app: &gtk4::Application, image: Image, done: impl Fn(Image) + 'static) {
    let window = gtk4::Window::builder()
        .application(app)
        .title("Annotate")
        .default_width(1100)
        .default_height(760)
        .build();
    // Checkerboard-free: a screenshot is opaque, so the canvas only ever
    // needs somewhere neutral to letterbox against.
    let canvas = gtk4::Overlay::new();
    canvas.set_hexpand(true);
    canvas.set_vexpand(true);
    crate::ui::canvas::adopt(&canvas);
    // The capture, fitted and never enlarged, as `Editor::scale` computes:
    // the strokes' layer maps through the same numbers.
    let picture = gtk4::Picture::for_paintable(&texture(&image));
    picture.set_content_fit(gtk4::ContentFit::ScaleDown);
    picture.set_can_shrink(true);
    canvas.set_child(Some(&picture));
    let area = gtk4::DrawingArea::new();
    canvas.add_overlay(&area);

    let editor = Rc::new(Editor {
        image,
        strokes: RefCell::new(Vec::new()),
        live: RefCell::new(None),
        tool: std::cell::Cell::new(Tool::Box_),
        colour: std::cell::Cell::new(0),
        palette: palette(),
        area: area.clone(),
        window: window.clone(),
    });

    let root = crate::ui::vbox(0);
    crate::ui::window::adopt(&root);
    root.append(&toolbar(&editor, done));
    root.append(&canvas);
    window.set_child(Some(&root));

    editor.wire();
    window.present();
}

fn toolbar(editor: &Rc<Editor>, done: impl Fn(Image) + 'static) -> gtk4::Box {
    let bar = crate::ui::toolbar(2);
    bar.add_css_class("annotate-toolbar");

    // Tools are a radio group: exactly one is active, and which one is the
    // single most important thing the bar says.
    let mut first: Option<gtk4::ToggleButton> = None;
    for (tool, icon, name) in [
        (Tool::Box_, "󰆠", "Box (B)"),
        (Tool::Arrow, "󰁚", "Arrow (A)"),
        (Tool::Pen, "󰏫", "Pen (P)"),
        (Tool::Highlight, "󰚄", "Highlight (H)"),
        (Tool::Pixelate, "󰸉", "Pixelate (X)"),
    ] {
        let face = gtk4::Label::new(Some(icon));
        crate::ui::glyph::adopt(&face, crate::ui::Text::TitleSm, crate::ui::Tone::Fg);
        let btn = crate::ui::toggle_button(
            crate::ui::Face::Icon {
                child: face.upcast_ref(),
                tooltip: name,
            },
            crate::ui::Kind::Flat,
            crate::ui::Size::Normal,
        );
        match &first {
            Some(group) => btn.set_group(Some(group)),
            None => first = Some(btn.clone()),
        }
        btn.set_active(tool == Tool::Box_);
        let editor = editor.clone();
        btn.connect_toggled(move |b| {
            if b.is_active() {
                editor.tool.set(tool);
            }
        });
        bar.append(&btn);
    }

    bar.append(&separator());

    let mut swatch_group: Option<gtk4::ToggleButton> = None;
    for (index, colour) in editor.palette.iter().enumerate() {
        // The swatch is drawn, not styled. A per-widget CSS provider loses to
        // the display-wide stylesheet at the same priority, which is how the
        // first attempt produced five identical grey buttons.
        let (r, g, b) = *colour;
        let dot = gtk4::DrawingArea::builder()
            .content_width(16)
            .content_height(16)
            .build();
        dot.set_draw_func(move |_, cr, w, h| {
            let (w, h) = (f64::from(w), f64::from(h));
            let radius = w.min(h) / 2.0;
            cr.set_source_rgb(r, g, b);
            cr.arc(w / 2.0, h / 2.0, radius, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
        });

        let btn = crate::ui::swatch(&dot);
        btn.set_valign(gtk4::Align::Center);
        match &swatch_group {
            Some(group) => btn.set_group(Some(group)),
            None => swatch_group = Some(btn.clone()),
        }
        btn.set_active(index == 0);
        let editor = editor.clone();
        btn.connect_toggled(move |b| {
            if b.is_active() {
                editor.colour.set(index);
            }
        });
        bar.append(&btn);
    }

    bar.append(&separator());

    let undo = crate::ui::button("Undo", crate::ui::Kind::Secondary);
    let editor_c = editor.clone();
    undo.connect_clicked(move |_| editor_c.undo());
    bar.append(&undo);

    // The right-hand pair: the toolbar's left half changes the drawing, its
    // right half ends the session.
    let spacer = crate::ui::hbox(0);
    spacer.set_hexpand(true);
    bar.append(&spacer);

    // The one primary action on the bar (§1.2).
    let keep = crate::ui::button("Copy & save", crate::ui::Kind::Primary);
    let editor_c = editor.clone();
    keep.connect_clicked(move |_| {
        done(editor_c.export());
        editor_c.window.close();
    });
    bar.append(&keep);

    let discard = crate::ui::button("Discard edits", crate::ui::Kind::Secondary);
    let editor_c = editor.clone();
    discard.connect_clicked(move |_| editor_c.window.close());
    bar.append(&discard);

    bar
}

fn separator() -> gtk4::Box {
    crate::ui::separator(gtk4::Orientation::Vertical)
}

impl Editor {
    fn wire(self: &Rc<Self>) {
        let this = self.clone();
        self.area.set_draw_func(move |_, cr, w, h| {
            this.draw(cr, f64::from(w), f64::from(h));
        });

        let drag = gtk4::GestureDrag::new();
        let start = Rc::new(std::cell::Cell::new((0.0, 0.0)));

        let this = self.clone();
        let start_c = start.clone();
        drag.connect_drag_begin(move |_, x, y| {
            let p = this.to_image((x, y));
            start_c.set(p);
            *this.live.borrow_mut() = Some(Stroke {
                tool: this.tool.get(),
                colour: this.palette[this.colour.get()],
                points: vec![p],
                blocks: Blocks::default(),
            });
            this.area.queue_draw();
        });

        let this = self.clone();
        let start_c = start.clone();
        drag.connect_drag_update(move |_, dx, dy| {
            let (sx, sy) = start_c.get();
            let scale = this.scale();
            let p = (sx + dx / scale, sy + dy / scale);
            if let Some(live) = this.live.borrow_mut().as_mut() {
                if live.tool == Tool::Pen || live.tool == Tool::Highlight {
                    live.points.push(p);
                } else {
                    live.points.truncate(1);
                    live.points.push(p);
                }
            }
            this.area.queue_draw();
        });

        let this = self.clone();
        drag.connect_drag_end(move |_, _, _| {
            let finished = this.live.borrow_mut().take();
            // A click that never moved leaves a zero-size box behind; drop it
            // rather than making Undo the price of a misclick.
            if let Some(stroke) = finished.filter(|s| s.points.len() > 1) {
                this.strokes.borrow_mut().push(stroke);
            }
            this.area.queue_draw();
        });
        self.area.add_controller(drag);

        let keys = gtk4::EventControllerKey::new();
        let this = self.clone();
        keys.connect_key_pressed(move |_, key, _, state| {
            if state.contains(gdk::ModifierType::CONTROL_MASK)
                && key == gdk::Key::z {
                    this.undo();
                    return glib::Propagation::Stop;
                }
            match key {
                gdk::Key::b | gdk::Key::B => {
                    this.tool.set(Tool::Box_);
                    this.area.queue_draw();
                }
                gdk::Key::a | gdk::Key::A => {
                    this.tool.set(Tool::Arrow);
                    this.area.queue_draw();
                }
                gdk::Key::p | gdk::Key::P => {
                    this.tool.set(Tool::Pen);
                    this.area.queue_draw();
                }
                gdk::Key::h | gdk::Key::H => {
                    this.tool.set(Tool::Highlight);
                    this.area.queue_draw();
                }
                gdk::Key::x | gdk::Key::X => {
                    this.tool.set(Tool::Pixelate);
                    this.area.queue_draw();
                }
                gdk::Key::u | gdk::Key::U => {
                    this.undo();
                }
                gdk::Key::Escape => this.window.close(),
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        self.window.add_controller(keys);
    }

    fn undo(&self) {
        self.strokes.borrow_mut().pop();
        self.area.queue_draw();
    }

    /// Pixels-per-image-pixel the canvas is currently showing.
    fn scale(&self) -> f64 {
        let w = f64::from(self.area.width()) / f64::from(self.image.width);
        let h = f64::from(self.area.height()) / f64::from(self.image.height);
        // Fit, never fill: a crop of the annotation would be worse than
        // letterboxing it.
        w.min(h).clamp(0.01, 1.0)
    }

    /// Where the image sits inside the canvas, so a click can be mapped back.
    fn origin(&self) -> (f64, f64) {
        let scale = self.scale();
        (
            (f64::from(self.area.width()) - f64::from(self.image.width) * scale) / 2.0,
            (f64::from(self.area.height()) - f64::from(self.image.height) * scale) / 2.0,
        )
    }

    fn to_image(&self, (x, y): (f64, f64)) -> (f64, f64) {
        let scale = self.scale();
        let (ox, oy) = self.origin();
        ((x - ox) / scale, (y - oy) / scale)
    }

    /// The strokes, over the capture the picture under this layer shows.
    fn draw(&self, cr: &cairo::Context, _w: f64, _h: f64) {
        let scale = self.scale();
        let (ox, oy) = self.origin();

        let _ = cr.save();
        cr.translate(ox, oy);
        cr.scale(scale, scale);
        for stroke in self.strokes.borrow().iter() {
            paint(cr, stroke, &self.image);
        }
        if let Some(live) = self.live.borrow().as_ref() {
            paint(cr, live, &self.image);
        }
        let _ = cr.restore();
    }

    /// Replay every stroke at full resolution.
    fn export(&self) -> Image {
        let Some(mut base) = to_cairo(&self.image) else {
            return self.image.clone();
        };
        {
            let Ok(cr) = cairo::Context::new(&base) else {
                return self.image.clone();
            };
            for stroke in self.strokes.borrow().iter() {
                paint(&cr, stroke, &self.image);
            }
        }
        from_cairo(&mut base).unwrap_or_else(|| self.image.clone())
    }
}

/// Draw one stroke in image coordinates.
fn paint(cr: &cairo::Context, stroke: &Stroke, source: &Image) {
    let (r, g, b) = stroke.colour;
    cr.set_source_rgb(r, g, b);
    cr.set_line_width(STROKE_WIDTH);
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_join(cairo::LineJoin::Round);

    let Some(&(x0, y0)) = stroke.points.first() else {
        return;
    };
    let Some(&(x1, y1)) = stroke.points.last() else {
        return;
    };

    match stroke.tool {
        Tool::Pen => {
            cr.move_to(x0, y0);
            for &(x, y) in &stroke.points[1..] {
                cr.line_to(x, y);
            }
            let _ = cr.stroke();
        }
        Tool::Highlight => {
            cr.save().unwrap();
            let (r, g, b) = stroke.colour;
            cr.set_source_rgba(r, g, b, 0.35); // 35% translucent marker
            cr.set_line_width(STROKE_WIDTH * 4.5);
            cr.set_line_cap(cairo::LineCap::Round);
            cr.set_line_join(cairo::LineJoin::Round);
            cr.move_to(x0, y0);
            for &(x, y) in &stroke.points[1..] {
                cr.line_to(x, y);
            }
            let _ = cr.stroke();
            cr.restore().unwrap();
        }
        Tool::Box_ => {
            cr.rectangle(x0.min(x1), y0.min(y1), (x1 - x0).abs(), (y1 - y0).abs());
            let _ = cr.stroke();
        }
        Tool::Arrow => {
            cr.move_to(x0, y0);
            cr.line_to(x1, y1);
            let _ = cr.stroke();

            // A head proportional to the shaft, so a short arrow is not all
            // head and a long one is not all line.
            let dx = x1 - x0;
            let dy = y1 - y0;
            let len = dx.hypot(dy);
            if len < 1.0 {
                return;
            }
            let head = (len * 0.22).clamp(8.0, 40.0);
            let angle = dy.atan2(dx);
            let spread = 0.42;
            cr.move_to(x1, y1);
            cr.line_to(
                x1 - head * (angle - spread).cos(),
                y1 - head * (angle - spread).sin(),
            );
            cr.line_to(
                x1 - head * (angle + spread).cos(),
                y1 - head * (angle + spread).sin(),
            );
            cr.close_path();
            let _ = cr.fill();
        }
        Tool::Pixelate => pixelate(
            cr,
            source,
            (x0, y0),
            (x1, y1),
            &mut stroke.blocks.borrow_mut(),
        ),
    }
}

/// Average the source in blocks and paint them back as flat squares.
///
/// A blur would be prettier and is not redaction: a Gaussian is invertible
/// enough that text has been recovered from one. Averaging whole blocks throws
/// the information away. Sampling is from the untouched capture, so pixelating
/// twice over the same area cannot slowly reveal it either.
///
/// The grid starts at the corner the drag started from, `from`, and grows
/// toward `to`, so while dragging the blocks already inside stay the same
/// and are taken from `memo`; only the new ones read the capture.
fn pixelate(
    cr: &cairo::Context,
    source: &Image,
    from: (f64, f64),
    to: (f64, f64),
    memo: &mut BlockMap,
) {
    let (sw, sh) = (source.width, source.height);
    let block = (sw.min(sh) / PIXELATE_DIVISOR).max(PIXELATE_MIN);
    let clamp = |v: f64, max: u32| (v.max(0.0) as u32).min(max);
    // Each axis steps away from the anchor, whichever way the drag went.
    let steps = |a: f64, b: f64, max: u32| -> Vec<(u32, u32)> {
        let (a, b) = (clamp(a, max), clamp(b, max));
        let mut out = Vec::new();
        if a <= b {
            let mut s = a;
            while s < b {
                out.push((s, (s + block).min(b)));
                s += block;
            }
        } else {
            let mut e = a;
            while e > b {
                out.push((e.saturating_sub(block).max(b), e));
                e = e.saturating_sub(block);
            }
        }
        out
    };
    for (by, ey) in steps(from.1, to.1, sh) {
        for (bx, ex) in steps(from.0, to.0, sw) {
            let (r, g, b) = *memo
                .entry((bx, by, ex, ey))
                .or_insert_with(|| average(source, bx, by, ex, ey));
            cr.set_source_rgb(r, g, b);
            cr.rectangle(
                f64::from(bx),
                f64::from(by),
                f64::from(ex - bx),
                f64::from(ey - by),
            );
            let _ = cr.fill();
        }
    }
}

/// The mean colour of a block of the capture, 0 to 1 per channel.
fn average(source: &Image, bx: u32, by: u32, ex: u32, ey: u32) -> (f64, f64, f64) {
    let (mut r, mut g, mut b) = (0u64, 0u64, 0u64);
    for py in by..ey {
        let row = ((py * source.width + bx) * 4) as usize;
        for px in source.pixels[row..row + ((ex - bx) * 4) as usize].chunks_exact(4) {
            r += u64::from(px[0]);
            g += u64::from(px[1]);
            b += u64::from(px[2]);
        }
    }
    let n = (u64::from(ex - bx) * u64::from(ey - by)).max(1) as f64 * 255.0;
    (r as f64 / n, g as f64 / n, b as f64 / n)
}

/// The capture as a texture, straight RGBA as it is: uploaded once, and
/// scaled on the GPU however the window is sized.
fn texture(image: &Image) -> gdk::Texture {
    gdk::MemoryTexture::new(
        image.width as i32,
        image.height as i32,
        gdk::MemoryFormat::R8g8b8a8,
        &glib::Bytes::from(&image.pixels[..]),
        (image.width * 4) as usize,
    )
    .upcast()
}

// ── Pixel format bridging ───────────────────────────────────────────────

/// RGBA to cairo's ARGB32, which is a native-endian word and therefore BGRA
/// in memory, with the colour channels premultiplied by alpha.
fn to_cairo(image: &Image) -> Option<cairo::ImageSurface> {
    let mut surface = cairo::ImageSurface::create(
        cairo::Format::ARgb32,
        image.width as i32,
        image.height as i32,
    )
    .ok()?;
    let stride = surface.stride() as usize;
    {
        let mut data = surface.data().ok()?;
        for y in 0..image.height as usize {
            for x in 0..image.width as usize {
                let s = (y * image.width as usize + x) * 4;
                let d = y * stride + x * 4;
                let a = u32::from(image.pixels[s + 3]);
                let mul = |c: u8| ((u32::from(c) * a + 127) / 255) as u8;
                data[d] = mul(image.pixels[s + 2]);
                data[d + 1] = mul(image.pixels[s + 1]);
                data[d + 2] = mul(image.pixels[s]);
                data[d + 3] = a as u8;
            }
        }
    }
    Some(surface)
}

/// The inverse, undoing the premultiply so the result is what PNG wants.
fn from_cairo(surface: &mut cairo::ImageSurface) -> Option<Image> {
    let width = surface.width() as u32;
    let height = surface.height() as u32;
    let stride = surface.stride() as usize;
    let data = surface.data().ok()?;

    let mut pixels = vec![0u8; (width * height * 4) as usize];
    for y in 0..height as usize {
        for x in 0..width as usize {
            let s = y * stride + x * 4;
            let d = (y * width as usize + x) * 4;
            let a = u32::from(data[s + 3]);
            let unmul = |c: u8| match a {
                0 => 0,
                a => ((u32::from(c) * 255 + a / 2) / a).min(255) as u8,
            };
            pixels[d] = unmul(data[s + 2]);
            pixels[d + 1] = unmul(data[s + 1]);
            pixels[d + 2] = unmul(data[s]);
            pixels[d + 3] = a as u8;
        }
    }
    Some(Image {
        width,
        height,
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(pixels: Vec<u8>, w: u32, h: u32) -> Image {
        Image {
            width: w,
            height: h,
            pixels,
        }
    }

    #[test]
    fn an_opaque_image_survives_the_round_trip_through_cairo() {
        let original = image(vec![10, 120, 240, 255, 0, 0, 0, 255], 2, 1);
        let mut surface = to_cairo(&original).unwrap();
        let back = from_cairo(&mut surface).unwrap();
        assert_eq!(back.pixels, original.pixels);
    }

    #[test]
    fn a_transparent_pixel_comes_back_transparent() {
        let original = image(vec![10, 120, 240, 0], 1, 1);
        let back = from_cairo(&mut to_cairo(&original).unwrap()).unwrap();
        assert_eq!(back.pixels[3], 0);
    }

    #[test]
    fn pixelating_a_gradient_flattens_it() {
        // 12x12 horizontal ramp: every column a different value.
        let mut pixels = Vec::new();
        for _ in 0..12 {
            for x in 0..12u8 {
                pixels.extend_from_slice(&[x * 20, x * 20, x * 20, 255]);
            }
        }
        let source = image(pixels, 12, 12);
        let mut target = to_cairo(&image(vec![0; 12 * 12 * 4], 12, 12)).unwrap();
        {
            let cr = cairo::Context::new(&target).unwrap();
            pixelate(&cr, &source, (0.0, 0.0), (12.0, 12.0), &mut HashMap::new());
        }

        let out = from_cairo(&mut target).unwrap();
        // PIXELATE_MIN is 6, so a 12-wide image becomes two flat blocks: the
        // first six columns share one value and differ from the last six.
        let at = |x: usize| out.pixels[x * 4];
        assert_eq!(at(0), at(5), "left block is flat");
        assert_eq!(at(6), at(11), "right block is flat");
        assert_ne!(at(0), at(6), "the two blocks still differ");
    }

    #[test]
    fn a_growing_drag_reads_only_its_new_blocks() {
        let source = image(vec![200; 60 * 60 * 4], 60, 60);
        let target = to_cairo(&image(vec![0; 60 * 60 * 4], 60, 60)).unwrap();
        let cr = cairo::Context::new(&target).unwrap();
        let mut memo = HashMap::new();
        // Blocks of 6 (PIXELATE_MIN): 12 by 12 is four.
        pixelate(&cr, &source, (0.0, 0.0), (12.0, 12.0), &mut memo);
        assert_eq!(memo.len(), 4);
        let first: Vec<_> = memo.keys().copied().collect();
        // Grown to 18 by 12: the four stay, two are added.
        pixelate(&cr, &source, (0.0, 0.0), (18.0, 12.0), &mut memo);
        assert_eq!(memo.len(), 6);
        assert!(first.iter().all(|k| memo.contains_key(k)));
    }

    #[test]
    fn a_drag_up_and_left_steps_from_where_it_started() {
        let source = image(vec![200; 60 * 60 * 4], 60, 60);
        let target = to_cairo(&image(vec![0; 60 * 60 * 4], 60, 60)).unwrap();
        let cr = cairo::Context::new(&target).unwrap();
        let mut memo = HashMap::new();
        pixelate(&cr, &source, (40.0, 40.0), (29.0, 33.0), &mut memo);
        let mut keys: Vec<_> = memo.keys().copied().collect();
        keys.sort();
        // Full blocks against the start corner, the part-block at the far
        // edge: x 34..40 and 29..34, y 34..40 and 33..34.
        assert_eq!(
            keys,
            [
                (29, 33, 34, 34),
                (29, 34, 34, 40),
                (34, 33, 40, 34),
                (34, 34, 40, 40)
            ]
        );
    }
}
