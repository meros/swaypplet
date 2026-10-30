//! The semantic colour sets beside the scales: status (§3.3), the
//! categorical identity colours (§3.2) and the text and line levels.

use super::{Contrast, Inputs, Mode, Oklch, Rgb, tint};

#[derive(Clone, Copy, Debug)]
pub struct Status {
    pub success: Rgb,
    pub success_bg: Rgb,
    pub warning: Rgb,
    pub warning_bg: Rgb,
    pub danger: Rgb,
    pub danger_bg: Rgb,
}

/// The status set for `inputs`: the shipped one per mode, each colour
/// harmonised toward the nearest wallpaper hue by at most `tint::STATUS_CAP`
/// under a tint.
pub fn status(inputs: Inputs) -> Status {
    let st = shipped_status(inputs.mode);
    let Some(p) = inputs.tint.palette() else {
        return st;
    };
    let t = |c: Rgb| tint::harmonized(c, p);
    Status {
        success: t(st.success),
        success_bg: t(st.success_bg),
        warning: t(st.warning),
        warning_bg: t(st.warning_bg),
        danger: t(st.danger),
        danger_bg: t(st.danger_bg),
    }
}

fn shipped_status(mode: Mode) -> Status {
    match mode {
        Mode::Dark => Status {
            success: Rgb::hex(0x8ec07c),
            success_bg: Rgb::hex(0x689d6a),
            warning: Rgb::hex(0xfabd2f),
            // Not #d79921: white on it reaches Lc 54.
            warning_bg: Rgb::hex(0xb57614),
            danger: Rgb::hex(0xfb6a5a),
            danger_bg: Rgb::hex(0xcc241d),
        },
        Mode::Light => Status {
            success: Rgb::hex(0x427b58),
            success_bg: Rgb::hex(0x427b58),
            warning: Rgb::hex(0x9a5b04),
            warning_bg: Rgb::hex(0xb57614),
            danger: Rgb::hex(0x9d0006),
            danger_bg: Rgb::hex(0xcc241d),
        },
    }
}

/// Text on a status fill.
pub const ON_STATUS: Rgb = Rgb::WHITE;

/// Identity colours: which task a workspace belongs to (t1–t4), which app a
/// notification came from (a1–a6). Never state, never decoration. Per slot:
/// the readable tone (text, dots, rails) and the fill (solid chips).
///
/// Under a tint all six turn by one angle (`tint::categorical_turn`): slot 1
/// onto the wallpaper's primary, nudged to bring another slot onto its
/// secondary. One angle, so they stay as far apart as they were (§2.2).
pub fn categorical(inputs: Inputs) -> [(Rgb, Rgb); 6] {
    let mode = inputs.mode;
    let tinted = inputs.tint.hue().is_some();
    // The readable tone is lifted (dark) or deepened (light) to a lightness
    // that clears Lc 45 on the glass, keeping the gruvbox hue and chroma
    // (tinted, giving up chroma rather than clipping; see `scales`).
    let legible = |c: Rgb| {
        let Oklch(l, ch, h) = Oklch::from(c);
        let l = match mode {
            Mode::Dark => l.max(0.80),
            Mode::Light => l.min(0.50),
        };
        let o = Oklch(l, ch, h);
        if tinted { tint::to_rgb(o) } else { o.into() }
    };
    let raw = raw_categorical(mode);
    let raw = match inputs.tint.palette() {
        Some(p) => {
            let delta = tint::categorical_turn(raw.map(|(_, f)| Oklch::from(f).2), p);
            raw.map(|(t, f)| (tint::rotated(t, delta), tint::rotated(f, delta)))
        }
        None => raw,
    };
    raw.map(|(t, f)| (legible(t), f))
}

