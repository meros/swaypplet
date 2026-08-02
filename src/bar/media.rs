//! Media mark — single dim achromatic ♪ in the right cluster
//! (docs/BAR_VISION.md, increments 4 and 8).
//!
//! Hidden while no player exists, `.paused` dims further when playback is
//! stopped/paused. No title text and no ambient progress — the mark is
//! ambient; prose (art + title/artist) and the play-pause action live in
//! the click-opened read-layer popover (bar/popover.rs). State comes from
//! `widgets::media::read_state` (playerctl batch on a worker thread),
//! refreshed every 3 s plus on every sway event — the pill's pre-existing
//! cadence, kept as-is: the event hook covers keyboard media bindings
//! (they emit window/tick traffic) without waiting out the poll, and a
//! `playerctl -F` follower child would need lifecycle management the poll
//! gets for free.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4::prelude::*;

use super::popover;
use crate::icons;
use crate::spawn::spawn_work;
use crate::sway_ipc::SwayService;
use crate::widgets::media::{self, MediaState, PlaybackStatus};

/// The mark, its popover and the last known player state — cloned into
/// every handler (GTK objects are refcounted, the state cell is shared).
#[derive(Clone)]
struct Ui {
    btn: gtk4::Button,
    pop: gtk4::Popover,
    body: gtk4::Box,
    state: Rc<RefCell<Option<MediaState>>>,
}

pub fn build(sway: &Rc<SwayService>) -> gtk4::Button {
    // Hidden until a player shows up.
    let btn = gtk4::Button::builder()
        .child(&gtk4::Label::new(Some("♪")))
        .css_classes(["bar-media-mark"])
        .visible(false)
        .build();
    let (pop, body) = popover::chassis(&btn);
    let ui = Ui {
        btn: btn.clone(),
        pop,
        body,
        state: Rc::new(RefCell::new(None)),
    };

    // Sway fires per-keystroke title snapshots; one playerctl batch in
    // flight at a time is plenty. Returns false once the button is gone
    // (output unplugged) so the poll timer can end itself.
    let refresh: Rc<dyn Fn() -> bool> = {
        let weak = btn.downgrade();
        let ui = ui.clone();
        let busy = Rc::new(Cell::new(false));
        Rc::new(move || {
            if weak.upgrade().is_none() {
                return false;
            }
            if busy.get() {
                return true;
            }
            busy.set(true);
            let busy = busy.clone();
            let ui = ui.clone();
            spawn_work(media::read_state, move |state| {
                busy.set(false);
                apply(&ui, state);
            });
            true
        })
    };

    refresh();
    {
        let refresh = refresh.clone();
        glib::timeout_add_seconds_local(3, move || {
            if refresh() {
                glib::ControlFlow::Continue
            } else {
                glib::ControlFlow::Break
            }
        });
    }
    {
        let refresh = refresh.clone();
        sway.connect_change(move || {
            refresh();
        });
    }

    {
        let ui = ui.clone();
        btn.connect_clicked(move |_| {
            render(&ui);
            ui.pop.popup();
        });
    }

    btn
}

fn apply(ui: &Ui, state: Option<MediaState>) {
    let present = state.is_some();
    let playing = matches!(&state, Some(ms) if ms.status == PlaybackStatus::Playing);
    *ui.state.borrow_mut() = state;
    ui.btn.set_visible(present);
    if playing {
        ui.btn.remove_css_class("paused");
    } else {
        ui.btn.add_css_class("paused");
    }
    if !present {
        ui.pop.popdown();
    } else if ui.pop.is_visible() {
        render(ui);
    }
}

/// Media section on the shared chassis: art + title/artist + play-pause.
/// Rebuilt at open and on state change while open — no per-second
/// position display, so nothing here needs a timer.
fn render(ui: &Ui) {
    while let Some(child) = ui.body.first_child() {
        ui.body.remove(&child);
    }
    let state = ui.state.borrow();
    let Some(ms) = &*state else {
        // The mark hides without a player; this only covers a race where
        // the popover outlives the state by one event.
        ui.body
            .append(&popover::line("No player", "bar-popover-empty"));
        return;
    };

    let row = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(12)
        .build();
    let frame = gtk4::Box::builder()
        .halign(gtk4::Align::Center)
        .valign(gtk4::Align::Center)
        .overflow(gtk4::Overflow::Hidden)
        .css_classes(["media-art-frame"])
        .build();
    match ms.art_path() {
        Some(path) => {
            let art = gtk4::Picture::builder()
                .content_fit(gtk4::ContentFit::Cover)
                .css_classes(["media-art"])
                .build();
            art.set_file(Some(&gtk4::gio::File::for_path(&path)));
            frame.append(&art);
        }
        None => {
            let fallback = gtk4::Label::new(Some("󰎆"));
            fallback.add_css_class("media-art-fallback");
            frame.append(&fallback);
        }
    }
    row.append(&frame);

    let info = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(2)
        .valign(gtk4::Align::Center)
        .build();
    let title = popover::line(
        if ms.title.is_empty() {
            "Unknown track"
        } else {
            &ms.title
        },
        "media-title",
    );
    title.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    title.set_max_width_chars(28);
    info.append(&title);
    if !ms.artist.is_empty() {
        let artist = popover::line(&ms.artist, "media-artist");
        artist.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        artist.set_max_width_chars(28);
        info.append(&artist);
    }
    row.append(&info);
    ui.body.append(&row);

    let play = gtk4::Button::builder()
        .label(if ms.status == PlaybackStatus::Playing {
            icons::MEDIA_PAUSE
        } else {
            icons::MEDIA_PLAY
        })
        .halign(gtk4::Align::Center)
        .css_classes(["media-btn", "media-play-pause"])
        .build();
    {
        let ui = ui.clone();
        play.connect_clicked(move |_| {
            let ui = ui.clone();
            spawn_work(
                || {
                    media::playerctl(&["play-pause"]);
                    media::read_state()
                },
                move |state| apply(&ui, state),
            );
        });
    }
    ui.body.append(&play);
}
