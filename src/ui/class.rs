//! The two ways a component changes its classes, so no builder hand-writes
//! a remove-all-add-one loop: [`toggle`] one modifier, or [`swap`] one of a
//! family (a tone, a size, a state) for another.

use gtk4::prelude::*;

/// Add `class` when `on`, remove it otherwise.
pub fn toggle(w: &impl IsA<gtk4::Widget>, class: &str, on: bool) {
    if on {
        w.add_css_class(class);
    } else {
        w.remove_css_class(class);
    }
}

/// Clear every class of a family and set `one` of them (or none). GTK
/// cannot be asked which of a family is set, so the family is named whole.
pub fn swap<'a>(
    w: &impl IsA<gtk4::Widget>,
    family: impl IntoIterator<Item = &'a str>,
    one: Option<&str>,
) {
    for c in family {
        if Some(c) != one {
            w.remove_css_class(c);
        }
    }
    if let Some(c) = one {
        w.add_css_class(c);
    }
}

/// Toggle a component modifier class.
pub fn set_class(w: &impl IsA<gtk4::Widget>, class: &str, on: bool) {
    toggle(w, class, on);
}
