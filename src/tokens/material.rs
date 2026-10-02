//! The glass material per mode (docs/design-system.md §4), and what its
//! body shows over a backdrop.

use super::{Contrast, Inputs, Mode, Rgb, apca, levels, scales};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Material {
    pub fill_color: Rgb,
    pub fill_alpha: f64,
    pub absorb: f64,
    /// > 0 a ceiling on luminance, < 0 a lift (a floor).
    pub photochromic: f64,
    pub frost: f64,
}

pub fn material(inputs: Inputs) -> Material {
    let s = scales(inputs);
    let high = inputs.contrast == Contrast::High;
    match inputs.mode {
        Mode::Dark => Material {
            fill_color: s.neutral[2],
            fill_alpha: if high { 0.72 } else { 0.50 },
            absorb: 1.0,
            photochromic: if high { 0.25 } else { 0.35 },
            frost: if high { 0.45 } else { 0.33 },
        },
        Mode::Light => Material {
            fill_color: s.neutral[1],
            fill_alpha: if high { 0.72 } else { 0.44 },
            absorb: if high { 0.20 } else { 0.25 },
            photochromic: if high { -0.50 } else { -0.48 },
            frost: if high { 0.45 } else { 0.30 },
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
    // The strongest the compensation may go: a lift past 0.70 turns a black
    // terminal behind into grey paper, a ceiling under 0.15 flattens every
    // highlight behind to one grey.
    let (strongest, step) = match inputs.mode {
        Mode::Light => (-0.70, -0.01),
        Mode::Dark => (0.15, -0.01),
    };
    let steps_alpha = ((base.fill_alpha - target).abs() / 0.01).round() as usize;
    let steps_photo = ((strongest - base.photochromic) / step).round().max(0.0) as usize;
    for i in 0..=steps_alpha {
        let fill_alpha = target + (base.fill_alpha - target).signum() * 0.01 * i as f64;
        for j in 0..=steps_photo {
            let m = Material {
                fill_alpha,
                photochromic: base.photochromic + step * j as f64,
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

/// Whether every text token meets its §5 target on `m` over every backdrop
/// in [`BEHIND`]: body text Lc 75, muted 60, faint 45, the accent's text 60.
/// A white page, which dark glass cannot fully overcome, is held to 15 less
/// (5 less at high contrast). `tokens::apca`'s tests hold the shipped
/// material to the same rule.
pub fn readable(inputs: Inputs, m: &Material) -> bool {
    let s = scales(inputs);
    let lv = levels(inputs.mode, inputs.contrast);
    let fg = s.neutral[11];
    BEHIND.iter().all(|behind| {
        let ground = glass_body(*behind, m);
        let slack = match (*behind == Rgb::WHITE, inputs.contrast) {
            (false, _) => 0.0,
            (true, Contrast::Standard) => 15.0,
            (true, Contrast::High) => 5.0,
        };
        [
            (fg, 1.0, 75.0),
            (fg, lv.muted, 60.0),
            (fg, lv.faint, 45.0),
            (s.accent[10], 1.0, 60.0),
        ]
        .iter()
        .all(|(color, alpha, need)| apca(color.over(*alpha, ground), ground).abs() >= need - slack)
    })
}

/// What the glass body shows over `backdrop`, following
/// `liquid_glass.frag` on the flat interior of a card: absorption along the
/// full path, the photochromic ceiling or lift, then the body fill. The
/// shader draws it; this model is what [`readable`] and the contrast tests
/// measure through.
/// The most light glass's lift may multiply a colour by (the shader's
/// `LIFT_GAIN_MAX`): at 4 a colour behind light glass keeps about the chroma
/// dark glass leaves it, so the two modes feel alike.
const LIFT_GAIN_MAX: f64 = 4.0;

pub fn glass_body(backdrop: Rgb, m: &Material) -> Rgb {
    let k = (-m.absorb).exp();
    let mut t = backdrop.to_linear().map(|x| x * k);
    let lum = 0.2126 * t[0] + 0.7152 * t[1] + 0.0722 * t[2];
    if m.photochromic > 0.0 && lum > 0.0005 {
        let p = m.photochromic;
        let s = p * (1.0 - (-lum / p).exp()) / lum;
        t = t.map(|x| x * s);
    } else if m.photochromic < 0.0 {
        // As the shader does (nixos patches/scenefx-photochromic-lift.patch):
        // the floor reaches the colour as a gain of at most LIFT_GAIN_MAX,
        // never past 1.0 on a channel, and the rest as white. The luminance
        // lands on the floor either way. An uncapped gain of lifted/lum
        // turned a dark red diff line behind light glass into neon.
        let f = -m.photochromic;
        let lifted = lum + f * (-lum / f).exp();
        let mut gain = if lum > 0.0005 {
            (lifted / lum).min(LIFT_GAIN_MAX)
        } else {
            0.0
        };
        let top = t[0].max(t[1]).max(t[2]);
        if top > lum + 1e-6 {
            gain = gain.min((1.0 - lifted).max(0.0) / (top - lum));
        }
        t = t.map(|x| x * gain + lifted - lum * gain);
    }
    let fill = m.fill_color.to_linear();
    let a = m.fill_alpha.clamp(0.0, 1.0);
    Rgb::from_linear([0, 1, 2].map(|i| t[i] * (1.0 - a) + fill[i] * a))
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

    /// OKLCH chroma, for comparing how much colour survives the glass.
    fn chroma(c: Rgb) -> f64 {
        let [r, g, b] = c.to_linear();
        let cbrt = |x: f64| x.max(0.0).cbrt();
        let l = cbrt(0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b);
        let m = cbrt(0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b);
        let s = cbrt(0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b);
        let a = 1.977_998_495_1 * l - 2.428_592_205 * m + 0.450_593_709_9 * s;
        let bb = 0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766 * s;
        a.hypot(bb)
    }

    /// Light glass shows a colour behind it about as strongly as dark glass
    /// does, and never brighter than its gamut: no neon from a terminal's
    /// dark red diff line or a pure red. Measured on the body (before the
    /// client's content), in OKLCH chroma, with the uncapped lift at 0.085
    /// over the diff line against dark glass's 0.031.
    #[test]
    fn light_and_dark_glass_carry_a_colour_alike() {
        let light = material(Inputs {
            mode: Mode::Light,
            ..Inputs::default()
        });
        let dark = material(Inputs::default());
        assert!(light.photochromic < 0.0 && dark.photochromic > 0.0);
        // Dark colours (a diff line's ground, a dim photo): within a
        // factor of 1.6 of dark glass either way.
        for c in [0x5f0000, 0x003f00, 0x3c1414, 0x458588, 0x00003f, 0xf4a6c0, 0x8ec8f0] {
            let c = Rgb::hex(c);
            let (l, d) = (chroma(glass_body(c, &light)), chroma(glass_body(c, &dark)));
            assert!(
                l <= d * 1.6 && l >= d / 1.6,
                "{c:?}: light {l:.3}, dark {d:.3}"
            );
        }
        // Vivid colours: never more than dark glass shows, where the old
        // gain clipped them to neon.
        for c in [0xff0000, 0xff00aa, 0x0000ff, 0xcc241d] {
            let c = Rgb::hex(c);
            let (l, d) = (chroma(glass_body(c, &light)), chroma(glass_body(c, &dark)));
            assert!(l <= d, "{c:?}: light {l:.3}, dark {d:.3}");
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

    /// And it does something: full clarity thins the body in every mode.
    #[test]
    fn clarity_lets_more_through() {
        for inputs in sample() {
            let base = material(inputs);
            let clear = material_at(inputs, 1.0);
            let dense = material_at(inputs, -1.0);
            assert!(
                clear.fill_alpha < base.fill_alpha - 0.05,
                "{inputs:?}: {} -> {}",
                base.fill_alpha,
                clear.fill_alpha
            );
            assert!(dense.fill_alpha > base.fill_alpha, "{inputs:?}");
            assert_eq!(material_at(inputs, 0.0), base);
        }
    }
}
