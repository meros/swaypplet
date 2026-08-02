//! Media module — center pill mirroring waybar's custom/media.
//!
//! State comes from `widgets::media::read_state` (playerctl batch on a
//! worker thread), refreshed every 3 s plus on every sway event — the
//! event hook covers keyboard media bindings (they emit window/tick
//! traffic) without waiting out the poll. A `playerctl -F` follower child
//! would push updates instantly; deferred for now — it needs child
//! lifecycle management the poll gets for free.

use std::cell::Cell;
use std::rc::Rc;

use gtk4::prelude::*;

use crate::spawn::spawn_work;
use crate::sway_ipc::SwayService;
use crate::widgets::media::{self, MediaState, PlaybackStatus};

// Waybar parity: format "󰐊 {}", max-length 40.
const ICON: &str = "󰐊";
const MAX_CHARS: i32 = 40;

pub fn build(sway: &Rc<SwayService>) -> gtk4::Button {
    let label = gtk4::Label::builder()
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .max_width_chars(MAX_CHARS)
        .build();
    // Hidden until a player shows up, like waybar's empty custom module.
    let btn = gtk4::Button::builder()
        .child(&label)
        .css_classes(["bar-media"])
        .visible(false)
        .build();

    // Sway fires per-keystroke title snapshots; one playerctl batch in
    // flight at a time is plenty. Returns false once the button is gone
    // (output unplugged) so the poll timer can end itself.
    let refresh: Rc<dyn Fn() -> bool> = {
        let weak = btn.downgrade();
        let label = label.clone();
        let busy = Rc::new(Cell::new(false));
        Rc::new(move || {
            let Some(btn) = weak.upgrade() else {
                return false;
            };
            if busy.get() {
                return true;
            }
            busy.set(true);
            let busy = busy.clone();
            let label = label.clone();
            spawn_work(media::read_state, move |state| {
                busy.set(false);
                apply(&btn, &label, &state);
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

    btn.connect_clicked(move |_| {
        let refresh = refresh.clone();
        spawn_work(
            || {
                media::playerctl(&["play-pause"]);
            },
            move |()| {
                refresh();
            },
        );
    });

    btn
}

fn apply(btn: &gtk4::Button, label: &gtk4::Label, state: &Option<MediaState>) {
    let Some(ms) = state else {
        btn.set_visible(false);
        return;
    };
    btn.set_visible(true);
    label.set_label(&format!("{ICON} {}", track_text(&ms.artist, &ms.title)));
    if ms.status == PlaybackStatus::Playing {
        btn.add_css_class("playing");
    } else {
        btn.remove_css_class("playing");
    }
}

/// "artist - title", dropping whichever side is missing (unit-tested).
/// Waybar rendered a bare " - title" for artistless streams; joining only
/// non-empty parts reads better.
fn track_text(artist: &str, title: &str) -> String {
    match (artist.is_empty(), title.is_empty()) {
        (false, false) => format!("{artist} - {title}"),
        (true, false) => title.to_string(),
        (false, true) => artist.to_string(),
        (true, true) => "Unknown track".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_text_joins_only_present_parts() {
        assert_eq!(track_text("Kraftwerk", "Autobahn"), "Kraftwerk - Autobahn");
        assert_eq!(track_text("", "Autobahn"), "Autobahn");
        assert_eq!(track_text("Kraftwerk", ""), "Kraftwerk");
        assert_eq!(track_text("", ""), "Unknown track");
    }
}
