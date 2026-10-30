//! Shared skeleton for the push services (`sway::ipc`, `system::tray`,
//! `audio`, `devices::bluetooth` and the rest): an observed state cell for
//! the GTK side, and the reconnect loop with its backoff ladder that their
//! worker threads share.

use std::cell::RefCell;
use std::fmt::Display;
use std::rc::Rc;
use std::time::{Duration, Instant};

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
    /// observers don't re-render for nothing. Whether it was set.
    pub fn set_if_changed(&self, value: T) -> bool
    where
        T: PartialEq,
    {
        let changed = *self.state.borrow() != value;
        if changed {
            self.set(value);
        }
        changed
    }

    /// Keep the state at the latest snapshot `rx` delivers, dropping the
    /// ones equal to it. The loop runs on the GTK thread, holds the `Rc`,
    /// and ends when the worker drops its sender.
    pub fn follow(self: &Rc<Self>, rx: async_channel::Receiver<T>)
    where
        T: PartialEq + 'static,
    {
        self.follow_with(rx, |state, value| {
            state.set_if_changed(value);
        });
    }

    /// [`Self::follow`] with the step spelled out: `apply` gets each message
    /// and decides what reaches the state, for a service whose messages are
    /// not plain snapshots or whose state has no `PartialEq`.
    pub fn follow_with<M: 'static>(
        self: &Rc<Self>,
        rx: async_channel::Receiver<M>,
        apply: impl Fn(&Self, M) + 'static,
    ) where
        T: 'static,
    {
        let this = self.clone();
        glib::spawn_future_local(async move {
            while let Ok(message) = rx.recv().await {
                apply(&this, message);
            }
        });
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

/// Connect, serve, reconnect: a worker thread's whole life.
///
/// `session` is one connection. `Ok(())` means the GTK side hung up and the
/// thread ends; `Err` means the connection dropped. `on_lost` runs first,
/// to tell the GTK side (a default snapshot, so a banner has a reason
/// behind it), and returns whether to try again: `false` for a failure no
/// retry fixes, or a GTK side that is gone. Then the thread waits its rung
/// on the [`Backoff`] ladder and connects again.
pub fn reconnect<E: Display>(
    name: &str,
    session: impl FnMut() -> Result<(), E>,
    on_lost: impl FnMut(&E) -> bool,
) {
    reconnect_with(name, session, on_lost, std::thread::sleep);
}

fn reconnect_with<E: Display>(
    name: &str,
    mut session: impl FnMut() -> Result<(), E>,
    mut on_lost: impl FnMut(&E) -> bool,
    mut sleep: impl FnMut(Duration),
) {
    let mut backoff = Backoff::new();
    loop {
        let started = Instant::now();
        let Err(e) = session() else { return };
        if !on_lost(&e) {
            return;
        }
        let delay = backoff.next_delay(started.elapsed());
        log::warn!("{name}: {e}; reconnecting in {delay:?}");
        sleep(delay);
    }
}

/// [`reconnect`] for a session that is a future, on a tokio worker
/// (`spawn::spawn_tokio_thread`). A plain closure returning the future
/// rather than an async closure: the latter's future is not inferred
/// `Send`, which the worker thread needs.
pub async fn reconnect_async<E: Display, F: Future<Output = Result<(), E>>>(
    name: &str,
    session: impl FnMut() -> F,
    on_lost: impl FnMut(&E) -> bool,
) {
    reconnect_async_with(name, session, on_lost, tokio::time::sleep).await;
}

async fn reconnect_async_with<E: Display, F, S>(
    name: &str,
    mut session: impl FnMut() -> F,
    mut on_lost: impl FnMut(&E) -> bool,
    mut sleep: impl FnMut(Duration) -> S,
) where
    F: Future<Output = Result<(), E>>,
    S: Future<Output = ()>,
{
    let mut backoff = Backoff::new();
    loop {
        let started = Instant::now();
        let Err(e) = session().await else { return };
        if !on_lost(&e) {
            return;
        }
        let delay = backoff.next_delay(started.elapsed());
        log::warn!("{name}: {e}; reconnecting in {delay:?}");
        sleep(delay).await;
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

    /// Sessions that fail `failures` times and then end with the GTK side
    /// gone, through the blocking loop: how many ran, and the waits between.
    fn run_blocking(failures: usize, retry: bool) -> (usize, Vec<Duration>) {
        let (mut runs, mut slept) = (0, Vec::new());
        reconnect_with(
            "test",
            || {
                runs += 1;
                if runs <= failures {
                    Err("dropped")
                } else {
                    Ok(())
                }
            },
            |_| retry,
            |d| slept.push(d),
        );
        (runs, slept)
    }

    #[test]
    fn reconnect_retries_on_the_ladder_until_the_session_ends_cleanly() {
        let (runs, slept) = run_blocking(3, true);
        assert_eq!(runs, 4);
        assert_eq!(slept, [250, 500, 1000].map(Duration::from_millis));
    }

    #[test]
    fn reconnect_ends_when_the_gtk_side_hangs_up_at_once() {
        assert_eq!(run_blocking(0, true), (1, Vec::new()));
    }

    #[test]
    fn reconnect_stops_without_waiting_when_on_lost_says_so() {
        assert_eq!(run_blocking(5, false), (1, Vec::new()));
    }

    #[test]
    fn on_lost_sees_each_failure_before_the_wait() {
        let mut seen = Vec::new();
        let mut runs = 0;
        reconnect_with(
            "test",
            || {
                runs += 1;
                match runs {
                    1 => Err("refused".to_string()),
                    2 => Err("fatal".to_string()),
                    _ => Ok(()),
                }
            },
            |e: &String| {
                seen.push(e.clone());
                e != "fatal"
            },
            |_| {},
        );
        assert_eq!(seen, ["refused", "fatal"]);
        assert_eq!(runs, 2);
    }

    #[test]
    fn the_async_loop_follows_the_same_rules() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime");
        let (runs, slept) = rt.block_on(async {
            let (mut runs, mut slept) = (0, Vec::new());
            reconnect_async_with(
                "test",
                || {
                    runs += 1;
                    std::future::ready(if runs <= 2 { Err("dropped") } else { Ok(()) })
                },
                |_| true,
                |d| {
                    slept.push(d);
                    std::future::ready(())
                },
            )
            .await;
            (runs, slept)
        });
        assert_eq!(runs, 3);
        assert_eq!(slept, [250, 500].map(Duration::from_millis));

        let runs = rt.block_on(async {
            let mut runs = 0;
            reconnect_async_with(
                "test",
                || {
                    runs += 1;
                    std::future::ready(Err::<(), _>("fatal"))
                },
                |_| false,
                |_| std::future::ready(()),
            )
            .await;
            runs
        });
        assert_eq!(runs, 1);
    }

    /// Run `ctx` until nothing is pending.
    fn drain(ctx: &glib::MainContext) {
        while ctx.pending() {
            ctx.iteration(false);
        }
    }

    #[test]
    fn follow_keeps_the_latest_snapshot_and_drops_repeats() {
        let ctx = glib::MainContext::new();
        ctx.with_thread_default(|| {
            let obs = Rc::new(Observed::new(0));
            let fired = Rc::new(Cell::new(0));
            let f = fired.clone();
            obs.connect_change(move || f.set(f.get() + 1));
            let (tx, rx) = async_channel::unbounded();
            obs.follow(rx);
            for v in [1, 1, 2] {
                tx.send_blocking(v).expect("open");
            }
            drain(&ctx);
            assert_eq!(obs.with(|v| *v), 2);
            assert_eq!(fired.get(), 2);
        })
        .expect("context");
    }

    #[test]
    fn follow_with_lets_the_service_decide_what_lands() {
        let ctx = glib::MainContext::new();
        ctx.with_thread_default(|| {
            let obs = Rc::new(Observed::new(Some(1)));
            let (tx, rx) = async_channel::unbounded();
            // A failed read keeps the last good one.
            obs.follow_with(rx, |state, reading: Option<i32>| {
                if reading.is_some() {
                    state.set(reading);
                }
            });
            tx.send_blocking(None).expect("open");
            drain(&ctx);
            assert_eq!(obs.with(|v| *v), Some(1));
            tx.send_blocking(Some(3)).expect("open");
            drain(&ctx);
            assert_eq!(obs.with(|v| *v), Some(3));
        })
        .expect("context");
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
