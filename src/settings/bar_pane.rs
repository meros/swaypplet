//! The Bar tab: what the bar does that is a matter of taste, and what the
//! volume and brightness keys do.
//!
//! Every row here is read live by something in this process — the clock
//! (`bar/clock.rs`), the segments (`bar/mod.rs`), the OSD and its route
//! (`osd.rs`, `app.rs`), the panel's volume rail (`widgets/audio.rs`) —
//! through `store::observe` or per press, so a switch takes effect on
//! release and the file is only there for the next start.
//!
//! Two sections on one tab: `bar` and `keys`. They share a footer, so Reset
//! and Copy as Nix cover both.

use std::cell::Cell;
use std::rc::Rc;

use gtk4::prelude::*;

use super::store::{self, Bar, Keys};
use super::ui::{self, dropdown_row, scale_row, section_box, switch_row};

/// Where a volume or brightness press draws, in the order the dropdown
/// lists them.
const OSD_PLACES: [(&str, bool); 2] = [("Centre card", false), ("In the bar", true)];

/// One segment switch: its row and which field it flips.
struct Segment {
    label: &'static str,
    hint: &'static str,
    get: fn(&Bar) -> bool,
    set: fn(&mut Bar, bool),
}

const SEGMENTS: [Segment; 5] = [
    Segment {
        label: "Media mark",
        hint: "What is playing, at the left of the right cluster.",
        get: |b| b.media,
        set: |b, v| b.media = v,
    },
    Segment {
        label: "Tray",
        hint: "Status-notifier icons from applications.",
        get: |b| b.tray,
        set: |b, v| b.tray = v,
    },
    Segment {
        label: "Battery",
        hint: "The battery segment of the instrument track. Nothing to hide on a machine without one.",
        get: |b| b.battery,
        set: |b, v| b.battery = v,
    },
    Segment {
        label: "Presence",
        hint: "The presence sensor's mark. Nothing to hide on a machine without the sensor.",
        get: |b| b.presence,
        set: |b, v| b.presence = v,
    },
    Segment {
        label: "Task board",
        hint: "The four-bay instrument in the right track, one bay per task 1–4.",
        get: |b| b.board,
        set: |b, v| b.board = v,
    },
];

/// The status line at the system default, naming what the default is.
fn describe(bar: &Bar, keys: &Keys) -> String {
    let hidden: Vec<&str> = SEGMENTS
        .iter()
        .filter(|s| !(s.get)(bar))
        .map(|s| s.label)
        .collect();
    format!(
        "System default: {} clock{}, volume and brightness {}, {} hidden, {}% steps{}",
        if bar.clock_24h { "24-hour" } else { "12-hour" },
        if bar.clock_date { " with the date" } else { "" },
        if bar.osd_in_bar {
            "in the bar"
        } else {
            "as the centre card"
        },
        if hidden.is_empty() {
            "nothing".to_string()
        } else {
            hidden.join(" and ").to_lowercase()
        },
        keys.volume_step,
        if keys.volume_boost {
            ", boost allowed"
        } else {
            ""
        },
    )
}

struct State {
    clock_24h: gtk4::Switch,
    clock_date: gtk4::Switch,
    osd: gtk4::DropDown,
    segments: Vec<gtk4::Switch>,
    volume_step: gtk4::Scale,
    brightness_step: gtk4::Scale,
    boost: gtk4::Switch,
    status: gtk4::Label,
    updating: Cell<bool>,
}

impl State {
    fn edit_bar(&self, f: impl FnOnce(&mut Bar)) {
        if self.updating.get() {
            return;
        }
        store::edit(f);
        self.sync();
    }

    fn edit_keys(&self, f: impl FnOnce(&mut Keys)) {
        if self.updating.get() {
            return;
        }
        store::edit(f);
        self.sync();
    }

    fn sync(&self) {
        self.updating.set(true);
        let settings = store::current();
        let bar = settings.bar();
        let keys = settings.keys();
        self.clock_24h.set_active(bar.clock_24h);
        self.clock_date.set_active(bar.clock_date);
        let osd = OSD_PLACES
            .iter()
            .position(|(_, in_bar)| *in_bar == bar.osd_in_bar);
        self.osd.set_selected(osd.unwrap_or(0) as u32);
        for (segment, switch) in SEGMENTS.iter().zip(&self.segments) {
            switch.set_active((segment.get)(&bar));
        }
        self.volume_step.set_value(f64::from(keys.volume_step));
        self.brightness_step
            .set_value(f64::from(keys.brightness_step));
        self.boost.set_active(keys.volume_boost);
        ui::set_source(
            &self.status,
            settings.bar.is_some() || settings.keys.is_some(),
            &describe(&bar, &keys),
        );
        self.updating.set(false);
    }
}

pub struct BarPane {
    root: gtk4::Box,
    state: Rc<State>,
}

