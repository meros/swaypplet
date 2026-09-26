//! The two 12-step scales per input set, neutral and accent (§3.1).
//!
//! Neutral is interpolated in OKLCH between three anchors per preset and
//! mode, so a preset keeps its exact identity (the gruvbox ground `#32302f`,
//! its text `#ebdbb2`); accent is a pair per name, the bright gruvbox tone
//! for dark and the deep one for light. The wallpaper tint (`tint.rs`) moves
//! hues and never lightness.

use super::color::mix;
use super::{Inputs, Mode, Oklch, Rgb, apca, tint};

/// Steps 4–11 of the neutral scale, as the fraction of the way from step 3
/// to step 12.
const NEUTRAL_CURVE: [f64; 8] = [0.05, 0.10, 0.17, 0.26, 0.40, 0.52, 0.66, 0.83];

#[derive(Clone, Copy, Debug)]
pub struct Scales {
    pub neutral: [Rgb; 12],
    pub accent: [Rgb; 12],
    /// `--accent-bg`: step 9, or step 8 when 9 cannot carry a label (§3.1).
    pub accent_bg: Rgb,
    pub on_accent: Rgb,
}

pub fn scales(inputs: Inputs) -> Scales {
    let hue = inputs.tint.hue();
    let anchors = inputs.neutral.anchors(inputs.mode);
    let anchors = match hue {
        Some(h) if inputs.tint.casts_neutral() => anchors.map(|c| tint::cast(c, h)),
        _ => anchors,
    };
    let [a1, a3, a12] = anchors.map(Oklch::from);
    let mut n = [a1; 12];
    n[1] = mix(a1, a3, 0.5);
    n[2] = a3;
    for (k, t) in NEUTRAL_CURVE.iter().enumerate() {
        n[3 + k] = mix(a3, a12, *t);
    }
    n[11] = a12;
    let neutral = n.map(Rgb::from);

    let (dark, light) = inputs.accent.pair();
    let input = match inputs.mode {
        Mode::Dark => dark,
        Mode::Light => light,
    };
    // Tinted, the accent is the wallpaper's hue at the pair's own lightness
    // and chroma (§2.2).
    let input = hue.map_or(input, |h| tint::with_hue(input, h));
    let Oklch(al, ac, ah) = Oklch::from(input);
    // A moved hue can leave sRGB at a kept chroma; tinted steps give up
    // chroma to fit. Untinted ones clip as they always have, so `Tint::Off`
    // is the shipped token set byte for byte.
    let rgb = |o: Oklch| {
        if hue.is_some() {
            tint::to_rgb(o)
        } else {
            o.into()
        }
    };
    let mut accent = [input; 12];
    for (i, slot) in accent.iter_mut().enumerate() {
        *slot = match i {
            8 => input,
            9 => {
                let l = match inputs.mode {
                    Mode::Dark => (al + 0.05).min(0.9),
                    Mode::Light => (al - 0.05).max(0.2),
                };
                rgb(Oklch(l, ac, ah))
            }
            10 | 11 => {
                let l = match (inputs.mode, i) {
                    (Mode::Dark, 10) => 0.84,
                    (Mode::Dark, _) => 0.95,
                    (Mode::Light, 10) => 0.42,
                    (Mode::Light, _) => 0.30,
                };
                let c = ac.min(0.10) * if i == 11 { 0.5 } else { 1.0 };
                rgb(Oklch(l, c, ah))
            }
            _ => {
                let l = n[i].0;
                rgb(Oklch(l, ac.min(0.02 + 0.012 * i as f64), ah))
            }
        };
    }
    // The label on the accent: white or the mode's ink, whichever reads
    // better. Gruvbox's mid tones carry neither well on some hues (dark
    // yellow reaches Lc 54 either way), and there the solid steps down to 8.
    let ink = match inputs.mode {
        Mode::Dark => neutral[0],
        Mode::Light => neutral[11],
    };
    let best = |bg: Rgb| {
        let (w, k) = (apca(Rgb::WHITE, bg).abs(), apca(ink, bg).abs());
        if w >= k { (Rgb::WHITE, w) } else { (ink, k) }
    };
    let (accent_bg, on_accent) = match best(accent[8]) {
        (on, lc) if lc >= 60.0 => (accent[8], on),
        _ => (accent[7], best(accent[7]).0),
    };
    Scales {
        neutral,
        accent,
        accent_bg,
        on_accent,
    }
}

#[cfg(test)]
mod tests {
    use super::super::every;
    use super::*;
    use crate::tokens::Tint;

    #[test]
    fn the_gruvbox_anchors_are_exact() {
        let s = scales(Inputs::default());
        assert_eq!(s.neutral[0].css(), "#1d2021");
        assert_eq!(s.neutral[2].css(), "#32302f");
        assert_eq!(s.neutral[11].css(), "#ebdbb2");
        assert_eq!(s.accent[8].css(), "#689d6a");
    }

    #[test]
    fn neutral_lightness_runs_one_way() {
        for inputs in every::input() {
            let l: Vec<f64> = scales(inputs)
                .neutral
                .iter()
                .map(|c| Oklch::from(*c).0)
                .collect();
            for w in l.windows(2) {
                match inputs.mode {
                    Mode::Dark => assert!(w[1] >= w[0] - 1e-6, "{inputs:?}: {l:?}"),
                    Mode::Light => assert!(w[1] <= w[0] + 1e-6, "{inputs:?}: {l:?}"),
                }
            }
        }
    }

    /// §2.2: the accent is the wallpaper's colour, and keeps the lightness
    /// its contrast was measured at.
    #[test]
    fn a_tinted_accent_takes_the_wallpapers_hue() {
        for inputs in every::input() {
            let Some(h) = inputs.tint.hue() else { continue };
            let untinted = scales(Inputs {
                tint: Tint::Off,
                ..inputs
            });
            let s = scales(inputs);
            let (was, now) = (Oklch::from(untinted.accent[8]), Oklch::from(s.accent[8]));
            assert!(
                tint::difference(now.2, h).abs() < 2.0,
                "{inputs:?}: accent-9 at hue {:.1}",
                now.2
            );
            assert!(
                (was.0 - now.0).abs() < 0.01,
                "{inputs:?}: {was:?} -> {now:?}"
            );
        }
    }

    /// Accents leaves the greys exactly alone; Full casts them toward the
    /// wallpaper at their own lightness.
    #[test]
    fn only_full_reaches_the_neutrals() {
        for inputs in every::input() {
            let off = scales(Inputs {
                tint: Tint::Off,
                ..inputs
            });
            let s = scales(inputs);
            match inputs.tint {
                Tint::Off => {}
                Tint::Accents(_) => assert_eq!(s.neutral, off.neutral, "{inputs:?}"),
                Tint::Full(h) => {
                    for (a, b) in off.neutral.iter().zip(&s.neutral) {
                        let (a, b) = (Oklch::from(*a), Oklch::from(*b));
                        assert!((a.0 - b.0).abs() < 0.01, "{inputs:?}: {a:?} -> {b:?}");
                        assert!(b.1 <= tint::CAST.1 + 0.002, "{inputs:?}: {b:?}");
                    }
                    let ground = Oklch::from(s.neutral[2]);
                    assert!(
                        tint::difference(ground.2, f64::from(h)).abs() < 3.0,
                        "{inputs:?}: the ground is at hue {:.1}",
                        ground.2
                    );
                }
            }
        }
    }
}
