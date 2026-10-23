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
//! The night timers are the same three dropdowns a second time, over the
//! same ladders. Night is Appearance's Day and night (`theme::sun`), when
//! the automatic mode is dark, so the machine has one night; this group has
//! no clock times of its own. It is one group rather than a per-timer "and
//! at night" column because it is one decision: either the night has its own
//! timers or it does not, and the switch says which. The rows below the
//! switch go insensitive when it is off, so the values stay visible and stay
//! saved.

use std::cell::Cell;
use std::rc::Rc;

use gtk4::prelude::*;

use super::form::{self, Durations, dropdown_row, scale_row, section_box, switch_row};
use super::store::{self, Elevate, Idle};

/// The rungs each dropdown offers. The saved value is added if it is not
/// one of them (`form::Durations`).
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

const TIMERS: [Timer; 5] = [
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
        label: "Screen off on battery",
        hint: "Counted while locked, on battery only. It can only shorten the screen-off tier, the night one included, never lengthen it; Never leaves that tier in charge.",
        ladder: BLANK_LADDER,
        get: |i| i.battery_blank_after_s,
        set: |i, v| i.battery_blank_after_s = v,
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
        hint: "How long idle before the screen dims, at night.",
        ladder: DIM_LADDER,
        get: |i| i.night_dim_after_s,
        set: |i, v| i.night_dim_after_s = v,
    },
    Timer {
        label: "Lock after",
        hint: "How long idle before the session locks, at night. If you have already been idle longer than this when night begins, it locks at once.",
        ladder: LOCK_LADDER,
        get: |i| i.night_lock_after_s,
        set: |i, v| i.night_lock_after_s = v,
    },
    Timer {
        label: "Screen off after",
        hint: "How long the lock screen stays lit at night before the screens go off. A countdown already running is shortened to this, never extended.",
        ladder: BLANK_LADDER,
        get: |i| i.night_blank_after_s,
        set: |i, v| i.night_blank_after_s = v,
    },
];

/// The status line at the system default, so it says what the default is
/// rather than that there is one.
fn describe(idle: &Idle) -> String {
    let mut text = format!(
        "System default: dim {} at {}%, lock {}, screen off {} into the lock ({} on battery), suspend {} on battery",
        form::duration_label(idle.dim_after_s).to_lowercase(),
        idle.dim_level,
        form::duration_label(idle.lock_after_s).to_lowercase(),
        form::duration_label(idle.blank_after_s).to_lowercase(),
        form::duration_label(idle.blank_after_on(true)).to_lowercase(),
        form::duration_label(idle.suspend_after_s).to_lowercase(),
    );
    if idle.night {
        text.push_str(&format!(
            "; at night dim {}, lock {}, screen off {}",
            form::duration_label(idle.night_dim_after_s).to_lowercase(),
            form::duration_label(idle.night_lock_after_s).to_lowercase(),
            form::duration_label(idle.night_blank_after_s).to_lowercase(),
        ));
    }
    text
}

