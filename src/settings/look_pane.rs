//! The Look tab: the wallpaper, the colours it produces, and how much the
//! shell moves.
//!
//! Three sections on one tab, `wallpaper` and two halves of `look`, with one
//! footer. The wallpaper half is a picker over `wallpaper.rs`, which owns
//! setting and reading it back; the theme half is one dropdown over
//! `crate::palette` with the palette it derived drawn beside it; the motion
//! half is one dropdown, read per animation by `anim::duration`.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gtk4::gdk;
use gtk4::gdk_pixbuf::Pixbuf;
use gtk4::prelude::*;

use super::store::{self, Look, Motion, Tint, Wallpaper, WallpaperMode};
use super::ui::{self, dropdown_row, section_box};
use super::wallpaper::{apply, candidates, candidates_dir, system_default};

/// Thumbnail size, in logical pixels. 16:9, four to a row in the card.
const THUMB_W: i32 = 132;
const THUMB_H: i32 = 74;

// ── Thumbnails ──────────────────────────────────────────────────────────

/// A decoded thumbnail's pixels, which is what crosses the thread boundary.
struct Thumb {
    bytes: glib::Bytes,
    width: i32,
    height: i32,
    stride: usize,
    alpha: bool,
}

impl Thumb {
    fn texture(&self) -> gdk::MemoryTexture {
        let format = if self.alpha {
            gdk::MemoryFormat::R8g8b8a8
        } else {
            gdk::MemoryFormat::R8g8b8
        };
        gdk::MemoryTexture::new(self.width, self.height, format, &self.bytes, self.stride)
    }
}

/// Decode `path` at twice the thumbnail size (for a 2x output), on whatever
/// thread this is called from.
fn decode_thumb(path: &Path) -> Option<Thumb> {
    let pixbuf = Pixbuf::from_file_at_scale(path, THUMB_W * 2, THUMB_H * 2, true).ok()?;
    Some(Thumb {
        bytes: pixbuf.read_pixel_bytes(),
        width: pixbuf.width(),
        height: pixbuf.height(),
        stride: pixbuf.rowstride() as usize,
        alpha: pixbuf.has_alpha(),
    })
}

// ── The derived palette, as a strip ─────────────────────────────────────

/// What the strip shows: the ground, the ink, and the five accents, in the
/// order the palette defines them. Enough to tell one derivation from
/// another at a glance, which is all this is for — the palette has forty
/// entries and a wall of chips reads as a wall.
const STRIP: [&str; 8] = [
    "bg0",
    "bg1",
    "fg",
    "accent",
    "accent_secondary",
    "accent_tertiary",
    "accent_quaternary",
    "accent_quinary",
];

const STRIP_H: i32 = 22;
const STRIP_GAP: f64 = 4.0;
const STRIP_RADIUS: f64 = 5.0;

/// A strip of the palette in force, repainted whenever `palette::observe`
/// says it moved. It reads `palette::current` at draw time rather than
/// holding a copy, so there is one answer to "what colour is @accent" in
/// this process and it is the one GTK is using.
fn swatch_strip() -> gtk4::DrawingArea {
    let area = gtk4::DrawingArea::builder()
        .content_height(STRIP_H)
        .hexpand(true)
        .build();
    area.set_draw_func(|_, cr, w, h| {
        let css = crate::palette::current();
        let count = STRIP.len() as f64;
        let width = (f64::from(w) - STRIP_GAP * (count - 1.0)) / count;
        if width <= 0.0 {
            return;
        }
        for (i, name) in STRIP.iter().enumerate() {
            let Some(color) = crate::palette::lookup(&css, name) else {
                continue;
            };
            let x = (width + STRIP_GAP) * i as f64;
            rounded_rect(cr, x, 0.0, width, f64::from(h), STRIP_RADIUS);
            cr.set_source_rgb(
                f64::from(color.red) / 255.0,
                f64::from(color.green) / 255.0,
                f64::from(color.blue) / 255.0,
            );
            let _ = cr.fill();
        }
    });
    area
}

