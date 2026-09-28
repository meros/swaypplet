//! The wallpaper tint, as an input to the tokens (docs/design-system.md §2.2).
//!
//! `Look.tint` used to rewrite a legacy named palette after the fact. It is
//! now one more input the scales are generated from, so the tokens, the
//! glass body, the Cairo drawing and sway's borders all move together, and
//! the contrast tests in `tokens/apca.rs` hold the tinted token
//! sets to the same targets as the shipped ones.
//!
//! What the wallpaper gives is a [`Palette`] of up to three OKLCH hues
//! (`theme::wallpaper` samples them), each with its own job. The image gives
//! hue only: every rule keeps a colour's **lightness**, because lightness is
//! what the contrast is made of.
//!
//! - **Primary**, the accent's colour: one of the up to four the image
//!   offers (Material's `Score`, which ranks by area and chroma), the first
//!   unless the Look pane picked another. The **accent** family takes it, each step
//!   keeping its lightness and chroma. The accent input still decides how
//!   loud the accent is; the wallpaper decides which colour it is.
//! - **Ground**, the hue that covers the most of the image. Under
//!   [`Tint::Full`] the three **neutral** anchors take it at a chroma held
//!   inside [`CAST`], so the surfaces pick up the wallpaper's ground while the
//!   accent can be its highlight: a blue sky with a red boat gives blue-grey
//!   glass and a red accent.
//! - **Secondary**, another offered colour of the image, at least
//!   [`SECONDARY_APART`] from the primary. The **categorical** set is turned
//!   rigidly, all six by one angle, so they stay as far apart as gruvbox put
//!   them: they exist to be told apart. The angle puts slot 1 on the primary,
//!   then moves by at most [`CATEGORICAL_SLACK`] to bring the nearest other
//!   slot onto the secondary. Assigning the slots straight from the image
//!   would squeeze them into a band, because most wallpapers are analogous.
//! - **Status** colours turn toward the nearest of the three by at most
//!   [`STATUS_CAP`] (Material's `Blend.harmonize`, capped tighter). Red stays
//!   red.
//!
//! A wallpaper with one usable colour gives a palette whose three hues are
//! that one, which is the single-hue tint this module had before.
//!
//! A hue moved at a kept chroma can fall outside sRGB; [`to_rgb`] then gives
//! up chroma, never lightness, until it fits.

use super::color::oklch_linear;
use super::{Oklch, Rgb};

/// The wallpaper's hues, in whole OKLCH degrees (so `Inputs` stays `Eq`
/// and a change below a degree does not reload the stylesheet).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Palette {
    /// The accent's hue: the offered colour picked in the Look pane, by
    /// default the one the image is about.
    pub primary: u16,
    /// The neutral cast's hue: the colour most of the image is.
    pub ground: u16,
    /// A second colour of the image, far enough from the primary to read as
    /// another colour, for the categorical set.
    pub secondary: Option<u16>,
}

impl Palette {
    /// One hue for every job: an image with a single usable colour.
    #[cfg(test)]
    pub const fn single(hue: u16) -> Palette {
        Palette {
            primary: hue,
            ground: hue,
            secondary: None,
        }
    }

    /// The hues, primary first, for the rules that turn toward the nearest.
    pub fn hues(self) -> impl Iterator<Item = f64> {
        [Some(self.primary), self.secondary, Some(self.ground)]
            .into_iter()
            .flatten()
            .map(|h| f64::from(h % 360))
    }
}

/// How far the wallpaper reaches into the tokens.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Tint {
    /// The shipped tokens, byte for byte.
    #[default]
    Off,
    /// Accent, categorical and status follow the wallpaper; the greys stay.
    Accents(Palette),
    /// The greys pick up the wallpaper's ground as well.
    Full(Palette),
}

impl Tint {
    /// The wallpaper's hues, when there is a tint at all.
    pub fn palette(self) -> Option<Palette> {
        match self {
            Tint::Off => None,
            Tint::Accents(p) | Tint::Full(p) => Some(p),
        }
    }

    /// The accent's hue, when there is a tint at all.
    pub fn hue(self) -> Option<f64> {
        self.palette().map(|p| f64::from(p.primary % 360))
    }

    /// The hue the neutral anchors take, under [`Tint::Full`] only.
    pub fn ground(self) -> Option<f64> {
        match self {
            Tint::Full(p) => Some(f64::from(p.ground % 360)),
            _ => None,
        }
    }

    /// Whether the neutral anchors take the cast.
    pub fn casts_neutral(self) -> bool {
        matches!(self, Tint::Full(_))
    }
}

/// How far apart the secondary must be from the primary. Closer than this
/// the two read as one colour, and the categorical set gains nothing.
pub const SECONDARY_APART: f64 = 45.0;

/// How far the categorical rotation may leave the primary to reach the
/// secondary. Slot 1 stays within this of the primary.
pub const CATEGORICAL_SLACK: f64 = 15.0;

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
        oklch_linear(Oklch(o.0, c, o.2))
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

