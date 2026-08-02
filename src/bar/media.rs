//! Media mark — single dim achromatic ♪ in the right cluster
//! (docs/BAR_VISION.md, increment 4; replaces the center media pill).
//!
//! Hidden while no player exists, `.paused` dims further when playback is
//! stopped/paused. No title text and no ambient progress — detail arrives
//! with the popover increment; click keeps the pill's play-pause action
//! meanwhile. State comes from `widgets::media::read_state` (playerctl
//! batch on a worker thread), refreshed every 3 s plus on every sway
//! event — the pill's pre-existing cadence, kept as-is: the event hook
//! covers keyboard media bindings (they emit window/tick traffic) without
//! waiting out the poll, and a `playerctl -F` follower child would need
//! lifecycle management the poll gets for free.

use std::cell::Cell;
use std::rc::Rc;

use gtk4::prelude::*;

use crate::spawn::spawn_work;
use crate::sway_ipc::SwayService;
use crate::widgets::media::{self, MediaState, PlaybackStatus};

pub fn build(sway: &Rc<SwayService>) -> gtk4::Button {
    // Hidden until a player shows up.
    let btn = gtk4::Button::builder()
        .child(&gtk4::Label::new(Some("♪")))
        .css_classes(["bar-media-mark"])
        .visible(false)
        .build();

    // Sway fires per-keystroke title snapshots; one playerctl batch in
    // flight at a time is plenty. Returns false once the button is gone
    // (output unplugged) so the poll timer can end itself.
    let refresh: Rc<dyn Fn() -> bool> = {
        let weak = btn.downgrade();
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
            spawn_work(media::read_state, move |state| {
                busy.set(false);
                apply(&btn, &state);
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

fn apply(btn: &gtk4::Button, state: &Option<MediaState>) {
    let Some(ms) = state else {
        btn.set_visible(false);
        return;
    };
    btn.set_visible(true);
    if ms.status == PlaybackStatus::Playing {
        btn.remove_css_class("paused");
    } else {
        btn.add_css_class("paused");
    }
}
