use std::cell::Cell;
use std::rc::Rc;
use std::sync::mpsc::Receiver;
use std::time::Duration;

/// Spawn `work` on a background thread. When it finishes, call `on_done` on
/// the GTK main thread via `glib::idle_add_local_once`.
///
/// `on_done` may capture `!Send` GTK objects — it runs only on the main thread.
pub fn spawn_work<T, W, D>(work: W, on_done: D)
where
    T: Send + 'static,
    W: FnOnce() -> T + Send + 'static,
    D: FnOnce(T) + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel::<T>();

    std::thread::spawn(move || {
        let result = work();
        let _ = tx.send(result);
    });

    glib::idle_add_local_once(move || poll_channel(rx, on_done));
}

/// Spawn a dedicated named thread hosting a current-thread tokio runtime that
/// blocks on `fut`. Thread-spawn or runtime-build failure is logged once; the
/// future is then dropped, so any channels it captured disconnect and their
/// receivers see the worker as gone.
pub fn spawn_tokio_thread(name: &str, fut: impl std::future::Future<Output = ()> + Send + 'static) {
    let log_name = name.to_string();
    let spawned = std::thread::Builder::new()
        .name(name.to_string())
        .spawn(move || {
            match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt.block_on(fut),
                Err(e) => log::error!("{log_name}: failed to build tokio runtime: {e}"),
            }
        });
    if let Err(e) = spawned {
        log::warn!("{name}: failed to spawn thread: {e}");
    }
}

/// Poll a channel on the GTK main thread until a value arrives, then call `on_done`.
pub fn poll_channel<T: 'static>(rx: Receiver<T>, on_done: impl FnOnce(T) + 'static) {
    match rx.try_recv() {
        Ok(value) => on_done(value),
        Err(std::sync::mpsc::TryRecvError::Empty) => {
            // Re-arm on a timeout, not an idle source — idle sources have a
            // 0ms timeout and busy-spin a core for the job's whole lifetime.
            glib::timeout_add_local_once(std::time::Duration::from_millis(30), move || {
                poll_channel(rx, on_done)
            });
        }
        Err(_) => {} // sender dropped
    }
}

/// Remove a glib source, tolerating one that has already ended itself.
///
/// `glib::SourceId::remove` unwraps the `g_source_remove` result, so removing a
/// source that already returned `ControlFlow::Break` panics. Such removals run
/// inside a glib trampoline, where a panic cannot unwind: glib turns it into
/// "panic in a function that cannot unwind" and the process aborts. A source
/// that ended on its own is a normal race, so look it up first and destroy it
/// only if it is still attached.
pub fn remove_source(id: glib::SourceId) {
    if let Some(source) = glib::MainContext::default().find_source_by_id(&id) {
        source.destroy();
    }
}

/// The one pending timeout of something that may be re-armed or called off
/// before it fires: a debounce, a dwell, a hide-after. Arming replaces what
/// was pending, cancelling forgets it, and dropping the slot cancels it, so
/// no callback outlives the thing it was armed for.
///
/// A source that ended clears the slot before its callback returns, so the
/// callback may arm the slot again, and a later cancel never chases a
/// source that is gone.
#[derive(Default)]
pub struct Timeout(Rc<Cell<Option<glib::SourceId>>>);

impl Timeout {
    pub fn new() -> Self {
        Self::default()
    }

    /// Run `f` once after `after`, in place of whatever was pending.
    pub fn arm(&self, after: Duration, f: impl FnOnce() + 'static) {
        self.cancel();
        let slot = Rc::downgrade(&self.0);
        let id = glib::timeout_add_local_once(after, move || {
            if let Some(slot) = slot.upgrade() {
                slot.take();
            }
            f();
        });
        self.0.set(Some(id));
    }

    /// Run `f` every `interval` until it breaks or the slot is cancelled,
    /// in place of whatever was pending.
    pub fn every(&self, interval: Duration, f: impl FnMut() -> glib::ControlFlow + 'static) {
        self.repeat(f, |g| glib::timeout_add_local(interval, g));
    }