/// `c` turned toward the nearest of `palette`'s hues by at most
/// [`STATUS_CAP`].
pub fn harmonized(c: Rgb, palette: Palette) -> Rgb {
    let h = Oklch::from(c).2;
    let nearest = palette
        .hues()
        .min_by(|a, b| difference(h, *a).abs().total_cmp(&difference(h, *b).abs()))
        .unwrap_or(h);
    with_hue(c, harmonize(h, nearest, STATUS_CAP))
}

/// The one angle the categorical set turns by, given the six slots' hues:
/// slot 1 onto the primary, then within [`CATEGORICAL_SLACK`] of that, the
/// angle that brings the nearest other slot closest to the secondary. Whole
/// degrees, so the search is 31 candidates.
pub fn categorical_turn(slots: [f64; 6], palette: Palette) -> f64 {
    let base = difference(slots[0], f64::from(palette.primary % 360));
    let Some(secondary) = palette.secondary.map(|s| f64::from(s % 360)) else {
        return base;
    };
    let miss = |turn: f64| {
        slots[1..]
            .iter()
            .map(|h| difference(h + turn, secondary).abs())
            .fold(f64::INFINITY, f64::min)
    };
    let slack = CATEGORICAL_SLACK as i32;
    (-slack..=slack)
        .map(|e| base + f64::from(e))
        // Ties go to the smaller departure from the primary.
        .min_by(|a, b| {
            miss(*a)
                .total_cmp(&miss(*b))
                .then((a - base).abs().total_cmp(&(b - base).abs()))
        })
        .unwrap_or(base)
}

/// A neutral anchor with the wallpaper's cast: its lightness, the
/// wallpaper's hue, its chroma held inside [`CAST`].
pub fn cast(c: Rgb, hue: f64) -> Rgb {
    let Oklch(l, ch, _) = Oklch::from(c);
    to_rgb(Oklch(l, ch.clamp(CAST.0, CAST.1), hue.rem_euclid(360.0)))
}

/// The colour `t` of the way from `a` to `b`, in OKLCH: lightness and
/// chroma straight, hue the shorter way round. A near-grey end has no hue
/// worth keeping (CSS calls it powerless), so the path takes the other end's
/// hue and only the chroma grows, rather than sweeping through the circle
/// from whatever angle rounding left on a grey. Chroma gives way where the
/// path leaves sRGB, as everywhere else here.
pub fn blend(a: Rgb, b: Rgb, t: f64) -> Rgb {
    const POWERLESS: f64 = 0.02;
    let (x, y) = (Oklch::from(a), Oklch::from(b));
    let (hx, hy) = match (x.1 < POWERLESS, y.1 < POWERLESS) {
        (true, false) => (y.2, y.2),
        (false, true) => (x.2, x.2),
        _ => (x.2, y.2),
    };
    let t = t.clamp(0.0, 1.0);
    to_rgb(Oklch(
        x.0 + (y.0 - x.0) * t,
        x.1 + (y.1 - x.1) * t,
        (hx + difference(hx, hy) * t).rem_euclid(360.0),
    ))
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
    fn the_categorical_turn_reaches_for_the_secondary_within_its_slack() {
        let slots = [150.0, 80.0, 200.0, 0.0, 110.0, 55.0];
        // No secondary: slot 1 lands on the primary exactly.
        let turn = categorical_turn(slots, Palette::single(260));
        assert!(difference(slots[0] + turn, 260.0).abs() < 1e-9);
        // A secondary 10° off where a slot would land: the set turns 10°.
        let lone = categorical_turn(slots, Palette::single(260));
        let target = (slots[2] + lone + 10.0).rem_euclid(360.0);
        let p = Palette {
            primary: 260,
            ground: 260,
            secondary: Some(target.round() as u16),
        };
        let turn = categorical_turn(slots, p);
        assert!((turn - lone - 10.0).abs() < 1.0, "{turn} vs {lone}");
        // Never further than the slack from the primary, however far away
        // the secondary is.
        for s in (0..360).step_by(7) {
            let p = Palette {
                secondary: Some(s),
                ..Palette::single(260)
            };
            let turn = categorical_turn(slots, p);
            assert!(difference(slots[0] + turn, 260.0).abs() <= CATEGORICAL_SLACK + 1e-9);
        }
    }

    #[test]
    fn status_turns_toward_the_nearest_hue() {
        // Gruvbox red (hue ~29) with a palette of blue and orange: it turns
        // toward the orange, not the blue.
        let p = Palette {
            primary: 260,
            ground: 260,
            secondary: Some(50),
        };
        let h = Oklch::from(harmonized(Rgb::hex(0xcc241d), p)).2;
        assert!(h > Oklch::from(Rgb::hex(0xcc241d)).2, "{h}");
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
