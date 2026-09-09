//! The Idle & Lock tab: the idle manager's timers, what locks and unlocks,
//! and what `sudo` and `pkexec` may ask for.
//!
//! The manager is another process (`swaypplet idle`, `idle/mod.rs`) with no
//! channel to this one, so an edit here is a file write and nothing else;
//! the manager watches the file's mtime and re-arms within about a second.
//! That is slow for a slider and fine for a dropdown, which is one reason
//! the timers are dropdowns. The other is that a duration is a choice
//! between a few sensible rungs, and a rail from 0 to an hour puts most of
//! its length on values nobody wants.
//!
//! The night window is the same three dropdowns a second time, over the same
//! ladders, plus the two clock times that bound it. It is one group rather
//! than a per-timer "and at night" column because the window is one decision:
//! either the evening has its own timers or it does not, and the switch says
//! which. The rows below the switch go insensitive when it is off, so the
//! values stay visible and stay saved.

use std::cell::Cell;
use std::rc::Rc;

use gtk4::prelude::*;

use super::store::{self, Elevate, Idle};
use super::ui::{self, Durations, dropdown_row, kind_row, scale_row, section_box, switch_row};

/// The rungs each dropdown offers. The saved value is added if it is not
/// one of them (`ui::Durations`).
const DIM_LADDER: &[u32] = &[0, 30, 60, 120, 180, 240, 300, 600, 900];
const LOCK_LADDER: &[u32] = &[0, 60, 120, 300, 600, 900, 1800, 3600];
const BLANK_LADDER: &[u32] = &[0, 60, 120, 300, 600, 900, 1800, 3600];
const SUSPEND_LADDER: &[u32] = &[0, 600, 900, 1200, 1800, 2700, 3600, 5400];

/// One timer's dropdown: which field it edits and the rungs it offers.
struct Timer {
    label: &'static str,
    hint: &'static str,
    ladder: &'static [u32],
    get: fn(&Idle) -> u32,
    set: fn(&mut Idle, u32),
}

const TIMERS: [Timer; 4] = [
    Timer {
        label: "Dim after",
        hint: "Fade the backlight after this much idle time. Any input restores it.",
        ladder: DIM_LADDER,
        get: |i| i.dim_after_s,
        set: |i, v| i.dim_after_s = v,
    },
    Timer {
        label: "Lock after",
        hint: "Lock the session after this much idle time. Suppressed while the presence sensor sees you; No Lock stops it outright.",
        ladder: LOCK_LADDER,
        get: |i| i.lock_after_s,
        set: |i, v| i.lock_after_s = v,
    },
    Timer {
        label: "Screen off after",
        hint: "Counted while locked. Locking leaves the screen lit; the outputs go off only after this much idle time on the lock screen.",
        ladder: BLANK_LADDER,
        get: |i| i.blank_after_s,
        set: |i, v| i.blank_after_s = v,
    },
    Timer {
        label: "Suspend after",
        hint: "On battery only. Never on AC, never with No Sleep armed, never from a session that is not on the seat.",
        ladder: SUSPEND_LADDER,
        get: |i| i.suspend_after_s,
        set: |i, v| i.suspend_after_s = v,
    },
];

/// The three tiers the night window replaces, over the same ladders as
/// their daytime counterparts so both halves of a decision read alike.
/// Suspend is absent on purpose: it is battery-only, and a shorter night
/// suspend would stop an overnight job (`Idle::resolve`).
const NIGHT_TIMERS: [Timer; 3] = [
    Timer {
        label: "Dim after",
        hint: "The dim tier inside the window.",
        ladder: DIM_LADDER,
        get: |i| i.night_dim_after_s,
        set: |i, v| i.night_dim_after_s = v,
    },
    Timer {
        label: "Lock after",
        hint: "The lock tier inside the window. Crossing into the window when you have already been idle longer than this locks at once.",
        ladder: LOCK_LADDER,
        get: |i| i.night_lock_after_s,
        set: |i, v| i.night_lock_after_s = v,
    },
    Timer {
        label: "Screen off after",
        hint: "The screen-off tier inside the window, counted while locked. A countdown already running is shortened to this, never extended.",
        ladder: BLANK_LADDER,
        get: |i| i.night_blank_after_s,
        set: |i, v| i.night_blank_after_s = v,
    },
];

