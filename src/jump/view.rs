//! A workspace, live, in a card: the one picture behind a pin and the bar's
//! peek.
//!
//! The two showed the same thing and were built twice: the pin hugged the
//! workspace's own shape and said what it was, the peek letterboxed it into
//! a fixed 16:10 box with no name. Now both are a [`View`]: the picture,
//! fitted to the workspace's aspect inside [`picture_box`], over a footer
//! with the pin mark, the workspace's name, a hint that shows while the
//! pointer is on the card, and the caller's actions (the pin's ×, the
//! peek's pin button). The caller owns only where the card is: a corner
//! surface for a pin, the bar's popover for a peek.
//!
//! Frames come through `feed.rs`, so a pin and a peek of the same workspace
//! share one capture. [`View::stop`] lets go of it; a hidden view costs
//! nothing.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;

use super::card::{self, Live, LivePicture};
use super::feed::{self, Feed};
use super::live::Crop;
use super::scene::{self, Scene};

/// The picture's box, 16:10, at the size the Pins & previews setting names
/// (`settings::schema::PinSize`). A workspace is fitted inside it at its own
/// aspect, so the card hugs the picture.
///
/// Read once, when a view is made. A pin takes a new size by having its
/// surface rebuilt (`pin.rs`), a peek on its next open.
pub fn picture_box() -> (i32, i32) {
    crate::settings::store::with(|s| s.pins().size.picture())
}

/// Frames a second per window, for a pin and a peek alike: the setting's,
/// 30 unless it was changed (`settings::schema::Pins`).
///
/// Every frame costs a GPU copy in sway and a 0.6 ms average on the GPU
/// here at 2x (`gpu.rs`); neither reads the window back to the CPU any
/// more. Pins once took every frame (no cap), then 20, which read as
/// sub-par next to the real thing. At 30 a frame lands on every second
/// refresh of a 60 Hz screen, and the worker's one-grid batching still
/// repaints the picture once per step. One number for both, so a pin and a
/// peek of the same workspace share a stream without one of them
/// restarting it faster. 60 follows a video frame for frame; 15 is for a
/// pin that only needs to show that something moves.
pub fn fps() -> u32 {
    crate::settings::store::with(|s| s.pins().fps)
}

/// The pin mark, filled and struck through.
pub const PIN_GLYPH: &str = "\u{f0403}";
pub const UNPIN_GLYPH: &str = "\u{f0404}";

/// What the picture shows, to rebuild only when it changes.
#[derive(PartialEq)]
enum Shown {
    Nothing,
    Workspace(Option<Scene>),
    /// A piece of one window: its identifier, the piece, the window's size.
    Region(String, (i32, i32)),
}

pub struct View {
    root: gtk4::Box,
    holder: gtk4::Box,
    actions: gtk4::Box,
    live: Rc<RefCell<Live>>,
    feed: Option<Feed>,
    shown: Shown,
    /// Device pixels to one of GTK's where the card is drawn, for the size
    /// frames are cut to.
    scale: f64,
    /// The picture's box, [`picture_box`] when the view was made.
    size: (i32, i32),
}

impl View {
    /// An empty view named `label`, with `hint` for what a click does.
    pub fn new(label: &str, hint: &str) -> View {
        let root = crate::ui::vbox(2);
        root.add_css_class("jump-view");

        // The picture: a click on it is the caller's.
        let holder = crate::ui::vbox(0);
        holder.set_cursor_from_name(Some("pointer"));
        root.append(&holder);

        // The footer: what this is, what a click does, and the actions. The
        // hint and the actions are quiet until the pointer is on the card.
        let footer = crate::ui::hbox(3);
        footer.add_css_class("jump-view-footer");
        let mark = gtk4::Label::new(Some(PIN_GLYPH));
        crate::ui::glyph::adopt(&mark, crate::ui::Text::Label, crate::ui::Tone::Muted);
        footer.append(&mark);
        let name = crate::ui::text(label, crate::ui::Text::Label, crate::ui::Tone::Muted);
        crate::ui::set_weight(&name, crate::ui::Weight::Strong);
        footer.append(&name);
        let hint = crate::ui::text(hint, crate::ui::Text::Caption, crate::ui::Tone::Faint);
        hint.set_xalign(1.0);
        hint.set_hexpand(true);
        hint.add_css_class("jump-view-hint");
        footer.append(&hint);
        let actions = crate::ui::hbox(1);
        footer.append(&actions);
        root.append(&footer);

        View {
            root,
            holder,
            actions,
            live: Rc::default(),
            feed: None,
            shown: Shown::Nothing,
            scale: 1.0,
            size: picture_box(),
        }
    }

