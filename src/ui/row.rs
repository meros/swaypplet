//! The list row, the pressable row, and the list that holds them.

use gtk4::Align;
use gtk4::prelude::*;

use super::class::toggle;
use super::*;

/// One list row: an icon, a title over a subtitle, and an end slot.
pub struct Row {
    pub root: gtk4::Box,
    pub icon: gtk4::Label,
    pub title: gtk4::Label,
    pub subtitle: gtk4::Label,
    pub end: gtk4::Box,
}

/// `icon` is a glyph from the icon font; empty for none.
pub fn row(icon: &str, title: &str, subtitle: &str) -> Row {
    let root = hbox(4);
    root.add_css_class("ui-row");
    let icon_l = gtk4::Label::new(Some(icon));
    icon_l.add_css_class("ui-row-icon");
    icon_l.set_visible(!icon.is_empty());
    let texts = vbox(0);
    texts.set_valign(Align::Center);
    texts.set_hexpand(true);
    let title_l = gtk4::Label::new(Some(title));
    title_l.add_css_class("ui-row-title");
    title_l.set_xalign(0.0);
    title_l.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    let sub = gtk4::Label::new(Some(subtitle));
    sub.add_css_class("ui-row-subtitle");
    sub.set_xalign(0.0);
    sub.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    sub.set_visible(!subtitle.is_empty());
    texts.append(&title_l);
    texts.append(&sub);
    let end = hbox(3);
    end.add_css_class("ui-row-end");
    end.set_valign(Align::Center);
    root.append(&icon_l);
    root.append(&texts);
    root.append(&end);
    Row {
        root,
        icon: icon_l,
        title: title_l,
        subtitle: sub,
        end,
    }
}

/// A row you can press: the row inside a flat button.
pub fn row_button(icon: &str, title: &str, subtitle: &str) -> (gtk4::Button, Row) {
    let r = row(icon, title, subtitle);
    r.root.remove_css_class("ui-row");
    let b = gtk4::Button::new();
    b.add_css_class("ui-row");
    b.add_css_class("activatable");
    b.set_child(Some(&r.root));
    (b, r)
}

pub fn set_selected(w: &impl IsA<gtk4::Widget>, selected: bool) {
    toggle(w, "selected", selected);
}

/// A ListBox that draws nothing itself, for rows of `ui-row` content that
/// light under the pointer or the keyboard.
pub fn list() -> gtk4::ListBox {
    let l = gtk4::ListBox::builder()
        .selection_mode(gtk4::SelectionMode::None)
        .build();
    l.add_css_class("ui-list");
    l
}

/// A row of a `ui::list`: `content` becomes the row's padded, lit face.
pub fn list_row(content: &impl IsA<gtk4::Widget>) -> gtk4::ListBoxRow {
    content.add_css_class("ui-row");
    gtk4::ListBoxRow::builder().child(content).build()
}

/// A row whose state changes in one frame (see `.ui-row.instant`).
pub fn instant(w: &impl IsA<gtk4::Widget>) {
    w.add_css_class("instant");
}
