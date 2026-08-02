//! Shared skeleton for the bar's push services (`sway_ipc`, `bar::tray`):
//! an observed state cell for the GTK side plus the reconnect-backoff
//! ladder their worker loops share.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

/// State cell plus observer list for a service living on the GTK main
/// thread behind `Rc`. The recv loop spawned by the service's `start`
/// holds a strong ref to the service, so it persists for the process.
pub struct Observed<T> {
    state: RefCell<T>,
    on_change: RefCell<Vec<Rc<dyn Fn()>>>,
}

impl<T> Observed<T> {
    pub fn new(initial: T) -> Self {
        Self {
            state: RefCell::new(initial),
            on_change: RefCell::new(Vec::new()),
        }
    }

    pub fn connect_change(&self, cb: impl Fn() + 'static) {
        self.on_change.borrow_mut().push(Rc::new(cb));
    }

    /// Read the state without handing out the `RefCell` guard.
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        f(&self.state.borrow())
    }

    /// Replace the state and notify observers. The list is cloned and
    /// fired outside the borrow — a callback may re-enter a getter or
    /// connect another observer.
    pub fn set(&self, value: T) {
        *self.state.borrow_mut() = value;
        let callbacks: Vec<_> = self.on_change.borrow().clone();
        for cb in &callbacks {
            cb();
        }
    }

    /// [`Self::set`] that drops snapshots equal to the current state, so
    /// observers don't re-render for nothing.
    pub fn set_if_changed(&self, value: T)
    where
        T: PartialEq,
    {
        if *self.state.borrow() != value {
            self.set(value);
        }
    }
}

/// Reconnect ladder for the worker loops: exponential from 250 ms to 8 s.
/// A session that lived a while was a real connection that died (sway
/// restart, bus exit), not a refused socket, so the ladder restarts for
/// the failure after it.
pub struct Backoff {
    current: Duration,
}

impl Backoff {
    const INITIAL: Duration = Duration::from_millis(250);
    const MAX: Duration = Duration::from_secs(8);
    /// Sessions longer than this count as real connections.
    const LONG_SESSION: Duration = Duration::from_secs(10);

    pub fn new() -> Self {
        Self {
            current: Self::INITIAL,
        }
    }

    /// Delay to wait before the next attempt, given how long the session
    /// that just failed lived.
    pub fn next_delay(&mut self, session_lived: Duration) -> Duration {
        let delay = self.current;
        self.current = if session_lived > Self::LONG_SESSION {
            Self::INITIAL
        } else {
            (self.current * 2).min(Self::MAX)
        };
        delay
    }
}

impl Default for Backoff {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn set_fires_and_a_callback_may_connect_another_observer() {
        let obs = Rc::new(Observed::new(0));
        let fired = Rc::new(Cell::new(0));
        let (for_cb, fired_outer) = (obs.clone(), fired.clone());
        obs.connect_change(move || {
            fired_outer.set(fired_outer.get() + 1);
            let fired_inner = fired_outer.clone();
            for_cb.connect_change(move || fired_inner.set(fired_inner.get() + 100));
        });
        obs.set(1);
        assert_eq!(fired.get(), 1);
        assert_eq!(obs.with(|v| *v), 1);
    }

    #[test]
    fn set_if_changed_skips_no_op_snapshots() {
        let obs = Observed::new(7);
        let fired = Rc::new(Cell::new(0));
        let f = fired.clone();
        obs.connect_change(move || f.set(f.get() + 1));
        obs.set_if_changed(7);
        assert_eq!(fired.get(), 0);
        obs.set_if_changed(8);
        assert_eq!(fired.get(), 1);
    }

    #[test]
    fn backoff_doubles_to_the_cap_on_fast_failures() {
        let mut b = Backoff::new();
        let refused = Duration::from_millis(5);
        let delays: Vec<Duration> = (0..7).map(|_| b.next_delay(refused)).collect();
        assert_eq!(
            delays,
            [250, 500, 1000, 2000, 4000, 8000, 8000].map(Duration::from_millis)
        );
    }

    #[test]
    fn long_session_restarts_the_ladder_for_the_next_failure() {
        let mut b = Backoff::new();
        for _ in 0..4 {
            b.next_delay(Duration::from_millis(5));
        }
        // The long session's own failure still waits the current rung...
        assert_eq!(
            b.next_delay(Duration::from_secs(60)),
            Duration::from_secs(4)
        );
        // ...the reset applies from the following attempt.
        assert_eq!(
            b.next_delay(Duration::from_millis(5)),
            Duration::from_millis(250)
        );
    }
}
