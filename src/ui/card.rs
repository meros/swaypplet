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
}

/// Make `w` a card of `kind`. A glass card also tells the compositor its
/// exact outline while it is mapped (`crate::effect_shape`).
pub fn adopt(w: &impl IsA<gtk4::Widget>, kind: Card) {
    let first = !w.has_css_class("ui-card");
    w.add_css_class("ui-card");
    if first && kind != Card::Solid {
        crate::effect_shape::follow(w.upcast_ref());
    }
    match kind {
        Card::Floating => {}
        Card::Thin => w.add_css_class("thin"),
        Card::Solid => w.add_css_class("solid"),
    }
}

/// The corner radius of a glass card, or `None` when `w` is not one.
///
/// From the radius tokens, never read back from the style: GTK has no public
/// API for a computed `border-radius`. The card's stylesheet must round the
/// same corners by the same token, which the test below holds it to.
pub fn glass_radius(w: &impl IsA<gtk4::Widget>) -> Option<f64> {
    if !w.has_css_class("ui-card") || w.has_css_class("solid") {
        return None;
    }
    let name = if w.has_css_class("thin") {
        "thin"
    } else {
        "card"
    };
    radius(name)
}

fn radius(name: &str) -> Option<f64> {
    crate::tokens::RADIUS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|&(_, px)| f64::from(px))
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `border-radius` in a rule that styles a glass card, across all
    /// the stylesheets, as (selector, value).
    fn card_radii() -> Vec<(String, String)> {
        let mut out = Vec::new();
        for (_, css) in crate::theme::RULES {
            let mut rest = *css;
            while let Some(open) = rest.find('{') {
                let Some(close) = rest[open..].find('}') else {
                    break;
                };
                let selector = rest[..open].rsplit(['}', '/']).next().unwrap_or("").trim();
                let body = &rest[open + 1..open + close];
                rest = &rest[open + close + 1..];
                let last = selector.split(',').map(str::trim);
                for sel in last {
                    let compound = sel.split_whitespace().last().unwrap_or("");
                    if !compound.contains(".ui-card") && !compound.contains(".ui-face-pill") {
                        continue;
                    }
                    for decl in body.split(';') {
                        if let Some((prop, value)) = decl.split_once(':')
                            && prop.trim().ends_with("radius")
                        {
                            out.push((sel.to_string(), value.trim().to_string()));
                        }
                    }
                }
            }
        }
        out
    }

    #[test]
    fn the_glass_radius_is_the_one_the_stylesheet_draws() {
        // `crate::effect_shape` sends a uniform radius from the tokens; a
        // stylesheet that rounded a card differently would put the glass and
        // the pixels at odds.
        let radii = card_radii();
        assert_eq!(
            radii,
            vec![
                (".ui-card".to_string(), "var(--radius-card)".to_string()),
                (
                    ".ui-card.thin".to_string(),
                    "var(--radius-thin)".to_string()
                ),
            ],
            "a glass card's radius changed in CSS: teach ui::card::glass_radius the same"
        );
        assert_eq!(radius("card"), Some(18.0));
        assert_eq!(radius("thin"), Some(14.0));
    }
}
