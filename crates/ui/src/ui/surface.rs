//! A window's root: the design-system surface, a solid window, the canvas
//! an editor letterboxes against.
//!
//! `ui::surface::adopt(&root)`, `ui::window::adopt(&root)`,
//! `ui::canvas::adopt(&area)`.

use gtk4::prelude::*;

/// Mark a window's root child as a design-system surface: base type and
/// colour. Never the window itself: GTK's `window.background` outranks
/// `.ui-surface` there, so the class would do nothing (design lint
/// `surface-on-window`).
pub fn adopt(w: &impl IsA<gtk4::Widget>) {
    debug_assert!(
        !w.is::<gtk4::Window>(),
        "ui::surface goes on the window's root child, not the window"
    );
    w.add_css_class("ui-surface");
}

pub mod window {
    use gtk4::prelude::*;

    /// A normal, solid window rather than glass: an editor you sit in. Goes
    /// on the window's root child, which it also makes the surface; the
    /// window node itself stays transparent.
    pub fn adopt(w: &impl IsA<gtk4::Widget>) {
        super::adopt(w);
        w.add_css_class("ui-window");
    }
}

pub mod canvas {
    use gtk4::prelude::*;

    /// A shade below the window's ground, for content to letterbox against.
    pub fn adopt(w: &impl IsA<gtk4::Widget>) {
        w.add_css_class("ui-canvas");
    }
}