fn rounded_rect(cr: &gtk4::cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0);
    let (right, bottom) = (x + w, y + h);
    cr.new_sub_path();
    cr.arc(right - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
    cr.arc(right - r, bottom - r, r, 0.0, std::f64::consts::FRAC_PI_2);
    cr.arc(
        x + r,
        bottom - r,
        r,
        std::f64::consts::FRAC_PI_2,
        std::f64::consts::PI,
    );
    cr.arc(
        x + r,
        y + r,
        r,
        std::f64::consts::PI,
        1.5 * std::f64::consts::PI,
    );
    cr.close_path();
}

// ── The tab ─────────────────────────────────────────────────────────────

struct State {
    grid: gtk4::FlowBox,
    /// One thumbnail per path the grid shows, in grid order.
    thumbs: RefCell<Vec<(PathBuf, gtk4::Button)>>,
    mode: gtk4::DropDown,
    motion: gtk4::DropDown,
    launch_zoom: gtk4::Switch,
    tint: gtk4::DropDown,
    /// The palette the tint produced, for the strip to repaint.
    strip: gtk4::DrawingArea,
    status: gtk4::Label,
    /// Read once from the compositor; `None` until it answers, and still
    /// `None` if the config sets no wallpaper.
    system: RefCell<Option<Wallpaper>>,
    /// True while the controls are being set from the store, so their
    /// handlers do not write the same value straight back.
    updating: Cell<bool>,
}

impl State {
    /// What is on the screen: the pick, or the system default.
    fn shown(&self) -> Option<Wallpaper> {
        store::current()
            .wallpaper
            .or_else(|| self.system.borrow().clone())
    }

    fn pick(self: &Rc<Self>, path: PathBuf) {
        let mode = self.shown().map(|w| w.mode).unwrap_or_default();
        self.set(Wallpaper { path, mode });
    }

    fn set_mode(self: &Rc<Self>, mode: WallpaperMode) {
        let Some(current) = self.shown() else {
            return;
        };
        self.set(Wallpaper {
            path: current.path,
            mode,
        });
    }

    fn set(self: &Rc<Self>, w: Wallpaper) {
        apply(&w);
        store::update(|s| s.wallpaper = Some(w));
        self.sync();
    }

    fn reset(self: &Rc<Self>) {
        store::update(|s| s.wallpaper = None);
        store::reset::<Look>();
        match self.system.borrow().as_ref() {
            Some(system) => apply(system),
            // Nothing to put back: the config sets no wallpaper, so what is
            // on screen stays until the next reload.
            None => log::warn!("wallpaper: no system default to reset to"),
        }
        self.sync();
    }

    /// Bring the grid and the dropdown in line with the store.
    fn sync(&self) {
        self.updating.set(true);
        let shown = self.shown();
        let settings = store::current();
        let overridden = settings.wallpaper.is_some() || settings.look.is_some();
        let motion = Motion::ALL
            .iter()
            .position(|m| *m == settings.look().motion);
        self.motion.set_selected(motion.unwrap_or(0) as u32);
        self.launch_zoom.set_active(settings.look().launch_zoom);
        let tint = Tint::ALL.iter().position(|t| *t == settings.look().tint);
        self.tint.set_selected(tint.unwrap_or(0) as u32);
        // The derivation runs on a worker, so this paints the palette that
        // is in force now; `palette::observe` paints the new one when it
        // lands, a beat later.
        self.strip.queue_draw();
        for (path, button) in self.thumbs.borrow().iter() {
            let selected = shown.as_ref().is_some_and(|w| w.path == *path);
            if selected {
                button.add_css_class("selected");
            } else {
                button.remove_css_class("selected");
            }
        }
        if let Some(w) = &shown {
            let index = WallpaperMode::ALL.iter().position(|m| *m == w.mode);
            self.mode.set_selected(index.unwrap_or(0) as u32);
        }
        ui::set_source(
            &self.status,
            overridden,
            "System default: the sway config's wallpaper, full motion",
        );
        self.updating.set(false);
    }

    /// Put the candidates, the pick and the system default in the grid,
    /// decoding thumbnails for any path not already there.
    fn rescan(self: &Rc<Self>) {
        let mut wanted = candidates();
        for extra in [
            store::current().wallpaper.map(|w| w.path),
            self.system.borrow().as_ref().map(|w| w.path.clone()),
        ]
        .into_iter()
        .flatten()
        {
            if !wanted.contains(&extra) && extra.is_file() {
                wanted.push(extra);
            }
        }

        let known: Vec<PathBuf> = self
            .thumbs
            .borrow()
            .iter()
            .map(|(p, _)| p.clone())
            .collect();
        for path in wanted {
            if known.contains(&path) {
                continue;
            }
            self.add_thumb(path);
        }
        self.sync();
    }

