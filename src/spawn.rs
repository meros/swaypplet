use std::sync::mpsc::Receiver;

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

/// Poll a channel on the GTK main thread until a value arrives, then call `on_done`.
pub fn poll_channel<T: 'static>(rx: Receiver<T>, on_done: impl FnOnce(T) + 'static) {
    match rx.try_recv() {
        Ok(value) => on_done(value),
        Err(std::sync::mpsc::TryRecvError::Empty) => {
            glib::idle_add_local_once(move || poll_channel(rx, on_done));
        }
        Err(_) => {} // sender dropped
    }
}