    /// The scale of the output the card is on, fractional when the output
    /// is. Frames are cut to the pictures' size in device pixels, so a 2x
    /// screen gets twice the pixels; a change starts the capture again at
    /// the new size. Call it before a show.
    pub fn set_scale(&mut self, scale: f64) {
        if (scale - self.scale).abs() > f64::EPSILON {
            self.scale = scale;
            self.feed = None;
        }
    }

    /// The card's content, for the caller to place.
    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    /// The picture, for the caller's click.
    pub fn picture(&self) -> &gtk4::Box {
        &self.holder
    }

    /// A small quiet button at the footer's end, `glyph` with `tooltip`.
    pub fn action(&self, glyph: &'static str, tooltip: &'static str) -> gtk4::Button {
        let button = crate::ui::button_with(
            crate::ui::Face::Glyph { glyph, tooltip },
            crate::ui::Kind::Flat,
            crate::ui::Size::Normal,
        );
        button.add_css_class("pill");
        button.add_css_class("small");
        button.add_css_class("jump-view-action");
        self.actions.append(&button);
        button
    }

    /// Show `scene` live at up to `fps` frames a second per window (0: every
    /// frame). Rebuilds only when the workspace changed shape or the view
    /// was stopped.
    pub fn show_scene(&mut self, scene: Option<Scene>, fps: u32) {
        let wanted = Shown::Workspace(scene);
        if self.shown == wanted && self.feed.is_some() {
            return;
        }
        let Shown::Workspace(scene) = &wanted else {
            unreachable!()
        };
        self.clear();
        let (bw, bh) = self.size;
        let (w, h) = scene.as_ref().map_or((bw, bh), |scene| {
            let (s, _, _) = scene::fit(scene.width, scene.height, bw, bh);
            (
                ((f64::from(scene.width) * s).round() as i32).clamp(8, bw),
                ((f64::from(scene.height) * s).round() as i32).clamp(8, bh),
            )
        });
        let picture = card::preview(
            scene.as_ref(),
            w,
            h,
            self.scale,
            &mut self.live.borrow_mut(),
        );
        self.holder.append(&picture);
        let wants = self.live.borrow().wants(None, self.scale);
        self.feed = feed::subscribe(wants, fps, &self.live);
        self.shown = wanted;
    }

    /// Show `crop` of the window `id`, which is `size` in sway's pixels.
    pub fn show_region(&mut self, id: &str, crop: Crop, size: (i32, i32), fps: u32) {
        let wanted = Shown::Region(id.to_string(), size);
        if self.shown == wanted && self.feed.is_some() {
            return;
        }
        self.clear();
        // The piece's own shape, fitted into the box.
        let (_, _, fw, fh) = crop;
        let (pw, ph) = (f64::from(size.0) * fw, f64::from(size.1) * fh);
        let (bw, bh) = self.size;
        let (s, _, _) = scene::fit(pw.round() as i32, ph.round() as i32, bw, bh);
        let picture = LivePicture::new();
        picture.set_size_request(
            ((pw * s).round() as i32).max(8),
            ((ph * s).round() as i32).max(8),
        );
        picture.set_halign(gtk4::Align::Center);
        self.holder.append(&picture);
        self.live.borrow_mut().add(id.to_string(), picture);
        let wants = self.live.borrow().wants(Some(crop), self.scale);
        self.feed = feed::subscribe(wants, fps, &self.live);
        self.shown = wanted;
    }

    /// Stop the frames. The picture stays as it was, and the next show
    /// starts them again.
    pub fn stop(&mut self) {
        self.feed = None;
    }

    fn clear(&mut self) {
        self.feed = None;
        while let Some(child) = self.holder.first_child() {
            self.holder.remove(&child);
        }
        *self.live.borrow_mut() = Live::default();
        self.shown = Shown::Nothing;
    }
}
