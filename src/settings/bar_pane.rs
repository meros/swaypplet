//! The Bar tab: what the bar does that is a matter of taste, where a volume
//! or brightness press draws, and the pins and peeks the bar keeps. How far
//! a press goes is on the Input tab (`input_pane.rs`), with the rest of the
//! keys.
//!
//! Two sections, `bar` and `pins`, on one tab with one footer, the way
//! Alerts carries `alerts` and `capture`. Pins are here rather than on a tab
//! of their own because the bar is where they live when they are not
//! floating (`bar/pins.rs`), and the peek is the bar's.
//!
//! Every row here is read live by something in this process — the clock
//! (`bar/clock.rs`), the segments (`bar/mod.rs`), the OSD and its route
//! (`osd.rs`, `app.rs`), the panel's volume rail (`widgets/audio.rs`), the
//! pins (`jump/pin.rs`) and the peek (`bar/peek.rs`) — through
//! `store::observe` or per use, so a switch takes effect on release and the
//! file is only there for the next start.

use std::cell::Cell;
use std::rc::Rc;

use gtk4::prelude::*;

use super::form::{self, dropdown_row, section_box, switch_row};
use super::store::{self, Bar, Corner, PinSize, Pins};

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

const SEGMENTS: [Segment; 7] = [
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
        label: "Backup",
        hint: "One glyph for the nightly backup: quiet when both jobs are fresh, amber when one failed or is two nights old.",
        get: |b| b.backup,
        set: |b, v| b.backup = v,
    },
    Segment {
        label: "Light or dark",
        hint: "The mode's glyph beside the clock. A click moves Auto to Dark to Light.",
        get: |b| b.look_mode,
        set: |b, v| b.look_mode = v,
    },
    Segment {
        label: "Task board",
        hint: "The four-bay instrument in the right track, one bay per task 1–4.",
        get: |b| b.board,
        set: |b, v| b.board = v,
    },
];

/// The frame-rate dropdown's rows, in [`Pins::FRAME_RATES`] order.
const FRAME_RATE_LABELS: [&str; 3] = [
    "15 a second — lightest",
    "30 a second",
    "60 a second — smoothest",
];

/// The status line at the system default, naming what the default is.
fn describe(bar: &Bar, pins: &Pins) -> String {
    let hidden: Vec<&str> = SEGMENTS
        .iter()
        .filter(|s| !(s.get)(bar))
        .map(|s| s.label)
        .collect();
    format!(
        "System default: {} clock{}, volume and brightness {}, {} hidden; {} pins at {} a second, {}",
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
        pins.size
            .label()
            .split(' ')
            .next()
            .unwrap_or("")
            .to_lowercase(),
        pins.fps,
        pins.corner.label().to_lowercase(),
    )
}

struct State {
    clock_24h: gtk4::Switch,
    clock_date: gtk4::Switch,
    osd: gtk4::DropDown,
    segments: Vec<gtk4::Switch>,
    pin_size: gtk4::DropDown,
    pin_fps: gtk4::DropDown,
    pin_corner: gtk4::DropDown,
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

