//! Presence module — walk-away readout on the instrument track.
//!
//! Shows what the idle manager is acting on: an eye while the ISH
//! human-presence sensor sees someone, a struck-through eye once it does not.
//! It earns bar space because presence now decides when the session locks, and
//! a lock with no visible cause (or a session that pointedly refuses to lock)
//! is otherwise unexplainable from the outside.
//!
//! Polled at 1 Hz, not the idle manager's 250 ms: this is a readout for a
//! human, and the sensor's own debounce is measured in seconds. The instant
//! value is used rather than a debounced one so the bar reflects the sensor
//! itself, including the blips the idle rules deliberately swallow.
//!
//! Returns None on hardware without the sensor, and the track simply omits the
//! segment.

use std::rc::Rc;

use gtk4::prelude::*;

use crate::presence::Presence;

// Nerd Font: md-eye / md-eye_off.
const ICON_PRESENT: &str = "󰈈";
const ICON_AWAY: &str = "󰈉";

pub fn build() -> Option<gtk4::Button> {
    let sensor = Rc::new(Presence::detect()?);

    let label = gtk4::Label::new(Some(ICON_PRESENT));
    let btn = gtk4::Button::builder()
        .child(&label)
        .css_classes(["bar-presence", "bar-seg"])
        .build();

    let apply = {
        let sensor = sensor.clone();
        move |btn: &gtk4::Button, label: &gtk4::Label| {
            let present = sensor.read();
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
            let attention = sensor
                .attention()
                .map(|a| format!(", attention {a}"))
                .unwrap_or_default();
            btn.set_tooltip_text(Some(&match present {
                Some(true) => format!("Present{attention}"),
                Some(false) => format!("Away{attention}"),
                None => "Presence sensor unreadable".to_string(),
            }));
        }
    };

    apply(&btn, &label);

    let weak_btn = btn.downgrade();
    let weak_label = label.downgrade();
    glib::timeout_add_seconds_local(1, move || {
        let (Some(btn), Some(label)) = (weak_btn.upgrade(), weak_label.upgrade()) else {
            return glib::ControlFlow::Break;
        };
        apply(&btn, &label);
        glib::ControlFlow::Continue
    });

    Some(btn)
}
