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

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::gdk;
use gtk4::prelude::*;

use super::capture::Image;

/// Colours a mark can be, in the order they appear in the palette.
///
/// Gruvbox, like the rest of the shell, except that these are the *bright*
/// variants: a mark has to survive being drawn on top of an arbitrary
/// screenshot, which the muted UI palette does not reliably do.
const PALETTE: [(f64, f64, f64); 5] = [
    (0.984, 0.286, 0.204), // red
    (0.980, 0.741, 0.184), // yellow
    (0.721, 0.733, 0.149), // green
    (0.514, 0.647, 0.596), // blue
    (0.922, 0.859, 0.698), // fg
];

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
}

struct Editor {
    image: Image,
    strokes: RefCell<Vec<Stroke>>,
    /// The stroke being dragged, drawn but not yet committed.
    live: RefCell<Option<Stroke>>,
    tool: std::cell::Cell<Tool>,
    colour: std::cell::Cell<usize>,
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
    window.add_css_class("panel");
    window.add_css_class("annotate-window");

    let area = gtk4::DrawingArea::builder()
        .hexpand(true)
        .vexpand(true)
        .build();
    area.add_css_class("annotate-canvas");

    let editor = Rc::new(Editor {
        image,
        strokes: RefCell::new(Vec::new()),
        live: RefCell::new(None),
        tool: std::cell::Cell::new(Tool::Box_),
        colour: std::cell::Cell::new(0),
        area: area.clone(),
        window: window.clone(),
    });

    let root = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .build();
    root.append(&toolbar(&editor, done));
    root.append(&area);
    window.set_child(Some(&root));

    editor.wire();
    window.present();
}

fn toolbar(editor: &Rc<Editor>, done: impl Fn(Image) + 'static) -> gtk4::Box {
    let bar = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(6)
        .build();
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
        let btn = gtk4::ToggleButton::builder().label(icon).build();
        btn.set_tooltip_text(Some(name));
        btn.add_css_class("annotate-tool");
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
    for (index, colour) in PALETTE.iter().enumerate() {
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

        let btn = gtk4::ToggleButton::builder().child(&dot).build();
        btn.add_css_class("annotate-swatch");
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

    let undo = gtk4::Button::with_label("Undo");
    undo.add_css_class("annotate-action");
    let editor_c = editor.clone();
    undo.connect_clicked(move |_| editor_c.undo());
    bar.append(&undo);

    // The right-hand pair: the toolbar's left half changes the drawing, its
    // right half ends the session.
    let spacer = gtk4::Box::builder().hexpand(true).build();
    bar.append(&spacer);

    let keep = gtk4::Button::with_label("Copy & save");
    keep.add_css_class("annotate-action");
    keep.add_css_class("suggested-action");
    let editor_c = editor.clone();
    keep.connect_clicked(move |_| {
        done(editor_c.export());
        editor_c.window.close();
    });
    bar.append(&keep);

    let discard = gtk4::Button::with_label("Discard edits");
    discard.add_css_class("annotate-action");
    let editor_c = editor.clone();
    discard.connect_clicked(move |_| editor_c.window.close());
    bar.append(&discard);

    bar
}

fn separator() -> gtk4::Separator {
    let sep = gtk4::Separator::new(gtk4::Orientation::Vertical);
    sep.add_css_class("annotate-separator");
    sep
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
                colour: PALETTE[this.colour.get()],
                points: vec![p],
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
            if state.contains(gdk::ModifierType::CONTROL_MASK) {
                if key == gdk::Key::z {
                    this.undo();
                    return glib::Propagation::Stop;
                }
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

    fn draw(&self, cr: &cairo::Context, _w: f64, _h: f64) {
        let Some(base) = to_cairo(&self.image) else {
            return;
        };
        let scale = self.scale();
        let (ox, oy) = self.origin();

        let _ = cr.save();
        cr.translate(ox, oy);
        cr.scale(scale, scale);
        let _ = cr.set_source_surface(&base, 0.0, 0.0);
        let _ = cr.paint();

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
            x0.min(x1),
            y0.min(y1),
            (x1 - x0).abs(),
            (y1 - y0).abs(),
        ),
    }
}

/// Average the source in blocks and paint them back as flat squares.
///
/// A blur would be prettier and is not redaction: a Gaussian is invertible
/// enough that text has been recovered from one. Averaging whole blocks throws
/// the information away. Sampling is from the untouched capture, so pixelating
/// twice over the same area cannot slowly reveal it either.
fn pixelate(cr: &cairo::Context, source: &Image, x: f64, y: f64, w: f64, h: f64) {
    if w < 1.0 || h < 1.0 {
        return;
    }
    let (sw, sh) = (source.width, source.height);
    let block = (sw.min(sh) / PIXELATE_DIVISOR).max(PIXELATE_MIN);
    let x0 = x.max(0.0) as u32;
    let y0 = y.max(0.0) as u32;
    let x1 = ((x + w) as u32).min(sw);
    let y1 = ((y + h) as u32).min(sh);

    let mut by = y0;
    while by < y1 {
        let mut bx = x0;
        while bx < x1 {
            let ex = (bx + block).min(x1);
            let ey = (by + block).min(y1);
            let (mut r, mut g, mut b, mut n) = (0u64, 0u64, 0u64, 0u64);
            for py in by..ey {
                for px in bx..ex {
                    let i = ((py * sw + px) * 4) as usize;
                    r += u64::from(source.pixels[i]);
                    g += u64::from(source.pixels[i + 1]);
                    b += u64::from(source.pixels[i + 2]);
                    n += 1;
                }
            }
            if n > 0 {
                cr.set_source_rgb(
                    r as f64 / n as f64 / 255.0,
                    g as f64 / n as f64 / 255.0,
                    b as f64 / n as f64 / 255.0,
                );
                cr.rectangle(
                    f64::from(bx),
                    f64::from(by),
                    f64::from(ex - bx),
                    f64::from(ey - by),
                );
                let _ = cr.fill();
            }
            bx += block;
        }
        by += block;
    }
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
            pixelate(&cr, &source, 0.0, 0.0, 12.0, 12.0);
        }

        let out = from_cairo(&mut target).unwrap();
        // PIXELATE_MIN is 6, so a 12-wide image becomes two flat blocks: the
        // first six columns share one value and differ from the last six.
        let at = |x: usize| out.pixels[x * 4];
        assert_eq!(at(0), at(5), "left block is flat");
        assert_eq!(at(6), at(11), "right block is flat");
        assert_ne!(at(0), at(6), "the two blocks still differ");
    }
}
