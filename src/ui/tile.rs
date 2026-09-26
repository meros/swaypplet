//! The toggle tile: a glyph and a name on one button; accent means on.
//!
//! `ui::tile_toggle(icon, title)`; `ui::set_loading` at runtime.
//!
//! `ui::tile_split(icon, title)` is the tile with a detail: the body
//! toggles, the chevron opens what is behind it, and a status line under
//! the name says why the tile is in its state ("Until 07:00", "3500 K").
//! One convention for every tile, from Android's and GNOME's quick
//! settings (docs/prior-art/mobile/android-quick-settings.md): a click on
//! the body never navigates, and the chevron never toggles.

use gtk4::prelude::*;

use super::class::toggle;
use super::{Text, Tone, glyph, hbox, vbox};

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

/// A split tile's parts.
pub struct SplitTile {
    /// The whole tile, for placing it.
    pub root: gtk4::Box,
    /// The body: the toggle.
    pub toggle: gtk4::ToggleButton,
    /// The chevron: opens the detail.
    pub detail: gtk4::Button,
    /// The line under the name; empty hides it.
    pub status: gtk4::Label,
}

/// A tile whose body toggles and whose chevron opens a detail.
pub fn tile_split(icon: &str, title: &str) -> SplitTile {
    let root = hbox(0);
    root.add_css_class("ui-tile");
    root.add_css_class("ui-tile-split");
    root.set_overflow(gtk4::Overflow::Hidden);

    let toggle = gtk4::ToggleButton::new();
    toggle.add_css_class("ui-tile-toggle");
    toggle.add_css_class("ui-tile-body");
    toggle.set_hexpand(true);
    let line = hbox(3);
    let icon_l = gtk4::Label::new(Some(icon));
    glyph::adopt(&icon_l, Text::Title, Tone::Fg);
    let texts = vbox(0);
    texts.set_valign(gtk4::Align::Center);
    let title_l = gtk4::Label::new(Some(title));
    title_l.add_css_class("ui-tile-title");
    title_l.set_xalign(0.0);
    let status = gtk4::Label::new(None);
    status.add_css_class("ui-tile-status");
    status.set_xalign(0.0);
    status.set_visible(false);
    status.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    texts.append(&title_l);
    texts.append(&status);
    line.append(&icon_l);
    line.append(&texts);
    toggle.set_child(Some(&line));

    let detail = gtk4::Button::new();
    detail.add_css_class("ui-tile-toggle");
    detail.add_css_class("ui-tile-detail");
    detail.set_child(Some(&gtk4::Image::from_icon_name("pan-end-symbolic")));
    detail.set_tooltip_text(Some(&format!("{title} settings")));

    root.append(&toggle);
    root.append(&detail);
    let r = root.clone();
    toggle.connect_toggled(move |b| toggle_class(&r, b.is_active()));
    SplitTile {
        root,
        toggle,
        detail,
        status,
    }
}

fn toggle_class(root: &gtk4::Box, on: bool) {
    super::class::toggle(root, "on", on);
}

/// The split tile's status line; empty hides it.
pub fn set_tile_status(tile: &SplitTile, text: &str) {
    tile.status.set_label(text);
    tile.status.set_visible(!text.is_empty());
}
