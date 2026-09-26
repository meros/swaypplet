//! The wallpaper tint, as an input to the tokens (docs/design-system.md §2.2).
//!
//! `Look.tint` used to rewrite a legacy named palette after the fact. It is
//! now one more input the scales are generated from, so the tokens, the
//! glass body, the Cairo drawing and sway's borders all move together, and
//! the contrast tests at the bottom of `tokens/mod.rs` hold the tinted token
//! sets to the same targets as the shipped ones.
//!
//! What the wallpaper gives is one number: the OKLCH hue of its source colour
//! (`theme::wallpaper` samples it). Each colour family takes it its own way,
//! and every rule keeps a colour's **lightness**, because lightness is what
//! the contrast is made of:
//!
//! - **Accent**: the whole family takes the wallpaper's hue, each step keeping
//!   its lightness and chroma. The accent input still decides how loud the
//!   accent is (its chroma and its tone per mode); the wallpaper decides which
//!   colour it is.
//! - **Categorical**: rotated rigidly, all six by the one angle that puts slot
//!   1 (aqua, the shipped accent) on the wallpaper's hue. Rigid, so the six
//!   stay as far apart as gruvbox put them: they exist to be told apart, and
//!   pulling each toward the source would squeeze them into a band. The angle
//!   is anchored on the categorical set itself rather than on the accent
//!   input, so which app a colour means does not depend on the accent picked.
//! - **Status**: harmonised toward the wallpaper by at most [`STATUS_CAP`]
//!   degrees (Material's `Blend.harmonize`, capped tighter). Red stays red.
//! - **Neutral**, under [`Tint::Full`] only: the three anchors take the
//!   wallpaper's hue at a chroma held inside [`CAST`], so the ground picks up
//!   a cast of the wallpaper without becoming a colour, and a grey preset
//!   (`pure`) gets one too.
//!
//! A hue moved at a kept chroma can fall outside sRGB; [`to_rgb`] then gives
//! up chroma, never lightness, until it fits.

use super::{Oklch, Rgb};

/// How far the wallpaper reaches into the tokens, with its hue in whole
/// OKLCH degrees (so `Inputs` stays `Eq` and a change below a degree does
/// not reload the stylesheet).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Tint {
    /// The shipped tokens, byte for byte.
    #[default]
    Off,
    /// Accent, categorical and status follow the wallpaper; the greys stay.
    Accents(u16),
    /// The greys pick up a cast of it as well.
    Full(u16),
}

impl Tint {
    /// The wallpaper's hue, when there is a tint at all.
    pub fn hue(self) -> Option<f64> {
        match self {
            Tint::Off => None,
            Tint::Accents(h) | Tint::Full(h) => Some(f64::from(h % 360)),
        }
    }

    /// Whether the neutral anchors take the cast.
    pub fn casts_neutral(self) -> bool {
        matches!(self, Tint::Full(_))
    }
}

/// How far a status colour may turn toward the wallpaper. Material uses 15
/// for the same job; 12 keeps gruvbox red (OKLCH hue 29) out of orange
/// whatever the wallpaper is.
pub const STATUS_CAP: f64 = 12.0;

/// The chroma a neutral anchor holds under [`Tint::Full`]: at least enough
/// for the cast to show on a grey preset, at most what still reads as grey
/// beside an accent. Gruvbox's own ground is 0.004 and its cream text 0.05.
pub const CAST: (f64, f64) = (0.010, 0.025);

/// The shorter way round from `from` to `to`, in degrees, signed.
pub fn difference(from: f64, to: f64) -> f64 {
    let d = (to - from).rem_euclid(360.0);
    if d > 180.0 { d - 360.0 } else { d }
}

/// `hue` turned toward `toward` by at most `cap` degrees.
pub fn harmonize(hue: f64, toward: f64, cap: f64) -> f64 {
    (hue + difference(hue, toward).clamp(-cap, cap)).rem_euclid(360.0)
}

/// `o` in sRGB, giving up chroma (never lightness or hue) until it fits.
pub fn to_rgb(o: Oklch) -> Rgb {
    let fits = |c: f64| {
        super::oklch_linear(Oklch(o.0, c, o.2))
            .iter()
            .all(|v| (-1e-4..=1.0 + 1e-4).contains(v))
    };
    if fits(o.1) {
        return o.into();
    }
    let (mut lo, mut hi) = (0.0, o.1);
    for _ in 0..24 {
        let mid = (lo + hi) / 2.0;
        if fits(mid) { lo = mid } else { hi = mid }
    }
    Oklch(o.0, lo, o.2).into()
}

/// `c` at `hue`, lightness and chroma kept.
pub fn with_hue(c: Rgb, hue: f64) -> Rgb {
    let Oklch(l, ch, _) = Oklch::from(c);
    to_rgb(Oklch(l, ch, hue.rem_euclid(360.0)))
}

/// `c` turned by `delta` degrees.
pub fn rotated(c: Rgb, delta: f64) -> Rgb {
    let h = Oklch::from(c).2;
    with_hue(c, h + delta)
}

/// `c` turned toward `hue` by at most [`STATUS_CAP`].
pub fn harmonized(c: Rgb, hue: f64) -> Rgb {
    let h = Oklch::from(c).2;
    with_hue(c, harmonize(h, hue, STATUS_CAP))
}

/// A neutral anchor with the wallpaper's cast: its lightness, the
/// wallpaper's hue, its chroma held inside [`CAST`].
pub fn cast(c: Rgb, hue: f64) -> Rgb {
    let Oklch(l, ch, _) = Oklch::from(c);
    to_rgb(Oklch(l, ch.clamp(CAST.0, CAST.1), hue.rem_euclid(360.0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hue_arithmetic_wraps() {
        assert!((difference(350.0, 10.0) - 20.0).abs() < 1e-9);
        assert!((difference(10.0, 350.0) + 20.0).abs() < 1e-9);
        assert!((harmonize(350.0, 10.0, 5.0) - 355.0).abs() < 1e-9);
        assert!((harmonize(0.0, 180.0, 12.0) - 12.0).abs() < 1e-9);
        // Already there: no movement, and no drift past the target.
        assert!((harmonize(100.0, 104.0, 12.0) - 104.0).abs() < 1e-9);
    }

    #[test]
    fn a_hue_out_of_gamut_gives_up_chroma_not_lightness() {
        // Gruvbox's saturated orange at a blue hue does not fit in sRGB.
        let c = with_hue(Rgb::hex(0xd65d0e), 265.0);
        let (was, now) = (Oklch::from(Rgb::hex(0xd65d0e)), Oklch::from(c));
        assert!((was.0 - now.0).abs() < 0.01, "{was:?} -> {now:?}");
        assert!(now.1 <= was.1 + 1e-6);
        assert!(difference(now.2, 265.0).abs() < 1.0, "{now:?}");
    }
}
