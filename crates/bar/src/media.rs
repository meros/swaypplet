//! Media mark — single dim achromatic ♪ in the right cluster
//! (docs/BAR_VISION.md, increments 4 and 8).
//!
//! Hidden while no player exists; a stopped or paused player recedes
//! (`ui::set_receded`). No title text and no ambient progress — the mark is
//! ambient; prose (art + title/artist) and the play-pause action live in
//! the click-opened read-layer popover (`popover.rs`). State comes from
//! `swaypplet_system::mpris`, which the players push over D-Bus: nothing is
//! polled, and a mark whose state did not change is not touched.

use std::cell::RefCell;
use std::rc::Rc;
use swaypplet_system::mpris::Control;

use gtk4::prelude::*;

use super::popover;
use swaypplet_system::mpris::{MediaState, PlaybackStatus};
use swaypplet_ui::ui;
use swaypplet_ui::ui::icons;

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
    /// "Play on": the output picker, kept across renders
    /// (`swaypplet_widgets::audio::output_picker`).
    outputs: gtk4::Box,
    /// Where the transport keys go: the player the bar shows.
    mpris: Rc<swaypplet_system::mpris::MprisService>,
}

/// A transport key: a mark, muted, ink under the pointer.
fn control_button(face: &gtk4::Label) -> gtk4::Button {
    let b = ui::mark(face, false);
    b.add_css_class("bar-media-btn");
    b
}

/// Album-art edge in the bar. Small enough to sit inside the bar's height
/// without forcing the row taller.
const ART_PX: i32 = 18;

pub fn build(
    mpris: &Rc<swaypplet_system::mpris::MprisService>,
    audio: &Rc<swaypplet_audio::AudioService>,
) -> gtk4::Box {
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
        outputs: swaypplet_widgets::audio::output_picker(audio),
        mpris: mpris.clone(),
    };

    // Pushed by the players themselves (`swaypplet_system::mpris`): no poll,
    // and nothing runs while nothing changes. The bar shows no position, so
    // a read that only moved it is the state already on screen.
    let snapshot = |mpris: &swaypplet_system::mpris::MprisService| {
        mpris.snapshot().map(MediaState::without_position)
    };
    apply(&ui, snapshot(mpris));
    {
        let weak = btn.downgrade();
        let ui = ui.clone();
        let mpris_c = mpris.clone();
        mpris.connect_change(move || {
            // The service outlives a bar unplugged with its output.
            if weak.upgrade().is_some() {
                apply(&ui, snapshot(&mpris_c));
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

    for (button, control) in [
        (&prev_btn, Control::Previous),
        (&play_btn, Control::PlayPause),
        (&next_btn, Control::Next),
    ] {
        let mpris = mpris.clone();
        button.connect_clicked(move |_| mpris.control(control));
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
        // The label ellipsizes, so a long track name cannot push the clock
        // off the bar.
        ui.text.set_label(&ms.pill_text());
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
            ui::glyph::adopt(&fallback, ui::Text::DisplaySm, ui::Tone::Muted);
            frame.append(&fallback);
        }
    }
    row.append(&frame);

    let info = ui::vbox(1);
    info.set_valign(gtk4::Align::Center);
    let title = ui::text(ms.display_title(), ui::Text::Body, ui::Tone::Fg);
    swaypplet_ui::ui::set_weight(&title, swaypplet_ui::ui::Weight::Strong);
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
    ui::glyph::adopt(&face, ui::Text::DisplaySm, ui::Tone::Fg);
    let play = gtk4::Button::builder()
        .child(&face)
        .halign(gtk4::Align::Center)
        .tooltip_text("Play or pause")
        .build();
    ui::button::adopt(&play, ui::Kind::Flat, ui::Size::Normal);
    play.add_css_class("icon");
    play.add_css_class("pill");
    play.add_css_class("bar-media-play");
    let mpris = ui.mpris.clone();
    play.connect_clicked(move |_| mpris.control(Control::PlayPause));
    ui.body.append(&play);
    ui.body.append(&ui.outputs);
}
