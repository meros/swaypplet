//! The popup cards' auto-dismiss timers: how long a card stays, and the
//! pause while the pointer is on the stack or a reply is being typed.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use super::stack::{State, alerts};
use crate::services::notifications::store::{self, NotificationStore};
use crate::services::notifications::{CloseReason, Notification, Urgency};

pub(super) enum Timer {
    None,
    Running {
        source: glib::SourceId,
        deadline: Instant,
    },
    Paused {
        remaining: Duration,
    },
}

pub(super) fn cancel_timer(timer: &mut Timer) {
    if let Timer::Running { source, .. } = std::mem::replace(timer, Timer::None) {
        crate::spawn::remove_source(source);
    }
}

fn timeout_for(notif: &Notification) -> Option<u64> {
    // Critical notifications with no explicit timeout are persistent
    if notif.urgency == Urgency::Critical && notif.expire_timeout <= 0 {
        return None;
    }
    // Timeout 0 means persistent (spec: server decides; we honor 0 as "never")
    if notif.expire_timeout == 0 {
        return None;
    }
    if notif.expire_timeout > 0 {
        Some(notif.expire_timeout as u64)
    } else {
        // -1 means server decides: scale with content length, from the
        // Alerts tab's linger.
        let (base, per_char) = alerts().linger.ms();
        let char_count = notif.summary.len() + notif.body.len();
        Some(base + (char_count as u64) * per_char)
    }
}

pub(super) fn make_timer(
    store: &Rc<RefCell<NotificationStore>>,
    notif: &Notification,
    hovered: bool,
) -> Timer {
    let Some(ms) = timeout_for(notif) else {
        return Timer::None;
    };
    let remaining = Duration::from_millis(ms);
    if hovered {
        return Timer::Paused { remaining };
    }
    schedule_close(store, notif.id, remaining)
}

fn schedule_close(store: &Rc<RefCell<NotificationStore>>, id: u32, after: Duration) -> Timer {
    let store = store.clone();
    let source = glib::timeout_add_local_once(after, move || {
        store::store_close(&store, id, CloseReason::Expired);
    });
    Timer::Running {
        source,
        deadline: Instant::now() + after,
    }
}

pub(super) fn pause_timers(st: &Rc<RefCell<State>>) {
    let mut s = st.borrow_mut();
    s.hovered = true;
    let now = Instant::now();
    for card in &mut s.cards {
        if matches!(card.timer, Timer::Running { .. }) {
            if let Timer::Running { source, deadline } =
                std::mem::replace(&mut card.timer, Timer::None)
            {
                crate::spawn::remove_source(source);
                card.timer = Timer::Paused {
                    remaining: deadline.saturating_duration_since(now),
                };
            }
        }
    }
}

pub(super) fn resume_timers(st: &Rc<RefCell<State>>) {
    let mut s = st.borrow_mut();
    s.hovered = false;
    let store = s.store.clone();
    for card in &mut s.cards {
        let Timer::Paused { remaining } = card.timer else {
            continue;
        };
        // Give a short grace period so a timer that expired mid-hover
        // doesn't vanish the instant the pointer leaves
        let after = remaining.max(Duration::from_millis(500));
        card.timer = schedule_close(&store, card.id, after);
    }
}
