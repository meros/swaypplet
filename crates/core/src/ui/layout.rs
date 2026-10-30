//! Boxes on the space scale, padding, separators, and the fills that
//! group controls in a row (pill group, toolbar).

use gtk4::Orientation;
use gtk4::prelude::*;

use crate::tokens::space;

/// A box whose spacing is a step of the space scale (1-based, §3.5).
pub fn stack(orientation: Orientation, step: usize) -> gtk4::Box {
    gtk4::Box::new(orientation, space(step))
}

pub fn vbox(step: usize) -> gtk4::Box {
    stack(Orientation::Vertical, step)
}

pub fn hbox(step: usize) -> gtk4::Box {
    stack(Orientation::Horizontal, step)
}

/// Padding on all four sides from the space scale, as margins.
pub fn pad(w: &impl IsA<gtk4::Widget>, step: usize) {
    let p = space(step);
    w.set_margin_top(p);
    w.set_margin_bottom(p);
    w.set_margin_start(p);
    w.set_margin_end(p);
}

/// A hairline: `Horizontal` between rows, `Vertical` between runs of
/// controls in a toolbar.
pub fn separator(line: Orientation) -> gtk4::Box {
    let s = gtk4::Box::new(line, 0);
    s.add_css_class("ui-separator");
    if line == Orientation::Vertical {
        s.add_css_class("vertical");
    }
    s
}

/// Controls that belong together on one pill-shaped fill.
pub fn pill_group(step: usize) -> gtk4::Box {
    let b = hbox(step);
    b.add_css_class("ui-pill-group");
    b
}

/// A fill above the window's ground, for a toolbar.
pub fn toolbar(step: usize) -> gtk4::Box {
    let b = hbox(step);
    b.add_css_class("ui-toolbar");
    b
}
