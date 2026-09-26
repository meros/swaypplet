//! The token colours for Cairo drawing.

use crate::tokens::{self, Rgb, Status};

/// The token colours for code that paints with Cairo, from the same
/// generator and the same inputs as the stylesheet on screen
/// ([`super::shown`]), so a drawn mark and a styled widget cannot disagree and
/// both follow the mode. Nothing reads a colour back out of the CSS. Read it
/// when a surface opens rather than per frame: it is a settings read.
#[derive(Clone, Copy, Debug)]
pub struct Paint {
    /// `--fg`.
    pub fg: Rgb,
    /// The darkest ground (`--neutral-1`), for a chip drawn over a picture.
    pub ground: Rgb,
    /// `--accent-bg`: the accent as a line or a fill.
    pub accent: Rgb,
    pub status: Status,
    /// `--cat-n`: the readable tone per categorical slot.
    pub categorical: [Rgb; 6],
}

pub fn paint() -> Paint {
    let inputs = super::shown();
    let s = tokens::scales(inputs);
    Paint {
        fg: s.neutral[11],
        ground: s.neutral[0],
        accent: s.accent_bg,
        status: tokens::status(inputs),
        categorical: tokens::categorical(inputs).map(|(text, _)| text),
    }
}