impl BarPane {
    pub fn new() -> Self {
        let root = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(14)
            .build();

        let settings = store::current();
        let bar = settings.bar();
        let keys = settings.keys();

        let clock = section_box(
            "Clock",
            "The rightmost segment of the bar. Clicking it still flips to the ISO date.",
        );
        let (row_24h, clock_24h) =
            switch_row("24-hour clock", "14:05 rather than 2:05 PM.", bar.clock_24h);
        clock.append(&row_24h);
        let (row_date, clock_date) = switch_row(
            "Show the date",
            "Weekday, day and month beside the time.",
            bar.clock_date,
        );
        clock.append(&row_date);

        let segments_group = section_box(
            "Segments",
            "The right cluster, one switch per segment. Hidden, not stopped: the service behind each still runs.",
        );
        let mut segments = Vec::new();
        for segment in &SEGMENTS {
            let (row, switch) = switch_row(segment.label, segment.hint, (segment.get)(&bar));
            segments_group.append(&row);
            segments.push(switch);
        }

        let keys_group = section_box(
            "Keys",
            "The volume and brightness keys: where the press draws, and how far it goes.",
        );
        let osd_labels: Vec<&str> = OSD_PLACES.iter().map(|(l, _)| *l).collect();
        let (row_osd, osd) = dropdown_row(
            "Volume & brightness",
            "The centre card can be read through and works over fullscreen; the bar's decision slot costs a glance to the bottom edge and is skipped over a fullscreen view.",
            &osd_labels,
        );
        keys_group.append(&row_osd);
        let (row_vol, volume_step) = scale_row(
            "Volume step",
            "Percent per press of a volume key.",
            (1.0, 25.0, 1.0),
            |v| format!("{v:.0}%"),
        );
        keys_group.append(&row_vol);
        let (row_bri, brightness_step) = scale_row(
            "Brightness step",
            "Percent per press of a brightness key.",
            (1.0, 25.0, 1.0),
            |v| format!("{v:.0}%"),
        );
        keys_group.append(&row_bri);
        let (row_boost, boost) = switch_row(
            "Volume past 100 %",
            "Let the keys and the panel's rail go to the 150 % the sound server allows.",
            keys.volume_boost,
        );
        keys_group.append(&row_boost);

        let reset = ui::action_button(
            "Reset to system",
            "Put the system's choices back and drop the bar and keys sections from the settings file.",
        );
        let (footer, status) = ui::footer(&[&reset]);
        let copy = ui::copy_nix_button(
            &status,
            "The bar and keys sections as theme/settings.nix holds them, for promoting a keeper into the Nix side by hand.",
            || {
                let s = store::current();
                Some(format!(
                    "{}{}",
                    s.section_as_nix("bar")?,
                    s.section_as_nix("keys")?
                ))
            },
        );
        if let Some(row) = reset.parent().and_downcast::<gtk4::Box>() {
            row.append(&copy);
        }

        let state = Rc::new(State {
            clock_24h: clock_24h.clone(),
            clock_date: clock_date.clone(),
            osd: osd.clone(),
            segments: segments.clone(),
            volume_step: volume_step.clone(),
            brightness_step: brightness_step.clone(),
            boost: boost.clone(),
            status,
            updating: Cell::new(false),
        });

        {
            let state = state.clone();
            clock_24h.connect_active_notify(move |s| {
                let on = s.is_active();
                state.edit_bar(|b| b.clock_24h = on);
            });
        }
        {
            let state = state.clone();
            clock_date.connect_active_notify(move |s| {
                let on = s.is_active();
                state.edit_bar(|b| b.clock_date = on);
            });
        }
        {
            let state = state.clone();
            osd.connect_selected_notify(move |d| {
                if let Some((_, in_bar)) = OSD_PLACES.get(d.selected() as usize) {
                    state.edit_bar(|b| b.osd_in_bar = *in_bar);
                }
            });
        }
        for (index, switch) in segments.iter().enumerate() {
            let state = state.clone();
            switch.connect_active_notify(move |s| {
                let on = s.is_active();
                state.edit_bar(|b| (SEGMENTS[index].set)(b, on));
            });
        }
        {
            let state = state.clone();
            volume_step.connect_value_changed(move |s| {
                let step = s.value().round() as u8;
                state.edit_keys(|k| k.volume_step = step);
            });
        }
        {
            let state = state.clone();
            brightness_step.connect_value_changed(move |s| {
                let step = s.value().round() as u8;
                state.edit_keys(|k| k.brightness_step = step);
            });
        }
        {
            let state = state.clone();
            boost.connect_active_notify(move |s| {
                let on = s.is_active();
                state.edit_keys(|k| k.volume_boost = on);
            });
        }
        {
            let state = state.clone();
            reset.connect_clicked(move |_| {
                if state.updating.get() {
                    return;
                }
                store::reset::<Bar>();
                store::reset::<Keys>();
                state.sync();
            });
        }

        root.append(&clock);
        root.append(&segments_group);
        root.append(&keys_group);
        root.append(&footer);
        state.sync();

        BarPane { root, state }
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    pub fn refresh(&self) {
        self.state.sync();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_segment_switch_moves_exactly_one_field() {
        let base = Bar::default();
        let mut moved = Vec::new();
        for segment in &SEGMENTS {
            let mut b = base;
            (segment.set)(&mut b, !(segment.get)(&base));
            let json = serde_json::to_value(b).unwrap();
            let base_json = serde_json::to_value(base).unwrap();
            let changed: Vec<String> = json
                .as_object()
                .unwrap()
                .iter()
                .filter(|(k, v)| base_json[k.as_str()] != **v)
                .map(|(k, _)| k.clone())
                .collect();
            assert_eq!(changed.len(), 1, "{} moved {changed:?}", segment.label);
            moved.push(changed[0].clone());
        }
        moved.sort();
        assert_eq!(moved, ["battery", "board", "media", "presence", "tray"]);
    }
}
