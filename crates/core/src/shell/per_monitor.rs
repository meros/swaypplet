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
//!
//! [`only`](PerMonitor::only) narrows "every monitor" to the ones a rule
//! allows (an extra screen gets no bar unless asked, `shell::extra`). The
//! rule is asked again at every reconciliation, so a monitor it stops
//! allowing loses its entry and one it starts allowing gets one; an owner
//! whose rule reads a setting calls [`refresh`](PerMonitor::refresh) when
//! the setting changes.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use gtk4::prelude::*;
use gtk4::{gdk, gio};

type Build<T> = Box<dyn Fn(&gdk::Monitor) -> T>;
/// Whether a monitor gets an entry: its connector, and every connector.
type Only = Box<dyn Fn(&str, &[String]) -> bool>;

pub struct PerMonitor<T> {
    entries: RefCell<Vec<(gdk::Monitor, T)>>,
    build: RefCell<Option<Build<T>>>,
    after: RefCell<Option<Box<dyn Fn()>>>,
    only: RefCell<Option<Only>>,
    /// Monitors seen without a connector yet, each waiting on its
    /// `notify::connector` to be judged.
    unnamed: RefCell<Vec<gdk::Monitor>>,
    me: RefCell<Weak<Self>>,
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
            only: RefCell::new(None),
            unnamed: RefCell::new(Vec::new()),
            me: RefCell::new(Weak::new()),
        })
    }

    /// Build entries only for the monitors `rule` allows, given a monitor's
    /// connector and every connector. Set it before [`watch`](Self::watch).
    pub fn only(&self, rule: impl Fn(&str, &[String]) -> bool + 'static) {
        *self.only.borrow_mut() = Some(Box::new(rule));
    }

    /// Reconcile again: the rule given to [`only`](Self::only) may answer
    /// differently now.
    pub fn refresh(&self) {
        if let Some(display) = gdk::Display::default() {
            self.sync(&display.monitors());
        }
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
        *self.me.borrow_mut() = Rc::downgrade(self);
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
        let all: Vec<gdk::Monitor> = monitors
            .iter::<gdk::Monitor>()
            .filter_map(Result::ok)
            .collect();
        let current: Vec<gdk::Monitor> = match &*self.only.borrow() {
            None => all,
            Some(rule) => {
                // GDK lists a monitor before its output has said its name,
                // and a rule about names cannot judge one without it: it
                // waits, and is judged when the name arrives. Built at once,
                // an extra screen got a bar for as long as nothing asked
                // again.
                for m in all.iter().filter(|m| m.connector().is_none()) {
                    self.judge_when_named(m);
                }
                let names: Vec<String> = all
                    .iter()
                    .filter_map(|m| m.connector().map(|c| c.to_string()))
                    .collect();
                all.into_iter()
                    .filter(|m| m.connector().is_some_and(|c| rule(&c, &names)))
                    .collect()
            }
        };

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

    /// Reconcile once `monitor` has a connector.
    fn judge_when_named(&self, monitor: &gdk::Monitor) {
        if self.unnamed.borrow().contains(monitor) {
            return;
        }
        self.unnamed.borrow_mut().push(monitor.clone());
        let weak = self.me.borrow().clone();
        monitor.connect_connector_notify(move |m| {
            if let Some(this) = weak.upgrade() {
                this.unnamed.borrow_mut().retain(|u| u != m);
                this.refresh();
            }
        });
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
