//! The toggle tile: a glyph and a name on one button; accent means on.
//!
//! `ui::tile_toggle(icon, title)`; `ui::set_loading` at runtime.

use gtk4::prelude::*;

use super::class::toggle;
use super::{Text, Tone, glyph, hbox};

/// A toggle tile that is itself the button: a glyph and a name. `on` is
/// kept in step with the toggle's state.
pub fn tile_toggle(icon: &str, title: &str) -> gtk4::ToggleButton {
    let b = gtk4::ToggleButton::new();
    b.add_css_class("ui-tile");
    b.add_css_class("ui-tile-toggle");
    let line = hbox(3);
    let icon_l = gtk4::Label::new(Some(icon));
    glyph::adopt(&icon_l, Text::Title, Tone::Fg);
    let title_l = gtk4::Label::new(Some(title));
    title_l.add_css_class("ui-tile-title");
    title_l.set_xalign(0.0);
    title_l.set_hexpand(true);
    line.append(&icon_l);
    line.append(&title_l);
    b.set_child(Some(&line));
    b.connect_toggled(|b| toggle(b, "on", b.is_active()));
    b
}

/// Waiting on the tool the tile drives: the tile dims until it answers.
pub fn set_loading(w: &impl IsA<gtk4::Widget>, loading: bool) {
    toggle(w, "loading", loading);
}
