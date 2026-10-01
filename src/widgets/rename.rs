//! Rename, for any device row: a small flat button that turns the row's
//! title into an entry in the title's own place. Enter commits the text
//! (empty means "go back to the automatic name"), Escape leaves the name as
//! it was. What committing does is the caller's: a stored name
//! (`services::devices::rename`) or BlueZ's alias.

use std::rc::Rc;

use gtk4::prelude::*;

use crate::ui;

/// A Rename button for the row whose title label is `title`. `current` is
/// the name shown now (the entry starts with it), `auto` the automatic name
/// (the entry's placeholder, which is what an empty name restores). The
/// caller places the button, usually first in the row's end slot.
pub fn button(
    title: &gtk4::Label,
    current: &str,
    auto: &str,
    on_commit: impl Fn(String) + 'static,
) -> gtk4::Button {
    let b = ui::button_with(ui::Face::Label("Rename"), ui::Kind::Flat, ui::Size::Small);
    b.set_tooltip_text(Some(
        "Give this a name; leave it empty for the automatic one",
    ));
    let entry = gtk4::Entry::new();
    entry.set_placeholder_text(Some(auto));
    entry.set_hexpand(true);
    entry.set_visible(false);
    ui::entry::adopt(&entry, ui::FieldSize::Normal);
    if let Some(parent) = title.parent().and_downcast::<gtk4::Box>() {
        parent.insert_child_after(&entry, Some(title));
    }
    let editing = |on: bool, title: &gtk4::Label, entry: &gtk4::Entry| {
        title.set_visible(!on);
        entry.set_visible(on);
    };
    {
        let (title, entry, current) = (title.clone(), entry.clone(), current.to_string());
        b.connect_clicked(move |_| {
            entry.set_text(&current);
            editing(true, &title, &entry);
            entry.grab_focus();
        });
    }
    let on_commit = Rc::new(on_commit);
    {
        let title = title.clone();
        entry.connect_activate(move |e| {
            editing(false, &title, e);
            on_commit(e.text().trim().to_string());
        });
    }
    let keys = gtk4::EventControllerKey::new();
    {
        let (title, entry_c) = (title.clone(), entry.clone());
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gtk4::gdk::Key::Escape {
                editing(false, &title, &entry_c);
                return gtk4::glib::Propagation::Stop;
            }
            gtk4::glib::Propagation::Proceed
        });
    }
    entry.add_controller(keys);
    b
}
