use std::rc::Rc;

use super::{CloseReason, Notification, Urgency};

type NotifyCb = Rc<dyn Fn(&Notification)>;
type CloseCb = Rc<dyn Fn(u32, CloseReason)>;
type ChangeCb = Rc<dyn Fn()>;
type ActionCb = Rc<dyn Fn(u32, &str)>;

/// A Claude session located for stop-notification policy (vision O2):
/// which task owns it, and whether its workspace is showing on any output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskRef {
    /// Task number 1–4 (the workspace ":tN" infix).
    pub task: u8,
    /// The session's workspace is visible on some output right now.
    pub visible: bool,
}

/// pid → session location, injected from app.rs where the sway and task
/// services live — the store itself stays UI-agnostic.
type TaskResolver = Box<dyn Fn(i32) -> Option<TaskRef>>;

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
    /// IDs currently considered "open" by a D-Bus client — a superset of
    /// `notifications`, since transient notifications get an ID but are
    /// never stored in history. Used to tell a real close from a no-op.
    open_ids: std::collections::HashSet<u32>,
    next_id: u32,
    dnd_enabled: bool,
    on_notify: Vec<NotifyCb>,
    on_close: Vec<CloseCb>,
    on_change: Vec<ChangeCb>,
    on_action: Vec<ActionCb>,
    task_resolver: Option<TaskResolver>,
}

/// Deferred callbacks that must be fired after releasing the store's `RefCell`.
pub struct PendingCallbacks {
    notify: Vec<(NotifyCb, Notification)>,
    close: Vec<(CloseCb, u32, CloseReason)>,
    change: Vec<ChangeCb>,
    action: Vec<(ActionCb, u32, String)>,
}

impl PendingCallbacks {
    fn new() -> Self {
        Self {
            notify: Vec::new(),
            close: Vec::new(),
            change: Vec::new(),
            action: Vec::new(),
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
        for (cb, id, key) in self.action {
            cb(id, &key);
        }
    }
}

impl NotificationStore {
    pub fn new() -> Self {
        Self {
            notifications: Vec::new(),
            open_ids: std::collections::HashSet::new(),
            next_id: 1,
            dnd_enabled: false,
            on_notify: Vec::new(),
            on_close: Vec::new(),
            on_change: Vec::new(),
            on_action: Vec::new(),
            task_resolver: None,
        }
    }

