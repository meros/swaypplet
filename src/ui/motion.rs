//! Transitions and loops that go through `anim::duration`, so Look → Motion
//! and reduced motion reach every one (design lint `motion-bypass`).
//!
//! `ui::revealer(transition, motion)`, `ui::page_stack(transition,
//! motion)`; `ui::set_breathing` at runtime.

use gtk4::prelude::*;

use super::class::toggle;

/// A GTK transition length for a motion token, through `anim::ms`, so a
/// revealer or a stack follows Look → Motion and reduced motion like every
/// other animation. The only place Rust sets a `transition_duration`.
fn transition_ms(motion: crate::tokens::motion::Motion) -> u32 {
    crate::anim::ms(motion).round() as u32
}

/// A revealer that moves at `motion`, closed.
pub fn revealer(
    transition: gtk4::RevealerTransitionType,
    motion: crate::tokens::motion::Motion,
) -> gtk4::Revealer {
    gtk4::Revealer::builder()
        .transition_type(transition)
        .transition_duration(transition_ms(motion))
        .build()
}

/// A stack of pages that changes page at `motion`.
pub fn page_stack(
    transition: gtk4::StackTransitionType,
    motion: crate::tokens::motion::Motion,
) -> gtk4::Stack {
    gtk4::Stack::builder()
        .transition_type(transition)
        .transition_duration(transition_ms(motion))
        .build()
}

/// Shake `w` once: a "no" on the card that asked (a rejected password).
/// Every call replays it: the class comes off now and back on the next
/// main-loop turn, so the style recomputes between the two.
pub fn shake(w: &impl IsA<gtk4::Widget>) {
    w.remove_css_class("ui-shake");
    let w = w.clone().upcast::<gtk4::Widget>();
    gtk4::glib::idle_add_local_once(move || w.add_css_class("ui-shake"));
}

/// Stop a shake's class from lingering (a card being reset for reuse).
pub fn clear_shake(w: &impl IsA<gtk4::Widget>) {
    w.remove_css_class("ui-shake");
}

/// Breathe: the attention loop for something working in the background
/// (a notification's progress, the player's art).
pub fn set_breathing(w: &impl IsA<gtk4::Widget>, breathing: bool) {
    toggle(w, "ui-breathing", breathing);
}
