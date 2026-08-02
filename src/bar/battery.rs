//! Battery module — left segment of the bar's right instrument track.
//!
//! Sysfs readers come from `widgets::power`; the icon table here is
//! waybar's 11-step ladder (power.rs keeps its own coarser 5-step icon for
//! the panel summary row). States mirror waybar.nix: warning ≤30,
//! critical ≤15 (the CSS scopes the loud styling to `:not(.charging)`).

use gtk4::prelude::*;

use crate::widgets::power::{self, BatteryState};

const WARNING_PCT: u8 = 30;
const CRITICAL_PCT: u8 = 15;
const ICON_CHARGING: &str = "󰂄";
// Waybar's format-icons: one glyph per 10 % bucket, 0–100.
const ICONS: [&str; 11] = ["󰂎", "󰁺", "󰁻", "󰁼", "󰁽", "󰁾", "󰁿", "󰁿", "󰂁", "󰂂", "󰁹"];

/// `None` on machines without a battery — the caller skips the segment so
/// the clock keeps the track's rounded left end.
pub fn build() -> Option<gtk4::Label> {
    let path = power::find_battery_path()?;

    let label = gtk4::Label::builder()
        .css_classes(["bar-battery", "bar-seg"])
        .build();
    if let Some(bat) = power::read_battery(&path) {
        apply(&label, &bat);
    }

    // Own 30 s timer, deliberately not is_mapped-gated like the panel's
    // (widgets/power.rs): the bar is always on screen, and the critical
    // pulse must not wait for a map event to start.
    let weak = label.downgrade();
    glib::timeout_add_seconds_local(30, move || {
        let Some(label) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        if let Some(bat) = power::read_battery(&path) {
            apply(&label, &bat);
        }
        // A failed read keeps the last-known display (transient sysfs
        // hiccups around suspend); no reason to stop polling.
        glib::ControlFlow::Continue
    });

    Some(label)
}

fn apply(label: &gtk4::Label, bat: &BatteryState) {
    label.set_label(&format!(
        "{} {}%",
        icon(bat.capacity, bat.charging),
        bat.capacity
    ));
    label.set_tooltip_text(Some(&power::battery_summary_text(bat)));
    set_class(label, "charging", bat.charging);
    set_class(label, "warning", bat.capacity <= WARNING_PCT);
    set_class(label, "critical", bat.capacity <= CRITICAL_PCT);
}

fn set_class(widget: &impl IsA<gtk4::Widget>, class: &str, on: bool) {
    if on {
        widget.add_css_class(class);
    } else {
        widget.remove_css_class(class);
    }
}

/// Waybar's icon pick: linear 10 % buckets (unit-tested).
fn icon(capacity: u8, charging: bool) -> &'static str {
    if charging {
        return ICON_CHARGING;
    }
    ICONS[(capacity as usize / 10).min(ICONS.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_buckets_cover_the_range() {
        assert_eq!(icon(0, false), "󰂎");
        assert_eq!(icon(9, false), "󰂎");
        assert_eq!(icon(55, false), "󰁾");
        assert_eq!(icon(100, false), "󰁹");
        // Out-of-spec capacity must not index past the table.
        assert_eq!(icon(255, false), "󰁹");
    }

    #[test]
    fn charging_overrides_the_bucket() {
        assert_eq!(icon(3, true), ICON_CHARGING);
        assert_eq!(icon(100, true), ICON_CHARGING);
    }
}