    /// Inject the pid → session resolver (vision O2). Runs inside `add`
    /// while the store's `RefCell` borrow is held, so it must not call
    /// back into the store.
    pub fn set_task_resolver(&mut self, resolver: TaskResolver) {
        self.task_resolver = Some(resolver);
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

    pub fn connect_action(&mut self, cb: impl Fn(u32, &str) + 'static) {
        self.on_action.push(Rc::new(cb));
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
        // Stop-notification policy (vision O2): resolve the claude-pid
        // hint ONCE, now — attribution and suppression must reflect where
        // the session was when the notification arrived, not when it
        // renders. An unresolvable pid stays unattributed (normal rules).
        if let Some(pid) = notif.claude_pid
            && let Some(resolver) = &self.task_resolver
            && let Some(task_ref) = resolver(pid)
        {
            notif.task = Some(task_ref.task);
            notif.suppressed = task_ref.visible;
        }

        // Assign ID
        if notif.replaces_id > 0 {
            if let Some(pos) = self
                .notifications
                .iter()
                .position(|n| n.id == notif.replaces_id)
            {
                notif.id = notif.replaces_id;
                if notif.transient {
                    // A transient update must not leave a persistent entry behind.
                    self.notifications.remove(pos);
                } else {
                    self.notifications[pos] = notif.clone();
                }
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
        self.open_ids.insert(id);

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
        let mut pending = PendingCallbacks::new();
        // `open_ids` covers transient notifications too, so this is the only
        // reliable way to tell a real close from a no-op on an unknown id.
        if self.open_ids.remove(&id) {
            self.notifications.retain(|n| n.id != id);
            for cb in &self.on_close {
                pending.close.push((cb.clone(), id, reason));
            }
            self.collect_change(&mut pending);
        }
        pending
    }

    /// Remove all notifications from history.
    pub fn clear_all(&mut self) -> PendingCallbacks {
        let ids: Vec<u32> = self.notifications.iter().map(|n| n.id).collect();
        self.notifications.clear();
        for id in &ids {
            self.open_ids.remove(id);
        }

        let mut pending = PendingCallbacks::new();
        for id in ids {
            for cb in &self.on_close {
                pending.close.push((cb.clone(), id, CloseReason::Dismissed));
            }
        }
        self.collect_change(&mut pending);
        pending
    }

    /// Notify observers (e.g. the D-Bus layer) that an action was invoked.
    /// Does not mutate notification state.
    pub fn action_invoked(&self, id: u32, key: &str) -> PendingCallbacks {
        let mut pending = PendingCallbacks::new();
        for cb in &self.on_action {
            pending.action.push((cb.clone(), id, key.to_string()));
        }
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
        // A Claude session on a visible workspace already shows its state
        // on screen — the stop toast is noise there (vision O2). The
        // notification still enters history; Critical always pops.
        if notif.suppressed && notif.urgency != Urgency::Critical {
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

pub fn store_action_invoked(store: &StoreRef, id: u32, key: &str) {
    let pending = store.borrow().action_invoked(id, key);
    pending.fire();
}

pub fn store_clear_all(store: &StoreRef) {
    let pending = store.borrow_mut().clear_all();
    pending.fire();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notif(claude_pid: Option<i32>, urgency: Urgency) -> Notification {
        Notification {
            id: 0,
            app_name: "Claude".into(),
            summary: "stopped".into(),
            body: String::new(),
            urgency,
            actions: Vec::new(),
            expire_timeout: -1,
            timestamp: std::time::SystemTime::now(),
            transient: false,
            progress: None,
            replaces_id: 0,
            claude_pid,
            task: None,
            suppressed: false,
        }
    }

    /// Add through the policy and return the stored row.
    fn add_one(store: &mut NotificationStore, n: Notification) -> Notification {
        let (id, _pending) = store.add(n);
        store.all().iter().find(|n| n.id == id).unwrap().clone()
    }

    #[test]
    fn visible_session_is_suppressed_but_kept_in_history() {
        let mut store = NotificationStore::new();
        store.set_task_resolver(Box::new(|_| {
            Some(TaskRef {
                task: 2,
                visible: true,
            })
        }));
        let stored = add_one(&mut store, notif(Some(300), Urgency::Normal));
        assert_eq!(stored.task, Some(2));
        assert!(!store.should_popup(&stored));
        assert_eq!(store.all().len(), 1);
    }

    #[test]
    fn background_session_is_attributed_and_delivered() {
        let mut store = NotificationStore::new();
        store.set_task_resolver(Box::new(|_| {
            Some(TaskRef {
                task: 3,
                visible: false,
            })
        }));
        let stored = add_one(&mut store, notif(Some(300), Urgency::Normal));
        assert_eq!(stored.task, Some(3));
        assert!(store.should_popup(&stored));
    }

    #[test]
    fn no_hint_never_consults_the_resolver() {
        let calls = Rc::new(std::cell::Cell::new(0u32));
        let mut store = NotificationStore::new();
        let counter = calls.clone();
        store.set_task_resolver(Box::new(move |_| {
            counter.set(counter.get() + 1);
            Some(TaskRef {
                task: 1,
                visible: true,
            })
        }));
        let stored = add_one(&mut store, notif(None, Urgency::Normal));
        assert_eq!(calls.get(), 0);
        assert_eq!(stored.task, None);
        assert!(store.should_popup(&stored));
    }

    #[test]
    fn critical_is_attributed_but_never_suppressed() {
        let mut store = NotificationStore::new();
        store.set_task_resolver(Box::new(|_| {
            Some(TaskRef {
                task: 4,
                visible: true,
            })
        }));
        let stored = add_one(&mut store, notif(Some(300), Urgency::Critical));
        assert_eq!(stored.task, Some(4));
        assert!(store.should_popup(&stored));
    }

    #[test]
    fn unresolvable_pid_follows_normal_rules() {
        let mut store = NotificationStore::new();
        store.set_task_resolver(Box::new(|_| None));
        let stored = add_one(&mut store, notif(Some(300), Urgency::Normal));
        assert_eq!(stored.task, None);
        assert!(!stored.suppressed);
        assert!(store.should_popup(&stored));
    }

    #[test]
    fn without_a_resolver_the_hint_is_inert() {
        let mut store = NotificationStore::new();
        let stored = add_one(&mut store, notif(Some(300), Urgency::Normal));
        assert_eq!(stored.task, None);
        assert!(store.should_popup(&stored));
    }
}
