//! Per-frame timing for every window, for `dev/frame-bench.sh`.
//!
//! Off unless `SWAYPPLET_FRAME_STATS` names a file. Then every toplevel's
//! frame clock is followed, and each painted frame appends one line:
//!
//! ```text
//! <ms since start> <window> interval=<ms> work=<ms>
//! ```
//!
//! `interval` is the time since that window's previous frame (0 for its
//! first), `work` the wall time from `before-paint` to `after-paint`: style,
//! layout, snapshot and the renderer's submit on this thread. GTK's own
//! `GDK_DEBUG=frames` prints only a sample of frames, which cannot count a
//! stutter.
//!
//! Self-contained on purpose, so the same file can be dropped into an older
//! checkout to compare two builds on one compositor.

use std::cell::RefCell;
use std::io::Write;
use std::rc::Rc;
use std::time::Instant;

use gtk4::glib;
use gtk4::prelude::*;

/// Start following, if asked to. Call once, after GTK is initialised.
pub fn init() {
    let Some(path) = std::env::var_os("SWAYPPLET_FRAME_STATS") else {
        return;
    };
    let Ok(file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    else {
        return;
    };
    let out = Rc::new(RefCell::new(std::io::BufWriter::new(file)));
    let epoch = Instant::now();
    let seen: Rc<RefCell<Vec<glib::WeakRef<gtk4::Window>>>> = Rc::default();
    // New windows appear at any time. 50 ms is soon enough to catch a
    // surface on its first frames; this whole module only runs under the
    // bench, so the poll costs nothing in a session.
    glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
        for w in gtk4::Window::list_toplevels() {
            let Ok(win) = w.downcast::<gtk4::Window>() else {
                continue;
            };
            if seen
                .borrow()
                .iter()
                .any(|s| s.upgrade().as_ref() == Some(&win))
            {
                continue;
            }
            let Some(clock) = win.frame_clock() else {
                continue;
            };
            seen.borrow_mut().push(win.downgrade());
            let name = format!(
                "{}#{}",
                win.css_classes()
                    .iter()
                    .find(|c| !c.starts_with("background"))
                    .map_or_else(|| "window".to_string(), ToString::to_string),
                seen.borrow().len()
            );
            let started = Rc::new(RefCell::new(None::<Instant>));
            let last = Rc::new(RefCell::new(None::<Instant>));
            {
                let started = started.clone();
                clock.connect_before_paint(move |_| {
                    *started.borrow_mut() = Some(Instant::now());
                });
            }
            let out = out.clone();
            clock.connect_after_paint(move |_| {
                let now = Instant::now();
                let work = started
                    .borrow()
                    .map_or(0.0, |s| now.duration_since(s).as_secs_f64() * 1e3);
                let interval = last
                    .borrow()
                    .map_or(0.0, |l| now.duration_since(l).as_secs_f64() * 1e3);
                *last.borrow_mut() = Some(now);
                let mut o = out.borrow_mut();
                let _ = writeln!(
                    o,
                    "{:.1} {name} interval={interval:.2} work={work:.2}",
                    now.duration_since(epoch).as_secs_f64() * 1e3
                );
                let _ = o.flush();
            });
        }
        glib::ControlFlow::Continue
    });
}
