//! Which wired adapters are banned, for the bar's hazard lane.
//!
//! A ban ([`super::is_banned`]) is NetworkManager's runtime `Managed =
//! false`. NM also reports a freshly plugged adapter as unmanaged for the
//! moment udev takes to initialize it, so reading `Managed` alone would
//! flash the hazard on every dock plug-in. Without a timer, the tell is
//! history: an adapter counts as banned only once it has been seen managed
//! in this process, which a just-plugged one has not. NM's object path is
//! per plug-in, so a replug starts with no history.
//!
//! The one adapter that cannot have history is one already unmanaged when
//! the bar starts. Those count from the first read: a ban that outlives a
//! bar restart is the common case, and an adapter initializing in the same
//! instant the bar starts is not.
//!
//! Cadence: NM's device signals (`watch::devices`), one read per burst, on
//! a worker thread. No timer, no poll.

use std::collections::HashSet;

use gtk4::glib;

use super::Wired;
use crate::service::Observed;

thread_local! {
    /// The banned adapters, in NM's order.
    pub static BANNED: Observed<Vec<Wired>> = Observed::new(Vec::new());
    static STARTED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Start following NM, once per process; later calls do nothing.
pub fn start() {
    if STARTED.with(|s| s.replace(true)) {
        return;
    }
    let (tx, rx) = async_channel::bounded::<()>(1);
    super::watch::devices(tx.clone());
    // The first read, before any signal.
    let _ = tx.try_send(());
    glib::spawn_future_local(async move {
        // Held here so the subscription lives as long as this loop.
        let _tx = tx;
        let mut tracker = Tracker::default();
        while rx.recv().await.is_ok() {
            let (done_tx, done_rx) = async_channel::bounded(1);
            std::thread::spawn(move || {
                let _ = done_tx.send_blocking(super::wired_adapters());
            });
            let Ok(adapters) = done_rx.recv().await else {
                continue;
            };
            let banned = tracker.update(&adapters);
            BANNED.with(|b| b.set_if_changed(banned));
        }
    });
}

/// The history behind the plug-in rule in the module docs.
#[derive(Default)]
pub struct Tracker {
    primed: bool,
    seen_managed: HashSet<String>,
}

impl Tracker {
    /// The banned ones among `adapters`, a full read of NM's wired
    /// adapters.
    pub fn update(&mut self, adapters: &[Wired]) -> Vec<Wired> {
        if !self.primed {
            self.primed = true;
            self.seen_managed
                .extend(adapters.iter().map(|a| a.path.clone()));
        }
        self.seen_managed.extend(
            adapters
                .iter()
                .filter(|a| a.managed)
                .map(|a| a.path.clone()),
        );
        // Unplugged adapters take their history with them.
        self.seen_managed
            .retain(|p| adapters.iter().any(|a| &a.path == p));
        adapters
            .iter()
            .filter(|a| !a.managed && self.seen_managed.contains(&a.path))
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wired(path: &str, managed: bool) -> Wired {
        Wired {
            path: format!("/org/freedesktop/NetworkManager/Devices/{path}"),
            interface: format!("enp{path}"),
            managed,
            label: "Ethernet on USB hub".into(),
            mac: None,
        }
    }

    fn names(banned: Vec<Wired>) -> Vec<String> {
        banned.into_iter().map(|w| w.interface).collect()
    }

    #[test]
    fn a_ban_already_in_place_at_start_shows() {
        let mut t = Tracker::default();
        assert_eq!(names(t.update(&[wired("7", false)])), vec!["enp7"]);
    }

    #[test]
    fn a_just_plugged_adapter_is_not_banned_while_it_initializes() {
        let mut t = Tracker::default();
        assert!(t.update(&[]).is_empty());
        // Plug-in: unmanaged for a moment, then managed.
        assert!(t.update(&[wired("8", false)]).is_empty());
        assert!(t.update(&[wired("8", true)]).is_empty());
    }

    #[test]
    fn a_managed_adapter_that_loses_management_is_banned() {
        let mut t = Tracker::default();
        assert!(t.update(&[wired("8", true)]).is_empty());
        assert_eq!(names(t.update(&[wired("8", false)])), vec!["enp8"]);
        // Lifting the ban clears it.
        assert!(t.update(&[wired("8", true)]).is_empty());
    }

    #[test]
    fn a_replug_starts_without_history() {
        let mut t = Tracker::default();
        t.update(&[wired("8", true)]);
        assert_eq!(names(t.update(&[wired("8", false)])), vec!["enp8"]);
        // Unplugged: the ban goes with the device.
        assert!(t.update(&[]).is_empty());
        // Plugged back in under a new path, initializing.
        assert!(t.update(&[wired("9", false)]).is_empty());
    }
}
