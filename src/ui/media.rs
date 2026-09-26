//! Pictures: the thumbnail frame, the picked thumbnail, the choice grid,
//! the swatch, the ring, the placeholder and the lifted picture.

use gtk4::prelude::*;
use gtk4::{Align, Orientation};

/// A rounded frame for an image, filled while it has none.
pub fn thumb() -> gtk4::Box {
    let b = gtk4::Box::builder()
        .halign(Align::Center)
        .valign(Align::Center)
        .overflow(gtk4::Overflow::Hidden)
        .build();
    b.add_css_class("ui-thumb");
    b
}

/// A picture you choose; `set_selected` frames the chosen one.
pub fn pick_thumb(child: &impl IsA<gtk4::Widget>) -> gtk4::Button {
    let b = gtk4::Button::new();
    b.add_css_class("ui-pick-thumb");
    b.set_child(Some(child));
    b
}

/// A flow box whose children are picked: hover overlay, selected ring.
pub fn choice_grid(g: &gtk4::FlowBox) {
    g.add_css_class("ui-choice-grid");
}

/// A colour you pick, drawn by `child`; the button carries the ring.
pub fn swatch(child: &impl IsA<gtk4::Widget>) -> gtk4::ToggleButton {
    let b = gtk4::ToggleButton::new();
    b.add_css_class("ui-swatch");
    b.set_child(Some(child));
    b
}

/// An accent ring round something the shell frames but does not draw.
pub fn ring() -> gtk4::Box {
    let b = gtk4::Box::new(Orientation::Horizontal, 0);
    b.add_css_class("ui-ring");
    b
}

/// Where a picture will be before its first frame.
pub fn placeholder(w: &impl IsA<gtk4::Widget>) {
    w.add_css_class("ui-placeholder");
}

/// A picture lifted off what is behind it by a shadow.
pub fn lifted(w: &impl IsA<gtk4::Widget>) {
    w.add_css_class("ui-lifted");
}
