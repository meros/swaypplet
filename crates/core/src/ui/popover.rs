//! A popover that draws nothing itself; its child is the card.

use gtk4::prelude::*;

/// A popover that draws nothing itself; its child is the card.
pub fn popover(child: &impl IsA<gtk4::Widget>, position: gtk4::PositionType) -> gtk4::Popover {
    gtk4::Popover::builder()
        .position(position)
        .has_arrow(false)
        .css_classes(["ui-popover"])
        .child(child)
        .build()
}
