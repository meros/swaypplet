use std::cell::RefCell;
use std::rc::Rc;
use std::time::SystemTime;

use gtk4::prelude::*;

use crate::notifications::CloseReason;
use crate::notifications::store::{self, NotificationStore};
use crate::ui;
use crate::ui::icons;

pub struct NotificationsSection {
    section: Rc<ui::Section>,
    list_box: gtk4::Box,
    empty_label: gtk4::Label,
    list_scroller: gtk4::ScrolledWindow,
    store: Rc<RefCell<NotificationStore>>,
}

impl NotificationsSection {
    pub fn new(store: Rc<RefCell<NotificationStore>>) -> Self {
        let section = Rc::new(ui::section(icons::NOTIFICATION, "Notifications", "None"));
        ui::glyph(&section.icon, ui::Text::Title, ui::Tone::Fg);

        // Header row: title + clear all button
        let header = ui::hbox(3);

        let title = ui::heading("Notifications");
        title.set_hexpand(true);

        let clear_btn = ui::glyph_button(icons::NOTIFICATION_CLEAR, "Clear all", ui::Kind::Flat);

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
            list_scroller: scroll.clone(),
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

    /// Cap the internal list scroller. Embedding contexts (start menu) use
    /// this instead of wrapping the section in a second ScrolledWindow —
    /// nested scrollers fight over scroll events and clip each other.
    pub fn set_list_max_height(&self, px: i32) {
        self.list_scroller.set_max_content_height(px);
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
    let notifications: Vec<crate::notifications::Notification> = {
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

    // Group by app_name, show newest first
    let mut grouped: std::collections::BTreeMap<String, Vec<&crate::notifications::Notification>> =
        std::collections::BTreeMap::new();
    for notif in notifications.iter().rev() {
        grouped
            .entry(notif.app_name.clone())
            .or_default()
            .push(notif);
    }

    for (app_name, notifs) in &grouped {
        if !app_name.is_empty() {
            list_box.append(&ui::heading(app_name));
        }

        for notif in notifs {
            let entry = build_entry(notif, store);
            list_box.append(&entry);
        }
    }

    true
}

fn build_entry(
    notif: &crate::notifications::Notification,
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
        ui::breathing(&bar);
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
            let btn = ui::small_button(label, ui::Kind::Secondary);

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
    let dismiss_btn = ui::glyph_button(icons::CLOSE, "Dismiss", ui::Kind::Flat);

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
