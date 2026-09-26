//! Quiet hours: Do Not Disturb on a schedule.
//!
//! Edge-triggered, not level-triggered. Entering the window arms DND and
//! leaving it disarms DND; inside the window the tile is the user's, so a
//! manual toggle is not fought every half minute. The schedule and the
//! switch come from the Alerts tab (`settings::store::Alerts`), and a
//! change there re-evaluates at once: turning the switch on inside the
//! window arms now, turning it off inside the window disarms now.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use super::store::NotificationStore;
use crate::settings::store;

/// How often the clock is consulted. A window starts on a whole hour, so
/// this is the most it can start late.
const TICK: Duration = Duration::from_secs(30);

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
    {
        let evaluate = evaluate.clone();
        glib::timeout_add_local(TICK, move || {
            evaluate();
            glib::ControlFlow::Continue
        });
    }
    store::observe(move || evaluate());
}
