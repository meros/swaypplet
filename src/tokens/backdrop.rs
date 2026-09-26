//! Text on bare wallpaper (docs/design-system.md §3.3, "on wallpaper"): the
//! ink and the halo, chosen from what the wallpaper is behind the text.
//!
//! The lock screen's clock, date and switch-user button, and the switcher's
//! caption, stand on the wallpaper with no card behind them. White ink with
//! a dark halo reads on a dark image and turns into white on white on a
//! bright one, in either mode: the mode does not change the wallpaper. So
//! the tone follows the wallpaper instead. The panel measures the region
//! the text sits over once per wallpaper (`theme::wallpaper`), and this
//! picks, from the region's luminance and how busy it is:
//!
//! - **the ink**: light (`--on-status`, white) or dark ([`INK_DARK`]);
//! - **the halo**: the opposite colour, stacked under the glyphs as four
//!   tight shadow layers at one alpha; a busier region gets a denser halo.
//!
//! The choice is measured, not guessed: the halo darkens or lightens the
//! ground right at the glyph edge, so the contrast that counts is the ink's
//! APCA over the region blended with the part of the halo that reaches the
//! edge. It aims for [`CLOCK_LC`] (the clock and date) against the darkest
//! and the brightest the region can be, taken as the mean minus and plus
//! one standard deviation, with and without the lock's scrim darkening it.
//! Over a bright and busy region the densest halo falls a few points short
//! ([`BUSY_CLOCK_LC`]); the smaller text's [`SMALL_LC`] holds everywhere.
//!
//! With no measurement (no sample yet) the answer is the shipped look: light
//! ink, a black halo at [`HALO_ALPHAS`]`[0]` per layer.

use super::{ON_STATUS, Rgb, apca};

/// The luminance behind wallpaper text: the region's mean relative
/// luminance and its standard deviation, in whole percent, so [`Inputs`]
/// stays `Eq` and a change below a percent does not reload the stylesheet.
///
/// [`Inputs`]: super::Inputs
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Backdrop {
    /// Mean relative (linear) luminance, 0–100.
    pub luminance: u8,
    /// Standard deviation of the relative luminance, 0–100: how busy.
    pub spread: u8,
}

/// The dark ink: gruvbox's deepest ground, which is also the dark mode's
/// darkest neutral. Mode independent, like the wallpaper.
pub const INK_DARK: Rgb = Rgb::hex(0x1d2021);

/// The halo layer alphas, lightest first. Four layers stack to a core of
/// `1 - (1 - a)^4`: 0.59, 0.76, 0.87, 0.94; half of that reaches a glyph
/// edge ([`EDGE`]). The first is the shipped scrim alpha.
pub const HALO_ALPHAS: [f64; 4] = [0.2, 0.3, 0.4, 0.5];

/// Tight halo layers stacked under the glyphs (`text.css`).
const LAYERS: i32 = 4;

/// The lock's full-screen scrim over the wallpaper (`--scrim`), black.
pub const SCRIM_ALPHA: f64 = 0.2;

/// The clock and the date: body text on the wallpaper.
pub const CLOCK_LC: f64 = 75.0;

/// The switch-user button and the switcher's caption.
#[cfg_attr(not(test), allow(dead_code))]
pub const SMALL_LC: f64 = 60.0;

/// What the clock is held to over a bright region busier than 10 %: no
/// halo short of a card gets it all the way to [`CLOCK_LC`] there.
#[cfg_attr(not(test), allow(dead_code))]
pub const BUSY_CLOCK_LC: f64 = 68.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OnWallpaper {
    pub ink: Rgb,
    pub halo: Rgb,
    /// Per shadow layer.
    pub halo_alpha: f64,
}

/// How much of the halo's core reaches the edge of a glyph, where the
/// contrast is read. The shadows are blurred 3 px and offset 1 px, so a
/// stroke's edge sits on the shoulder of its halo, not its core. 0.5 is
/// calibrated on renders: at 1.0 the model called white ink on a white page
/// Lc 85, and it is not readable there.
const EDGE: f64 = 0.5;

/// The halo's coverage at a glyph edge at `alpha` per layer.
fn core(alpha: f64) -> f64 {
    (1.0 - (1.0 - alpha).powi(LAYERS)) * EDGE
}

/// `a` over `b` at `t`, in sRGB (how GTK composites).
fn over(a: Rgb, t: f64, b: Rgb) -> Rgb {
    Rgb(
        a.0 * t + b.0 * (1.0 - t),
        a.1 * t + b.1 * (1.0 - t),
        a.2 * t + b.2 * (1.0 - t),
    )
}

/// A grey of relative luminance `y`.
fn grey(y: f64) -> Rgb {
    Rgb::from_linear([y.clamp(0.0, 1.0); 3])
}

