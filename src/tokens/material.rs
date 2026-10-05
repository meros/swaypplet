//! The glass material per mode (docs/design-system.md §4), and what its
//! body shows over a backdrop.

use super::color::{Oklch, oklch_linear};
use super::{Contrast, Inputs, Mode, Rgb, apca, levels, scales};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Material {
    pub fill_color: Rgb,
    pub fill_alpha: f64,
    pub absorb: f64,
    /// The shader's OKLab tone (`photochromic_tone`): > 0 dark, the brights
    /// fold down and white lands at OKLab L `p / (1 + p)`; < 0 light, the
    /// darks fold up and black lands at `-p / (1 - p)`; 0 off.
    pub photochromic: f64,
    pub frost: f64,
}

pub fn material(inputs: Inputs) -> Material {
    let s = scales(inputs);
    let high = inputs.contrast == Contrast::High;
    match inputs.mode {
        // Standard contrast in both modes is clear glass with the tone alone
        // (2026-10-05, from a grey or white veil over a luminance ceiling or
        // lift): dark a little darker, light a little lighter, every colour
        // keeping its hue. The veils washed a colour toward one grey or one
        // pastel, and the light lift oversaturated what it lifted. Chosen by
        // eye in an OKLab model of the shader over the wallpapers and the
        // bench backdrops. High contrast keeps its dense veil.
        Mode::Dark => Material {
            fill_color: s.neutral[2],
            fill_alpha: if high { 0.72 } else { 0.0 },
            absorb: if high { 1.0 } else { 0.0 },
            // 0.89 lands a white page at OKLab L 0.47, about what the old
            // veil and ceiling reached, and leaves the darks alone.
            photochromic: if high { 0.25 } else { 0.89 },
            frost: if high { 0.45 } else { 0.33 },
        },
        Mode::Light => Material {
            fill_color: Rgb::WHITE,
            fill_alpha: if high { 0.72 } else { 0.0 },
            absorb: if high { 0.20 } else { 0.0 },
            // -2 lands black at OKLab L 0.67 and leaves the brights alone.
            // -1.27 (L 0.56) left a black backdrop a muddy mid grey, the one
            // lightness where mid-tone colours on the glass vanish.
            photochromic: if high { -3.0 } else { -2.0 },
            frost: if high { 0.45 } else { 0.33 },
        },
    }
}

/// How far the Glass tab's clarity moves the body fill: +1 takes it to
/// `1 − CLARITY_REACH` of the mode's own, −1 to `1 + CLARITY_REACH`.
const CLARITY_REACH: f64 = 0.5;

/// The mode's material with the Glass tab's clarity applied (§4): the one
/// move a person may make on the five values the mode owns, and the same
/// move in both modes.
///
/// Positive clarity thins the body fill so more of the backdrop shows. A
/// thinner fill costs contrast over the backdrop that is hardest for the
/// mode: black behind light glass, white behind dark glass. So the lift
/// (light) or the ceiling (dark) strengthens until [`readable`] holds again,
/// and where no strength is enough the fill steps back toward the mode's
/// own, which is readable by the contrast tests. Clarity is best effort:
/// the text always wins.
pub fn material_at(inputs: Inputs, clarity: f64) -> Material {
    let base = material(inputs);
    let c = clarity.clamp(-1.0, 1.0);
    if c.abs() < 1e-6 {
        return base;
    }
    let target = (base.fill_alpha * (1.0 - CLARITY_REACH * c)).clamp(0.0, 0.95);
    // With no body fill to thin (light standard ships alpha 0), clarity acts
    // on the lift instead: +1 takes it toward none of the mode's own, −1
    // past it, the same reach CLARITY_REACH gives the fill. The readable()
    // walk below still has the last word, and over a black backdrop it
    // usually keeps some of the thinning from happening.
    // The tone's strength is 1/p dark and -p light (glass_fade.rs fades it
    // the same way), so clear divides it and dense multiplies it.
    let (fill_target, photo_start) = if base.fill_alpha > 0.0 {
        (target, base.photochromic)
    } else {
        let k = 1.0 - CLARITY_REACH * c;
        let p = match inputs.mode {
            Mode::Dark => base.photochromic / k,
            Mode::Light => base.photochromic * k,
        };
        (base.fill_alpha, p)
    };
    // The strongest the compensation may go: a light fold past -4 lands a
    // black terminal behind at grey paper (L 0.8), a dark one under 0.15
    // flattens every highlight behind to one dark grey (L 0.13).
    let (strongest, step) = match inputs.mode {
        Mode::Light => (-4.0, -0.05),
        Mode::Dark => (0.15, -0.01),
    };
    let steps_alpha = ((base.fill_alpha - fill_target).abs() / 0.01).round() as usize;
    let steps_photo = ((strongest - photo_start) / step).round().max(0.0) as usize;
    for i in 0..=steps_alpha {
        let fill_alpha = fill_target + (base.fill_alpha - fill_target).signum() * 0.01 * i as f64;
        for j in 0..=steps_photo {
            let m = Material {
                fill_alpha,
                photochromic: photo_start + step * j as f64,
                ..base
            };
            if readable(inputs, &m) {
                return m;
            }
        }
    }
    base
}

