//! Presence module — walk-away readout on the instrument track.
//!
//! Shows what the idle manager is acting on: an eye while the ISH
//! human-presence sensor sees someone, a struck-through eye once it does not.
//! It earns bar space because presence now decides when the session locks, and
//! a lock with no visible cause (or a session that pointedly refuses to lock)
//! is otherwise unexplainable from the outside.
//!
//! Pushed, not polled. This used to read the sensor itself on a 1 Hz timer,
//! twice a tick for the value and the tooltip, and those reads run on the GTK
//! main thread for 250–400 ms apiece (see `crate::presence`). A readout was
//! parking the main loop for roughly half of every second, and every keypress
//! that arrived in that window queued behind it. Nothing here touches the
//! device now; the idle manager owns it and this listens.
//!
//! Returns None on hardware without the sensor, and the track simply omits the
//! segment.

use gtk4::prelude::*;

use crate::presence::{self, Event};

// Nerd Font: md-eye / md-eye_off.
const ICON_PRESENT: &str = "󰈈";
const ICON_AWAY: &str = "󰈉";

pub fn build() -> Option<gtk4::Button> {
    // Detection is one read at startup, off the hot path, and it decides
    // whether the segment exists at all.
    presence::Presence::detect()?;

    let label = gtk4::Label::new(Some(ICON_PRESENT));
    let btn = gtk4::Button::builder()
        .child(&label)
        .css_classes(["bar-presence", "bar-seg"])
        .build();

    let events = presence::subscribe();

    let weak_btn = btn.downgrade();
    let weak_label = label.downgrade();
    glib::spawn_future_local(async move {
        // Held between events so the tooltip can carry both without either
        // one arriving forcing a read of the other.
        let mut present: Option<bool> = None;
        let mut attention: Option<i32> = None;

        while let Ok(event) = events.recv().await {
            match event {
                Event::Changed(state) => present = Some(state),
                Event::Attention(value) => attention = Some(value),
            }

            let (Some(btn), Some(label)) = (weak_btn.upgrade(), weak_label.upgrade()) else {
                return;
            };
            draw(&btn, &label, present, attention);
        }
    });

    Some(btn)
}

fn draw(btn: &gtk4::Button, label: &gtk4::Label, present: Option<bool>, attention: Option<i32>) {
    label.set_label(if present == Some(true) {
        ICON_PRESENT
    } else {
        ICON_AWAY
    });
    if present == Some(true) {
        btn.remove_css_class("presence-away");
    } else {
        btn.add_css_class("presence-away");
    }

    let attention = attention
        .map(|a| format!(", attention {a}"))
        .unwrap_or_default();
    btn.set_tooltip_text(Some(&match present {
        Some(true) => format!("Present{attention}"),
        Some(false) => format!("Away{attention}"),
        None => "Presence sensor unreadable".to_string(),
    }));
}