/// When night is, as the night timers will see it: tonight's switch times
/// by Day and night, or why there is none. True when it is a warning.
fn night_when(d: &super::store::Daylight) -> (String, bool) {
    if !d.fixed_times && d.place().is_none() {
        return (
            "No place set in Appearance › Day and night, so night never starts and these timers are not used".into(),
            true,
        );
    }
    let switches = crate::theme::sun::switches_today(d);
    let dark = switches
        .iter()
        .find(|(_, m)| *m == crate::tokens::Mode::Dark)
        .map(|(m, _)| *m);
    let light = switches
        .iter()
        .find(|(_, m)| *m == crate::tokens::Mode::Light)
        .map(|(m, _)| *m);
    let clock = |m: u16| format!("{:02}:{:02}", m / 60, m % 60);
    match (dark, light) {
        (Some(dark), Some(light)) => (
            format!(
                "Night is from {} to {} today, as Appearance › Day and night sets it",
                clock(dark),
                clock(light)
            ),
            false,
        ),
        _ => (
            "No sunset or sunrise here today: night follows Appearance › Day and night".into(),
            false,
        ),
    }
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
    /// When night is tonight, by Day and night, or why it never comes.
    night_when: gtk4::Label,
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
        let (when, warn) = night_when(&settings.daylight());
        self.night_when.set_text(&when);
        crate::ui::set_tone(
            &self.night_when,
            if warn && idle.night {
                crate::ui::Tone::Warning
            } else {
                crate::ui::Tone::Muted
            },
        );
        self.dim_level.set_value(f64::from(idle.dim_level));
        self.walk_away.set_active(idle.walk_away_lock);
        self.face.set_active(idle.face_unlock);
        let elevate = settings.elevate();
        for (toggle, switch) in ELEVATE.iter().zip(&self.elevate) {
            switch.set_active((toggle.get)(&elevate));
        }
        form::set_source(
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
        let root = form::pane();

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
            "At night",
            "Shorter timers for the night. Night is when Appearance › Day and night turns the mode dark. Suspend has no night timer.",
        );
        let (night_row, night) = switch_row(
            "Night timers",
            "Off, the timers above hold day and night and the rows below are not used.",
            idle.night,
        );
        night_group.append(&night_row);
        let night_when = form::status_label();
        night_when.set_xalign(0.0);
        night_group.append(&night_when);

        let night_body = crate::ui::vbox(1);
        let mut night_dropdowns = Vec::new();
        for timer in &NIGHT_TIMERS {
            let durations = Durations::new(timer.ladder, (timer.get)(&idle));
            let labels = durations.labels();
            let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
            let (row, dropdown) = dropdown_row(timer.label, timer.hint, &labels);
            form::dependent(&[&row]);
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

        let reset = form::ResetButtons::new(
            "Put the system's timers and access switches back and drop both sections from the settings file.",
        );
        let (footer, status) = form::footer(&[&reset.button, &reset.undo]);
        let copy = form::copy_nix_button(
            &status,
            "The timers as the `idle` attrset of theme/settings.nix, for promoting a keeper into the Nix side by hand.",
            || store::current().section_as_nix("idle"),
        );
        // Beside Reset, in the row `footer` built.
        if let Some(row) = reset.button.parent().and_downcast::<gtk4::Box>() {
            row.append(&copy);
        }

        let state = Rc::new(State {
            dropdowns,
            night_dropdowns,
            night: night.clone(),
            night_body,
            night_when,
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
            let undone = state.clone();
            reset.connect(
                move || {
                    store::reset::<Idle>();
                    store::reset::<Elevate>();
                    state.sync();
                },
                move || undone.sync(),
            );
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

// ── Search ──────────────────────────────────────────────────────────────

use super::search::{Entry, row};

/// This tab's rows as the launcher finds them (`search.rs`). A row added
/// to the tab gets a line here; the test there fails until it does.
#[rustfmt::skip]
pub(super) const SEARCH: &[Entry] = &[
    row("Idle timers", "", "Timers from the last input; closing the lid is not one of them", &["lid", "laptop lid", "close lid"]),
    row("Idle timers", "Dim after", "Dim the backlight when idle", &["dim", "screen dim", "idle", "backlight", "inactivity"]).keys(&["idle.dim_after_s"]),
    row("Idle timers", "Lock after", "Lock the session when idle", &["auto lock", "autolock", "screen lock", "lock screen", "timeout", "idle", "screensaver", "screen saver"]).keys(&["idle.lock_after_s"]),
    row("Idle timers", "Screen off after", "Turn the screens off while locked", &["screen timeout", "display off", "monitor off", "screen off", "blank", "dpms", "timeout", "screensaver"]).keys(&["idle.blank_after_s"]),
    row("Idle timers", "Screen off on battery", "A shorter screen-off while locked on battery", &["battery", "screen timeout", "display off", "screen off", "blank", "dpms", "power saving", "unplugged"]).keys(&["idle.battery_blank_after_s"]),
    row("Idle timers", "Suspend after", "Sleep on battery when idle", &["sleep", "suspend", "hibernate", "battery", "standby"]).keys(&["idle.suspend_after_s"]),
    row("Idle timers", "Dim to", "How far the dim tier turns the backlight down", &["dim level", "brightness", "backlight"]).keys(&["idle.dim_level"]),
    row("At night", "Night timers", "Shorter timers for the night", &["night window", "schedule", "bedtime"]).keys(&["idle.night"]),
    row("At night", "Dim after", "The dim timer at night", &["night dim", "dim", "backlight"]).keys(&["idle.night_dim_after_s"]),
    row("At night", "Lock after", "The lock timer at night", &["night lock", "auto lock", "timeout"]).keys(&["idle.night_lock_after_s"]),
    row("At night", "Screen off after", "The screen-off timer at night", &["screen timeout", "display off", "timeout"]).keys(&["idle.night_blank_after_s"]),
    row("Lock", "Lock when I walk away", "The presence sensor locks when you leave", &["walk away", "presence", "proximity", "away", "leave", "sensor"]).keys(&["idle.walk_away_lock"]),
    row("Lock", "Face unlock", "The camera unlocks the lock screen", &["face", "camera", "howdy", "face id", "biometric", "ir camera", "unlock", "fingerprint"]).keys(&["idle.face_unlock"]),
    row("Administrator access", "Face for administrator access", "The camera for sudo and pkexec", &["sudo", "pkexec", "polkit", "admin", "root", "elevate", "face", "camera"]).keys(&["elevate.face"]),
    row("Administrator access", "Card for terminal sudo", "A password card for sudo in a terminal", &["sudo", "terminal", "polkit", "password", "prompt"]).keys(&["elevate.terminal_card"]),
    row("Administrator access", "Pill under the lens", "Says what the camera sees", &["camera", "face", "caption", "status"]).keys(&["elevate.cue"]),
    row("Administrator access", "Typing ends the face check", "A keystroke stops the camera", &["typing", "keyboard", "camera", "password"]).keys(&["elevate.typing_abandons_face"]),
];

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
            (
                "battery_blank_after_s",
                t.battery_blank_after_s != base.battery_blank_after_s,
            ),
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
                "battery_blank_after_s",
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

}
