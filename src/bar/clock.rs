//! Clock module — rightmost segment of the bar's instrument track.
//!
//! Ticks are one-shot timers aimed at the next minute boundary instead of
//! a free-running 60 s interval: every fire re-aims, so timer drift can
//! never let the readout lag the wall clock. Click toggles waybar's alt
//! format (date instead of time).
//!
//! The format comes from the Bar tab of the settings pane
//! (`settings::store::Bar`): 24-hour or 12-hour, and whether the date rides
//! beside the time. Both are read at every tick, so a change lands on the
//! next minute at the latest and, through the observer, at once.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gtk4::prelude::*;

use crate::settings::store::{self, Bar};

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
                let bar = store::with(|s| s.bar());
                label.set_label(&clock_text(&now, show_date.get(), &bar));
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
    {
        let update = update.clone();
        store::observe(move || {
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
///
/// The alt view is always the ISO date, whatever the settings say: it is
/// the one you click for, and it is there to be copied.
fn clock_text(now: &glib::DateTime, show_date: bool, bar: &Bar) -> String {
    if show_date {
        return match now.format("%Y-%m-%d") {
            Ok(s) => format!("{ICON_DATE} {s}"),
            Err(_) => ICON_DATE.to_string(),
        };
    }
    let time = if bar.clock_24h { "%H:%M" } else { "%-I:%M %p" };
    let pattern = if bar.clock_date {
        format!("{time}  %a %-d %b")
    } else {
        time.to_string()
    };
    match now.format(&pattern) {
        Ok(s) => format!("{ICON_TIME} {s}"),
        Err(_) => ICON_TIME.to_string(),
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
            clock_text(&at("2026-08-02T14:05:09+02:00"), false, &Bar::default()),
            "󰥔 14:05"
        );
    }

    #[test]
    fn alt_format_is_iso_date_whatever_the_settings() {
        let bar = Bar {
            clock_24h: false,
            clock_date: true,
            ..Bar::default()
        };
        assert_eq!(
            clock_text(&at("2026-08-02T14:05:09+02:00"), true, &bar),
            "󰃮 2026-08-02"
        );
    }

    #[test]
    fn twelve_hour_and_the_date_beside_the_time() {
        let bar = Bar {
            clock_24h: false,
            clock_date: true,
            ..Bar::default()
        };
        assert_eq!(
            clock_text(&at("2026-08-02T14:05:09+02:00"), false, &bar),
            "󰥔 2:05 PM  Sun 2 Aug"
        );
        let bar = Bar {
            clock_date: true,
            ..Bar::default()
        };
        assert_eq!(
            clock_text(&at("2026-08-02T09:05:09+02:00"), false, &bar),
            "󰥔 09:05  Sun 2 Aug"
        );
    }
}
