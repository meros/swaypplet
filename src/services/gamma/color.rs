//! The colour arithmetic of the night light, pure: a colour temperature to a
//! white point, the white point to a gamma ramp, and the sun's elevation to
//! a temperature.

use crate::settings::store::{NightLight, NightSchedule};

/// Above this elevation it is day: the same +3° the automatic mode switches
/// to light at (`theme::sun::BAND`).
pub const DAY_ELEVATION: f64 = 3.0;

/// Below this it is night: the end of civil twilight.
pub const NIGHT_ELEVATION: f64 = -6.0;

/// The white point of a black body at `kelvin`, as linear-light sRGB
/// multipliers, normalised so 6500 K is exactly `[1, 1, 1]` and the largest
/// channel never exceeds 1.
///
/// The Planckian locus in CIE xy by Kim et al.'s cubic spline (valid
/// 1667–25000 K), to XYZ at unit luminance, to linear sRGB. Normalising by
/// 6500 K's own value rather than D65 makes the day setting the identity,
/// so "day" never tints the screen by the few percent between the locus
/// and D65.
pub fn white_point(kelvin: f64) -> [f64; 3] {
    let raw = |k: f64| xyz_to_linear_srgb(planck_xy(k.clamp(1667.0, 25_000.0)));
    let day = raw(f64::from(NightLight::DAY_K));
    let w = raw(kelvin);
    let n = [w[0] / day[0], w[1] / day[1], w[2] / day[2]].map(|c| c.max(0.0));
    let max = n.iter().copied().fold(0.0, f64::max);
    if max <= 0.0 {
        [1.0; 3]
    } else {
        n.map(|c| c / max)
    }
}

fn planck_xy(t: f64) -> (f64, f64) {
    let (t2, t3) = (t * t, t * t * t);
    let x = if t <= 4000.0 {
        -0.266_123_9e9 / t3 - 0.234_358_9e6 / t2 + 0.877_695_6e3 / t + 0.179_910
    } else {
        -3.025_846_9e9 / t3 + 2.107_037_9e6 / t2 + 0.222_634_7e3 / t + 0.240_390
    };
    let y = if t <= 2222.0 {
        -1.106_381_4 * x.powi(3) - 1.348_110_20 * x.powi(2) + 2.185_558_32 * x - 0.202_196_83
    } else if t <= 4000.0 {
        -0.954_947_6 * x.powi(3) - 1.374_185_93 * x.powi(2) + 2.091_370_15 * x - 0.167_488_67
    } else {
        3.081_758_0 * x.powi(3) - 5.873_386_70 * x.powi(2) + 3.751_129_97 * x - 0.370_014_83
    };
    (x, y)
}

fn xyz_to_linear_srgb((x, y): (f64, f64)) -> [f64; 3] {
    let (cx, cz) = (x / y, (1.0 - x - y) / y);
    [
        3.240_454_2 * cx - 1.537_138_5 - 0.498_531_4 * cz,
        -0.969_266_0 * cx + 1.876_010_8 + 0.041_556_0 * cz,
        0.055_643_4 * cx - 0.204_025_9 + 1.057_225_2 * cz,
    ]
}