fn raw_categorical(mode: Mode) -> [(Rgb, Rgb); 6] {
    match mode {
        // gruvbox bright tones to read on dark glass, neutral tones to fill.
        Mode::Dark => [
            (Rgb::hex(0x8ec07c), Rgb::hex(0x689d6a)),
            (Rgb::hex(0xfabd2f), Rgb::hex(0xb57614)),
            (Rgb::hex(0x83a598), Rgb::hex(0x458588)),
            (Rgb::hex(0xd3869b), Rgb::hex(0xb16286)),
            (Rgb::hex(0xb8bb26), Rgb::hex(0x79740e)),
            (Rgb::hex(0xfe8019), Rgb::hex(0xaf3a03)),
        ],
        // gruvbox deep tones to read on light glass.
        Mode::Light => [
            (Rgb::hex(0x427b58), Rgb::hex(0x427b58)),
            (Rgb::hex(0xa06a0e), Rgb::hex(0xb57614)),
            (Rgb::hex(0x076678), Rgb::hex(0x076678)),
            (Rgb::hex(0x8f3f71), Rgb::hex(0x8f3f71)),
            (Rgb::hex(0x6a6608), Rgb::hex(0x79740e)),
            (Rgb::hex(0xaf3a03), Rgb::hex(0xaf3a03)),
        ],
    }
}

/// `--fg-muted`, `--fg-faint`, `--border-subtle`, `--border-strong` as the
/// share of `--neutral-12`.
#[derive(Clone, Copy, Debug)]
pub struct Levels {
    pub muted: f64,
    pub faint: f64,
    pub disabled: f64,
    pub border_subtle: f64,
    pub border_strong: f64,
}

pub fn levels(mode: Mode, contrast: Contrast) -> Levels {
    match (mode, contrast) {
        (Mode::Dark, Contrast::Standard) => Levels {
            muted: 0.84,
            faint: 0.67,
            disabled: 0.40,
            border_subtle: 0.08,
            border_strong: 0.16,
        },
        (Mode::Dark, Contrast::High) => Levels {
            muted: 0.88,
            faint: 0.72,
            disabled: 0.45,
            border_subtle: 0.16,
            border_strong: 0.30,
        },
        (Mode::Light, Contrast::Standard) => Levels {
            muted: 0.82,
            faint: 0.62,
            disabled: 0.40,
            border_subtle: 0.10,
            border_strong: 0.20,
        },
        (Mode::Light, Contrast::High) => Levels {
            muted: 0.88,
            faint: 0.72,
            disabled: 0.45,
            border_subtle: 0.16,
            border_strong: 0.30,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::super::every;
    use super::*;
    use crate::tokens::Tint;

    fn hue_of(c: Rgb) -> f64 {
        Oklch::from(c).2
    }

    /// Red is a word, not a decoration: a status colour turns toward the
    /// wallpaper by at most the cap, whatever the hue.
    #[test]
    fn status_stays_what_it_says() {
        for mode in Mode::ALL {
            let shipped = shipped_status(mode);
            for tint in every::tint() {
                let st = status(Inputs {
                    mode,
                    tint,
                    ..Inputs::default()
                });
                for (name, a, b) in [
                    ("success", shipped.success, st.success),
                    ("success-bg", shipped.success_bg, st.success_bg),
                    ("warning", shipped.warning, st.warning),
                    ("warning-bg", shipped.warning_bg, st.warning_bg),
                    ("danger", shipped.danger, st.danger),
                    ("danger-bg", shipped.danger_bg, st.danger_bg),
                ] {
                    let moved = tint::difference(hue_of(a), hue_of(b)).abs();
                    assert!(
                        moved <= tint::STATUS_CAP + 1.0,
                        "{mode:?} {tint:?}: {name} turned {moved:.1}°"
                    );
                }
            }
        }
    }

    /// The categorical colours are a channel: one app, one colour. A tint
    /// turns them together, so they stay as far apart as gruvbox put them.
    #[test]
    fn the_categorical_hues_stay_apart() {
        let closest = |set: [(Rgb, Rgb); 6]| {
            let hues = set.map(|(_, fill)| hue_of(fill));
            let mut min: f64 = 360.0;
            for (i, a) in hues.iter().enumerate() {
                for b in &hues[i + 1..] {
                    min = min.min(tint::difference(*a, *b).abs());
                }
            }
            min
        };
        for mode in Mode::ALL {
            let at = |tint| {
                categorical(Inputs {
                    mode,
                    tint,
                    ..Inputs::default()
                })
            };
            let floor = closest(at(Tint::Off));
            for tint in every::tint() {
                let got = closest(at(tint));
                assert!(
                    got >= floor - 2.0,
                    "{mode:?} {tint:?}: two categorical hues {got:.1}° apart, shipped {floor:.1}°"
                );
            }
        }
    }
}