    fn add_thumb(self: &Rc<Self>, path: PathBuf) {
        let button = gtk4::Button::builder()
            .has_frame(false)
            .css_classes(["settings-wallpaper-thumb"])
            .tooltip_text(
                path.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default(),
            )
            .build();
        let picture = gtk4::Picture::builder()
            .content_fit(gtk4::ContentFit::Cover)
            .width_request(THUMB_W)
            .height_request(THUMB_H)
            .hexpand(true)
            .halign(gtk4::Align::Fill)
            .build();
        button.set_child(Some(&picture));
        {
            let this = self.clone();
            let path = path.clone();
            button.connect_clicked(move |_| {
                if this.updating.get() {
                    return;
                }
                this.pick(path.clone());
            });
        }
        self.grid.append(&button);
        self.thumbs.borrow_mut().push((path.clone(), button));

        // Decoded at thumbnail size on a worker: a 4K JPEG is 33 MB as a
        // texture and a hundred milliseconds to decode, and there are
        // several of them. `from_file_at_scale` lets the loader downsample
        // as it decodes. A `Pixbuf` is not `Send`, so the worker hands back
        // its pixels and the texture is built here.
        crate::spawn::spawn_work(
            move || decode_thumb(&path),
            move |thumb| {
                if let Some(thumb) = thumb {
                    picture.set_paintable(Some(&thumb.texture()));
                }
            },
        );
    }
}

pub struct LookPane {
    root: gtk4::Box,
    state: Rc<State>,
}