/// The backdrops the contrast targets are held over: a white page, a grey
/// desktop, a black terminal.
const BEHIND: [Rgb; 3] = [Rgb::WHITE, Rgb(0.5, 0.5, 0.5), Rgb::BLACK];

/// The §5 contrast targets, in APCA Lc, for text over the glass.
#[derive(Clone, Copy, Debug)]
pub struct Targets {
    pub fg: f64,
    pub muted: f64,
    pub faint: f64,
    pub accent: f64,
    /// A categorical colour's dot, rail or short label. Read only by the
    /// contrast tests: no runtime path picks a categorical colour.
    #[cfg_attr(not(test), allow(dead_code))]
    pub categorical: f64,
}

/// Body text Lc 75, muted 60, faint 45, the accent's text 60, categorical
/// 45 - except at standard contrast, where both modes are clear glass and
/// the tone alone holds the text up.
///
/// Light glass reaches dark text's contrast only by lifting whatever is
/// behind it, black included, and lifting black far enough for Lc 75 erases
/// the backdrop. Measured on the shipped light material (2026-10-05, black
/// landing at OKLab L 0.67): Lc 42.9 fg, 37.1 muted, 28.8 faint, 28.8
/// accent and 18 categorical over black, 60.6 fg over mid grey, and more
/// over every lighter backdrop. These targets are that reach with a small
/// cushion: a trade for a light material that shows the wallpaper, made for
/// the look and not for the text.
///
/// Dark glass leaves the darks as they are, so faint text over a black
/// terminal has no lift under it: Lc 44.3 on the untinted neutral and down
/// to 43.x on the warmest tints, against 45. It holds the full targets
/// everywhere else.
///
/// High contrast keeps the full targets behind its own dense veil, and is
/// the setting for anyone who needs them.
pub fn targets(mode: Mode, contrast: Contrast) -> Targets {
    match (mode, contrast) {
        (Mode::Light, Contrast::Standard) => Targets {
            fg: 40.0,
            muted: 35.0,
            faint: 27.0,
            accent: 27.0,
            categorical: 16.0,
        },
        (Mode::Dark, Contrast::Standard) => Targets {
            fg: 75.0,
            muted: 60.0,
            faint: 43.0,
            accent: 60.0,
            categorical: 45.0,
        },
        _ => Targets {
            fg: 75.0,
            muted: 60.0,
            faint: 45.0,
            accent: 60.0,
            categorical: 45.0,
        },
    }
}