fn decode(v: f64) -> f64 {
    if v <= 0.040_45 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

fn encode(v: f64) -> f64 {
    if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

/// The gamma table for one output: `size` entries each for red, green and
/// blue, in that order, as the protocol wants them. The multiplier is
/// applied in linear light, so a grey stays the same grey at every
/// brightness instead of drifting in hue toward the dark end, which is what
/// multiplying the encoded values would do.
pub fn ramp(size: usize, white: [f64; 3]) -> Vec<u16> {
    let mut out = Vec::with_capacity(size * 3);
    let last = size.saturating_sub(1).max(1) as f64;
    for w in white {
        for i in 0..size {
            let v = encode(decode(i as f64 / last) * w);
            out.push((v.clamp(0.0, 1.0) * 65_535.0).round() as u16);
        }
    }
    out
}

/// The temperature the settings ask for at the sun's `elevation`. Off, or
/// no location to follow, is the day: no change.
///
/// Between night and day the temperature moves linearly in mired (a million
/// over kelvin), the scale on which equal steps look like equal steps; in
/// kelvin the warm end would rush and the cool end crawl.
pub fn target_kelvin(settings: NightLight, elevation: Option<f64>) -> f64 {
    let day = f64::from(NightLight::DAY_K);
    let night = f64::from(settings.night_k.clamp(NightLight::MIN_K, NightLight::DAY_K));
    if !settings.enabled {
        return day;
    }
    let t = match settings.schedule {
        NightSchedule::Always => 0.0,
        NightSchedule::Sun => match elevation {
            Some(e) => ((e - NIGHT_ELEVATION) / (DAY_ELEVATION - NIGHT_ELEVATION)).clamp(0.0, 1.0),
            None => return day,
        },
    };
    let mired = 1e6 / night + (1e6 / day - 1e6 / night) * t;
    1e6 / mired
}

/// Kelvin to mired, the scale the ramps step on.
pub fn mired(kelvin: f64) -> f64 {
    1e6 / kelvin
}

/// Whether `kelvin` is the day, where the night light holds no control at
/// all and the compositor's own tables are back.
pub fn is_day(kelvin: f64) -> bool {
    (mired(kelvin) - mired(f64::from(NightLight::DAY_K))).abs() < 0.25
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_day_is_the_identity() {
        let w = white_point(6500.0);
        for c in w {
            assert!((c - 1.0).abs() < 1e-9, "{w:?}");
        }
        let r = ramp(256, w);
        for (i, v) in r[..256].iter().enumerate() {
            let want = (i as f64 / 255.0 * 65_535.0).round() as i64;
            assert!((i64::from(*v) - want).abs() <= 1, "{i}: {v} vs {want}");
        }
    }

    #[test]
    fn warmer_is_redder_and_never_above_one() {
        let mut last_blue = 1.0;
        for k in (1700..=6500).rev().step_by(100) {
            let w = white_point(f64::from(k));
            assert!(
                w.iter().all(|c| (0.0..=1.0 + 1e-9).contains(c)),
                "{k}: {w:?}"
            );
            assert!(
                (w[0] - 1.0).abs() < 1e-9,
                "{k}: red is the brightest channel {w:?}"
            );
            assert!(w[2] <= last_blue + 1e-9, "{k}: blue rose");
            last_blue = w[2];
        }
        // 3500 K, gammastep's night here: blue well down, green some.
        let w = white_point(3500.0);
        assert!(w[2] < 0.6 && w[1] < 0.9 && w[1] > w[2], "{w:?}");
    }

    #[test]
    fn a_ramp_has_three_channels_and_runs_one_way() {
        let r = ramp(1024, white_point(3000.0));
        assert_eq!(r.len(), 3 * 1024);
        for ch in r.chunks(1024) {
            assert_eq!(ch[0], 0);
            assert!(ch.windows(2).all(|w| w[1] >= w[0]));
        }
        // Red at full, blue held down at the top.
        assert_eq!(r[1023], 65_535);
        assert!(r[3 * 1024 - 1] < 60_000);
    }

    #[test]
    fn the_sun_moves_the_temperature_one_way_and_within_bounds() {
        let s = NightLight::default();
        let mut last = 0.0;
        for tenth in -300..=300 {
            let e = f64::from(tenth) / 10.0;
            let k = target_kelvin(s, Some(e));
            assert!((3499.0..=6500.0 + 1e-6).contains(&k), "{e}: {k}");
            assert!(k >= last - 1e-9, "{e}: {k} < {last}");
            last = k;
        }
        assert!((target_kelvin(s, Some(-10.0)) - 3500.0).abs() < 1e-6);
        assert!(is_day(target_kelvin(s, Some(10.0))));
    }

    #[test]
    fn off_and_nowhere_are_the_day_and_always_is_the_night() {
        let off = NightLight {
            enabled: false,
            ..NightLight::default()
        };
        assert!(is_day(target_kelvin(off, Some(-30.0))));
        assert!(is_day(target_kelvin(NightLight::default(), None)));
        let always = NightLight {
            schedule: NightSchedule::Always,
            night_k: 2700,
            ..NightLight::default()
        };
        assert!((target_kelvin(always, Some(40.0)) - 2700.0).abs() < 1e-6);
    }
}