/// The grounds a glyph edge can land on: the darkest and brightest the
/// region can be, each with and without the lock's scrim.
fn grounds(b: Backdrop) -> [Rgb; 4] {
    let mean = f64::from(b.luminance) / 100.0;
    let spread = f64::from(b.spread) / 100.0;
    let (lo, hi) = (grey(mean - spread), grey(mean + spread));
    let scrimmed = |c: Rgb| over(Rgb::BLACK, SCRIM_ALPHA, c);
    [lo, hi, scrimmed(lo), scrimmed(hi)]
}

/// The weakest contrast `choice` gives over `b`.
pub fn worst_lc(choice: OnWallpaper, b: Backdrop) -> f64 {
    let halo_core = core(choice.halo_alpha);
    grounds(b)
        .iter()
        .map(|g| apca(choice.ink, over(choice.halo, halo_core, *g)).abs())
        .fold(f64::INFINITY, f64::min)
}

/// The ink and halo for text over `backdrop`: the lightest halo that gets
/// the clock to [`CLOCK_LC`], light ink first at a tie (the shipped look);
/// past the densest halo, whichever ink does better.
pub fn on_wallpaper(backdrop: Option<Backdrop>) -> OnWallpaper {
    let light = |a| OnWallpaper {
        ink: ON_STATUS,
        halo: Rgb::BLACK,
        halo_alpha: a,
    };
    let dark = |a| OnWallpaper {
        ink: INK_DARK,
        halo: Rgb::WHITE,
        halo_alpha: a,
    };
    let Some(b) = backdrop else {
        return light(HALO_ALPHAS[0]);
    };
    for a in HALO_ALPHAS {
        let (l, d) = (light(a), dark(a));
        let (ll, dl) = (worst_lc(l, b), worst_lc(d, b));
        if ll >= CLOCK_LC && ll >= dl - 5.0 {
            return l;
        }
        if dl >= CLOCK_LC {
            return d;
        }
        if ll >= CLOCK_LC {
            return l;
        }
    }
    let a = HALO_ALPHAS[HALO_ALPHAS.len() - 1];
    if worst_lc(light(a), b) >= worst_lc(dark(a), b) {
        light(a)
    } else {
        dark(a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(luminance: u8, spread: u8) -> Backdrop {
        Backdrop { luminance, spread }
    }

    #[test]
    fn no_measurement_is_the_shipped_look() {
        let c = on_wallpaper(None);
        assert_eq!(c.ink, ON_STATUS);
        assert_eq!(c.halo, Rgb::BLACK);
        assert!((c.halo_alpha - 0.2).abs() < 1e-9);
    }

    #[test]
    fn a_dark_region_keeps_light_ink_and_a_white_page_gets_dark_ink() {
        let night = on_wallpaper(Some(at(3, 2)));
        assert_eq!(night.ink, ON_STATUS);
        assert!((night.halo_alpha - HALO_ALPHAS[0]).abs() < 1e-9);
        let page = on_wallpaper(Some(at(100, 0)));
        assert_eq!(page.ink, INK_DARK);
        assert_eq!(page.halo, Rgb::WHITE);
    }

    #[test]
    fn a_busier_region_never_gets_a_lighter_halo() {
        for l in (0..=100).step_by(5) {
            let mut last = 0.0;
            for s in (0..=30).step_by(5) {
                let a = on_wallpaper(Some(at(l, s))).halo_alpha;
                // Across an ink flip the halo may start over lighter; within
                // one ink it only grows.
                let ink = on_wallpaper(Some(at(l, s))).ink;
                let prev = on_wallpaper(Some(at(l, s.saturating_sub(5)))).ink;
                if ink == prev {
                    assert!(a >= last - 1e-9, "L{l} S{s}: {a} < {last}");
                }
                last = a;
            }
        }
    }

    /// §5, as far as a halo can take it. Over a region up to 10 % busy the
    /// clock clears Lc 75 (within a point, the rounding of the percentages);
    /// over a bright region busier than that, up to 30 % (a spread above that
    /// is a checkerboard, not a photo), the densest halo reaches Lc 69–75,
    /// and this holds it to 68. The small text's Lc 60 holds everywhere.
    #[test]
    fn every_region_meets_the_targets() {
        let mut failures = Vec::new();
        for l in 0..=100 {
            for s in 0..=30 {
                let b = at(l, s);
                let c = on_wallpaper(Some(b));
                let lc = worst_lc(c, b);
                let need = if s <= 10 { CLOCK_LC - 1.0 } else { BUSY_CLOCK_LC };
                if lc < need || lc < SMALL_LC {
                    failures.push(format!("L{l} S{s}: {c:?} Lc {lc:.1} < {need}"));
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}