/// The minute rungs a clock dropdown offers, five apart, plus whatever the
/// file says. Same reasoning as `ui::Durations`: a minute typed by hand
/// (`"night_from_m": 37`) shows itself rather than being snapped away.
fn minute_rungs(current: u8) -> Vec<u8> {
    let mut rungs: Vec<u8> = (0..60).step_by(5).map(|m| m as u8).collect();
    if !rungs.contains(&current) {
        rungs.push(current);
        rungs.sort_unstable();
    }
    rungs
}

/// A label and one clock time: an hour dropdown, a colon, a minute
/// dropdown. Returns the rungs the minute dropdown was built from, which
/// `sync` needs to turn a saved minute back into an index.
fn time_row(
    label: &str,
    hint: &str,
    minute: u8,
) -> (gtk4::Box, gtk4::DropDown, gtk4::DropDown, Vec<u8>) {
    let hours: Vec<String> = (0..24).map(|h| format!("{h:02}")).collect();
    let hours: Vec<&str> = hours.iter().map(String::as_str).collect();
    let hour = gtk4::DropDown::from_strings(&hours);
    hour.add_css_class("settings-dropdown");

    let rungs = minute_rungs(minute);
    let labels: Vec<String> = rungs.iter().map(|m| format!("{m:02}")).collect();
    let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
    let minutes = gtk4::DropDown::from_strings(&labels);
    minutes.add_css_class("settings-dropdown");

    let clock = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(6)
        .build();
    clock.append(&hour);
    clock.append(&gtk4::Label::new(Some(":")));
    clock.append(&minutes);

    let row = kind_row(label, &clock);
    row.set_tooltip_text(Some(hint));
    (row, hour, minutes, rungs)
}

/// The status line at the system default, so it says what the default is
/// rather than that there is one.
fn describe(idle: &Idle) -> String {
    let mut text = format!(
        "System default: dim {} at {}%, lock {}, screen off {} into the lock, suspend {} on battery",
        ui::duration_label(idle.dim_after_s).to_lowercase(),
        idle.dim_level,
        ui::duration_label(idle.lock_after_s).to_lowercase(),
        ui::duration_label(idle.blank_after_s).to_lowercase(),
        ui::duration_label(idle.suspend_after_s).to_lowercase(),
    );
    if idle.night {
        text.push_str(&format!(
            "; {:02}:{:02}–{:02}:{:02} dim {}, lock {}, screen off {}",
            idle.night_from_h,
            idle.night_from_m,
            idle.night_to_h,
            idle.night_to_m,
            ui::duration_label(idle.night_dim_after_s).to_lowercase(),
            ui::duration_label(idle.night_lock_after_s).to_lowercase(),
            ui::duration_label(idle.night_blank_after_s).to_lowercase(),
        ));
    }
    text
}

/// One clock bound of the night window: its two dropdowns and the minute
/// rungs the second was built from.
struct Clock {
    hour: gtk4::DropDown,
    minute: gtk4::DropDown,
    rungs: Vec<u8>,
}

struct State {
    /// One per [`TIMERS`] entry, in order, with the rungs its dropdown was
    /// built from.
    dropdowns: Vec<(gtk4::DropDown, Durations)>,
    /// The same, one per [`NIGHT_TIMERS`] entry.
    night_dropdowns: Vec<(gtk4::DropDown, Durations)>,
    night: gtk4::Switch,
    /// Everything the switch governs, so one `set_sensitive` covers it.
    night_body: gtk4::Box,
    opens: Clock,
    closes: Clock,
    dim_level: gtk4::Scale,
    walk_away: gtk4::Switch,
    face: gtk4::Switch,
    /// One per [`ELEVATE`] entry, in order.
    elevate: Vec<gtk4::Switch>,
    status: gtk4::Label,
    updating: Cell<bool>,
}

/// One switch on the Administrator access group: which field it edits.
struct Toggle {
    label: &'static str,
    hint: &'static str,
    get: fn(&Elevate) -> bool,
    set: fn(&mut Elevate, bool),
}

