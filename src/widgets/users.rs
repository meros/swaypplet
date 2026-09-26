//! Session-aware user list for the start-menu's quick-settings column.
//!
//! Replaces the old single "Switch user" rail button: on every panel open we
//! ask [`switch_user::list`] who is configured, then render one row per user
//! (avatar + name + fingerprint badge). Clicking another user locks this
//! session and jumps to theirs; the current user is marked and inert.
//!
//! Degradation: host not configured for switching → the section stays hidden;
//! the query fails → a single "Switch user" row falls back to the legacy cycle.

use gtk4::prelude::*;

use crate::avatar::avatar;
use crate::icons;
use crate::spawn::spawn_work;
use crate::switch_user::{self, SwitchUser};
use crate::ui;
use crate::widgets::power::hide_panel_for_widget;

/// Avatar diameter for panel rows.
const AVATAR_SIZE: i32 = 30;

pub struct UserSection {
    root: gtk4::Box,
    list: gtk4::Box,
}

impl UserSection {
    pub fn new() -> Self {
        let root = ui::group(2);
        root.add_css_class("section-group");
        root.append(&ui::heading("Users"));

        let list = ui::vbox(1);
        root.append(&list);

        // Host not configured for switching → no section at all (matches
        // pre-switcher behaviour).
        root.set_visible(switch_user::available());

        Self { root, list }
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    /// Re-query the configured users on a background thread and rebuild the
    /// rows. Called on panel open (via `Sections::refresh`).
    pub fn refresh(&self) {
        if !switch_user::available() {
            self.root.set_visible(false);
            return;
        }
        let list = self.list.clone();
        let root = self.root.clone();
        spawn_work(switch_user::list, move |users| {
            root.set_visible(true);
            match users {
                Some(users) if !users.is_empty() => rebuild_rows(&list, &users),
                // The query failed: fall back to the legacy no-arg cycle.
                _ => rebuild_fallback(&list),
            }
        });
    }
}

fn clear(list: &gtk4::Box) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
}

fn rebuild_rows(list: &gtk4::Box, users: &[SwitchUser]) {
    clear(list);
    for u in users {
        list.append(&user_row(u));
    }
}

fn rebuild_fallback(list: &gtk4::Box) {
    clear(list);
    let (btn, _) = ui::row_button(icons::SWITCH_USER, "Switch user", "");
    btn.connect_clicked(|b| {
        hide_panel_for_widget(b.upcast_ref());
        switch_user::cycle();
    });
    list.append(&btn);
}

fn user_row(u: &SwitchUser) -> gtk4::Button {
    let (btn, r) = ui::row_button("", &u.user, "");

    let av = avatar(&u.user, u.icon.as_deref(), AVATAR_SIZE, u.logged_in);
    if u.current {
        av.add_css_class("active");
    }
    r.root.prepend(&av);

    if u.fingerprint == Some(true) {
        let fp = gtk4::Label::new(Some(icons::FINGERPRINT));
        fp.set_tooltip_text(Some("Fingerprint enrolled"));
        r.end.append(&fp);
    }

    if u.current {
        // The invoking user: marked, non-actionable. A click just closes the
        // panel rather than pointlessly re-switching to self.
        ui::set_selected(&btn, true);
        btn.connect_clicked(|b| hide_panel_for_widget(b.upcast_ref()));
    } else {
        let user = u.user.clone();
        btn.connect_clicked(move |b| {
            hide_panel_for_widget(b.upcast_ref());
            switch_user::switch_to(&user);
        });
    }

    btn
}
