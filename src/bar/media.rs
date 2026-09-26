//! Media mark — single dim achromatic ♪ in the right cluster
//! (docs/BAR_VISION.md, increments 4 and 8).
//!
//! Hidden while no player exists; a stopped or paused player recedes
//! (`ui::set_receded`). No title text and no ambient progress — the mark is
//! ambient; prose (art + title/artist) and the play-pause action live in
//! the click-opened read-layer popover (bar/popover.rs). State comes from
//! `crate::mpris`, which the players push over D-Bus: nothing is polled,
//! and a mark whose state did not change is not touched.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;

use super::popover;
use crate::spawn::spawn_work;
use crate::ui;
use crate::ui::icons;
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

/// A transport key: a mark, muted, ink under the pointer.
fn control_button(face: &gtk4::Label) -> gtk4::Button {
    let b = ui::mark(face, false);
    b.add_css_class("bar-media-btn");
    b
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
    let art = gtk4::Image::builder().pixel_size(ART_PX).build();
    let text = gtk4::Label::builder()
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .max_width_chars(24)
        .xalign(0.0)
        .build();
    let info = ui::hbox(2);
    info.append(&art);
    info.append(&text);
    // The mark is ambient, so it sits a level lower than the transport.
    let btn = ui::mark(&info, true);
    btn.add_css_class("bar-media-mark");

    let play = gtk4::Label::new(Some(icons::MEDIA_PLAY));
    let prev_btn = control_button(&gtk4::Label::new(Some(icons::MEDIA_PREV)));
    let play_btn = control_button(&play);
    let next_btn = control_button(&gtk4::Label::new(Some(icons::MEDIA_NEXT)));

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
    // A stopped player recedes without disappearing.
    ui::set_receded(&ui.root, !playing);
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
            .append(&ui::text("No player", ui::Text::Label, ui::Tone::Muted));
        return;
    };

    let row = ui::hbox(4);
    let frame = ui::thumb();
    frame.add_css_class("bar-media-art-frame");
    match ms.art_path() {
        Some(path) => {
            let art = gtk4::Picture::builder()
                .content_fit(gtk4::ContentFit::Cover)
                .build();
            art.set_file(Some(&gtk4::gio::File::for_path(&path)));
            frame.append(&art);
        }
        None => {
            let fallback = gtk4::Label::new(Some("󰎆"));
            ui::glyph(&fallback, ui::Text::DisplaySm, ui::Tone::Muted);
            frame.append(&fallback);
        }
    }
    row.append(&frame);

    let info = ui::vbox(1);
    info.set_valign(gtk4::Align::Center);
    let title = ui::text(
        if ms.title.is_empty() {
            "Unknown track"
        } else {
            &ms.title
        },
        ui::Text::Body,
        ui::Tone::Fg,
    );
    title.add_css_class("ui-strong");
    title.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    title.set_max_width_chars(28);
    info.append(&title);
    if !ms.artist.is_empty() {
        let artist = ui::text(&ms.artist, ui::Text::Caption, ui::Tone::Muted);
        artist.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        artist.set_max_width_chars(28);
        info.append(&artist);
    }
    row.append(&info);
    ui.body.append(&row);

    let face = gtk4::Label::new(Some(if ms.status == PlaybackStatus::Playing {
        icons::MEDIA_PAUSE
    } else {
        icons::MEDIA_PLAY
    }));
    ui::glyph(&face, ui::Text::DisplaySm, ui::Tone::Fg);
    let play = gtk4::Button::builder()
        .child(&face)
        .halign(gtk4::Align::Center)
        .tooltip_text("Play or pause")
        .build();
    ui::make_button(&play, ui::Kind::Flat);
    play.add_css_class("icon");
    play.add_css_class("pill");
    play.add_css_class("bar-media-play");
    play.connect_clicked(|_| send("play-pause"));
    ui.body.append(&play);
}
