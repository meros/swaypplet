//! The components of the design system (docs/design-system.md §6).
//!
//! One module per component, each handing back plain GTK widgets with the
//! `ui-*` classes `data/css/00-components.css` styles. The modules are
//! private; everything they export is re-exported here, so a surface writes
//! `ui::button(..)`, never `ui::button::..`. A surface is
//! assembled from these; its own classes may place things but not colour,
//! size type or round corners.
//!
//! Spacing between children comes from `tokens::space`, never a literal.

mod avatar;
mod bar;
mod button;
mod card;
mod chip;
mod class;
mod expander;
mod face;
mod field;
pub mod icons;
mod layout;
mod media;
mod menu;
mod motion;
mod popover;
mod progress;
mod row;
mod slider;
mod surface;
mod text;
mod tile;

pub use avatar::avatar;
pub use bar::*;
pub use button::*;
pub use card::*;
pub use chip::*;
pub use class::set_class;
pub use expander::*;
pub use face::*;
pub use field::*;
pub use layout::*;
pub use media::*;
pub use menu::*;
pub use motion::*;
pub use popover::*;
pub use progress::*;
pub use row::*;
pub use slider::*;
pub use surface::*;
pub use text::*;
pub use tile::*;

// ── Cairo colours ───────────────────────────────────────────────────────

/// The token colours for code that paints with Cairo, from the same
/// generator and the same inputs as the stylesheet on screen
/// (`theme::shown`), so a drawn mark and a styled widget cannot disagree and
/// both follow the mode. Nothing reads a colour back out of the CSS. Read it
/// when a surface opens rather than per frame: it is a settings read.
#[derive(Clone, Copy, Debug)]
pub struct Paint {
    /// `--fg`.
    pub fg: crate::tokens::Rgb,
    /// The darkest ground (`--neutral-1`), for a chip drawn over a picture.
    pub ground: crate::tokens::Rgb,
    /// `--accent-bg`: the accent as a line or a fill.
    pub accent: crate::tokens::Rgb,
    pub status: crate::tokens::Status,
    /// `--cat-n`: the readable tone per categorical slot.
    pub categorical: [crate::tokens::Rgb; 6],
}

pub fn paint() -> Paint {
    use crate::tokens;
    let inputs = crate::theme::shown();
    let s = tokens::scales(inputs);
    Paint {
        fg: s.neutral[11],
        ground: s.neutral[0],
        accent: s.accent_bg,
        status: tokens::status(inputs),
        categorical: tokens::categorical(inputs).map(|(text, _)| text),
    }
}

/// Set `c` as the Cairo source, at `alpha`.
pub fn set_source(cr: &gtk4::cairo::Context, c: crate::tokens::Rgb, alpha: f64) {
    cr.set_source_rgba(c.0, c.1, c.2, alpha);
}
