//! Whether the session is locked, from logind's LockedHint.
//!
//! The lock runs in its own process; logind is where it says so. The theme
//! reads it to time a sun-driven mode switch for when nobody is looking
//! (docs/design-system.md §2.1). Followed as a property stream, not polled.

use std::cell::Cell;

thread_local! {
    static LOCKED: Cell<bool> = const { Cell::new(false) };
    /// Called on the main thread when LockedHint moves.
    static ON_CHANGE: std::cell::RefCell<Vec<Box<dyn Fn()>>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Run `cb` whenever the lock state changes: a sun switch waits for the
/// lock, so the lock is one of the theme's triggers.
pub(super) fn on_change(cb: impl Fn() + 'static) {
    ON_CHANGE.with(|c| c.borrow_mut().push(Box::new(cb)));
}

pub(super) fn is_locked() -> bool {
    LOCKED.with(Cell::get)
}

/// Start following LockedHint (`base::logind::watch_locked_hint`), for as
/// long as this process lives.
pub(super) fn follow() {
    let rx = swaypplet_base::logind::watch_locked_hint();
    glib::spawn_future_local(async move {
        while let Ok(locked) = rx.recv().await {
            if LOCKED.with(|l| l.replace(locked)) != locked {
                ON_CHANGE.with(|c| c.borrow().iter().for_each(|cb| cb()));
            }
        }
    });
}
