//! StatusService — what wants the owner's eye, from every source, as one
//! sorted list.
//!
//! A source (Claude Code today; CI and the like later) turns its own state
//! into [`StatusItem`]s, and a surface reads only the items. The lock screen
//! is the first reader; the bar can move onto the same list later without a
//! change here. How an item is shown (a row, a count, a hidden title) is the
//! surface's decision, so nothing in an item is about one surface.
//!
//! No sway and no D-Bus: the lock process runs this, and it keeps its
//! dependencies to GTK (src/lock/mod.rs). What a surface needs from sway
//! (the workspace behind a PID, focus as acknowledgment) it adds on top.
//!
//! Cadence: a GFileMonitor per directory a source names, plus
//! [`StatusService::refresh`] for the surface's own tick. A process that
//! dies without a hook firing changes no file, so only a rescan sees it go.

pub mod claude;

use std::cell::RefCell;
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::SystemTime;

use gio::prelude::*;

use crate::service::Observed;

/// How much an item asks of the owner. The same three steps as the lid
/// dot's patterns (nixos users/modules/lid-dot.py), in rising order so
/// that `Ord` ranks the most urgent last and a sort can reverse it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Done or idle; good to know, nothing to do.
    Info,
    /// In progress; nothing to do yet.
    Active,
    /// Cannot go on without the owner.
    Attention,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusItem {
    /// The source's name, as the settings name it ("claude").
    pub source: &'static str,
    /// Unique within the source: a PID, a PR number.
    pub id: String,
    pub severity: Severity,
    pub title: String,
    pub detail: Option<String>,
    /// When the item entered its current severity; the oldest item of a
    /// severity sorts first.
    pub since: Option<SystemTime>,
}

pub trait StatusSource {
    /// The directories whose changes can change this source's items.
    fn watch(&self) -> Vec<PathBuf>;
    fn scan(&self) -> Vec<StatusItem>;
}

pub struct StatusService {
    sources: Vec<Box<dyn StatusSource>>,
    state: Observed<Vec<StatusItem>>,
    /// A dropped GFileMonitor stops watching.
    _monitors: RefCell<Vec<gio::FileMonitor>>,
}

impl StatusService {
    pub fn start(sources: Vec<Box<dyn StatusSource>>) -> Rc<Self> {
        let service = Rc::new(Self {
            sources,
            state: Observed::new(Vec::new()),
            _monitors: RefCell::new(Vec::new()),
        });
        let dirs: Vec<PathBuf> = service.sources.iter().flat_map(|s| s.watch()).collect();
        for dir in dirs {
            // The watcher needs the directory before the first writer makes it.
            if let Err(e) = fs::create_dir_all(&dir) {
                log::warn!("status: create {}: {e}", dir.display());
            }
            match gio::File::for_path(&dir)
                .monitor_directory(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
            {
                Ok(monitor) => {
                    let weak = Rc::downgrade(&service);
                    monitor.connect_changed(move |_, _, _, _| {
                        if let Some(service) = weak.upgrade() {
                            service.refresh();
                        }
                    });
                    service._monitors.borrow_mut().push(monitor);
                }
                Err(e) => log::warn!("status: watch {}: {e}", dir.display()),
            }
        }
        service.refresh();
        service
    }

    pub fn connect_change(&self, cb: impl Fn() + 'static) {
        self.state.connect_change(cb);
    }

    /// Every item, the most urgent first.
    pub fn items(&self) -> Vec<StatusItem> {
        self.state.with(Clone::clone)
    }

    /// Rescan every source. Observers fire only when the list changed.
    pub fn refresh(&self) {
        let items = merge(self.sources.iter().flat_map(|s| s.scan()).collect());
        self.state.set_if_changed(items);
    }
}

/// Severity first, the most urgent on top; within a severity the oldest
/// first, an unknown age last; source and id keep the order stable.
fn merge(mut items: Vec<StatusItem>) -> Vec<StatusItem> {
    items.sort_by(|a, b| {
        b.severity
            .cmp(&a.severity)
            .then_with(|| match (a.since, b.since) {
                (Some(a), Some(b)) => a.cmp(&b),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            })
            .then_with(|| a.source.cmp(b.source))
            .then_with(|| a.id.cmp(&b.id))
    });
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn item(id: &str, severity: Severity, since: Option<u64>) -> StatusItem {
        StatusItem {
            source: "test",
            id: id.into(),
            severity,
            title: id.into(),
            detail: None,
            since: since.map(|s| SystemTime::UNIX_EPOCH + Duration::from_secs(s)),
        }
    }

    fn ids(items: &[StatusItem]) -> Vec<&str> {
        items.iter().map(|i| i.id.as_str()).collect()
    }

    #[test]
    fn attention_sorts_first_then_active_then_info() {
        let merged = merge(vec![
            item("info", Severity::Info, Some(1)),
            item("attention", Severity::Attention, Some(3)),
            item("active", Severity::Active, Some(2)),
        ]);
        assert_eq!(ids(&merged), ["attention", "active", "info"]);
    }

    #[test]
    fn the_oldest_sorts_first_within_a_severity() {
        let merged = merge(vec![
            item("unknown", Severity::Attention, None),
            item("new", Severity::Attention, Some(200)),
            item("old", Severity::Attention, Some(100)),
        ]);
        assert_eq!(ids(&merged), ["old", "new", "unknown"]);
    }

    #[test]
    fn ties_break_on_id() {
        let merged = merge(vec![
            item("b", Severity::Active, None),
            item("a", Severity::Active, None),
        ]);
        assert_eq!(ids(&merged), ["a", "b"]);
    }
}
