//! Chips, badges, keys and statuses: small labelled pills.

use gtk4::prelude::*;

use super::class::swap;

pub fn badge(text: &str) -> gtk4::Label {
    let l = gtk4::Label::new(Some(text));
    l.add_css_class("ui-badge");
    l
}

pub fn key(text: &str) -> gtk4::Label {
    let l = gtk4::Label::new(Some(text));
    l.add_css_class("ui-key");
    l
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Ok,
    Warn,
    Bad,
    Neutral,
}

impl Status {
    pub const ALL: [Status; 4] = [Status::Ok, Status::Warn, Status::Bad, Status::Neutral];

    pub(super) fn class(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Warn => "warn",
            Status::Bad => "bad",
            Status::Neutral => "neutral",
        }
    }
}

/// A status: a dot and a word, both in the status colour.
pub fn status(kind: Status, label: &str) -> gtk4::Label {
    let l = gtk4::Label::new(Some(&format!("\u{25cf} {label}")));
    l.add_css_class("ui-status");
    set_status(&l, kind);
    l
}

pub fn set_status(l: &gtk4::Label, kind: Status) {
    swap(l, Status::ALL.map(Status::class), Some(kind.class()));
}

/// A chip whose face is a widget rather than a word: an avatar and a name.
pub fn chip_with(child: &impl IsA<gtk4::Widget>) -> gtk4::Button {
    let b = gtk4::Button::new();
    b.add_css_class("ui-chip");
    b.add_css_class("rich");
    b.set_child(Some(child));
    b
}

/// A neutral count: "there is more of this", not an alert.
pub fn badge_neutral(text: &str) -> gtk4::Label {
    let l = badge(text);
    l.add_css_class("neutral");
    l
}

/// A chip that stays selected: one of a group of tabs or filters.
pub fn toggle_chip(label: &str) -> gtk4::ToggleButton {
    let b = gtk4::ToggleButton::with_label(label);
    b.add_css_class("ui-chip");
    b
}
