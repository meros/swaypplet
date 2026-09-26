//! The menu and its items.

use gtk4::prelude::*;

use super::*;

pub fn menu() -> gtk4::Box {
    let b = vbox(0);
    b.add_css_class("ui-menu");
    b
}

pub fn menu_item(label: &str, accel: &str, danger: bool) -> gtk4::Button {
    let b = gtk4::Button::new();
    b.add_css_class("ui-menu-item");
    if danger {
        b.add_css_class("danger");
    }
    let line = hbox(4);
    let l = gtk4::Label::new(Some(label));
    l.set_hexpand(true);
    l.set_xalign(0.0);
    line.append(&l);
    if !accel.is_empty() {
        let a = gtk4::Label::new(Some(accel));
        a.add_css_class("ui-menu-accel");
        line.append(&a);
    }
    b.set_child(Some(&line));
    b
}
