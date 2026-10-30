//! What a click on a notification does, wherever it is shown: a popup card
//! or a row in the notification centre.
//!
//! A sender that offers a default action knows which of its windows, tabs or
//! chats the notification is about, and is the one to say so. It gets the
//! action together with an xdg-activation token (the spec's
//! `ActivationToken`), which is what lets a Wayland client raise its own
//! window; without one, sway only marks the window urgent. When the
//! notification also names exactly one window, sway focuses that window as
//! well, so a sender that ignores the token still comes to the front.
//! Several windows of the sender are its call alone: guessing one of them
//! used to pick the window already in front, and the click did nothing.
//!
//! A notification without a default action is looked up in the tree (a
//! Claude session by its pid, anything else by name) and the best match is
//! focused.

use swaypplet_base::sway::tree::Pick;
use swaypplet_notifyd::{CloseReason, ImageSource, Notification};
use swaypplet_notifyd::{store, store::StoreRef};

/// An xdg-activation token from the compositor, for handing to another
/// client. Taken while the surface that was clicked still has the seat's
/// keyboard: GTK names that surface in the request, and a token that names
/// no focused surface is valid but raises nothing.
pub fn activation_token() -> Option<String> {
    use gtk4::prelude::*;
    let display = gtk4::gdk::Display::default()?;
    display
        .app_launch_context()
        .startup_notify_id(None::<&gio::AppInfo>, &[])
        .map(|token| token.to_string())
}

/// The action key a sender uses to ask for a reply field (KDE's convention,
/// which is what the `inline-reply` capability promises).
pub const INLINE_REPLY_KEY: &str = "inline-reply";

/// Where a notification's action buttons are drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// A popup card: the card itself is the default action and the reply
    /// field is the inline reply, so neither gets a button.
    Popup,
    /// A row in the panel's list: every action gets a button, "default"
    /// included, because a click on the row is out of reach of the keyboard.
    Panel,
}

/// One action button to draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VisibleAction<'a> {
    pub key: &'a str,
    pub label: &'a str,
    /// The icon to draw in place of the label, when the sender says its
    /// keys are icon names (`action-icons`) and this place honours that.
    /// The caller still checks the theme has it; the label is the tooltip.
    pub icon: Option<&'a str>,
}

/// The buttons `notif`'s actions get at `place`, in the sender's order.
pub fn visible_actions(notif: &Notification, place: Place) -> Vec<VisibleAction<'_>> {
    notif
        .actions
        .iter()
        .filter(|(key, _)| match place {
            Place::Popup => key != "default" && key != INLINE_REPLY_KEY,
            Place::Panel => true,
        })
        .map(|(key, label)| VisibleAction {
            key,
            label,
            icon: (place == Place::Popup && notif.action_icons).then_some(key.as_str()),
        })
        .collect()
}

/// Run a button's action: send it to the sender and close the notification,
/// unless it is `resident`, which is the spec's way of saying an action does
/// not dismiss. Without honouring it every button is terminal, so a
/// "snooze" or a "mark read" could never be offered by a sender.
pub fn run_action(store: &StoreRef, id: u32, key: &str, resident: bool) {
    invoke(store, id, key);
    if !resident {
        store::store_close(store, id, CloseReason::Dismissed);
    }
}

/// Invoke one of the sender's actions, with a token to act on it.
pub fn invoke(store: &StoreRef, id: u32, key: &str) {
    send_action(store, id, key, activation_token());
}

fn send_action(store: &StoreRef, id: u32, key: &str, token: Option<String>) {
    let with = if token.is_some() {
        ", with a token"
    } else {
        ""
    };
    log::info!("Action invoked: notification {id}, action {key}{with}");
    store::store_action_invoked(store, id, key, token.as_deref());
}

