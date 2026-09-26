use std::cell::RefCell;
use std::rc::Rc;
use std::time::SystemTime;

use gtk4::prelude::*;

use crate::services::notifications::{CloseReason, group};
use crate::services::notifications::store::{self, NotificationStore};
use crate::ui;
use crate::ui::icons;

pub struct NotificationsSection {
    section: Rc<ui::Section>,
    list_box: gtk4::Box,
    empty_label: gtk4::Label,
    store: Rc<RefCell<NotificationStore>>,
}

impl NotificationsSection {
    pub fn new(store: Rc<RefCell<NotificationStore>>) -> Self {
        let section = Rc::new(ui::section(icons::NOTIFICATION, "Notifications", "None"));
        ui::glyph::adopt(&section.icon, ui::Text::Title, ui::Tone::Fg);

        // Header row: title + clear all button
        let header = ui::hbox(3);

        let title = ui::heading("Notifications");
        title.set_hexpand(true);

        let clear_btn = ui::button_with(
            ui::Face::Glyph {
                glyph: icons::NOTIFICATION_CLEAR,
                tooltip: "Clear all",
            },
            ui::Kind::Flat,
            ui::Size::Normal,
        );

        let store_clear = store.clone();
        clear_btn.connect_clicked(move |_| {
            store::store_clear_all(&store_clear);
        });

        header.append(&title);
        header.append(&clear_btn);
        section.body.append(&header);

        // Scrollable list area
        let scroll = gtk4::ScrolledWindow::builder()
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .propagate_natural_height(true)
            .max_content_height(300)
            .build();

        let list_box = ui::vbox(2);

        let empty_label = ui::text("No notifications", ui::Text::Body, ui::Tone::Muted);
        empty_label.set_halign(gtk4::Align::Center);
        empty_label.add_css_class("section-empty");
        list_box.append(&empty_label);

        scroll.set_child(Some(&list_box));
        section.body.append(&scroll);

        let notifications = Self {
            section,
            list_box,
            empty_label,
            store: store.clone(),
        };

        // Subscribe to changes for live updates
        let list_box_c = notifications.list_box.clone();
        let empty_label_c = notifications.empty_label.clone();
        let section_c = notifications.section.clone();
        let store_change = store.clone();
        store.borrow_mut().connect_change(move || {
            let has_notifications = rebuild_list(
                &list_box_c,
                &empty_label_c,
                &section_c.summary,
                &store_change,
            );
            // Auto-expand when new notifications arrive
            if has_notifications && !section_c.revealer.reveals_child() {
                section_c.set_open(true);
            }
        });

        notifications.rebuild();
        notifications
    }

    pub fn refresh(&self) {
        self.rebuild();
    }

    /// Switch into page mode: reveal detail immediately, hide the summary
    /// toggle row.
    pub fn expand_for_page(&self) {
        self.section.show_as_page();
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.section.root
    }

    fn rebuild(&self) {
        rebuild_list(
            &self.list_box,
            &self.empty_label,
            &self.section.summary,
            &self.store,
        );
    }
}

/// Rebuilds the notification list and updates the summary text.
/// Returns `true` if there are notifications (used to auto-expand).
fn rebuild_list(
    list_box: &gtk4::Box,
    empty_label: &gtk4::Label,
    summary_text: &gtk4::Label,
    store: &Rc<RefCell<NotificationStore>>,
) -> bool {
    // Remove all children except the empty label
    while let Some(child) = list_box.last_child() {
        if child == *empty_label {
            break;
        }
        list_box.remove(&child);
    }

    // Clone notification data out of the borrow to avoid holding RefCell
    // across widget creation (which could trigger re-entrant GTK callbacks).
    let notifications: Vec<crate::services::notifications::Notification> = {
        let store_ref = store.borrow();
        store_ref.all().to_vec()
    };

    let count = notifications.len();

    // Update summary text
    match count {
        0 => summary_text.set_label("None"),
        n => summary_text.set_label(&format!("{n}")),
    }

    if notifications.is_empty() {
        empty_label.set_visible(true);
        return false;
    }
    empty_label.set_visible(false);

    // One entry per sender, the sender with the newest notification first
    // (`group`): the newest in full, the rest under a disclosure, and a
    // button that closes the lot. Critical ones stand alone, as on the popups.
    for g in group::groups(&notifications) {
        let newest = g.newest();
        let count = g.items.len();
        let head = ui::hbox(2);
        let name = if newest.app_name.is_empty() {
            "Other".to_string()
        } else {
            newest.app_name.clone()
        };
        let title = ui::heading(&name);
        title.set_hexpand(true);
        title.set_xalign(0.0);
        head.append(&title);
        if let Some(words) = group::describe(count, &newest.app_name) {
            let badge = ui::badge(&count.to_string(), ui::BadgeTone::Neutral);
            badge.set_valign(gtk4::Align::Center);
            badge.set_tooltip_text(Some(&words));
            head.append(&badge);
            head.update_property(&[gtk4::accessible::Property::Label(&words)]);
            let close = ui::button_with(
                ui::Face::Glyph {
                    glyph: icons::NOTIFICATION_CLEAR,
                    tooltip: "Dismiss all from this sender",
                },
                ui::Kind::Flat,
                ui::Size::Small,
            );
            let ids = g.ids();
            let store_c = store.clone();
            close.connect_clicked(move |_| {
                store::store_close_all_of(&store_c, &ids, CloseReason::Dismissed);
            });
            head.append(&close);
        }
        list_box.append(&head);
        list_box.append(&build_entry(newest, store));
        if count > 1 {
            let more = ui::disclosure(&format!("{} earlier", count - 1));
            for notif in &g.items[1..] {
                more.body.append(&build_entry(notif, store));
            }
            list_box.append(&more.root);
        }
    }

    true
}

