//! The Look tab's "Day and night" group: when the automatic mode turns light
//! and dark and when the night light warms (`settings::schema::Daylight`,
//! read by `theme::sun`).
//!
//! By the sun at the place picked on the map, sunrise and sunset moved
//! earlier or later if you like, or by fixed clock times instead. Sunrise
//! and sunset are instants in UTC that follow from the place alone; the
//! system's time zone only turns them into the clock times the last row
//! shows, so a change is never a guess.

use std::cell::Cell;
use std::rc::Rc;

use gtk4::prelude::*;

use super::form::{self, kind_row, scale_row, section_box, switch_row, time_row};
use super::location_map::LocationMap;
use super::store::{self, Daylight};
use crate::tokens::Mode;

/// One clock time: its two dropdowns and the minute rungs the second was
/// built from.
struct Clock {
    hour: gtk4::DropDown,
    minute: gtk4::DropDown,
    rungs: Vec<u8>,
}

impl Clock {
    fn set(&self, h: u8, m: u8) {
        self.hour.set_selected(u32::from(h.min(23)));
        let i = self.rungs.iter().position(|r| *r == m).unwrap_or(0);
        self.minute.set_selected(i as u32);
    }

    fn get(&self) -> (u8, u8) {
        let h = self.hour.selected().min(23) as u8;
        let m = self
            .rungs
            .get(self.minute.selected() as usize)
            .copied()
            .unwrap_or(0);
        (h, m)
    }
}

/// What a control's handler does: change one field of the section.
type Edit = dyn Fn(&dyn Fn(&Group, &mut Daylight));

struct Group {
    fixed: gtk4::Switch,
    day_row: gtk4::Box,
    day: Clock,
    night_row: gtk4::Box,
    night: Clock,
    map: Rc<LocationMap>,
    place_row: gtk4::Box,
    rise_row: gtk4::Box,
    rise: gtk4::Scale,
    set_row: gtk4::Box,
    set: gtk4::Scale,
    today: gtk4::Label,
    updating: Cell<bool>,
}

/// An offset as the row shows it.
fn offset_label(m: f64) -> String {
    let m = m.round() as i32;
    match m {
        0 => "As the sun".to_string(),
        m if m < 0 => format!("{} min earlier", -m),
        m => format!("{m} min later"),
    }
}

fn clock(m: u16) -> String {
    format!("{:02}:{:02}", m / 60, m % 60)
}

