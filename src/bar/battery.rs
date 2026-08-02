//! Battery module — left segment of the bar's right instrument track.
//!
//! Instrument quieting (docs/BAR_VISION.md, increment 6): a dim gray
//! glyph at rest — no %, no hue — and the charging bolt stays gray too.
//! At ≤30 % the segment turns amber and the % text Reveals in; at ≤15 %
//! it turns static red with one onset nudge (P2: onset-only, nothing
//! loops). CSS classes change on threshold edges only, so the 30 s poll
//! never restyles a resting instrument. The decision slot's
//! battery-critical occupant shares [`CRITICAL_PCT`] (bar/decision.rs),
//! so slot escalation and the red tier fire on the same edge.
//!
//! Sysfs readers come from `widgets::power`; the icon table here is
//! waybar's 11-step ladder (power.rs keeps its own coarser 5-step icon
//! for the panel summary row). Click opens the read-layer chassis
//! (bar/popover.rs) with the battery section: time-to-empty prose + the
//! present draw in watts, one fresh sysfs read per open.

use std::cell::Cell;
use std::rc::Rc;

use gtk4::prelude::*;

use super::popover;
use crate::anim::{self, SlideBin};
use crate::widgets::power::{self, BatteryState};

const WARNING_PCT: u8 = 30;
/// Shared with the decision slot's battery occupant (bar/decision.rs).
pub(crate) const CRITICAL_PCT: u8 = 15;
const ICON_CHARGING: &str = "󰂄";
// Waybar's format-icons: one glyph per 10 % bucket, 0–100.
const ICONS: [&str; 11] = ["󰂎", "󰁺", "󰁻", "󰁼", "󰁽", "󰁾", "󰁿", "󰁿", "󰂁", "󰂂", "󰁹"];

/// Loudness tier; classes and the % reveal change only on tier edges.
/// Charging is always `Rest` — the stand-down table clears both warn and
/// critical on charger connect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tier {
    Rest,
    Warning,
    Critical,
}

fn tier(capacity: u8, charging: bool) -> Tier {
    if charging {
        Tier::Rest
    } else if capacity <= CRITICAL_PCT {
        Tier::Critical
    } else if capacity <= WARNING_PCT {
        Tier::Warning
    } else {
        Tier::Rest
    }
}

struct Ui {
    icon: gtk4::Label,
    pct: gtk4::Label,
    pct_reveal: gtk4::Revealer,
    slide: SlideBin,
    /// Last applied tier; `None` before the first read so startup styling
    /// is an edge (a bar born at 12 % must still go red) but never a
    /// nudge-worthy onset.
    tier: Cell<Option<Tier>>,
}

/// `None` on machines without a battery — the caller skips the segment so
/// the board keeps the track's rounded left end. `on_state` rides the
/// same 30 s poll (it feeds the decision slot's battery occupant, which
/// adds no poll of its own).
pub fn build(on_state: impl Fn(&BatteryState) + 'static) -> Option<gtk4::Box> {
    let path = power::find_battery_path()?;

    let icon = gtk4::Label::new(None);
    let pct = gtk4::Label::builder()
        .css_classes(["bar-battery-pct"])
        .build();
    let pct_reveal = gtk4::Revealer::builder()
        .transition_type(gtk4::RevealerTransitionType::SlideRight)
        .transition_duration(200)
        .child(&pct)
        .build();
    let content = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .build();
    content.append(&icon);
    content.append(&pct_reveal);
    let slide = SlideBin::new();
    slide.set_child(&content);
    let root = gtk4::Box::builder()
        .css_classes(["bar-battery", "bar-seg"])
        .build();
    root.append(&slide);

    // Read layer (vision increment 8): click opens the battery section.
    // One sysfs read per open — no timer while closed.
    {
        let (pop, body) = popover::chassis(&root);
        let path = path.clone();
        let click = gtk4::GestureClick::new();
        click.connect_released(move |_, _, _, _| {
            let Some(bat) = power::read_battery(&path) else {
                return;
            };
            while let Some(child) = body.first_child() {
                body.remove(&child);
            }
            body.append(&popover::line(
                &power::battery_summary_text(&bat),
                "bar-popover-desc",
            ));
            if let Some(watts) = power::watts_text(&bat) {
                body.append(&popover::line(&format!("Draw {watts}"), "bar-popover-meta"));
            }
            pop.popup();
        });
        root.add_controller(click);
    }

    let ui = Rc::new(Ui {
        icon,
        pct,
        pct_reveal,
        slide,
        tier: Cell::new(None),
    });
    if let Some(bat) = power::read_battery(&path) {
        apply(&root, &ui, &bat);
        on_state(&bat);
    }

    // Own 30 s timer, deliberately not is_mapped-gated like the panel's
    // (widgets/power.rs): the bar is always on screen, and the critical
    // escalation must not wait for a map event to start.
    let weak = root.downgrade();
    glib::timeout_add_seconds_local(30, move || {
        let Some(root) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        if let Some(bat) = power::read_battery(&path) {
            apply(&root, &ui, &bat);
            on_state(&bat);
        }
        // A failed read keeps the last-known display (transient sysfs
        // hiccups around suspend); no reason to stop polling.
        glib::ControlFlow::Continue
    });

    Some(root)
}

fn apply(root: &gtk4::Box, ui: &Ui, bat: &BatteryState) {
    ui.icon.set_label(icon(bat.capacity, bat.charging));
    root.set_tooltip_text(Some(&power::battery_summary_text(bat)));

    let t = tier(bat.capacity, bat.charging);
    if t != Tier::Rest {
        ui.pct.set_label(&format!("{}%", bat.capacity));
    }
    let prev = ui.tier.replace(Some(t));
    if prev == Some(t) {
        return;
    }
    // Threshold edge: the only place classes (and the % reveal) change.
    set_class(root, "warning", t == Tier::Warning);
    set_class(root, "critical", t == Tier::Critical);
    ui.pct_reveal.set_reveal_child(t != Tier::Rest);
    // One onset nudge on entering critical — never on a startup read.
    if t == Tier::Critical && prev.is_some() {
        anim::nudge(&ui.slide);
    }
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

    #[test]
    fn tiers_step_on_the_spec_thresholds() {
        assert_eq!(tier(100, false), Tier::Rest);
        assert_eq!(tier(31, false), Tier::Rest);
        assert_eq!(tier(30, false), Tier::Warning);
        assert_eq!(tier(16, false), Tier::Warning);
        assert_eq!(tier(15, false), Tier::Critical);
        assert_eq!(tier(0, false), Tier::Critical);
    }

    #[test]
    fn charging_stands_every_tier_down() {
        // Stand-down table: warn and critical both clear on charger
        // connect — the bolt at 5 % is a resting gray glyph.
        assert_eq!(tier(30, true), Tier::Rest);
        assert_eq!(tier(5, true), Tier::Rest);
    }
}
