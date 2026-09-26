//! Notifications grouped by sender: what both the popup stack and the
//! notification centre show as one card or one entry with a count.
//!
//! A burst from one sender (a CI run, a chat) is one conversation, and a
//! stack of copies of it costs every other sender their slots. So each
//! sender gets one card: the newest notification on it, and a count of the
//! rest behind a disclosure.
//!
//! The sender is the `desktop-entry` hint when the notification carries one
//! (the sender naming its own `.desktop` file is the stronger identity) and
//! the app name otherwise, compared without case. Two exceptions stay on
//! their own:
//!
//! - **Critical** urgency. A critical notification is the one kind that
//!   never expires, and folding it under a chatty sender's count would hide
//!   exactly what must not be missed.
//! - A notification with **no sender** at all: there is nothing to group it
//!   by.
//!
//! `replaces_id` needs nothing here: the store replaces the notification in
//! place under the same id, so a group's members and count do not move.

use super::{Notification, Urgency};

/// The sender a notification groups under, or `None` when it stands alone.
pub fn key(n: &Notification) -> Option<String> {
    if n.urgency == Urgency::Critical {
        return None;
    }
    let sender = n
        .desktop_entry
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(&n.app_name);
    (!sender.is_empty()).then(|| sender.to_lowercase())
}

/// One sender's notifications, newest first.
#[derive(Debug)]
pub struct Group<'a> {
    /// `None` for a notification that stands alone ([`key`]).
    pub key: Option<String>,
    /// Newest first; never empty.
    pub items: Vec<&'a Notification>,
}

impl Group<'_> {
    /// The notification the group shows.
    pub fn newest(&self) -> &Notification {
        self.items[0]
    }

    /// Every id in the group, for closing it as one.
    pub fn ids(&self) -> Vec<u32> {
        self.items.iter().map(|n| n.id).collect()
    }
}

/// `all` (oldest first, as the store keeps it) as groups, the group with the
/// newest notification first.
pub fn groups(all: &[Notification]) -> Vec<Group<'_>> {
    let mut out: Vec<Group<'_>> = Vec::new();
    for n in all.iter().rev() {
        let key = key(n);
        match key
            .as_ref()
            .and_then(|k| out.iter_mut().find(|g| g.key.as_ref() == Some(k)))
        {
            Some(group) => group.items.push(n),
            None => out.push(Group {
                key,
                items: vec![n],
            }),
        }
    }
    out
}

/// A popup card's members, oldest first, after `id` arrives on it: an id
/// already there (a `replaces_id` update) keeps its place, a new one goes
/// last. Returns whether it was new.
pub fn join(members: &mut Vec<u32>, id: u32) -> bool {
    if members.contains(&id) {
        return false;
    }
    members.push(id);
    true
}

/// The count a group card shows, in words for the accessible label and the
/// tooltip. `None` for a group of one, which has no count.
pub fn describe(count: usize, sender: &str) -> Option<String> {
    (count > 1).then(|| {
        if sender.is_empty() {
            format!("{count} notifications")
        } else {
            format!("{count} notifications from {sender}")
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::notifications::store::NotificationStore;
    use crate::services::notifications::CloseReason;

    fn n(app: &str, summary: &str, urgency: Urgency) -> Notification {
        Notification {
            app_name: app.into(),
            summary: summary.into(),
            urgency,
            timestamp: std::time::SystemTime::now(),
            ..Default::default()
        }
    }

    fn add(store: &mut NotificationStore, notif: Notification) -> u32 {
        store.add(notif).0
    }

    #[test]
    fn a_sender_is_one_group_and_the_newest_group_leads() {
        let mut s = NotificationStore::new();
        let ci1 = add(&mut s, n("CI", "build 1", Urgency::Normal));
        let chat = add(&mut s, n("Chat", "hi", Urgency::Normal));
        let ci2 = add(&mut s, n("ci", "build 2", Urgency::Normal));
        let g = groups(s.all());
        assert_eq!(g.len(), 2, "{g:?}");
        // CI's newest arrived last, so CI leads; case does not split it.
        assert_eq!(g[0].ids(), vec![ci2, ci1]);
        assert_eq!(g[0].newest().summary, "build 2");
        assert_eq!(g[1].ids(), vec![chat]);
    }

    #[test]
    fn the_desktop_entry_outranks_the_app_name() {
        let a = Notification {
            desktop_entry: Some("org.gnome.Fractal".into()),
            ..n("Fractal", "a", Urgency::Normal)
        };
        let b = Notification {
            desktop_entry: Some("org.gnome.Fractal".into()),
            ..n("fractal-notify", "b", Urgency::Normal)
        };
        assert_eq!(key(&a), key(&b));
        assert_eq!(key(&a).as_deref(), Some("org.gnome.fractal"));
    }

    #[test]
    fn a_replacement_keeps_its_place_and_the_count() {
        let mut s = NotificationStore::new();
        let first = add(&mut s, n("CI", "build 1 running", Urgency::Normal));
        let second = add(&mut s, n("CI", "build 2 running", Urgency::Normal));
        let replaced = add(
            &mut s,
            Notification {
                replaces_id: first,
                ..n("CI", "build 1 passed", Urgency::Normal)
            },
        );
        assert_eq!(replaced, first);
        let g = groups(s.all());
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].items.len(), 2);
        assert_eq!(g[0].ids(), vec![second, first]);
        assert!(g[0].items.iter().any(|m| m.summary == "build 1 passed"));

        let mut members = vec![first, second];
        assert!(!join(&mut members, first), "a replacement is not a new member");
        assert!(join(&mut members, 9));
        assert_eq!(members, vec![first, second, 9]);
    }

    #[test]
    fn critical_and_senderless_stand_alone() {
        let mut s = NotificationStore::new();
        add(&mut s, n("Disk", "90 % full", Urgency::Normal));
        let urgent = add(&mut s, n("Disk", "98 % full", Urgency::Critical));
        add(&mut s, n("Disk", "95 % full", Urgency::Normal));
        add(&mut s, n("", "anonymous 1", Urgency::Normal));
        add(&mut s, n("", "anonymous 2", Urgency::Normal));
        let g = groups(s.all());
        // Disk (2 normal), the critical one alone, and two senderless ones.
        assert_eq!(g.len(), 4, "{g:?}");
        let critical: Vec<_> = g.iter().filter(|g| g.key.is_none()).collect();
        assert_eq!(critical.len(), 3);
        assert!(critical.iter().any(|g| g.ids() == vec![urgent]));
        let disk = g.iter().find(|g| g.key.as_deref() == Some("disk")).unwrap();
        assert_eq!(disk.items.len(), 2);
    }

    #[test]
    fn closing_a_group_closes_every_member() {
        let mut s = NotificationStore::new();
        add(&mut s, n("CI", "1", Urgency::Normal));
        add(&mut s, n("Chat", "hi", Urgency::Normal));
        add(&mut s, n("CI", "2", Urgency::Normal));
        add(&mut s, n("CI", "3", Urgency::Normal));
        let ci = groups(s.all())
            .into_iter()
            .find(|g| g.key.as_deref() == Some("ci"))
            .unwrap()
            .ids();
        assert_eq!(ci.len(), 3);
        let _ = s.close_all_of(&ci, CloseReason::Dismissed);
        let g = groups(s.all());
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].newest().app_name, "Chat");
    }

    #[test]
    fn the_count_is_said_in_words() {
        assert_eq!(describe(1, "CI"), None);
        assert_eq!(describe(3, "CI").as_deref(), Some("3 notifications from CI"));
        assert_eq!(describe(2, "").as_deref(), Some("2 notifications"));
    }
}
