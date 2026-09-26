//! The popup card's `⋯` menu: snooze it, mute its app, or dismiss the stack.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gtk4::prelude::*;

use super::stack::{State, dismiss_all};
use crate::services::notifications::store::{self, NotificationStore};
use crate::services::notifications::{CloseReason, Notification};

/// The per-card menu: postpone it, silence the app, or go read the rest in
/// the centre.
pub(super) fn card_menu(
    notif: &Notification,
    store: &Rc<RefCell<NotificationStore>>,
    st: &Rc<RefCell<State>>,
) -> gtk4::Popover {
    // A popup, outside the compositor's layer effects: no glass behind it,
    // so the menu sits on the solid card.
    let list = crate::ui::menu();
    crate::ui::solid_card(&list);
    let popover = crate::ui::popover(&list, gtk4::PositionType::Bottom);

    let item = |label: &str| {
        let b = crate::ui::menu_item(label, "", false);
        b.set_halign(gtk4::Align::Fill);
        b
    };

    for (label, mins) in [("Snooze 5 min", 5u64), ("Snooze 30 min", 30)] {
        let b = item(label);
        let notif_c = notif.clone();
        let store_c = store.clone();
        let pop = popover.clone();
        b.connect_clicked(move |_| {
            pop.popdown();
            snooze(&store_c, &notif_c, Duration::from_secs(mins * 60));
        });
        list.append(&b);
    }

    if !notif.app_name.is_empty() {
        // A muted app can still be on screen: Critical overrides the mute, so
        // the one card you do see from it is the place to lift the silence.
        let muted = store.borrow().is_muted(&notif.app_name);
        let b = item(&if muted {
            format!("Unmute {}", notif.app_name)
        } else {
            format!("Mute {} for 1 h", notif.app_name)
        });
        let app = notif.app_name.clone();
        let id = notif.id;
        let store_c = store.clone();
        let pop = popover.clone();
        b.connect_clicked(move |_| {
            pop.popdown();
            if muted {
                store_c.borrow_mut().unmute_app(&app);
            } else {
                store_c
                    .borrow_mut()
                    .mute_app(&app, Duration::from_secs(3600));
                store::store_close(&store_c, id, CloseReason::Dismissed);
            }
        });
        list.append(&b);
    }

    let b = item("Dismiss all");
    let st_c = st.clone();
    let pop = popover.clone();
    b.connect_clicked(move |_| {
        pop.popdown();
        dismiss_all(&st_c);
    });
    list.append(&b);

    popover
}

/// Take a notification off screen and bring it back later, unchanged.
///
/// The re-presented copy carries no `replaces_id`: the original was closed,
/// so replacing it would target an id the store no longer holds open.
fn snooze(store: &Rc<RefCell<NotificationStore>>, notif: &Notification, after: Duration) {
    store::store_close(store, notif.id, CloseReason::Dismissed);
    let store_c = store.clone();
    let mut again = notif.clone();
    again.id = 0;
    again.replaces_id = 0;
    // Transient snoozed notifications would come back and immediately refuse
    // to be stored; the point of snoozing is that it comes back at all.
    again.transient = false;
    glib::timeout_add_local_once(after, move || {
        store::store_add(&store_c, again.clone());
    });
}
