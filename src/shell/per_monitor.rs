//! One of something per output, following hotplug.
//!
//! A layer surface is bound to its `wl_output`: it cannot be moved to another
//! one, so a surface that belongs on every screen (the bar, the OSD, the
//! keybinding sheet) is built per monitor, and a monitor that leaves takes
//! its surface with it. [`PerMonitor`] is that reconciliation, written once:
//! drop the entry of a monitor that went away, build one for a monitor that
//! arrived, leave the rest alone.
//!
//! Dropping an entry is its teardown, so an entry that owns a
//! [`Surface`](super::Surface) needs nothing else to leave cleanly.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use gtk4::prelude::*;
use gtk4::{gdk, gio};

type Build<T> = Box<dyn Fn(&gdk::Monitor) -> T>;

pub struct PerMonitor<T> {
    entries: RefCell<Vec<(gdk::Monitor, T)>>,
    build: RefCell<Option<Build<T>>>,
    after: RefCell<Option<Box<dyn Fn()>>>,
}

impl<T: 'static> PerMonitor<T> {
    /// An empty set, following nothing yet: [`watch`](Self::watch) starts it.
    /// Split in two so an owner can hand the build closure a weak handle to
    /// itself.
    pub fn new() -> Rc<Self> {
        Rc::new(PerMonitor {
            entries: RefCell::new(Vec::new()),
            build: RefCell::new(None),
            after: RefCell::new(None),
        })
    }

    /// Run `f` after every reconciliation that added or dropped an entry
    /// (the greeter moves keyboard focus to a card that still exists). Set it
    /// before [`watch`](Self::watch) to have it run for the first one too.
    pub fn after_change(&self, f: impl Fn() + 'static) {
        *self.after.borrow_mut() = Some(Box::new(f));
    }

    /// Build an entry for every monitor now, and keep doing so as monitors
    /// come and go. `build` runs with nothing borrowed, so it may read this
    /// set (to copy what the others show, say).
    pub fn watch(self: &Rc<Self>, build: impl Fn(&gdk::Monitor) -> T + 'static) {
        *self.build.borrow_mut() = Some(Box::new(build));
        let Some(display) = gdk::Display::default() else {
            return;
        };
        let monitors = display.monitors();
        // Weak: the monitor list lives for the display, and a strong
        // capture would keep the set, and every surface in it, alive with it.
        let weak: Weak<Self> = Rc::downgrade(self);
        monitors.connect_items_changed(move |monitors, _, _, _| {
            if let Some(this) = weak.upgrade() {
                this.sync(monitors);
            }
        });
        self.sync(&monitors);
    }

    fn sync(&self, monitors: &gio::ListModel) {
        let current: Vec<gdk::Monitor> = monitors
            .iter::<gdk::Monitor>()
            .filter_map(Result::ok)
            .collect();

        // Taken out before they drop: an entry's teardown may call back into
        // GTK, and GTK back into this set.
        let gone: Vec<(gdk::Monitor, T)> = {
            let mut entries = self.entries.borrow_mut();
            let (keep, gone) = std::mem::take(&mut *entries)
                .into_iter()
                .partition(|(m, _)| current.contains(m));
            *entries = keep;
            gone
        };
        let mut changed = !gone.is_empty();
        drop(gone);

        for monitor in current {
            if self.entries.borrow().iter().any(|(m, _)| *m == monitor) {
                continue;
            }
            let entry = {
                let build = self.build.borrow();
                let Some(build) = build.as_ref() else { return };
                build(&monitor)
            };
            self.entries.borrow_mut().push((monitor, entry));
            changed = true;
        }
        if changed && let Some(after) = &*self.after.borrow() {
            after();
        }
    }

    /// Visit every entry.
    pub fn for_each(&self, mut f: impl FnMut(&T)) {
        for (_, entry) in self.entries.borrow().iter() {
            f(entry);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.borrow().is_empty()
    }
}
