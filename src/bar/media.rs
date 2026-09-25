//! Media mark — single dim achromatic ♪ in the right cluster
//! (docs/BAR_VISION.md, increments 4 and 8).
//!
//! Hidden while no player exists, `.paused` dims further when playback is
//! stopped/paused. No title text and no ambient progress — the mark is
//! ambient; prose (art + title/artist) and the play-pause action live in
//! the click-opened read-layer popover (bar/popover.rs). State comes from
//! `crate::mpris`, which the players push over D-Bus: nothing is polled,
//! and a mark whose state did not change is not touched.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;

use super::popover;
use crate::icons;
use crate::spawn::spawn_work;
use crate::widgets::media::{self, MediaState, PlaybackStatus};

/// The mark, its popover and the last known player state — cloned into
/// every handler (GTK objects are refcounted, the state cell is shared).
#[derive(Clone)]
struct Ui {
    root: gtk4::Box,
    art: gtk4::Image,
    text: gtk4::Label,
    play: gtk4::Label,
    pop: gtk4::Popover,
    body: gtk4::Box,
    state: Rc<RefCell<Option<MediaState>>>,
}

fn control_button(glyph: &str) -> gtk4::Button {
    gtk4::Button::builder()
        .child(&gtk4::Label::new(Some(glyph)))
        .css_classes(["bar-media-btn"])
        .build()
}

/// Fire a playerctl transport command off-thread; playerctl blocks on
/// D-Bus, which must not happen on the GTK thread. The player announces
/// the result itself (crate::mpris), so nothing is read back here.
fn send(cmd: &'static str) {
    spawn_work(
        move || {
            media::playerctl(&[cmd]);
        },
        |_| {},
    );
}

/// Album-art edge in the bar. Small enough to sit inside the bar's height
/// without forcing the row taller.
const ART_PX: i32 = 18;

pub fn build(mpris: &Rc<crate::mpris::MprisService>) -> gtk4::Box {
    // Art + title/artist open the popover; the transport keys are siblings,
    // not children, because GTK4 gives a Button's clicks to the Button and a
    // nested control would never see them.
    let art = gtk4::Image::builder()
        .pixel_size(ART_PX)
        .css_classes(["bar-media-art"])
        .build();
    let text = gtk4::Label::builder()
        .css_classes(["bar-media-text"])
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .max_width_chars(24)
        .xalign(0.0)
        .build();
    let info = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(6)
        .build();
    info.append(&art);
    info.append(&text);
    let btn = gtk4::Button::builder()
        .child(&info)
        .css_classes(["bar-media-mark"])
        .build();

    let play = gtk4::Label::new(Some(icons::MEDIA_PLAY));
    let prev_btn = control_button(icons::MEDIA_PREV);
    let play_btn = gtk4::Button::builder()
        .child(&play)
        .css_classes(["bar-media-btn"])
        .build();
    let next_btn = control_button(icons::MEDIA_NEXT);

    // Hidden until a player shows up.
    let root = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .css_classes(["bar-media"])
        .visible(false)
        .build();
    root.append(&btn);
    root.append(&prev_btn);
    root.append(&play_btn);
    root.append(&next_btn);

    let (pop, body) = popover::chassis(&btn);
    let ui = Ui {
        root: root.clone(),
        art,
        text,
        play,
        pop,
        body,
        state: Rc::new(RefCell::new(None)),
    };

    // Pushed by the players themselves (crate::mpris): no poll, and nothing
    // runs while nothing changes.
    apply(&ui, mpris.snapshot());
    {
        let weak = btn.downgrade();
        let ui = ui.clone();
        let mpris_c = mpris.clone();
        mpris.connect_change(move || {
            // The service outlives a bar unplugged with its output.
            if weak.upgrade().is_some() {
                apply(&ui, mpris_c.snapshot());
            }
        });
    }

    {
        let ui = ui.clone();
        btn.connect_clicked(move |_| {
            render(&ui);
            ui.pop.popup();
        });
    }

    for (button, cmd) in [
        (&prev_btn, "previous"),
        (&play_btn, "play-pause"),
        (&next_btn, "next"),
    ] {
        button.connect_clicked(move |_| send(cmd));
    }

    root
}

fn apply(ui: &Ui, state: Option<MediaState>) {
    // Nothing to redraw for a state already on screen. Setting the art again
    // decodes the image from disk and lays the bar out anew.
    if *ui.state.borrow() == state && ui.root.is_visible() == state.is_some() {
        return;
    }
    let art_before = ui.state.borrow().as_ref().and_then(MediaState::art_path);
    let present = state.is_some();
    let playing = matches!(&state, Some(ms) if ms.status == PlaybackStatus::Playing);

    if let Some(ms) = &state {
        // Art is optional and often absent (remote URLs are not fetched), so
        // the image collapses rather than holding an empty box.
        match ms.art_path() {
            Some(path) => {
                if art_before.as_deref() != Some(path.as_str()) {
                    ui.art.set_from_file(Some(&path));
                }
                ui.art.set_visible(true);
            }
            None => ui.art.set_visible(false),
        }
        ui.text.set_label(&pill_text(ms));
        ui.play.set_label(if playing {
            icons::MEDIA_PAUSE
        } else {
            icons::MEDIA_PLAY
        });
    }

    *ui.state.borrow_mut() = state;
    ui.root.set_visible(present);
    if playing {
        ui.root.remove_css_class("paused");
    } else {
        ui.root.add_css_class("paused");
    }
    if !present {
        ui.pop.popdown();
    } else if ui.pop.is_visible() {
        render(ui);
    }
}

/// One line for the pill: "Title · Artist", or whichever half exists. The
/// label ellipsizes, so a long track name cannot push the clock off the bar.
fn pill_text(ms: &MediaState) -> String {
    match (ms.title.trim(), ms.artist.trim()) {
        ("", "") => "♪".to_owned(),
        (title, "") => title.to_owned(),
        ("", artist) => artist.to_owned(),
        (title, artist) => format!("{} · {}", title, artist),
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
    play.connect_clicked(|_| send("play-pause"));
    ui.body.append(&play);
}
