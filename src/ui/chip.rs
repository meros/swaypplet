//! Chips, badges, keys and statuses: small labelled pills.
//!
//! `ui::chip(face)`, `ui::toggle_chip(label)`, `ui::badge(text, tone)`,
//! `ui::key(text)`, `ui::status(status, label)`; `ui::set_status`,
//! `ui::set_handoff` at runtime.

use gtk4::prelude::*;

use super::Face;
use super::class::swap;

/// A chip you press. Words, or a widget (an avatar and a name), which gets
/// the room a richer face needs.
pub fn chip(face: Face) -> gtk4::Button {
    let b = gtk4::Button::new();
    b.add_css_class("ui-chip");
    match face {
        Face::Label(l) => b.set_label(l),
        Face::Glyph { glyph, tooltip } => {
            b.set_label(glyph);
            b.set_tooltip_text(Some(tooltip));
        }
        Face::Icon { child, tooltip } => {
            b.set_child(Some(child));
            b.set_tooltip_text(Some(tooltip));
            b.add_css_class("rich");
        }
        Face::Child(child) => {
            b.set_child(Some(child));
            b.add_css_class("rich");
        }
    }
    b
}

/// A chip that stays selected: one of a group of tabs or filters.
pub fn toggle_chip(label: &str) -> gtk4::ToggleButton {
    let b = gtk4::ToggleButton::with_label(label);
    b.add_css_class("ui-chip");
    b
}

/// Where a chip stands in the lock's handoff to another session: the one
/// picked blooms, the rest step back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handoff {
    Picked,
    Dropped,
}

impl Handoff {
    pub const ALL: [Handoff; 2] = [Handoff::Picked, Handoff::Dropped];

    fn class(self) -> &'static str {
        match self {
            Handoff::Picked => "picked",
            Handoff::Dropped => "dropped",
        }
    }
}

/// Put a chip in a handoff state, or back at rest.
pub fn set_handoff(chip: &impl IsA<gtk4::Widget>, handoff: Option<Handoff>) {
    swap(
        chip,
        Handoff::ALL.map(Handoff::class),
        handoff.map(Handoff::class),
    );
}

/// What a badge's count says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BadgeTone {
    /// Something wants you: the danger fill.
    Alert,
    /// "There is more of this", not an alert: a quiet fill.
    Neutral,
}

pub fn badge(text: &str, tone: BadgeTone) -> gtk4::Label {
    let l = gtk4::Label::new(Some(text));
    l.add_css_class("ui-badge");
    if tone == BadgeTone::Neutral {
        l.add_css_class("neutral");
    }
    l
}

pub fn key(text: &str) -> gtk4::Label {
    let l = gtk4::Label::new(Some(text));
    l.add_css_class("ui-key");
    l
}

/// A verdict, in the status colours (§3.2). The same words as the text
/// tones (`Tone::Success` …), plus the neutral "nothing to say".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Success,
    Warning,
    Danger,
    Neutral,
}

impl Status {
    pub const ALL: [Status; 4] = [
        Status::Success,
        Status::Warning,
        Status::Danger,
        Status::Neutral,
    ];

    pub(super) fn class(self) -> &'static str {
        match self {
            Status::Success => "success",
            Status::Warning => "warning",
            Status::Danger => "danger",
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
