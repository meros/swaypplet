//! The glass material per mode (docs/design-system.md §4), and what its
//! body shows over a backdrop.

use super::{Contrast, Inputs, Mode, Rgb, scales};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Material {
    pub fill_color: Rgb,
    pub fill_alpha: f64,
    pub absorb: f64,
    /// > 0 a ceiling on luminance, < 0 a lift (a floor).
    pub photochromic: f64,
    pub edge_light: f64,
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
            edge_light: if high { 0.12 } else { 0.09 },
            frost: if high { 0.45 } else { 0.33 },
        },
        Mode::Light => Material {
            fill_color: s.neutral[1],
            fill_alpha: if high { 0.80 } else { 0.65 },
            absorb: if high { 0.20 } else { 0.25 },
            photochromic: if high { -0.50 } else { -0.40 },
            edge_light: if high { 0.18 } else { 0.14 },
            frost: if high { 0.55 } else { 0.45 },
        },
    }
}

/// What the glass body shows over `backdrop`, following
/// `liquid_glass.frag` on the flat interior of a card: absorption along the
/// full path, the photochromic ceiling or lift, then the body fill.
pub fn glass_body(backdrop: Rgb, m: &Material) -> Rgb {
    let k = (-m.absorb).exp();
    let mut t = backdrop.to_linear().map(|x| x * k);
    let lum = 0.2126 * t[0] + 0.7152 * t[1] + 0.0722 * t[2];
    if m.photochromic > 0.0 && lum > 0.0005 {
        let p = m.photochromic;
        let s = p * (1.0 - (-lum / p).exp()) / lum;
        t = t.map(|x| x * s);
    } else if m.photochromic < 0.0 {
        let f = -m.photochromic;
        let lifted = lum + f * (-lum / f).exp();
        t = if lum > 0.0005 {
            t.map(|x| x * lifted / lum)
        } else {
            [lifted; 3]
        };
    }
    let fill = m.fill_color.to_linear();
    let a = m.fill_alpha.clamp(0.0, 1.0);
    Rgb::from_linear([0, 1, 2].map(|i| t[i] * (1.0 - a) + fill[i] * a))
}