const ELEVATE: [Toggle; 4] = [
    Toggle {
        label: "Face for administrator access",
        hint: "Ask the camera when sudo or pkexec asks for you. Off, the reader and the password remain; nothing here can make it easier than the password.",
        get: |e| e.face,
        set: |e, v| e.face = v,
    },
    Toggle {
        label: "Card for terminal sudo",
        hint: "Draw the same card pkexec gets when sudo asks in a terminal: type the password there or in the terminal, whichever is nearer. Off, the terminal keeps the prompt to itself and the camera is not asked for it.",
        get: |e| e.terminal_card,
        set: |e, v| e.terminal_card = v,
    },
    Toggle {
        label: "Pill under the lens",
        hint: "While the camera runs, a pill by the lens says what it sees. Off, the card's caption reports instead.",
        get: |e| e.cue,
        set: |e, v| e.cue = v,
    },
    Toggle {
        label: "Typing ends the face check",
        hint: "The first keystroke in the password field stops the camera. Off, it runs out its window alongside the typing.",
        get: |e| e.typing_abandons_face,
        set: |e, v| e.typing_abandons_face = v,
    },
];

impl State {
    fn edit(&self, f: impl FnOnce(&mut Idle)) {
        if self.updating.get() {
            return;
        }
        store::edit(f);
        self.sync();
    }

    fn edit_elevate(&self, f: impl FnOnce(&mut Elevate)) {
        if self.updating.get() {
            return;
        }
        store::edit(f);
        self.sync();
    }

    fn sync(&self) {
        self.updating.set(true);
        let settings = store::current();
        let idle = settings.idle();
        for (timer, (dropdown, durations)) in TIMERS.iter().zip(&self.dropdowns) {
            let value = (timer.get)(&idle);
            // A value outside the rungs this dropdown was built with (the
            // file changed under us) has nowhere to go; the nearest rung is
            // wrong, so leave the selection where it was and log.
            match durations.index_of(value) {
                Some(i) => dropdown.set_selected(i as u32),
                None => log::warn!("idle settings: {} = {value}s is not a rung", timer.label),
            }
        }
        for (timer, (dropdown, durations)) in NIGHT_TIMERS.iter().zip(&self.night_dropdowns) {
            let value = (timer.get)(&idle);
            match durations.index_of(value) {
                Some(i) => dropdown.set_selected(i as u32),
                None => log::warn!(
                    "idle settings: night {} = {value}s is not a rung",
                    timer.label
                ),
            }
        }
        self.night.set_active(idle.night);
        self.night_body.set_sensitive(idle.night);
        for (clock, (hour, minute)) in [&self.opens, &self.closes].into_iter().zip([
            (idle.night_from_h, idle.night_from_m),
            (idle.night_to_h, idle.night_to_m),
        ]) {
            clock.hour.set_selected(u32::from(hour.min(23)));
            match clock.rungs.iter().position(|m| *m == minute) {
                Some(i) => clock.minute.set_selected(i as u32),
                None => log::warn!("idle settings: night window minute {minute} is not a rung"),
            }
        }
        self.dim_level.set_value(f64::from(idle.dim_level));
        self.walk_away.set_active(idle.walk_away_lock);
        self.face.set_active(idle.face_unlock);
        let elevate = settings.elevate();
        for (toggle, switch) in ELEVATE.iter().zip(&self.elevate) {
            switch.set_active((toggle.get)(&elevate));
        }
        ui::set_source(
            &self.status,
            settings.idle.is_some() || settings.elevate.is_some(),
            &describe(&idle),
        );
        self.updating.set(false);
    }
}

pub struct IdlePane {
    root: gtk4::Box,
    state: Rc<State>,
}

