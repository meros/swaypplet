//! Quiet hours: Do Not Disturb on a schedule.
//!
//! Edge-triggered, not level-triggered. Entering the window arms DND and
//! leaving it disarms DND; inside the window the tile is the user's, so a
//! manual toggle is not fought at every check. The schedule and the
//! switch come from the Alerts tab (`settings::store::Alerts`), and a
//! change there re-evaluates at once: turning the switch on inside the
//! window arms now, turning it off inside the window disarms now.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use super::store::NotificationStore;
use crate::settings::store;

/// The longest the clock goes unchecked. A window starts and ends on a
/// whole hour, so the check is aimed at the next one; but GLib's timers run
/// on the monotonic clock, which stops while the machine sleeps, so a single
/// wait until the hour could fire late by however long it slept. Capped at
/// this, the check wakes at most four times an hour, and a window is at most
/// this late after a resume. It used to wake every 30 s.
const MAX_WAIT: Duration = Duration::from_secs(15 * 60);

/// Until the next whole hour (and a second past it), at most [`MAX_WAIT`].
fn until_next_check() -> Duration {
    let into_hour = glib::DateTime::now_local()
        .map(|t| u64::from(t.minute() as u32) * 60 + t.second() as u64)
        .unwrap_or(0);
    Duration::from_secs(3600 - into_hour.min(3599) + 1).min(MAX_WAIT)
}

/// Run `evaluate` at each [`until_next_check`], for the life of the process.
fn schedule(evaluate: Rc<dyn Fn()>) {
    glib::timeout_add_local_once(until_next_check(), move || {
        evaluate();
        schedule(evaluate);
    });
}

pub fn install(store: Rc<std::cell::RefCell<NotificationStore>>) {
    // What the last evaluation found; `None` until the first.
    let last = Rc::new(Cell::new(None::<bool>));

    let evaluate = {
        let store = store.clone();
        let last = last.clone();
        Rc::new(move || {
            let alerts = store::current().alerts();
            let hour = glib::DateTime::now_local()
                .map(|t| t.hour() as u8)
                .unwrap_or(12);
            let inside = alerts.quiet && alerts.in_quiet_hours(hour);
            match (last.get(), inside) {
                (Some(true), true) | (Some(false), false) | (None, false) => {}
                (_, true) => {
                    log::info!(
                        "quiet hours: {}–{} — DND on",
                        alerts.quiet_from_h,
                        alerts.quiet_to_h
                    );
                    store.borrow_mut().set_dnd(true);
                }
                (Some(true), false) => {
                    log::info!("quiet hours: over — DND off");
                    store.borrow_mut().set_dnd(false);
                }
            }
            last.set(Some(inside));
        })
    };

    evaluate();
    schedule(evaluate.clone());
    store::observe(move || evaluate());
}