impl LookPane {
    pub fn new() -> Self {
        let root = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(14)
            .hexpand(true)
            .build();

        let group = section_box(
            "Wallpaper",
            "Applied to every output at once. The lock screen shows the same image; the greeter has its own.",
        );

        let grid = gtk4::FlowBox::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .selection_mode(gtk4::SelectionMode::None)
            // Up to four thumbnails per row across the card width, flowing
            // down to 3 or 2 when squeezed onto narrower displays.
            .min_children_per_line(2)
            .max_children_per_line(4)
            .row_spacing(6)
            .column_spacing(6)
            .homogeneous(true)
            .build();
        grid.add_css_class("settings-presets");
        grid.add_css_class("settings-wallpaper-grid");
        group.append(&grid);

        let mode_labels: Vec<&str> = WallpaperMode::ALL.iter().map(|m| m.label()).collect();
        let (mode_row, mode) = dropdown_row(
            "Scaling",
            "How the image meets the output's aspect ratio.",
            &mode_labels,
        );
        group.append(&mode_row);

        let theme = section_box(
            "Theme colour",
            "Take the shell's colours from the wallpaper. Each colour keeps its \
             lightness and only its hue moves, so the contrast the shell is \
             built on holds whatever the image is.",
        );
        let tint_labels: Vec<&str> = Tint::ALL.iter().map(|t| t.label()).collect();
        let (tint_row, tint) = dropdown_row(
            "Tint",
            "Off is the shipped palette. Accents takes the five accent hues from \
             the wallpaper. Full tints the greys with them too. Red stays red \
             either way.",
            &tint_labels,
        );
        theme.append(&tint_row);
        let strip = swatch_strip();
        theme.append(&strip);

        let look = section_box(
            "Motion",
            "How much the shell animates. GTK's own reduced-motion switch still wins when it is set.",
        );
        let motion_labels: Vec<&str> = Motion::ALL.iter().map(|m| m.label()).collect();
        let (motion_row, motion) = dropdown_row(
            "Motion",
            "Full, half as long, or one frame. Read per animation, so it lands on the next one.",
            &motion_labels,
        );
        look.append(&motion_row);
        let (zoom_row, launch_zoom) = ui::switch_row(
            "Launch zoom",
            "An app you start from the launcher grows out of its row. Needs the swayfx handoff patch.",
            false,
        );
        look.append(&zoom_row);

        let browse = ui::action_button(
            "Browse…",
            &format!(
                "Pick an image from anywhere. The grid shows {}.",
                candidates_dir()
                    .map(|d| ui::pretty_path(&d))
                    .unwrap_or_else(|| "~/Pictures/wallpapers".into())
            ),
        );
        let reset = ui::action_button(
            "Reset to system",
            "Put the sway config's wallpaper back, and the system's theme colour and motion.",
        );
        let (footer, status) = ui::footer(&[&browse, &reset]);

        let state = Rc::new(State {
            grid: grid.clone(),
            thumbs: RefCell::new(Vec::new()),
            mode: mode.clone(),
            motion: motion.clone(),
            launch_zoom: launch_zoom.clone(),
            tint: tint.clone(),
            strip: strip.clone(),
            status: status.clone(),
            system: RefCell::new(None),
            updating: Cell::new(false),
        });

        {
            let state = state.clone();
            mode.connect_selected_notify(move |d| {
                if state.updating.get() {
                    return;
                }
                if let Some(mode) = WallpaperMode::ALL.get(d.selected() as usize).copied() {
                    state.set_mode(mode);
                }
            });
        }

        {
            let state = state.clone();
            browse.connect_clicked(move |button| {
                // The panel is a keyboard-exclusive overlay: a dialog opened
                // under it could not be typed into or clicked. Hide the panel
                // first, as the screenshot actions do; `Panel::toggle` heals
                // the reveal state when it is next opened.
                if let Some(window) = button.root().and_downcast::<gtk4::Window>() {
                    window.set_visible(false);
                }
                let filter = gtk4::FileFilter::new();
                filter.set_name(Some("Images"));
                filter.add_mime_type("image/*");
                let filters = gio::ListStore::new::<gtk4::FileFilter>();
                filters.append(&filter);
                let dialog = gtk4::FileDialog::builder()
                    .title("Choose a wallpaper")
                    .modal(false)
                    .filters(&filters)
                    .build();
                if let Some(dir) = candidates_dir() {
                    dialog.set_initial_folder(Some(&gio::File::for_path(dir)));
                }
                let state = state.clone();
                dialog.open(
                    None::<&gtk4::Window>,
                    None::<&gio::Cancellable>,
                    move |result| match result {
                        Ok(file) => {
                            if let Some(path) = file.path() {
                                state.pick(path);
                                state.rescan();
                            }
                        }
                        // Cancelled is the usual outcome and not worth a
                        // line; anything else is.
                        Err(e) if e.matches(gtk4::DialogError::Dismissed) => {}
                        Err(e) => log::warn!("wallpaper: file dialog: {e}"),
                    },
                );
            });
        }

        {
            let state = state.clone();
            reset.connect_clicked(move |_| state.reset());
        }
        {
            let state = state.clone();
            motion.connect_selected_notify(move |d| {
                if state.updating.get() {
                    return;
                }
                let Some(motion) = Motion::ALL.get(d.selected() as usize).copied() else {
                    return;
                };
                store::edit::<Look>(|l| l.motion = motion);
                state.sync();
            });
        }
        {
            let state = state.clone();
            launch_zoom.connect_active_notify(move |s| {
                if state.updating.get() {
                    return;
                }
                let on = s.is_active();
                store::edit::<Look>(|l| l.launch_zoom = on);
                state.sync();
            });
        }
        {
            let state = state.clone();
            tint.connect_selected_notify(move |d| {
                if state.updating.get() {
                    return;
                }
                let Some(tint) = Tint::ALL.get(d.selected() as usize).copied() else {
                    return;
                };
                // The panel is watching the store and does the deriving
                // (`palette::follow_settings`); this only records the
                // choice. In `swaypplet settings`, which has no panel, the
                // strip stays on the palette the pane started with and the
                // running panel repaints its own.
                store::edit::<Look>(|l| l.tint = tint);
                state.sync();
            });
        }
        {
            let strip = strip.clone();
            crate::palette::observe(move || strip.queue_draw());
        }

        root.append(&group);
        root.append(&theme);
        root.append(&look);
        root.append(&footer);

        // Ask the compositor what the config shipped, then build the grid
        // once the answer is in so the system image gets a thumbnail too.
        {
            let state = state.clone();
            crate::spawn::spawn_work(system_default, move |system| {
                *state.system.borrow_mut() = system;
                state.rescan();
            });
        }

        LookPane { root, state }
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    /// Pick up files added to the candidates directory since the pane was
    /// built, and re-mark the selection.
    pub fn refresh(&self) {
        self.state.rescan();
    }
}