impl IdlePane {
    pub fn new() -> Self {
        let root = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(14)
            .build();

        let group = section_box(
            "Idle timers",
            "Counted from the last input. A video player's idle inhibitor and No Lock hold these off. The lid, the Lock key and suspend are not timers and are not here.",
        );

        let idle = store::current().idle();
        let mut dropdowns = Vec::new();
        for timer in &TIMERS {
            let durations = Durations::new(timer.ladder, (timer.get)(&idle));
            let labels = durations.labels();
            let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
            let (row, dropdown) = dropdown_row(timer.label, timer.hint, &labels);
            group.append(&row);
            dropdowns.push((dropdown, durations));
        }

        let (dim_row, dim_level) = scale_row(
            "Dim to",
            "The backlight level the dim tier fades to.",
            (5.0, 80.0, 5.0),
            |v| format!("{v:.0}%"),
        );
        group.append(&dim_row);

        // The night window. The switch is the first row and the rest hangs
        // off `night_body`, so turning it off greys the values instead of
        // hiding them: what the window would do stays readable, and stays in
        // the file, while it is not doing it.
        let night_group = section_box(
            "Night window",
            "A second set of timers for a time range, for the hours when a lit, unlocked screen should not sit there. Outside the range the timers above hold. Suspend is not in the window.",
        );
        let (night_row, night) = switch_row(
            "Use a night window",
            "Off, the timers above hold all day and the rows below are inert.",
            idle.night,
        );
        night_group.append(&night_row);

        let night_body = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(4)
            .build();
        let (opens_row, opens_h, opens_m, opens_rungs) = time_row(
            "Window opens",
            "Local time. A window whose end is at or before its start crosses midnight, so 21:00 to 07:00 is the evening and the night.",
            idle.night_from_m,
        );
        night_body.append(&opens_row);
        let (closes_row, closes_h, closes_m, closes_rungs) = time_row(
            "Window closes",
            "Local time, exclusive: at this minute the daytime timers are back.",
            idle.night_to_m,
        );
        night_body.append(&closes_row);

        let mut night_dropdowns = Vec::new();
        for timer in &NIGHT_TIMERS {
            let durations = Durations::new(timer.ladder, (timer.get)(&idle));
            let labels = durations.labels();
            let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
            let (row, dropdown) = dropdown_row(timer.label, timer.hint, &labels);
            night_body.append(&row);
            night_dropdowns.push((dropdown, durations));
        }
        night_group.append(&night_body);

        let lock = section_box(
            "Lock",
            "What locks besides the timer, and what unlocks. Neither switch touches the password or the fingerprint.",
        );
        let (walk_row, walk_away) = switch_row(
            "Lock when I walk away",
            "The presence sensor's absence edge locks the session. Off, walking away is no different from sitting still.",
            idle.walk_away_lock,
        );
        lock.append(&walk_row);
        let (face_row, face) = switch_row(
            "Face unlock",
            "Try the camera while the lock screen is up. Read when the lock starts.",
            idle.face_unlock,
        );
        lock.append(&face_row);

        // Administrator access. Read by the polkit agent, which draws the
        // card for both `sudo` and `pkexec` and tells pam_race whether the
        // camera may be asked; every switch removes a way in or a surface.
        let admin = section_box(
            "Administrator access",
            "What sudo and pkexec may ask for besides the password, and where. The password itself is always there.",
        );
        let elevate_now = store::current().elevate();
        let mut elevate = Vec::new();
        for toggle in &ELEVATE {
            let (row, switch) = switch_row(toggle.label, toggle.hint, (toggle.get)(&elevate_now));
            admin.append(&row);
            elevate.push(switch);
        }

        let reset = ui::action_button(
            "Reset to system",
            "Put the system's timers and access switches back and drop both sections from the settings file.",
        );
        let (footer, status) = ui::footer(&[&reset]);
        let copy = ui::copy_nix_button(
            &status,
            "The timers as the `idle` attrset of theme/settings.nix, for promoting a keeper into the Nix side by hand.",
            || store::current().section_as_nix("idle"),
        );
        // Beside Reset, in the row `footer` built.
        if let Some(row) = reset.parent().and_downcast::<gtk4::Box>() {
            row.append(&copy);
        }

        let state = Rc::new(State {
            dropdowns,
            night_dropdowns,
            night: night.clone(),
            night_body,
            opens: Clock {
                hour: opens_h.clone(),
                minute: opens_m.clone(),
                rungs: opens_rungs,
            },
            closes: Clock {
                hour: closes_h.clone(),
                minute: closes_m.clone(),
                rungs: closes_rungs,
            },
            dim_level: dim_level.clone(),
            walk_away: walk_away.clone(),
            face: face.clone(),
            elevate,
            status,
            updating: Cell::new(false),
        });

        for (index, switch) in state.elevate.iter().enumerate() {
            let state = state.clone();
            switch.connect_active_notify(move |s| {
                let on = s.is_active();
                state.edit_elevate(|e| (ELEVATE[index].set)(e, on));
            });
        }

        for (index, (dropdown, _)) in state.dropdowns.iter().enumerate() {
            let state = state.clone();
            dropdown.connect_selected_notify(move |d| {
                let (_, durations) = &state.dropdowns[index];
                let Some(secs) = durations.at(d.selected() as usize) else {
                    return;
                };
                state.edit(|idle| (TIMERS[index].set)(idle, secs));
            });
        }
        for (index, (dropdown, _)) in state.night_dropdowns.iter().enumerate() {
            let state = state.clone();
            dropdown.connect_selected_notify(move |d| {
                let (_, durations) = &state.night_dropdowns[index];
                let Some(secs) = durations.at(d.selected() as usize) else {
                    return;
                };
                state.edit(|idle| (NIGHT_TIMERS[index].set)(idle, secs));
            });
        }
        {
            let state = state.clone();
            night.connect_active_notify(move |s| {
                let on = s.is_active();
                state.edit(|idle| idle.night = on);
            });
        }
        // The four clock dropdowns, each writing one field. The hour is its
        // own index; the minute goes through the rungs its dropdown was built
        // from, which may carry a hand-typed value.
        for (hour, set) in [
            (
                &opens_h,
                (|i: &mut Idle, v: u8| i.night_from_h = v) as fn(&mut Idle, u8),
            ),
            (&closes_h, |i: &mut Idle, v: u8| i.night_to_h = v),
        ] {
            let state = state.clone();
            hour.connect_selected_notify(move |d| {
                let h = d.selected().min(23) as u8;
                state.edit(|idle| set(idle, h));
            });
        }
        for (which, set) in [
            (
                true,
                (|i: &mut Idle, v: u8| i.night_from_m = v) as fn(&mut Idle, u8),
            ),
            (false, |i: &mut Idle, v: u8| i.night_to_m = v),
        ] {
            let state = state.clone();
            let minute = if which { &opens_m } else { &closes_m };
            minute.connect_selected_notify(move |d| {
                let clock = if which { &state.opens } else { &state.closes };
                let Some(m) = clock.rungs.get(d.selected() as usize).copied() else {
                    return;
                };
                state.edit(|idle| set(idle, m));
            });
        }
        {
            let state = state.clone();
            walk_away.connect_active_notify(move |s| {
                let on = s.is_active();
                state.edit(|idle| idle.walk_away_lock = on);
            });
        }
        {
            let state = state.clone();
            face.connect_active_notify(move |s| {
                let on = s.is_active();
                state.edit(|idle| idle.face_unlock = on);
            });
        }
        {
            let state = state.clone();
            dim_level.connect_value_changed(move |s| {
                let level = (s.value() / 5.0).round() as u8 * 5;
                state.edit(|idle| idle.dim_level = level);
            });
        }
        {
            let state = state.clone();
            reset.connect_clicked(move |_| {
                if state.updating.get() {
                    return;
                }
                store::reset::<Idle>();
                store::reset::<Elevate>();
                state.sync();
            });
        }

        root.append(&group);
        root.append(&night_group);
        root.append(&lock);
        root.append(&admin);
        root.append(&footer);
        state.sync();

        IdlePane { root, state }
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
    fn every_ladder_starts_at_never_and_climbs() {
        for timer in TIMERS.iter().chain(&NIGHT_TIMERS) {
            assert_eq!(timer.ladder[0], 0, "{}", timer.label);
            assert!(
                timer.ladder.windows(2).all(|w| w[0] < w[1]),
                "{}",
                timer.label
            );
        }
    }

    #[test]
    fn the_defaults_are_rungs_so_a_fresh_pane_has_a_selection() {
        let idle = Idle::default();
        for timer in TIMERS.iter().chain(&NIGHT_TIMERS) {
            assert!(
                timer.ladder.contains(&(timer.get)(&idle)),
                "{} default {} is not on its ladder",
                timer.label,
                (timer.get)(&idle)
            );
        }
        assert_eq!(idle.dim_level % 5, 0);
    }

    #[test]
    fn every_access_switch_moves_exactly_one_field() {
        let base = Elevate::default();
        let mut seen = Vec::new();
        for toggle in &ELEVATE {
            let mut e = base;
            (toggle.set)(&mut e, false);
            assert!(!(toggle.get)(&e), "{}", toggle.label);
            let fields = [
                ("face", e.face != base.face),
                ("terminal_card", e.terminal_card != base.terminal_card),
                ("cue", e.cue != base.cue),
                (
                    "typing_abandons_face",
                    e.typing_abandons_face != base.typing_abandons_face,
                ),
            ];
            let changed: Vec<&str> = fields.iter().filter(|(_, c)| *c).map(|(n, _)| *n).collect();
            assert_eq!(changed.len(), 1, "{} moved {changed:?}", toggle.label);
            seen.push(changed[0]);
        }
        seen.sort_unstable();
        assert_eq!(
            seen,
            ["cue", "face", "terminal_card", "typing_abandons_face"]
        );
    }

    /// Which field a `Timer` moved, or every field it moved if it moved
    /// more than one. The night tiers double the number of look-alike
    /// fields, which is exactly when a copied `set` closure goes unnoticed.
    fn moved(base: &Idle, t: &Idle) -> Vec<&'static str> {
        [
            ("dim_after_s", t.dim_after_s != base.dim_after_s),
            ("lock_after_s", t.lock_after_s != base.lock_after_s),
            ("blank_after_s", t.blank_after_s != base.blank_after_s),
            ("suspend_after_s", t.suspend_after_s != base.suspend_after_s),
            ("dim_level", t.dim_level != base.dim_level),
            ("walk_away_lock", t.walk_away_lock != base.walk_away_lock),
            ("face_unlock", t.face_unlock != base.face_unlock),
            ("night", t.night != base.night),
            ("night_from_h", t.night_from_h != base.night_from_h),
            ("night_from_m", t.night_from_m != base.night_from_m),
            ("night_to_h", t.night_to_h != base.night_to_h),
            ("night_to_m", t.night_to_m != base.night_to_m),
            (
                "night_dim_after_s",
                t.night_dim_after_s != base.night_dim_after_s,
            ),
            (
                "night_lock_after_s",
                t.night_lock_after_s != base.night_lock_after_s,
            ),
            (
                "night_blank_after_s",
                t.night_blank_after_s != base.night_blank_after_s,
            ),
        ]
        .iter()
        .filter(|(_, c)| *c)
        .map(|(n, _)| *n)
        .collect()
    }

