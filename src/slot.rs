//! Reserved space in the authentication cards.
//!
//! The lock screen, the greeter and the polkit dialog all show the same kind
//! of card: a password box surrounded by facts that arrive later than the
//! card does. Whether a fingerprint reader armed, whether PAM wants a
//! password, whether Caps Lock is on, what went wrong on the last attempt —
//! none of it is known when the surface is built, and every one of them used
//! to be shown by making a widget visible.
//!
//! A widget appearing is a layout change, and a layout change is the card
//! resizing: it grows downward, re-centres, and everything the eye had fixed
//! on moves. On the lock screen that lands mid-entrance, so the card pops
//! while it is still fading in; on the polkit dialog it lands under a pointer
//! that is on its way to a button. Both are the same bug, and hiding the
//! widget again on the way out is the same bug twice.
//!
//! The rule this module exists to enforce: **a card's geometry is decided
//! when it is built and never changes again.** Anything that can appear later
//! is laid out from the first frame and merely painted or not — [`reserve`]
//! puts a widget in that state, [`show`] fades it in and out without touching
//! the allocation. What genuinely varies between one card and the next (a
//! greeter's username row, a polkit identity picker, whether there is a
//! fingerprint reader at all) is decided *before* the surface is presented,
//! where a size change costs nothing because nobody has seen the card yet.
//!
//! Cost of the rule: a card carries the space for things it is not currently
//! saying. That is the trade being made deliberately — blank space is quiet,
//! and a card that moves is not.

use gtk4::prelude::*;

/// CSS class marking a widget whose space is permanently reserved.
const SLOT: &str = "slot";
/// CSS class on a reserved widget that is currently painting nothing.
const EMPTY: &str = "slot-empty";

/// Hold `w`'s space open for the lifetime of the card, starting blank.
///
/// The widget stays visible to the layout for good; `show` decides whether it
/// is painted. Call this once, at build time, on anything that would
/// otherwise be revealed later.
pub fn reserve(w: &impl IsA<gtk4::Widget>) {
    let w = w.as_ref();
    w.set_visible(true);
    w.add_css_class(SLOT);
    set_empty(w, true);
}

/// Reserve `w`'s space, or take it away entirely, for the card about to be
/// presented.
///
/// The only legal way to change a card's geometry, and only before it is on
/// screen: a polkit dialog for a request with no fingerprint reader behind it
/// should not carry a fingerprint-sized hole, and it knows which it is before
/// the user sees anything.
pub fn reserve_if(w: &impl IsA<gtk4::Widget>, reserved: bool) {
    let w = w.as_ref();
    w.set_visible(reserved);
    w.add_css_class(SLOT);
    set_empty(w, true);
}

/// Paint a reserved widget, or stop painting it. Never changes its size.
pub fn show(w: &impl IsA<gtk4::Widget>, shown: bool) {
    set_empty(w.as_ref(), !shown);
}

fn set_empty(w: &gtk4::Widget, empty: bool) {
    if w.has_css_class(EMPTY) == empty {
        return;
    }
    if empty {
        w.add_css_class(EMPTY);
    } else {
        w.remove_css_class(EMPTY);
    }
    // A pill nobody can see must not be clickable or tabbable either — the
    // reservation is for the layout, not for the input stack.
    w.set_can_target(!empty);
    w.set_can_focus(!empty);
}
