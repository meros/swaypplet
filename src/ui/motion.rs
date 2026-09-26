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

/// Breathe: the attention loop for something working in the background
/// (a notification's progress, the player's art).
pub fn set_breathing(w: &impl IsA<gtk4::Widget>, breathing: bool) {
    toggle(w, "ui-breathing", breathing);
}
