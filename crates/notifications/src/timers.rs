//! The popup cards' auto-dismiss timers: how long a card stays, and the
//! pause while the pointer is on the stack or a reply is being typed.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use super::stack::{State, alerts};
use swaypplet_notifyd::{CloseReason, Notification, Urgency};
use swaypplet_notifyd::{store, store::NotificationStore};

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
        swaypplet_base::spawn::remove_source(source);
    }
}

/// How long `notif`'s card stays, in ms, or `None` for until dismissed.
/// `linger` is the Alerts tab's (base, per character) for a sender that
/// leaves the timeout to the server.
fn timeout_for(notif: &Notification, linger: (u64, u64)) -> Option<u64> {
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
        // -1 means server decides: scale with content length.
        let (base, per_char) = linger;
        let char_count = notif.summary.len() + notif.body.len();
        Some(base + (char_count as u64) * per_char)
    }
}

pub(super) fn make_timer(
    store: &Rc<RefCell<NotificationStore>>,
    notif: &Notification,
    hovered: bool,
) -> Timer {
    let Some(ms) = timeout_for(notif, alerts().linger.ms()) else {
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
        if matches!(card.timer, Timer::Running { .. })
            && let Timer::Running { source, deadline } =
                std::mem::replace(&mut card.timer, Timer::None)
        {
            swaypplet_base::spawn::remove_source(source);
            card.timer = Timer::Paused {
                remaining: deadline.saturating_duration_since(now),
            };
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

#[cfg(test)]
mod tests {
    use super::*;

    const LINGER: (u64, u64) = (5000, 40);

    fn notif(urgency: Urgency, expire_timeout: i32, summary: &str, body: &str) -> Notification {
        Notification {
            urgency,
            expire_timeout,
            summary: summary.into(),
            body: body.into(),
            ..Default::default()
        }
    }

    #[test]
    fn the_sender_s_own_timeout_wins_and_zero_is_never() {
        assert_eq!(
            timeout_for(&notif(Urgency::Normal, 1200, "", ""), LINGER),
            Some(1200)
        );
        assert_eq!(
            timeout_for(&notif(Urgency::Normal, 0, "", ""), LINGER),
            None
        );
        assert_eq!(
            timeout_for(&notif(Urgency::Critical, 1200, "", ""), LINGER),
            Some(1200)
        );
    }

    #[test]
    fn the_server_s_timeout_grows_with_the_text_and_spares_critical() {
        // Ten characters at 40 ms each on top of the 5 s base.
        assert_eq!(
            timeout_for(&notif(Urgency::Normal, -1, "Hello", "World"), LINGER),
            Some(5400)
        );
        assert_eq!(
            timeout_for(&notif(Urgency::Low, -1, "", ""), (3000, 25)),
            Some(3000)
        );
        assert_eq!(
            timeout_for(&notif(Urgency::Critical, -1, "Hello", ""), LINGER),
            None
        );
    }
}
