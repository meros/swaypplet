use std::rc::Rc;

use super::{CloseReason, Notification, Urgency};

type NotifyCb = Rc<dyn Fn(&Notification)>;
type CloseCb = Rc<dyn Fn(u32, CloseReason)>;
type ChangeCb = Rc<dyn Fn()>;

/// Single source of truth for all notification state.
///
/// Lives on the GTK main thread behind `Rc<RefCell<…>>`.
///
/// **Callback safety:** Mutating methods (`add`, `close`, `clear_all`) do NOT
/// fire callbacks directly — they return deferred work via `PendingCallbacks`.
/// The caller must call `.fire()` *after* releasing the `RefCell` borrow.
/// Maximum number of notifications to keep in history.
const MAX_NOTIFICATIONS: usize = 50;

pub struct NotificationStore {
    notifications: Vec<Notification>,
    next_id: u32,
    dnd_enabled: bool,
    on_notify: Vec<NotifyCb>,
    on_close: Vec<CloseCb>,
    on_change: Vec<ChangeCb>,
}

/// Deferred callbacks that must be fired after releasing the store's `RefCell`.
pub struct PendingCallbacks {
    notify: Vec<(NotifyCb, Notification)>,
    close: Vec<(CloseCb, u32, CloseReason)>,
    change: Vec<ChangeCb>,
}

impl PendingCallbacks {
    fn new() -> Self {
        Self {
            notify: Vec::new(),
            close: Vec::new(),
            change: Vec::new(),
        }
    }

    /// Fire all deferred callbacks. Must be called **outside** any `borrow_mut()`.
    pub fn fire(self) {
        for (cb, notif) in self.notify {
            cb(&notif);
        }
        for (cb, id, reason) in self.close {
            cb(id, reason);
        }
        for cb in self.change {
            cb();
        }
    }
}

impl NotificationStore {
    pub fn new() -> Self {
        Self {
            notifications: Vec::new(),
            next_id: 1,
            dnd_enabled: false,
            on_notify: Vec::new(),
            on_close: Vec::new(),
            on_change: Vec::new(),
        }
    }

    // ── Observer registration ────────────────────────────────────────────

    pub fn connect_notify(&mut self, cb: impl Fn(&Notification) + 'static) {
        self.on_notify.push(Rc::new(cb));
    }

    pub fn connect_close(&mut self, cb: impl Fn(u32, CloseReason) + 'static) {
        self.on_close.push(Rc::new(cb));
    }

    pub fn connect_change(&mut self, cb: impl Fn() + 'static) {
        self.on_change.push(Rc::new(cb));
    }

    // ── DND ──────────────────────────────────────────────────────────────

    pub fn is_dnd(&self) -> bool {
        self.dnd_enabled
    }

    pub fn set_dnd(&mut self, enabled: bool) {
        self.dnd_enabled = enabled;
    }

    // ── Core operations ──────────────────────────────────────────────────

    /// Add or replace a notification. Returns `(assigned_id, pending_callbacks)`.
    /// Caller **must** call `pending.fire()` after releasing the borrow.
    pub fn add(&mut self, mut notif: Notification) -> (u32, PendingCallbacks) {
        // Assign ID
        if notif.replaces_id > 0 {
            if let Some(existing) = self
                .notifications
                .iter_mut()
                .find(|n| n.id == notif.replaces_id)
            {
                notif.id = existing.id;
                *existing = notif.clone();
            } else {
                notif.id = self.next_id;
                self.next_id += 1;
                if !notif.transient {
                    self.notifications.push(notif.clone());
                }
            }
        } else {
            notif.id = self.next_id;
            self.next_id += 1;
            if !notif.transient {
                self.notifications.push(notif.clone());
            }
        }

        let id = notif.id;

        // Trim oldest notifications if over the limit
        while self.notifications.len() > MAX_NOTIFICATIONS {
            self.notifications.remove(0);
        }

        let mut pending = PendingCallbacks::new();
        for cb in &self.on_notify {
            pending.notify.push((cb.clone(), notif.clone()));
        }
        self.collect_change(&mut pending);

        (id, pending)
    }

    /// Close a notification by ID. Returns `PendingCallbacks`.
    pub fn close(&mut self, id: u32, reason: CloseReason) -> PendingCallbacks {
        self.notifications.retain(|n| n.id != id);

        let mut pending = PendingCallbacks::new();
        for cb in &self.on_close {
            pending.close.push((cb.clone(), id, reason));
        }
        self.collect_change(&mut pending);
        pending
    }

    /// Remove all notifications from history.
    pub fn clear_all(&mut self) -> PendingCallbacks {
        let ids: Vec<u32> = self.notifications.iter().map(|n| n.id).collect();
        self.notifications.clear();

        let mut pending = PendingCallbacks::new();
        for id in ids {
            for cb in &self.on_close {
                pending.close.push((cb.clone(), id, CloseReason::Dismissed));
            }
        }
        self.collect_change(&mut pending);
        pending
    }

    /// Get all notifications (newest first).
    pub fn all(&self) -> &[Notification] {
        &self.notifications
    }

    /// Check whether a notification should show a popup.
    pub fn should_popup(&self, notif: &Notification) -> bool {
        // Low urgency → silent-to-center (no popup)
        if notif.urgency == Urgency::Low {
            return false;
        }
        // DND suppresses everything except critical
        if self.dnd_enabled && notif.urgency != Urgency::Critical {
            return false;
        }
        true
    }

    fn collect_change(&self, pending: &mut PendingCallbacks) {
        for cb in &self.on_change {
            pending.change.push(cb.clone());
        }
    }
}

// ── Convenience free functions ──────────────────────────────────────────
//
// These ensure the RefCell borrow is released before callbacks fire.
// Using `store.borrow_mut().close(...).fire()` is WRONG because the
// temporary RefMut lives until the end of the statement, so callbacks
// execute while the borrow is still held.

use std::cell::RefCell;

pub type StoreRef = Rc<RefCell<NotificationStore>>;

pub fn store_add(store: &StoreRef, notif: Notification) -> u32 {
    let (id, pending) = store.borrow_mut().add(notif);
    pending.fire();
    id
}

pub fn store_close(store: &StoreRef, id: u32, reason: CloseReason) {
    let pending = store.borrow_mut().close(id, reason);
    pending.fire();
}

pub fn store_clear_all(store: &StoreRef) {
    let pending = store.borrow_mut().clear_all();
    pending.fire();
}