    #[test]
    fn every_timer_moves_exactly_one_field() {
        let base = Idle::default();
        let mut seen = Vec::new();
        for timer in &TIMERS {
            let mut t = base;
            (timer.set)(&mut t, 7);
            let changed = moved(&base, &t);
            assert_eq!(changed.len(), 1, "{} moved {changed:?}", timer.label);
            assert_eq!((timer.get)(&t), 7);
            seen.push(changed[0]);
        }
        seen.sort_unstable();
        assert_eq!(
            seen,
            [
                "blank_after_s",
                "dim_after_s",
                "lock_after_s",
                "suspend_after_s"
            ]
        );
    }

    #[test]
    fn every_night_timer_moves_exactly_one_night_field() {
        let base = Idle::default();
        let mut seen = Vec::new();
        for timer in &NIGHT_TIMERS {
            let mut t = base;
            (timer.set)(&mut t, 7);
            let changed = moved(&base, &t);
            assert_eq!(changed.len(), 1, "night {} moved {changed:?}", timer.label);
            assert_eq!((timer.get)(&t), 7);
            seen.push(changed[0]);
        }
        seen.sort_unstable();
        assert_eq!(
            seen,
            [
                "night_blank_after_s",
                "night_dim_after_s",
                "night_lock_after_s"
            ]
        );
    }

    #[test]
    fn minute_rungs_are_five_apart_and_admit_a_typed_value() {
        let five = minute_rungs(0);
        assert_eq!(five.len(), 12);
        assert_eq!(five[0], 0);
        assert_eq!(*five.last().unwrap(), 55);
        assert!(five.windows(2).all(|w| w[1] - w[0] == 5));

        let typed = minute_rungs(37);
        assert_eq!(typed.len(), 13);
        assert_eq!(typed.iter().position(|m| *m == 37), Some(8));
        assert!(typed.windows(2).all(|w| w[0] < w[1]));
        // A value already on the ladder is not doubled.
        assert_eq!(minute_rungs(30).len(), 12);
    }
}
