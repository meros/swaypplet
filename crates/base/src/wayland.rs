//! The start and the life of a Wayland client that runs beside GTK's own
//! connection (`displays`, the night light, the clipboard history): connect,
//! then dispatch on a worker thread for as long as the compositor answers.
//! What each one binds and holds stays with it.

use std::sync::{Mutex, MutexGuard};

use wayland_client::{Connection, EventQueue};

/// A connection of its own, from the environment. `None` off Wayland, said
/// in the log under `who`.
pub fn connect(who: &str) -> Option<Connection> {
    Connection::connect_to_env()
        .map_err(|e| log::info!("{who}: no Wayland connection: {e}"))
        .ok()
}

/// Dispatch `queue` into `state` on a thread named `thread`, until the
/// connection fails; `on_stop` then runs on that thread. `None` when the
/// thread could not be started.
pub fn dispatch_on_thread<S: Send + 'static>(
    thread: &str,
    who: &str,
    mut queue: EventQueue<S>,
    mut state: S,
    on_stop: impl FnOnce() + Send + 'static,
) -> Option<()> {
    let who_ = who.to_string();
    std::thread::Builder::new()
        .name(thread.to_string())
        .spawn(move || {
            loop {
                if let Err(e) = queue.blocking_dispatch(&mut state) {
                    log::warn!("{who_}: dispatch stopped: {e}");
                    on_stop();
                    return;
                }
            }
        })
        .map_err(|e| log::warn!("{who}: thread: {e}"))
        .ok()
        .map(drop)
}

/// Lock state shared with the dispatch thread. A panic on the other side
/// leaves the data as it was, which is still the best picture there is, so
/// a poisoned lock is taken rather than passed on as a second panic.
pub fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_poisoned_lock_still_hands_out_the_data() {
        let mutex = std::sync::Arc::new(Mutex::new(5));
        let poisoner = mutex.clone();
        let _ = std::thread::spawn(move || {
            let _guard = poisoner.lock().unwrap();
            panic!("poison it");
        })
        .join();
        assert!(mutex.is_poisoned());
        assert_eq!(*lock(&mutex), 5);
    }
}
