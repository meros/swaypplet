//! The glass card and what sits in it: the group fill, the well, and the
//! card's tints and states (§4).
//!
//! `ui::card::adopt(&w, Card)` on a box the surface built; `ui::group(n)`,
//! `ui::well()`; `ui::set_card_tint`, `ui::set_success`.

use gtk4::prelude::*;

use super::class::toggle;
use super::vbox;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Card {
    /// A floating card: panel, launcher, notifications, polkit.
    Floating,
    /// Thin glass: the bar, the OSD, the face cue.
    Thin,
    /// No glass behind it (a popup, outside the compositor's layer
    /// effects): the floating shape, a solid raised fill.
    Solid,
    /// The glass card that sits over a [`scrim`](super::scrim): the lock's
    /// and the greeter's. It paints the key pre-compensated for the black
    /// under it, so the two layers composite to exactly the key every other
    /// card paints and the compositor drops them (`.ui-card.over-scrim` in
    /// the card's stylesheet has the arithmetic). Only ever over a scrim: on
    /// its own it lands in the band `glass.nix` reserves for nothing, a flat
    /// slab with no bevel.
    OverScrim,
}

/// Make `w` a card of `kind`.
pub fn adopt(w: &impl IsA<gtk4::Widget>, kind: Card) {
    w.add_css_class("ui-card");
    match kind {
        Card::Floating => {}
        Card::Thin => w.add_css_class("thin"),
        Card::Solid => w.add_css_class("solid"),
        Card::OverScrim => w.add_css_class("over-scrim"),
    }
}

/// A fill inside a card, for grouping (never glass on glass).
pub fn group(step: usize) -> gtk4::Box {
    let b = vbox(step);
    b.add_css_class("ui-group");
    b
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

/// A card that has just said yes (an unlock, an authorisation): its border
/// takes the accent.
pub fn set_success(w: &impl IsA<gtk4::Widget>, success: bool) {
    toggle(w, "success", success);
}
