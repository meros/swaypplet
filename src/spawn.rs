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
