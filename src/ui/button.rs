//! Buttons: kinds, sizes and faces.

use gtk4::prelude::*;

use super::class::{swap, toggle};
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Primary,
    Secondary,
    Flat,
    Destructive,
}

impl Kind {
    pub const ALL: [Kind; 4] = [
        Kind::Primary,
        Kind::Secondary,
        Kind::Flat,
        Kind::Destructive,
    ];

    fn class(self) -> Option<&'static str> {
        match self {
            Kind::Primary => Some("primary"),
            Kind::Flat => Some("flat"),
            Kind::Destructive => Some("destructive"),
            Kind::Secondary => None,
        }
    }
}

fn style_button(b: &gtk4::Button, kind: Kind) {
    b.add_css_class("ui-btn");
    if let Some(c) = kind.class() {
        b.add_css_class(c);
    }
}

pub fn button(label: &str, kind: Kind) -> gtk4::Button {
    let b = gtk4::Button::with_label(label);
    style_button(&b, kind);
    b
}

/// Restyle an existing button as a component button.
pub fn make_button(b: &gtk4::Button, kind: Kind) {
    style_button(b, kind);
}

/// A button whose face is a glyph from the icon font.
pub fn glyph_button(glyph: &str, tooltip: &str, kind: Kind) -> gtk4::Button {
    let b = gtk4::Button::with_label(glyph);
    style_button(&b, kind);
    b.add_css_class("icon");
    b.set_tooltip_text(Some(tooltip));
    b
}

/// A button in a dense place (a popover's footer): label size, low.
pub fn small_button(label: &str, kind: Kind) -> gtk4::Button {
    let b = button(label, kind);
    b.add_css_class("small");
    b
}

/// Turn a slider row's icon into a flat button (mute, say). The glyph
/// stays the row's icon label, so callers keep setting it there.
pub fn slider_icon_button(r: &SliderRow, tooltip: &str) -> gtk4::Button {
    r.root.remove(&r.icon);
    let b = gtk4::Button::new();
    style_button(&b, Kind::Flat);
    b.add_css_class("icon");
    b.set_tooltip_text(Some(tooltip));
    b.set_child(Some(&r.icon));
    r.root.prepend(&b);
    b
}

/// Restyle a button as another kind in place ("Forget" turning into its
/// destructive confirmation).
pub fn set_button_kind(b: &gtk4::Button, kind: Kind) {
    swap(b, Kind::ALL.iter().filter_map(|k| k.class()), kind.class());
    b.add_css_class("ui-btn");
}

/// Make an existing button a dense one, as `small_button` builds them.
pub fn make_small(b: &gtk4::Button) {
    b.add_css_class("small");
}

/// A toggle whose face is a glyph from the icon font, at title-sm size.
pub fn toggle_glyph_button(glyph: &str, tooltip: &str, kind: Kind) -> gtk4::ToggleButton {
    let face = gtk4::Label::new(Some(glyph));
    self::glyph(&face, Text::TitleSm, Tone::Fg);
    let b = gtk4::ToggleButton::new();
    b.set_child(Some(&face));
    style_button(b.upcast_ref(), kind);
    b.add_css_class("icon");
    b.set_tooltip_text(Some(tooltip));
    b
}

/// Arm a destructive button for its confirming second press, or disarm it.
pub fn set_armed(b: &impl IsA<gtk4::Widget>, armed: bool) {
    toggle(b, "armed", armed);
}
