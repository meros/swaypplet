//! The Look tab: the wallpaper, the colours it produces, and how much the
//! shell moves.
//!
//! Two sections on one tab, `wallpaper` and `look`, in four groups with one
//! footer. The wallpaper group is a picker over `wallpaper.rs`, which owns
//! setting and reading it back; the appearance group is the design system's
//! first four inputs (mode, accent, neutral, contrast; docs/design-system.md §2),
//! which `theme::watch` turns into the stylesheet within a second; the theme
//! colour group is the wallpaper tint (another token input, §2.2) with the
//! token scales it produces drawn below it; the motion group is one dropdown, read per
//! animation by `anim::duration` and scaled into the motion tokens.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gtk4::gdk;
use gtk4::gdk_pixbuf::Pixbuf;
use gtk4::prelude::*;

use super::form::{self, dropdown_row, section_box};
use super::schema::ThemeMode;
use super::store::{self, Look, Motion, Tint, Wallpaper, WallpaperMode};
use super::wallpaper::{apply, candidates, candidates_dir, system_default};
use crate::tokens::{Accent, Contrast, Neutral};

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

// ── The scales, as a strip ──────────────────────────────────────────────

/// Chip height; the strip is two rows of chips.
const STRIP_H: i32 = 22;
const STRIP_GAP: f64 = crate::tokens::space(2) as f64;
/// `--radius-control`: a chip's corners.
const STRIP_RADIUS: f64 = crate::tokens::RADIUS[0].1 as f64;

