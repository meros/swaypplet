//! The Idle & Lock tab: the idle manager's timers.
//!
//! The manager is another process (`swaypplet idle`, `idle/mod.rs`) with no
//! channel to this one, so an edit here is a file write and nothing else;
//! the manager watches the file's mtime and re-arms within about a second.
//! That is slow for a slider and fine for a dropdown, which is one reason
//! the timers are dropdowns. The other is that a duration is a choice
//! between a few sensible rungs, and a rail from 0 to an hour puts most of
//! its length on values nobody wants.

use std::cell::Cell;
use std::rc::Rc;

use gtk4::prelude::*;

use super::store::{self, Idle};
use super::ui::{self, Durations, dropdown_row, scale_row, section_box, switch_row};

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

/// The status line at the system default, so it says what the default is
/// rather than that there is one.
fn describe(idle: &Idle) -> String {
    format!(
        "System default: dim {} at {}%, lock {}, screen off {} into the lock, suspend {} on battery",
        ui::duration_label(idle.dim_after_s).to_lowercase(),
        idle.dim_level,
        ui::duration_label(idle.lock_after_s).to_lowercase(),
        ui::duration_label(idle.blank_after_s).to_lowercase(),
        ui::duration_label(idle.suspend_after_s).to_lowercase(),
    )
}

struct State {
    /// One per [`TIMERS`] entry, in order, with the rungs its dropdown was
    /// built from.
    dropdowns: Vec<(gtk4::DropDown, Durations)>,
    dim_level: gtk4::Scale,
    walk_away: gtk4::Switch,
    face: gtk4::Switch,
    status: gtk4::Label,
    updating: Cell<bool>,
}

impl State {
    fn edit(&self, f: impl FnOnce(&mut Idle)) {
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
        self.dim_level.set_value(f64::from(idle.dim_level));
        self.walk_away.set_active(idle.walk_away_lock);
        self.face.set_active(idle.face_unlock);
        ui::set_source(&self.status, settings.idle.is_some(), &describe(&idle));
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

        let reset = ui::action_button(
            "Reset to system",
            "Put the system's timers back and drop the section from the settings file.",
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
            dim_level: dim_level.clone(),
            walk_away: walk_away.clone(),
            face: face.clone(),
            status,
            updating: Cell::new(false),
        });

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
                state.sync();
            });
        }

        root.append(&group);
        root.append(&lock);
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
        for timer in &TIMERS {
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
        for timer in &TIMERS {
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
    fn every_timer_moves_exactly_one_field() {
        let base = Idle::default();
        let mut seen = Vec::new();
        for timer in &TIMERS {
            let mut t = base;
            (timer.set)(&mut t, 7);
            let fields = [
                ("dim_after_s", t.dim_after_s != base.dim_after_s),
                ("lock_after_s", t.lock_after_s != base.lock_after_s),
                ("blank_after_s", t.blank_after_s != base.blank_after_s),
                ("suspend_after_s", t.suspend_after_s != base.suspend_after_s),
                ("dim_level", t.dim_level != base.dim_level),
                ("walk_away_lock", t.walk_away_lock != base.walk_away_lock),
                ("face_unlock", t.face_unlock != base.face_unlock),
            ];
            let changed: Vec<&str> = fields.iter().filter(|(_, c)| *c).map(|(n, _)| *n).collect();
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
}