    /// [`every`](Self::every) at second granularity, which glib batches
    /// with other such timers into fewer wakeups.
    pub fn every_seconds(&self, secs: u32, f: impl FnMut() -> glib::ControlFlow + 'static) {
        self.repeat(f, |g| glib::timeout_add_seconds_local(secs, g));
    }

    fn repeat<F: FnMut() -> glib::ControlFlow + 'static>(
        &self,
        mut f: F,
        add: impl FnOnce(Box<dyn FnMut() -> glib::ControlFlow>) -> glib::SourceId,
    ) {
        self.cancel();
        let slot = Rc::downgrade(&self.0);
        let ours = Rc::new(Cell::new(0));
        let id = add(Box::new({
            let ours = ours.clone();
            move || {
                let flow = f();
                if flow.is_break()
                    && let Some(slot) = slot.upgrade()
                {
                    // Only our own entry: `f` may have armed the slot anew.
                    let held = slot.take();
                    if held.as_ref().map(glib::SourceId::as_raw) != Some(ours.get()) {
                        slot.set(held);
                    }
                }
                flow
            }
        }));
        ours.set(id.as_raw());
        self.0.set(Some(id));
    }

    /// Call off what is pending, if anything.
    pub fn cancel(&self) {
        if let Some(id) = self.0.take() {
            remove_source(id);
        }
    }

    /// Whether something is pending.
    pub fn is_armed(&self) -> bool {
        let id = self.0.take();
        let armed = id.is_some();
        self.0.set(id);
        armed
    }
}

impl Drop for Timeout {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Turn the default context until `done` or two seconds pass.
    fn run_until(done: impl Fn() -> bool) {
        let ctx = glib::MainContext::default();
        let start = std::time::Instant::now();
        while !done() && start.elapsed() < Duration::from_secs(2) {
            ctx.iteration(false);
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn run_for(span: Duration) {
        let start = std::time::Instant::now();
        run_until(|| start.elapsed() >= span);
    }

    // One test, because the slots share the default main context, which one
    // thread at a time may own.
    #[test]
    fn a_timeout_fires_once_and_can_be_replaced_cancelled_or_dropped() {
        let ctx = glib::MainContext::default();
        let _owner = ctx.acquire().expect("the default context is free");
        let hits = Rc::new(Cell::new(0));

        // Fires once, and the slot is empty after.
        let t = Timeout::new();
        let h = hits.clone();
        t.arm(Duration::from_millis(5), move || h.set(h.get() + 1));
        assert!(t.is_armed());
        run_until(|| hits.get() == 1);
        assert_eq!(hits.get(), 1);
        assert!(!t.is_armed());

        // A second arm replaces the first: only the last one runs.
        let h = hits.clone();
        t.arm(Duration::from_millis(5), move || h.set(h.get() + 100));
        let h = hits.clone();
        t.arm(Duration::from_millis(5), move || h.set(h.get() + 10));
        run_until(|| hits.get() > 1);
        run_for(Duration::from_millis(30));
        assert_eq!(hits.get(), 11);

        // Cancelled, and dropped while armed: neither runs.
        let h = hits.clone();
        t.arm(Duration::from_millis(5), move || h.set(h.get() + 1000));
        t.cancel();
        let dropped = Timeout::new();
        let h = hits.clone();
        dropped.arm(Duration::from_millis(5), move || h.set(h.get() + 1000));
        drop(dropped);

        // Repeats until it breaks, then the slot is empty.
        let ticks = Rc::new(Cell::new(0));
        let n = ticks.clone();
        t.every(Duration::from_millis(2), move || {
            n.set(n.get() + 1);
            if n.get() == 3 {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
        run_until(|| !t.is_armed());
        run_for(Duration::from_millis(30));
        assert_eq!(ticks.get(), 3);
        assert_eq!(hits.get(), 11);
    }
}
