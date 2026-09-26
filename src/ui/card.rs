//! The glass card and what sits in it: the group fill, the well, the solid
//! card of a popup, the card over a scrim, and the card tints (§4).

use gtk4::prelude::*;

use super::class::toggle;
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Card {
    /// A floating card: panel, launcher, notifications, polkit, lock.
    Floating,
    /// Thin glass: the bar, the OSD, the face cue.
    Thin,
}

/// Make `w` a glass card.
pub fn card(w: &impl IsA<gtk4::Widget>, kind: Card) {
    w.add_css_class("ui-card");
    if kind == Card::Thin {
        w.add_css_class("thin");
    }
}

/// A fill inside a card, for grouping (never glass on glass).
pub fn group(step: usize) -> gtk4::Box {
    let b = vbox(step);
    b.add_css_class("ui-group");
    b
}

/// A card with no glass behind it (a popup, outside the compositor's
/// layer effects): the same shape, a solid raised fill.
pub fn solid_card(w: &impl IsA<gtk4::Widget>) {
    card(w, Card::Floating);
    w.add_css_class("solid");
}

/// Make `w` the glass card that sits over a [`scrim`]: the lock's and the
/// greeter's. It paints the key pre-compensated for the black under it, so
/// the two layers composite to exactly the key every other card paints and
/// the compositor drops them (`.ui-card.over-scrim` in 00-components.css has
/// the arithmetic). Only ever over a scrim: on its own it lands in the band
/// `glass.nix` reserves for nothing, a flat slab with no bevel.
pub fn card_over_scrim(w: &impl IsA<gtk4::Widget>) {
    card(w, Card::Floating);
    w.add_css_class("over-scrim");
}

/// A well: a box sunk below the card, for text that came from outside it
/// (a command line, polkit's raw details).
pub fn well() -> gtk4::Box {
    let b = vbox(0);
    b.add_css_class("ui-well");
    b
}

/// A deliberate tint over a glass card's key (§4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardTint {
    /// Critical: the glass runs warm.
    Danger,
    /// Further back in a stack: dimmer.
    Recessed,
}

/// Set or clear one tint on a card, leaving the other as it is.
pub fn set_card_tint(w: &impl IsA<gtk4::Widget>, tint: CardTint, on: bool) {
    let class = match tint {
        CardTint::Danger => "danger",
        CardTint::Recessed => "recessed",
    };
    toggle(w, class, on);
}
