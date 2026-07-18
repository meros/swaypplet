//! Session-aware user list for the start-menu's quick-settings column.
//!
//! Replaces the old single "Switch user" rail button: on every panel open we
//! ask the host `SWAYPPLET_SWITCH_USER_CMD --list` who is configured, then
//! render one row per user (avatar + name + fingerprint badge). Clicking
//! another user locks this session and jumps to theirs; the current user is
//! marked and inert.
//!
//! Degradation: no switch command → the section stays hidden (as before);
//! `--list` fails → a single "Switch user" row falls back to the legacy cycle.

use std::rc::Rc;

use gtk4::prelude::*;

use crate::avatar::avatar;
use crate::spawn::spawn_work;
use crate::switch_user::{self, SwitchUser};
use crate::widgets::power::hide_panel_for_widget;

/// Fingerprint-enrolled glyph, dim, shown on the right of an enrolled user's
/// row (matches the lock/greeter fingerprint iconography, U+F0237).
const FP_GLYPH: &str = "\u{f0237}";
/// Avatar diameter for panel rows.
const AVATAR_SIZE: i32 = 30;

pub struct UserSection {
    root: gtk4::Box,
    list: gtk4::Box,
    cmd: Option<String>,
}

impl UserSection {
    pub fn new() -> Self {
        let root = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(6)
            .build();
        root.add_css_class("section");
        root.add_css_class("user-section");

        let title = gtk4::Label::builder()
            .label("Users")
            .halign(gtk4::Align::Start)
            .build();
        title.add_css_class("section-title");
        root.append(&title);

        let list = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(2)
            .build();
        root.append(&list);

        let cmd = switch_user::cmd();
        // No host switcher → no section at all (matches pre-switcher behaviour).
        root.set_visible(cmd.is_some());

        Self { root, list, cmd }
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    /// Re-query the configured users on a background thread and rebuild the
    /// rows. Called on panel open (via `Sections::refresh`).
    pub fn refresh(&self) {
        let Some(cmd) = self.cmd.clone() else {
            self.root.set_visible(false);
            return;
        };
        let list = self.list.clone();
        let root = self.root.clone();
        spawn_work(switch_user::list, move |users| {
            root.set_visible(true);
            match users {
                Some(users) if !users.is_empty() => rebuild_rows(&list, &users, &cmd),
                // `--list` failed: fall back to the legacy no-arg cycle.
                _ => rebuild_fallback(&list, &cmd),
            }
        });
    }
}

fn clear(list: &gtk4::Box) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
}

fn rebuild_rows(list: &gtk4::Box, users: &[SwitchUser], cmd: &str) {
    clear(list);
    for u in users {
        list.append(&user_row(u, cmd));
    }
}

fn rebuild_fallback(list: &gtk4::Box, cmd: &str) {
    clear(list);
    let btn = gtk4::Button::with_label("󰓤  Switch user");
    btn.add_css_class("user-row");
    btn.add_css_class("user-row-fallback");
    let cmd = cmd.to_owned();
    btn.connect_clicked(move |b| {
        hide_panel_for_widget(b.upcast_ref());
        switch_user::cycle(&cmd);
    });
    list.append(&btn);
}

fn user_row(u: &SwitchUser, cmd: &str) -> gtk4::Button {
    let content = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(10)
        .build();

    let av = avatar(&u.user, u.icon.as_deref(), AVATAR_SIZE, u.logged_in);
    if u.current {
        av.add_css_class("active");
    }
    content.append(&av);

    let name = gtk4::Label::builder()
        .label(&u.user)
        .halign(gtk4::Align::Start)
        .hexpand(true)
        .xalign(0.0)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .build();
    name.add_css_class("user-name");
    content.append(&name);

    if u.fingerprint {
        let fp = gtk4::Label::new(Some(FP_GLYPH));
        fp.add_css_class("user-fp");
        fp.set_tooltip_text(Some("Fingerprint enrolled"));
        content.append(&fp);
    }

    let btn = gtk4::Button::builder().child(&content).build();
    btn.add_css_class("user-row");

    if u.current {
        // The invoking user: marked, non-actionable. A click just closes the
        // panel rather than pointlessly re-switching to self.
        btn.add_css_class("active");
        btn.connect_clicked(|b| hide_panel_for_widget(b.upcast_ref()));
    } else {
        let cmd = cmd.to_owned();
        let user = u.user.clone();
        btn.connect_clicked(move |b| {
            hide_panel_for_widget(b.upcast_ref());
            switch_user::switch_to(&cmd, &user);
        });
    }

    btn
}