/// Whether every text token meets its [`targets`] on `m` over every backdrop
/// in [`BEHIND`]. A white page, which dark glass cannot fully overcome, is
/// held to 15 less (5 less at high contrast). `tokens::apca`'s tests hold
/// the shipped material to the same rule.
pub fn readable(inputs: Inputs, m: &Material) -> bool {
    let s = scales(inputs);
    let lv = levels(inputs.mode, inputs.contrast);
    let t = targets(inputs.mode, inputs.contrast);
    let fg = s.neutral[11];
    BEHIND.iter().all(|behind| {
        let ground = glass_body(*behind, m);
        let slack = match (*behind == Rgb::WHITE, inputs.contrast) {
            (false, _) => 0.0,
            (true, Contrast::Standard) => 15.0,
            (true, Contrast::High) => 5.0,
        };
        [
            (fg, 1.0, t.fg),
            (fg, lv.muted, t.muted),
            (fg, lv.faint, t.faint),
            (s.accent[10], 1.0, t.accent),
        ]
        .iter()
        .all(|(color, alpha, need)| apca(color.over(*alpha, ground), ground).abs() >= need - slack)
    })
}

/// What the glass body shows over `backdrop`, following
/// `liquid_glass.frag` on the flat interior of a card: absorption along the
/// full path, the photochromic tone, then the body fill. The shader draws
/// it; this model is what [`readable`] and the contrast tests measure
/// through.
pub fn glass_body(backdrop: Rgb, m: &Material) -> Rgb {
    let k = (-m.absorb).exp();
    let mut t = backdrop.to_linear().map(|x| x * k);
    if m.photochromic != 0.0 {
        t = photochromic_tone(t, m.photochromic);
    }
    let fill = m.fill_color.to_linear();
    let a = m.fill_alpha.clamp(0.0, 1.0);
    Rgb::from_linear([0, 1, 2].map(|i| t[i] * (1.0 - a) + fill[i] * a))
}

/// The shader's `CHROMA_CAP` and `GAMUT_KNEE` (nixos
/// patches/scenefx-liquid-glass.patch).
const CHROMA_CAP: f64 = 0.13;
const GAMUT_KNEE: f64 = 0.8;