/// What the settings add up to today, for the last row.
fn describe_today(d: &Daylight) -> String {
    if !d.fixed_times && d.place().is_none() {
        return "Pick your place on the map: until then the automatic mode stays dark and the night light off".into();
    }
    let switches = crate::theme::sun::switches_today(d);
    if switches.is_empty() {
        return "No switch today: the sun does not cross the horizon here".into();
    }
    switches
        .iter()
        .map(|(m, mode)| {
            let what = match mode {
                Mode::Light => "light",
                Mode::Dark => "dark",
            };
            format!("{what} {}", clock(*m))
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

impl Group {
    /// Bring the controls in line with the store, and show only the rows the
    /// switches make live.
    fn sync(&self) {
        self.updating.set(true);
        let d = store::with(|s| s.daylight());
        self.fixed.set_active(d.fixed_times);
        self.day.set(d.day_from_h, d.day_from_m);
        self.night.set(d.night_from_h, d.night_from_m);
        self.map.set(d.place());
        self.rise.set_value(f64::from(d.sunrise_offset_m));
        self.set.set_value(f64::from(d.sunset_offset_m));

        self.day_row.set_visible(d.fixed_times);
        self.night_row.set_visible(d.fixed_times);
        let sun = !d.fixed_times;
        self.map.widget().set_visible(sun);
        self.place_row.set_visible(sun);
        self.rise_row.set_visible(sun);
        self.set_row.set_visible(sun);
        self.today.set_text(&describe_today(&d));
        self.updating.set(false);
    }
}

/// The group, built and following the store. Append it to the tab.
pub fn build() -> gtk4::Box {
    let section = section_box(
        "Day and night",
        "When the automatic mode turns light and dark, and when the night light warms. By the sun unless you set otherwise.",
    );

    let (fixed_row, fixed) = switch_row(
        "Fixed times",
        "Day and night start at the clock times below instead of by the sun. The mode switches on the minute; the night light fades over the half hour around it.",
        false,
    );
    let d = store::with(|s| s.daylight());
    let (day_row, day_h, day_m, day_rungs) = time_row(
        "Day from",
        "The mode turns light and the night light has cooled back to day.",
        d.day_from_m,
    );
    let (night_row, night_h, night_m, night_rungs) = time_row(
        "Night from",
        "The mode turns dark and the night light is warming.",
        d.night_from_m,
    );
    // A click on the map picks the place: the sun is computed there.
    let map = LocationMap::new(|lat, lon| {
        store::edit::<Daylight>(|d| {
            d.latitude = Some(lat);
            d.longitude = Some(lon);
        });
    });
    map.widget().set_margin_top(crate::tokens::space(2));
    let place_row = kind_row("Place", map.caption());
    place_row.set_tooltip_text(Some(
        "Where the sun is computed for. Click the map to move it; within a degree or two is plenty.",
    ));
    let max = f64::from(Daylight::MAX_OFFSET_M);
    let (rise_row, rise) = scale_row(
        "Sunrise",
        "Move the morning switch earlier or later than the sun, in five-minute steps.",
        (-max, max, 5.0),
        offset_label,
    );
    let (set_row, set) = scale_row(
        "Sunset",
        "Move the evening switch earlier or later than the sun, in five-minute steps.",
        (-max, max, 5.0),
        offset_label,
    );
    let today = form::value_label();
    today.set_wrap(true);
    today.set_xalign(1.0);
    let today_row = kind_row("Today", &today);
    today_row.set_tooltip_text(Some(
        "When the automatic mode switches today with these settings.",
    ));

    for row in [&fixed_row, &day_row, &night_row] {
        section.append(row);
    }
    section.append(map.widget());
    for row in [&place_row, &rise_row, &set_row, &today_row] {
        section.append(row);
    }

    let group = Rc::new(Group {
        fixed,
        day_row,
        day: Clock {
            hour: day_h,
            minute: day_m,
            rungs: day_rungs,
        },
        night_row,
        night: Clock {
            hour: night_h,
            minute: night_m,
            rungs: night_rungs,
        },
        map,
        place_row,
        rise_row,
        rise,
        set_row,
        set,
        today,
        updating: Cell::new(false),
    });

    // Each control writes one field; the store's observer below brings the
    // rows it hides or shows along.
    let edit = {
        let group = Rc::downgrade(&group);
        move |f: &dyn Fn(&Group, &mut Daylight)| {
            if let Some(g) = group.upgrade()
                && !g.updating.get()
            {
                store::edit::<Daylight>(|d| f(&g, d));
            }
        }
    };
    let edit: Rc<Edit> = Rc::new(edit);

    {
        let edit = edit.clone();
        group
            .fixed
            .connect_active_notify(move |s| edit(&|_, d| d.fixed_times = s.is_active()));
    }
    for dropdown in [
        &group.day.hour,
        &group.day.minute,
        &group.night.hour,
        &group.night.minute,
    ] {
        let edit = edit.clone();
        dropdown.connect_selected_notify(move |_| {
            edit(&|g, d| {
                (d.day_from_h, d.day_from_m) = g.day.get();
                (d.night_from_h, d.night_from_m) = g.night.get();
            });
        });
    }
    {
        let edit = edit.clone();
        group.rise.connect_value_changed(move |s| {
            let m = ((s.value() / 5.0).round() * 5.0) as i16;
            edit(&|_, d| d.sunrise_offset_m = m);
        });
    }
    {
        let edit = edit.clone();
        group.set.connect_value_changed(move |s| {
            let m = ((s.value() / 5.0).round() * 5.0) as i16;
            edit(&|_, d| d.sunset_offset_m = m);
        });
    }

    group.sync();
    {
        let group = Rc::downgrade(&group);
        store::observe(move || {
            if let Some(g) = group.upgrade() {
                g.sync();
            }
        });
    }
    // Held by the handlers' weak references only; the section owns the
    // widgets, so the group lives as long as the tab.
    std::mem::forget(group);
    section
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_offset_reads_as_a_direction() {
        assert_eq!(offset_label(0.0), "As the sun");
        assert_eq!(offset_label(-30.0), "30 min earlier");
        assert_eq!(offset_label(45.0), "45 min later");
    }
}