/// Take the user to where `notif` came from, then close it (a resident
/// notification whose default action ran stays). `then` runs once that is
/// done, on the GTK thread.
pub fn activate(notif: &Notification, store: &StoreRef, then: impl FnOnce() + 'static) {
    let id = notif.id;
    let resident = notif.resident;
    let has_default = notif.actions.iter().any(|(key, _)| key == "default");
    // Now, before the card or the panel goes away with the keyboard.
    let token = has_default.then(activation_token).flatten();
    let pick = if has_default {
        Pick::Unique
    } else {
        Pick::Best
    };
    let store = store.clone();
    swaypplet_base::sway::tree::focus_source(
        window_names(notif),
        notif.claude_pid,
        swaypplet_system::task_state::parent_pid,
        pick,
        move |_focused| {
            if has_default {
                send_action(&store, id, "default", token);
                if resident {
                    then();
                    return;
                }
            }
            store::store_close(&store, id, CloseReason::Dismissed);
            then();
        },
    );
}

/// What the notification says about itself that a window could be named
/// after, best evidence first.
///
/// The `desktop-entry` hint is the sender naming its own `.desktop` file,
/// which for a Wayland client is the same string sway reports as `app_id`.
/// `app_name` is free text but usually the program. A themed icon name is
/// what is left when a sender sets neither.
fn window_names(notif: &Notification) -> Vec<String> {
    let icon = match &notif.icon {
        Some(ImageSource::Named(name)) => Some(name.clone()),
        _ => None,
    };
    let mut names: Vec<String> = Vec::new();
    for name in [
        notif.desktop_entry.clone(),
        Some(notif.app_name.clone()),
        icon,
    ]
    .into_iter()
    .flatten()
    {
        let name = name.trim().trim_end_matches(".desktop").to_lowercase();
        if !name.is_empty() && !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_names_a_card_offers_are_ordered_by_how_much_they_are_worth() {
        let notif = Notification {
            app_name: "Fractal".into(),
            desktop_entry: Some("org.gnome.Fractal.desktop".into()),
            icon: Some(ImageSource::Named("fractal".into())),
            ..Default::default()
        };
        // The hint first, lowercased and stripped of its suffix; the icon
        // name is already the entry's tail, so it is not repeated.
        assert_eq!(window_names(&notif), ["org.gnome.fractal", "fractal"]);
        // A sender that says nothing about itself gets no lookup at all.
        assert!(window_names(&Notification::default()).is_empty());
    }

    fn offering(actions: &[(&str, &str)], action_icons: bool) -> Notification {
        Notification {
            actions: actions
                .iter()
                .map(|(key, label)| (key.to_string(), label.to_string()))
                .collect(),
            action_icons,
            ..Default::default()
        }
    }

    fn keys(actions: Vec<VisibleAction<'_>>) -> Vec<&str> {
        actions.into_iter().map(|action| action.key).collect()
    }

    #[test]
    fn a_popup_draws_neither_the_default_nor_the_reply_as_a_button() {
        let notif = offering(
            &[
                ("default", "Open"),
                (INLINE_REPLY_KEY, "Reply…"),
                ("archive", "Archive"),
                ("snooze", "Later"),
            ],
            false,
        );
        assert_eq!(
            keys(visible_actions(&notif, Place::Popup)),
            ["archive", "snooze"]
        );
        assert_eq!(
            keys(visible_actions(&notif, Place::Panel)),
            ["default", INLINE_REPLY_KEY, "archive", "snooze"]
        );
        assert!(visible_actions(&Notification::default(), Place::Popup).is_empty());
    }

    #[test]
    fn only_a_popup_draws_action_icons() {
        let notif = offering(&[("media-skip-forward", "Next")], true);
        assert_eq!(
            visible_actions(&notif, Place::Popup),
            [VisibleAction {
                key: "media-skip-forward",
                label: "Next",
                icon: Some("media-skip-forward"),
            }]
        );
        assert_eq!(visible_actions(&notif, Place::Panel)[0].icon, None);
        let plain = offering(&[("media-skip-forward", "Next")], false);
        assert_eq!(visible_actions(&plain, Place::Popup)[0].icon, None);
    }
}
