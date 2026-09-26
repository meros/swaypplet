//! Follow files by inotify (GIO's file monitor), not by polling.
//!
//! The settings, the wallpaper's cache line and the other files one process
//! writes and another reads used to be `stat`ed once a second in every
//! process that cared. That is a wake-up a second each, forever, for files
//! that change a few times a day. A monitor wakes the process only when the
//! file moved.
//!
//! The parent directory is watched, not the file: every writer here saves
//! by writing a temporary file and renaming it over the old one, which a
//! monitor on the old file's inode does not see, and the file may not exist
//! yet when the watch starts. Events for other names in the directory are
//! dropped.
//!
//! Bursts (a save is a create, a write and a rename) collapse into one call
//! after [`SETTLE`], so a reader never reloads a half-written file twice.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;

/// How long after the last event of a burst the callback runs.
pub const SETTLE: Duration = Duration::from_millis(60);

/// Handles that keep the monitors alive; dropping them stops the watch.
pub struct Monitors(#[allow(dead_code)] Vec<gio::FileMonitor>);

/// Call `on_change` on this thread's main context after any of `files`
/// changed, was created, replaced or removed. The returned handle must be
/// kept for as long as the watch should last.
pub fn files(files: &[PathBuf], on_change: impl Fn() + 'static) -> Monitors {
    let on_change: Rc<dyn Fn()> = Rc::new(on_change);
    let pending: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();
    let mut monitors = Vec::new();
    for path in files {
        let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
            continue;
        };
        let name = name.to_owned();
        let Ok(monitor) = gio::File::for_path(dir)
            .monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE)
        else {
            log::warn!("watch: cannot monitor {}", dir.display());
            continue;
        };
        let on_change = on_change.clone();
        let pending = pending.clone();
        monitor.connect_changed(move |_, file, other, _| {
            let named = |f: &gio::File| f.basename().is_some_and(|b| b.as_os_str() == name);
            if !named(file) && !other.is_some_and(named) {
                return;
            }
            if let Some(id) = pending.borrow_mut().take() {
                id.remove();
            }
            let on_change = on_change.clone();
            let slot = pending.clone();
            *pending.borrow_mut() = Some(glib::timeout_add_local_once(SETTLE, move || {
                slot.borrow_mut().take();
                on_change();
            }));
        });
        monitors.push(monitor);
    }
    Monitors(monitors)
}

/// [`files`] for a process with no GLib main loop of its own: a thread runs
/// one, and `on_change` is called on that thread.
pub fn files_on_thread(paths: Vec<PathBuf>, on_change: impl Fn() + Send + 'static) {
    let spawned = std::thread::Builder::new()
        .name("file-watch".into())
        .spawn(move || {
            let context = glib::MainContext::new();
            let Ok(()) = context.with_thread_default(|| {
                let _monitors = files(&paths, on_change);
                loop {
                    context.iteration(true);
                }
            }) else {
                log::warn!("watch: cannot own a main context for {paths:?}");
                return;
            };
        });
    if let Err(e) = spawned {
        log::warn!("watch: thread: {e}");
    }
}