fn build_entry(
    notif: &crate::services::notifications::Notification,
    store: &Rc<RefCell<NotificationStore>>,
) -> gtk4::Box {
    let r = ui::row("", &notif.summary, "");

    // The time sits after the summary, in the row's metadata tone.
    let time_label = ui::text(
        &format_relative_time(notif.timestamp),
        ui::Text::Caption,
        ui::Tone::Faint,
    );
    let texts = r
        .title
        .parent()
        .and_downcast::<gtk4::Box>()
        .expect("a row's title sits in its text column");
    let header_row = ui::hbox(3);
    texts.remove(&r.title);
    r.title.set_hexpand(true);
    header_row.append(&r.title);
    header_row.append(&time_label);
    texts.prepend(&header_row);

    if !notif.body.is_empty() {
        let markup = crate::notifications::markup::sanitize(&notif.body);
        let body = &r.subtitle;
        body.set_label(&markup);
        body.set_use_markup(true);
        body.set_wrap(true);
        body.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
        body.set_max_width_chars(50);
        body.set_lines(3);
        body.set_visible(true);
    }

    if let Some(progress) = notif.progress {
        let bar = ui::progress(progress as f64 / 100.0);
        bar.set_hexpand(true);
        ui::set_breathing(&bar, true);
        texts.append(&bar);
    }

    // The notification's own actions, which used to exist only on the popup
    // and only under a pointer. The panel opens from a keybinding and GTK
    // gives its buttons ordinary focus traversal, so putting them here is
    // what makes an action reachable without a pointer at all (P8) — and it
    // does that without an input grab on an overlay-layer surface.
    if !notif.actions.is_empty() {
        let actions_box = ui::hbox(2);

        for (key, label) in &notif.actions {
            // "default" is what clicking the notification itself means; in a
            // list of rows there is no such gesture to hang it on, so it gets
            // a button like any other.
            let btn = ui::button_with(ui::Face::Label(label), ui::Kind::Secondary, ui::Size::Small);

            let id = notif.id;
            let key_c = key.clone();
            let store_c = store.clone();
            let resident = notif.resident;
            btn.connect_clicked(move |_| {
                store::store_action_invoked(&store_c, id, &key_c);
                if !resident {
                    store::store_close(&store_c, id, CloseReason::Dismissed);
                }
            });
            actions_box.append(&btn);
        }
        texts.append(&actions_box);
    }

    // Dismiss button
    let dismiss_btn = ui::button_with(
        ui::Face::Glyph {
            glyph: icons::CLOSE,
            tooltip: "Dismiss",
        },
        ui::Kind::Flat,
        ui::Size::Normal,
    );

    let id = notif.id;
    let store_c = store.clone();
    dismiss_btn.connect_clicked(move |_| {
        store::store_close(&store_c, id, CloseReason::Dismissed);
    });
    r.end.append(&dismiss_btn);

    r.root
}

fn format_relative_time(timestamp: SystemTime) -> String {
    let elapsed = SystemTime::now()
        .duration_since(timestamp)
        .unwrap_or_default();

    let secs = elapsed.as_secs();
    if secs < 60 {
        "just now".to_string()
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86400)
    }
}