/// The shader's `photochromic_tone`, in linear sRGB: lightness along the
/// mode's curve, hue kept, chroma softly capped and then compressed into
/// the gamut at the new lightness.
fn photochromic_tone(t: [f64; 3], p: f64) -> [f64; 3] {
    let Oklch(l, c, h) = Oklch::from(Rgb::from_linear(t));
    let l = l.max(0.0);
    let l = if p > 0.0 {
        l / (1.0 + l / p)
    } else {
        let x = (1.0 - l).max(0.0);
        1.0 - x / (1.0 - p * x)
    };
    let mut c = CHROMA_CAP * (c / CHROMA_CAP).tanh();
    let inside = |c: f64| {
        oklch_linear(Oklch(l, c, h))
            .iter()
            .all(|v| (0.0..=1.0).contains(v))
    };
    let (mut lo, mut hi) = (0.0, 0.4);
    for _ in 0..8 {
        let mid = 0.5 * (lo + hi);
        if inside(mid) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let knee = GAMUT_KNEE * lo;
    if c > knee {
        let room = (lo - knee).max(1e-5);
        c = knee + room * ((c - knee) / room).tanh();
    }
    oklch_linear(Oklch(l, c, h)).map(|v| v.clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::super::every;
    use super::*;

    /// A sample of the input sets: every untinted one, and each tint at
    /// every 30°. The search inside `material_at` makes the full sweep slow
    /// in a debug build, and the hues between do not behave differently.
    fn sample() -> impl Iterator<Item = Inputs> {
        every::input()
            .into_iter()
            .filter(|i| i.tint.hue().is_none_or(|h| (h as u32).is_multiple_of(30)))
    }

    /// The tone moves lightness and nothing else a person would call the
    /// colour: in both modes, no colour behind the glass leaves with more
    /// chroma than it came in with (no neon), its hue holds, and dark glass
    /// never lightens nor light glass darkens. Before 2026-10-05 the light
    /// lift multiplied a dark red diff line by up to 20, and a cap was what
    /// kept it from going neon; here nothing multiplies chroma at all.
    #[test]
    fn the_tone_keeps_a_colour() {
        for mode in Mode::ALL {
            let m = material(Inputs {
                mode,
                ..Inputs::default()
            });
            for c in [
                0x5f0000, 0x003f00, 0x3c1414, 0x458588, 0x00003f, 0xf4a6c0, 0x8ec8f0, 0xff0000,
                0xff00aa, 0x0000ff, 0x00e5ff, 0xffd400, 0xcc241d, 0x808080,
            ] {
                let c = Rgb::hex(c);
                let (i, o) = (Oklch::from(c), Oklch::from(glass_body(c, &m)));
                assert!(
                    o.1 <= i.1 + 1e-6,
                    "{mode:?} {c:?}: chroma {:.3} -> {:.3}",
                    i.1,
                    o.1
                );
                if o.1 > 0.02 {
                    let dh = ((o.2 - i.2 + 540.0) % 360.0 - 180.0).abs();
                    assert!(dh < 3.0, "{mode:?} {c:?}: hue moved {dh:.1} degrees");
                }
                match mode {
                    Mode::Dark => assert!(o.0 <= i.0 + 1e-6, "{c:?}: {:.3} -> {:.3}", i.0, o.0),
                    Mode::Light => assert!(o.0 >= i.0 - 1e-6, "{c:?}: {:.3} -> {:.3}", i.0, o.0),
                }
            }
        }
    }

    #[test]
    #[ignore = "prints the clarity rail"]
    fn print_clarity() {
        for mode in Mode::ALL {
            for contrast in Contrast::ALL {
                let inputs = Inputs {
                    mode,
                    contrast,
                    ..Inputs::default()
                };
                for c in [-1.0, -0.5, 0.0, 0.5, 1.0] {
                    let m = material_at(inputs, c);
                    println!(
                        "{mode:?} {contrast:?} {c:+.1}: alpha {:.2} photo {:.2}",
                        m.fill_alpha, m.photochromic
                    );
                }
            }
        }
    }

    #[test]
    fn the_shipped_material_is_readable() {
        for inputs in sample() {
            assert!(readable(inputs, &material(inputs)), "{inputs:?}");
        }
    }

    /// Clarity never costs the text its contrast, at either end of the rail.
    #[test]
    fn clarity_keeps_the_text_readable() {
        for inputs in sample() {
            for c in [-1.0, -0.5, 0.5, 1.0] {
                let m = material_at(inputs, c);
                assert!(readable(inputs, &m), "{inputs:?} at clarity {c}");
            }
        }
    }

    /// And it does something: where the mode has a body fill, full clarity
    /// thins it; where it has none (standard contrast), the tone is the knob
    /// and dense clarity strengthens it. `readable()` may hold the clear end
    /// in place, so the clear assertion is on the effect, not the knob: clear
    /// never moves the mode's hardest backdrop further than dense does.
    #[test]
    fn clarity_lets_more_through() {
        for inputs in sample() {
            let base = material(inputs);
            let clear = material_at(inputs, 1.0);
            let dense = material_at(inputs, -1.0);
            assert_eq!(material_at(inputs, 0.0), base);
            if base.fill_alpha > 0.0 {
                assert!(
                    clear.fill_alpha < base.fill_alpha - 0.05,
                    "{inputs:?}: {} -> {}",
                    base.fill_alpha,
                    clear.fill_alpha
                );
                assert!(dense.fill_alpha > base.fill_alpha, "{inputs:?}");
            } else {
                // Stronger is a smaller p dark and a more negative one light.
                assert!(
                    dense.photochromic < base.photochromic,
                    "{inputs:?}: dense tone {:.2} vs base {:.2}",
                    dense.photochromic,
                    base.photochromic
                );
                let lum = |c: Rgb| {
                    let [r, g, b] = c.to_linear();
                    0.2126 * r + 0.7152 * g + 0.0722 * b
                };
                // The mode's hardest backdrop: black under light glass, white
                // under dark. Clear moves it less than dense does.
                let behind = match inputs.mode {
                    Mode::Light => Rgb::BLACK,
                    Mode::Dark => Rgb::WHITE,
                };
                let moved = |m: &Material| (lum(glass_body(behind, m)) - lum(behind)).abs();
                assert!(
                    moved(&clear) <= moved(&dense),
                    "{inputs:?}: clear moves the ground {:.3}, dense {:.3}",
                    moved(&clear),
                    moved(&dense)
                );
            }
        }
    }
}