/// The two scales the tokens are built from, as the stylesheet on screen
/// has them (`theme::shown`): neutral 1–12 above, accent 1–12 below. Mode,
/// accent, neutral and tint all show here, the tint as the hue both rows
/// turn to. Repainted whenever `theme::observe` says the stylesheet moved.
fn swatch_strip() -> gtk4::DrawingArea {
    let area = gtk4::DrawingArea::builder()
        .content_height(2 * STRIP_H + STRIP_GAP as i32)
        .hexpand(true)
        .build();
    area.set_draw_func(|_, cr, w, _| {
        let s = crate::tokens::scales(crate::theme::shown());
        let count = s.neutral.len() as f64;
        let width = (f64::from(w) - STRIP_GAP * (count - 1.0)) / count;
        if width <= 0.0 {
            return;
        }
        let h = f64::from(STRIP_H);
        for (row, scale) in [s.neutral, s.accent].iter().enumerate() {
            let y = (h + STRIP_GAP) * row as f64;
            for (i, color) in scale.iter().enumerate() {
                let x = (width + STRIP_GAP) * i as f64;
                rounded_rect(cr, x, y, width, h, STRIP_RADIUS);
                crate::ui::set_source(cr, *color, 1.0);
                let _ = cr.fill();
            }
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

// ── Appearance ──────────────────────────────────────────────────────────

fn accent_name(a: Accent) -> &'static str {
    match a {
        Accent::Aqua => "Aqua",
        Accent::Yellow => "Yellow",
        Accent::Blue => "Blue",
        Accent::Purple => "Purple",
        Accent::Orange => "Orange",
        Accent::Red => "Red",
    }
}

fn neutral_label(n: Neutral) -> &'static str {
    match n {
        Neutral::Gruvbox => "Gruvbox — warm",
        Neutral::Slate => "Slate — cool",
        Neutral::Pure => "Pure — grey",
    }
}

fn contrast_label(c: Contrast) -> &'static str {
    match c {
        Contrast::Standard => "Standard",
        Contrast::High => "High",
    }
}

/// One accent to pick: a dot in `--accent-bg` as the tokens would generate
/// it with this accent and every other input as it is on screen, so dark
/// mode shows the bright tone and light mode the deep one (and a hue that
/// steps down a shade to carry its label shows that shade), as the accent
/// would actually look.
fn accent_swatch(accent: Accent) -> (gtk4::ToggleButton, gtk4::DrawingArea) {
    let dot = gtk4::DrawingArea::builder()
        .content_width(16)
        .content_height(16)
        .build();
    dot.set_draw_func(move |_, cr, w, h| {
        let tone = crate::tokens::scales(crate::tokens::Inputs {
            accent,
            ..crate::theme::shown()
        })
        .accent_bg;
        let (w, h) = (f64::from(w), f64::from(h));
        crate::ui::set_source(cr, tone, 1.0);
        cr.arc(w / 2.0, h / 2.0, w.min(h) / 2.0, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
    });
    let button = crate::ui::swatch(&dot);
    button.set_tooltip_text(Some(accent_name(accent)));
    (button, dot)
}

// ── The tab ─────────────────────────────────────────────────────────────

struct State {
    grid: gtk4::FlowBox,
    /// One thumbnail per path the grid shows, in grid order.
    thumbs: RefCell<Vec<(PathBuf, gtk4::Button)>>,
    mode: gtk4::DropDown,
    theme_mode: gtk4::DropDown,
    /// One swatch per accent, and the dot it draws, to repaint when the mode
    /// on screen moves.
    accents: Vec<(Accent, gtk4::ToggleButton, gtk4::DrawingArea)>,
    neutral: gtk4::DropDown,
    contrast: gtk4::DropDown,
    motion: gtk4::DropDown,
    launch_zoom: gtk4::Switch,
    tint: gtk4::DropDown,
    /// The token scales, for the strip to repaint.
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
        let look = settings.look();
        let theme_mode = ThemeMode::ALL.iter().position(|m| *m == look.mode);
        self.theme_mode.set_selected(theme_mode.unwrap_or(0) as u32);
        for (accent, button, dot) in &self.accents {
            button.set_active(*accent == look.accent);
            dot.queue_draw();
        }
        let neutral = Neutral::ALL.iter().position(|n| *n == look.neutral);
        self.neutral.set_selected(neutral.unwrap_or(0) as u32);
        let contrast = Contrast::ALL.iter().position(|c| *c == look.contrast);
        self.contrast.set_selected(contrast.unwrap_or(0) as u32);
        let motion = Motion::ALL
            .iter()
            .position(|m| *m == settings.look().motion);
        self.motion.set_selected(motion.unwrap_or(0) as u32);
        self.launch_zoom.set_active(settings.look().launch_zoom);
        let tint = Tint::ALL.iter().position(|t| *t == settings.look().tint);
        self.tint.set_selected(tint.unwrap_or(0) as u32);
        // The stylesheet follows on `theme::watch`'s next tick, and a new
        // wallpaper's hue after the panel samples it; this paints what is on
        // screen now and `theme::observe` paints the rest when it lands.
        self.strip.queue_draw();
        for (path, button) in self.thumbs.borrow().iter() {
            let selected = shown.as_ref().is_some_and(|w| w.path == *path);
            crate::ui::set_selected(button, selected);
        }
        if let Some(w) = &shown {
            let index = WallpaperMode::ALL.iter().position(|m| *m == w.mode);
            self.mode.set_selected(index.unwrap_or(0) as u32);
        }
        form::set_source(
            &self.status,
            overridden,
            "System default: the sway config's wallpaper, auto mode in aqua on gruvbox, full motion",
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
        let picture = gtk4::Picture::builder()
            .content_fit(gtk4::ContentFit::Cover)
            .width_request(THUMB_W)
            .height_request(THUMB_H)
            .hexpand(true)
            .halign(gtk4::Align::Fill)
            .build();
        let button = crate::ui::pick_thumb(&picture);
        button.set_tooltip_text(
            path.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .as_deref(),
        );
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
        let root = form::pane();

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
            .row_spacing(crate::tokens::space(2) as u32)
            .column_spacing(crate::tokens::space(2) as u32)
            .homogeneous(true)
            .build();
        grid.add_css_class("settings-presets");
        group.append(&grid);

        let mode_labels: Vec<&str> = WallpaperMode::ALL.iter().map(|m| m.label()).collect();
        let (mode_row, mode) = dropdown_row(
            "Scaling",
            "How the image meets the output's aspect ratio.",
            &mode_labels,
        );
        group.append(&mode_row);

        let appearance = section_box(
            "Appearance",
            "Four of the design system's inputs; the fifth, the tint, is below. Every surface follows within a second.",
        );
        let mode_labels: Vec<&str> = ThemeMode::ALL.iter().map(|m| m.label()).collect();
        let (theme_mode_row, theme_mode) = dropdown_row(
            "Mode",
            "Dark, light, or by the sun at this machine's location: light from 3° above the horizon, dark from 3° below.",
            &mode_labels,
        );
        appearance.append(&theme_mode_row);
        let swatches = crate::ui::hbox(2);
        swatches.set_halign(gtk4::Align::Start);
        let mut accents = Vec::new();
        let mut first: Option<gtk4::ToggleButton> = None;
        for accent in Accent::ALL {
            let (button, dot) = accent_swatch(accent);
            match &first {
                Some(group) => button.set_group(Some(group)),
                None => first = Some(button.clone()),
            }
            swatches.append(&button);
            accents.push((accent, button, dot));
        }
        let accent_row = form::kind_row("Accent", &swatches);
        accent_row.set_tooltip_text(Some(
            "The one colour that means on, selected, or the primary action.",
        ));
        appearance.append(&accent_row);
        let neutral_labels: Vec<&str> = Neutral::ALL.iter().map(|n| neutral_label(*n)).collect();
        let (neutral_row, neutral) = dropdown_row(
            "Neutral",
            "The greys the glass, the text and the lines are drawn from.",
            &neutral_labels,
        );
        appearance.append(&neutral_row);
        let contrast_labels: Vec<&str> = Contrast::ALL.iter().map(|c| contrast_label(*c)).collect();
        let (contrast_row, contrast) = dropdown_row(
            "Contrast",
            "High lifts the quieter text levels and the lines, and gives the glass more body.",
            &contrast_labels,
        );
        appearance.append(&contrast_row);

        let theme = section_box(
            "Theme colour",
            "Take the shell's colours from the wallpaper. Each colour keeps its \
             lightness and only its hue moves, so the contrast the shell is \
             built on holds whatever the image is. Below: the neutral and \
             accent scales the shell is drawn from.",
        );
        let tint_labels: Vec<&str> = Tint::ALL.iter().map(|t| t.label()).collect();
        let (tint_row, tint) = dropdown_row(
            "Tint",
            "Off keeps the shipped colours. Accents gives the accent the \
             wallpaper's hue and turns the app colours with it. Full tints the \
             greys and the glass too. Red stays red either way.",
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
        let (zoom_row, launch_zoom) = form::switch_row(
            "Launch zoom",
            "An app you start from the launcher grows out of its row. Needs the swayfx handoff patch.",
            false,
        );
        look.append(&zoom_row);

        let browse = form::action_button(
            "Browse…",
            &format!(
                "Pick an image from anywhere. The grid shows {}.",
                candidates_dir()
                    .map(|d| form::pretty_path(&d))
                    .unwrap_or_else(|| "~/Pictures/wallpapers".into())
            ),
        );
        let reset = form::action_button(
            "Reset to system",
            "Put the sway config's wallpaper back, and the system's theme colour and motion.",
        );
        let (footer, status) = form::footer(&[&browse, &reset]);

        let state = Rc::new(State {
            grid: grid.clone(),
            thumbs: RefCell::new(Vec::new()),
            mode: mode.clone(),
            theme_mode: theme_mode.clone(),
            accents,
            neutral: neutral.clone(),
            contrast: contrast.clone(),
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
            theme_mode.connect_selected_notify(move |d| {
                if state.updating.get() {
                    return;
                }
                let Some(mode) = ThemeMode::ALL.get(d.selected() as usize).copied() else {
                    return;
                };
                store::edit::<Look>(|l| l.mode = mode);
                state.sync();
                // The stylesheet follows on `theme::watch`'s next tick; the
                // dots show the accents in whichever mode that lands on.
                let state = state.clone();
                glib::timeout_add_local_once(std::time::Duration::from_millis(1200), move || {
                    for (_, _, dot) in &state.accents {
                        dot.queue_draw();
                    }
                });
            });
        }
        for (accent, button, _) in &state.accents {
            let state = state.clone();
            let accent = *accent;
            button.connect_toggled(move |b| {
                if state.updating.get() || !b.is_active() {
                    return;
                }
                store::edit::<Look>(|l| l.accent = accent);
                state.sync();
            });
        }
        {
            let state = state.clone();
            neutral.connect_selected_notify(move |d| {
                if state.updating.get() {
                    return;
                }
                let Some(neutral) = Neutral::ALL.get(d.selected() as usize).copied() else {
                    return;
                };
                store::edit::<Look>(|l| l.neutral = neutral);
                state.sync();
            });
        }
        {
            let state = state.clone();
            contrast.connect_selected_notify(move |d| {
                if state.updating.get() {
                    return;
                }
                let Some(contrast) = Contrast::ALL.get(d.selected() as usize).copied() else {
                    return;
                };
                store::edit::<Look>(|l| l.contrast = contrast);
                state.sync();
            });
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
                // The panel is watching the store and samples the wallpaper
                // (`theme::wallpaper::follow_settings`); this only records
                // the choice. In `swaypplet settings`, which has no panel,
                // the strip stays on the scales the pane started with and
                // the running panel repaints its own.
                store::edit::<Look>(|l| l.tint = tint);
                state.sync();
            });
        }
        {
            let strip = strip.clone();
            crate::theme::observe(move || strip.queue_draw());
        }

        root.append(&group);
        root.append(&appearance);
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
