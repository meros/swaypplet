//! Clock module — rightmost segment of the bar's instrument track.
//!
//! Ticks are one-shot timers aimed at the next minute boundary instead of
//! a free-running 60 s interval: every fire re-aims, so timer drift can
//! never let the readout lag the wall clock. Click toggles waybar's alt
//! format (date instead of time).

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gtk4::prelude::*;

// Waybar's clock formats: "󰥔 {:%H:%M}" with alt "󰃮 {:%Y-%m-%d}".
const ICON_TIME: &str = "󰥔";
const ICON_DATE: &str = "󰃮";

pub fn build() -> gtk4::Button {
    let label = gtk4::Label::new(None);
    let btn = gtk4::Button::builder()
        .child(&label)
        .css_classes(["bar-clock", "bar-seg"])
        .build();

    let show_date = Rc::new(Cell::new(false));

    // `false` once the button is gone (output unplugged) — the tick chain
    // ends itself instead of re-arming forever.
    let update: Rc<dyn Fn() -> bool> = {
        let weak = label.downgrade();
        let show_date = show_date.clone();
        Rc::new(move || {
            let Some(label) = weak.upgrade() else {
                return false;
            };
            if let Ok(now) = glib::DateTime::now_local() {
                label.set_label(&clock_text(&now, show_date.get()));
            }
            true
        })
    };

    update();
    {
        let update = update.clone();
        btn.connect_clicked(move |_| {
            show_date.set(!show_date.get());
            update();
        });
    }
    schedule_tick(update);

    btn
}

/// Arm a one-shot for the next minute boundary; each fire updates and
/// re-arms while the widget is alive.
fn schedule_tick(update: Rc<dyn Fn() -> bool>) {
    glib::timeout_add_local_once(millis_to_next_minute(), move || {
        if update() {
            schedule_tick(update);
        }
    });
}

fn millis_to_next_minute() -> Duration {
    let remaining_ms = match glib::DateTime::now_local() {
        // seconds() carries the fraction; +50ms lands safely past the
        // boundary so the fresh minute is already readable.
        Ok(now) => ((60.0 - now.seconds()) * 1000.0).ceil() as u64 + 50,
        Err(_) => 60_000,
    };
    Duration::from_millis(remaining_ms.clamp(50, 60_050))
}

/// Label text for `now` in the active format (unit-tested).
fn clock_text(now: &glib::DateTime, show_date: bool) -> String {
    let (icon, pattern) = if show_date {
        (ICON_DATE, "%Y-%m-%d")
    } else {
        (ICON_TIME, "%H:%M")
    };
    match now.format(pattern) {
        Ok(s) => format!("{icon} {s}"),
        Err(_) => icon.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(iso: &str) -> glib::DateTime {
        glib::DateTime::from_iso8601(iso, None).expect("valid fixture timestamp")
    }

    #[test]
    fn time_format_is_hours_minutes() {
        assert_eq!(
            clock_text(&at("2026-08-02T14:05:09+02:00"), false),
            "󰥔 14:05"
        );
    }

    #[test]
    fn alt_format_is_iso_date() {
        assert_eq!(
            clock_text(&at("2026-08-02T14:05:09+02:00"), true),
            "󰃮 2026-08-02"
        );
    }
}
