use std::cell::RefCell;
use std::rc::Rc;
use std::time::SystemTime;

use gtk4::prelude::*;

use crate::icons;
use crate::notifications::store::{self, NotificationStore};
use crate::notifications::CloseReason;

pub struct NotificationsSection {
    root: gtk4::Box,
    list_box: gtk4::Box,
    empty_label: gtk4::Label,
    store: Rc<RefCell<NotificationStore>>,
}

impl NotificationsSection {
    pub fn new(store: Rc<RefCell<NotificationStore>>) -> Self {
        let root = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .build();
        root.add_css_class("section");
        root.add_css_class("notification-center");

        // Header row: title + clear all button
        let header = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(8)
            .build();

        let title = gtk4::Label::builder()
            .label("NOTIFICATIONS")
            .halign(gtk4::Align::Start)
            .hexpand(true)
            .build();
        title.add_css_class("section-title");

        let clear_btn = gtk4::Button::builder()
            .label(icons::NOTIFICATION_CLEAR)
            .tooltip_text("Clear all")
            .build();
        clear_btn.add_css_class("flat");
        clear_btn.add_css_class("notification-clear-btn");

        let store_clear = store.clone();
        clear_btn.connect_clicked(move |_| {
            store::store_clear_all(&store_clear);
        });

        header.append(&title);
        header.append(&clear_btn);
        root.append(&header);

        // Scrollable list area
        let scroll = gtk4::ScrolledWindow::builder()
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .propagate_natural_height(true)
            .max_content_height(300)
            .build();

        let list_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(4)
            .build();

        let empty_label = gtk4::Label::builder()
            .label("No notifications")
            .halign(gtk4::Align::Center)
            .build();
        empty_label.add_css_class("placeholder");
        list_box.append(&empty_label);

        scroll.set_child(Some(&list_box));
        root.append(&scroll);

        let section = Self {
            root,
            list_box,
            empty_label,
            store: store.clone(),
        };

        // Subscribe to changes for live updates
        let list_box_c = section.list_box.clone();
        let empty_label_c = section.empty_label.clone();
        let store_change = store.clone();
        store.borrow_mut().connect_change(move || {
            rebuild_list(&list_box_c, &empty_label_c, &store_change);
        });

        section.rebuild();
        section
    }

    pub fn refresh(&self) {
        self.rebuild();
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    fn rebuild(&self) {
        rebuild_list(&self.list_box, &self.empty_label, &self.store);
    }
}

fn rebuild_list(
    list_box: &gtk4::Box,
    empty_label: &gtk4::Label,
    store: &Rc<RefCell<NotificationStore>>,
) {
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

    if notifications.is_empty() {
        empty_label.set_visible(true);
        return;
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
            let group_label = gtk4::Label::builder()
                .label(app_name.to_uppercase())
                .halign(gtk4::Align::Start)
                .build();
            group_label.add_css_class("notification-app-name");
            list_box.append(&group_label);
        }

        for notif in notifs {
            let entry = build_entry(notif, store);
            list_box.append(&entry);
        }
    }
}

fn build_entry(
    notif: &crate::notifications::Notification,
    store: &Rc<RefCell<NotificationStore>>,
) -> gtk4::Box {
    let row = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(8)
        .build();
    row.add_css_class("notification-entry");

    // Text content
    let vbox = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(1)
        .hexpand(true)
        .build();

    let header_row = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(8)
        .build();

    let summary = gtk4::Label::builder()
        .label(&notif.summary)
        .halign(gtk4::Align::Start)
        .hexpand(true)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .build();
    summary.add_css_class("notification-summary");

    let time_label = gtk4::Label::builder()
        .label(format_relative_time(notif.timestamp))
        .halign(gtk4::Align::End)
        .build();
    time_label.add_css_class("notification-time");

    header_row.append(&summary);
    header_row.append(&time_label);
    vbox.append(&header_row);

    if !notif.body.is_empty() {
        let markup = crate::notifications::markup::sanitize(&notif.body);
        let body = gtk4::Label::builder()
            .label(&markup)
            .use_markup(true)
            .halign(gtk4::Align::Start)
            .wrap(true)
            .wrap_mode(gtk4::pango::WrapMode::WordChar)
            .max_width_chars(50)
            .lines(3)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .build();
        body.add_css_class("notification-body");
        vbox.append(&body);
    }

    if let Some(progress) = notif.progress {
        let bar = gtk4::ProgressBar::builder()
            .fraction(progress as f64 / 100.0)
            .hexpand(true)
            .build();
        bar.add_css_class("notification-progress");
        vbox.append(&bar);
    }

    row.append(&vbox);

    // Dismiss button
    let dismiss_btn = gtk4::Button::builder()
        .label(icons::CLOSE)
        .valign(gtk4::Align::Center)
        .build();
    dismiss_btn.add_css_class("flat");
    dismiss_btn.add_css_class("notification-dismiss-btn");

    let id = notif.id;
    let store_c = store.clone();
    dismiss_btn.connect_clicked(move |_| {
        store::store_close(&store_c, id, CloseReason::Dismissed);
    });
    row.append(&dismiss_btn);

    row
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