    fn edit_pins(&self, f: impl FnOnce(&mut Pins)) {
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
        self.clock_24h.set_active(bar.clock_24h);
        self.clock_date.set_active(bar.clock_date);
        let osd = OSD_PLACES
            .iter()
            .position(|(_, in_bar)| *in_bar == bar.osd_in_bar);
        self.osd.set_selected(osd.unwrap_or(0) as u32);
        for (segment, switch) in SEGMENTS.iter().zip(&self.segments) {
            switch.set_active((segment.get)(&bar));
        }
        let pins = settings.pins();
        let pos = |i: Option<usize>| i.unwrap_or(0) as u32;
        self.pin_size
            .set_selected(pos(PinSize::ALL.iter().position(|s| *s == pins.size)));
        self.pin_fps
            .set_selected(pos(Pins::FRAME_RATES.iter().position(|r| *r == pins.fps)));
        self.pin_corner
            .set_selected(pos(Corner::ALL.iter().position(|c| *c == pins.corner)));
        form::set_source(
            &self.status,
            settings.bar.is_some() || settings.pins.is_some(),
            &describe(&bar, &pins),
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
        let root = form::pane();

        let settings = store::current();
        let bar = settings.bar();

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

        let osd_group = section_box(
            "Volume & brightness",
            "Where a press of a volume or brightness key draws.",
        );
        let osd_labels: Vec<&str> = OSD_PLACES.iter().map(|(l, _)| *l).collect();
        let (row_osd, osd) = dropdown_row(
            "Shown as",
            "The centre card can be read through and works over fullscreen; the bar's decision slot costs a glance to the bottom edge and is skipped over a fullscreen view.",
            &osd_labels,
        );
        osd_group.append(&row_osd);

        let pins_group = section_box(
            "Pins & previews",
            "The floating pins, and the peek over a workspace button. A change redraws the pins on screen where they stand; an open peek keeps what it had until it closes.",
        );
        let size_labels: Vec<&str> = PinSize::ALL.iter().map(|s| s.label()).collect();
        let (row_size, pin_size) = dropdown_row(
            "Size",
            "The picture's box, 16:10; a workspace is fitted inside it at its own shape. The peek follows it; the rows in the pins popover do not.",
            &size_labels,
        );
        pins_group.append(&row_size);
        let (row_fps, pin_fps) = dropdown_row(
            "Frame rate",
            "Frames a second per window, for pins and the peek alike, so the two share one capture. Every frame is a full-size readback in the compositor: 60 costs twice what 30 does.",
            &FRAME_RATE_LABELS,
        );
        pins_group.append(&row_fps);
        let corner_labels: Vec<&str> = Corner::ALL.iter().map(|c| c.label()).collect();
        let (row_corner, pin_corner) = dropdown_row(
            "Corner",
            "Where the pins stand and stack from, and the side they slide in from. The bottom corners keep clear of the bar.",
            &corner_labels,
        );
        pins_group.append(&row_corner);

        let reset = form::action_button(
            "Reset to system",
            "Put the system's choices back and drop the bar and pins sections from the settings file.",
        );
        let (footer, status) = form::footer(&[&reset]);
        let copy = form::copy_nix_button(
            &status,
            "The bar and pins sections as theme/settings.nix holds them, for promoting a keeper into the Nix side by hand.",
            || {
                let s = store::current();
                Some(format!(
                    "{}{}",
                    s.section_as_nix("bar")?,
                    s.section_as_nix("pins")?
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
            pin_size: pin_size.clone(),
            pin_fps: pin_fps.clone(),
            pin_corner: pin_corner.clone(),
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
            pin_size.connect_selected_notify(move |d| {
                if let Some(s) = PinSize::ALL.get(d.selected() as usize).copied() {
                    state.edit_pins(|p| p.size = s);
                }
            });
        }
        {
            let state = state.clone();
            pin_fps.connect_selected_notify(move |d| {
                if let Some(r) = Pins::FRAME_RATES.get(d.selected() as usize).copied() {
                    state.edit_pins(|p| p.fps = r);
                }
            });
        }
        {
            let state = state.clone();
            pin_corner.connect_selected_notify(move |d| {
                if let Some(c) = Corner::ALL.get(d.selected() as usize).copied() {
                    state.edit_pins(|p| p.corner = c);
                }
            });
        }
        {
            let state = state.clone();
            reset.connect_clicked(move |_| {
                if state.updating.get() {
                    return;
                }
                store::reset::<Bar>();
                store::reset::<Pins>();
                state.sync();
            });
        }

        root.append(&clock);
        root.append(&segments_group);
        root.append(&osd_group);
        root.append(&pins_group);
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

// ── Search ──────────────────────────────────────────────────────────────

use super::search::{Entry, row};

/// This tab's rows as the launcher finds them (`search.rs`). A row added
/// to the tab gets a line here; the test there fails until it does.
#[rustfmt::skip]
pub(super) const SEARCH: &[Entry] = &[
    row("Clock", "24-hour clock", "14:05 rather than 2:05 PM", &["24h", "12h", "am pm", "time format", "military time", "clock format"]).keys(&["bar.clock_24h"]),
    row("Clock", "Show the date", "Weekday and date beside the time", &["date", "day", "weekday", "calendar"]).keys(&["bar.clock_date"]),
    row("Segments", "Media mark", "What is playing, on the bar", &["music", "now playing", "player", "mpris", "song"]).keys(&["bar.media"]),
    row("Segments", "Tray", "Status icons from applications", &["system tray", "systray", "status icons", "appindicator", "tray icons"]).keys(&["bar.tray"]),
    row("Segments", "Battery", "The battery segment of the bar", &["power", "charge", "percentage"]).keys(&["bar.battery"]),
    row("Segments", "Presence", "The presence sensor's mark", &["sensor", "proximity"]).keys(&["bar.presence"]),
    row("Segments", "Backup", "The nightly backup's glyph", &["restic", "borg", "backup status"]).keys(&["bar.backup"]),
    row("Segments", "Light or dark", "Switch the mode from the bar", &["dark mode", "light mode", "theme", "mode toggle", "night mode", "auto"]).keys(&["bar.look_mode"]),
    row("Segments", "Task board", "Tasks 1–4 on the bar", &["tasks", "todo", "board"]).keys(&["bar.board"]),
    row("Volume & brightness", "Shown as", "Where a volume or brightness key shows", &["osd", "on screen display", "popup", "volume popup", "overlay"]).keys(&["bar.osd_in_bar"]),
    row("Pins & previews", "Size", "How big pins and the peek are", &["pin size", "pinned workspace", "preview size", "peek", "thumbnail", "picture in picture", "pip"]).keys(&["pins.size"]),
    row("Pins & previews", "Frame rate", "How smooth pins and the peek are", &["fps", "frames per second", "pin fps", "refresh", "smooth", "live preview"]).keys(&["pins.fps"]),
    row("Pins & previews", "Corner", "Where pins stand on screen", &["pin position", "pin corner", "pinned workspace", "placement", "picture in picture", "pip"]).keys(&["pins.corner"]),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_frame_rate_rows_name_the_rates_they_pick() {
        for (label, rate) in FRAME_RATE_LABELS.iter().zip(Pins::FRAME_RATES) {
            assert!(label.starts_with(&format!("{rate} ")), "{label}");
        }
    }

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
        assert_eq!(
            moved,
            ["backup", "battery", "board", "look_mode", "media", "presence", "tray"]
        );
    }
}
