//! The Look tab: the wallpaper, and how much the shell moves.
//!
//! Two sections on one tab, `wallpaper` and `look`, with one footer. The
//! wallpaper half is a picker over `wallpaper.rs`, which owns setting and
//! reading it back; the motion half is one dropdown, read per animation by
//! `anim::duration`.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gtk4::gdk;
use gtk4::gdk_pixbuf::Pixbuf;
use gtk4::prelude::*;

use super::store::{self, Look, Motion, Wallpaper, WallpaperMode};
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

// ── The tab ─────────────────────────────────────────────────────────────

struct State {
    grid: gtk4::FlowBox,
    /// One thumbnail per path the grid shows, in grid order.
    thumbs: RefCell<Vec<(PathBuf, gtk4::Button)>>,
    mode: gtk4::DropDown,
    motion: gtk4::DropDown,
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
            .build();

        let group = section_box(
            "Wallpaper",
            "Applied to every output at once. The lock screen shows the same image; the greeter has its own.",
        );

        let grid = gtk4::FlowBox::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .selection_mode(gtk4::SelectionMode::None)
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
            "Put the sway config's wallpaper and the system's motion back.",
        );
        let (footer, status) = ui::footer(&[&browse, &reset]);

        let state = Rc::new(State {
            grid: grid.clone(),
            thumbs: RefCell::new(Vec::new()),
            mode: mode.clone(),
            motion: motion.clone(),
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

        root.append(&group);
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
