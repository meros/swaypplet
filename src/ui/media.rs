//! Pictures: the thumbnail frame, the picked thumbnail, the choice grid,
//! the swatch, the placeholder and the lifted picture.
//!
//! `ui::thumb()`, `ui::thumb::adopt(&picture)`, `ui::pick_thumb(child)`,
//! `ui::choice_grid::adopt(&flowbox)`, `ui::swatch(child)`,
//! `ui::placeholder::adopt(&w)`, `ui::lifted::adopt(&w)`.

use gtk4::prelude::*;
use gtk4::Align;

/// A rounded frame for an image, filled while it has none.
pub fn thumb() -> gtk4::Box {
    let b = gtk4::Box::builder()
        .halign(Align::Center)
        .valign(Align::Center)
        .overflow(gtk4::Overflow::Hidden)
        .build();
    thumb::adopt(&b);
    b
}

pub mod thumb {
    use gtk4::prelude::*;

    /// The thumbnail's frame on a picture the caller built.
    pub fn adopt(w: &impl IsA<gtk4::Widget>) {
        w.add_css_class("ui-thumb");
    }
}

/// A picture you choose; `set_selected` frames the chosen one.
pub fn pick_thumb(child: &impl IsA<gtk4::Widget>) -> gtk4::Button {
    let b = gtk4::Button::new();
    b.add_css_class("ui-pick-thumb");
    b.set_child(Some(child));
    b
}

pub mod choice_grid {
    use gtk4::prelude::*;

    /// A flow box whose children are picked: hover overlay, selected ring.
    pub fn adopt(g: &gtk4::FlowBox) {
        g.add_css_class("ui-choice-grid");
    }
}

/// A colour you pick, drawn by `child`; the button carries the ring.
pub fn swatch(child: &impl IsA<gtk4::Widget>) -> gtk4::ToggleButton {
    let b = gtk4::ToggleButton::new();
    b.add_css_class("ui-swatch");
    b.set_child(Some(child));
    b
}

pub mod placeholder {
    use gtk4::prelude::*;

    /// Where a picture will be before its first frame.
    pub fn adopt(w: &impl IsA<gtk4::Widget>) {
        w.add_css_class("ui-placeholder");
    }
}

pub mod lifted {
    use gtk4::prelude::*;

    /// A picture lifted off what is behind it by a shadow.
    pub fn adopt(w: &impl IsA<gtk4::Widget>) {
        w.add_css_class("ui-lifted");
    }
}
