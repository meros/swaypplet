//! Colour: sRGB, OKLCH, and the one interpolation the scales use.
//!
//! The scales are built in OKLCH (`scales.rs`), emitted as sRGB hex
//! (`emit.rs`), and composited in linear light where the glass material is
//! modelled (`material.rs`).

/// sRGB, 0..1 per channel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgb(pub f64, pub f64, pub f64);

impl Rgb {
    pub const WHITE: Rgb = Rgb(1.0, 1.0, 1.0);
    pub const BLACK: Rgb = Rgb(0.0, 0.0, 0.0);

    pub const fn hex(v: u32) -> Rgb {
        Rgb(
            ((v >> 16) & 0xff) as f64 / 255.0,
            ((v >> 8) & 0xff) as f64 / 255.0,
            (v & 0xff) as f64 / 255.0,
        )
    }

    pub fn css(self) -> String {
        let c = |x: f64| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
        format!("#{:02x}{:02x}{:02x}", c(self.0), c(self.1), c(self.2))
    }

    pub(super) fn to_linear(self) -> [f64; 3] {
        [lin(self.0), lin(self.1), lin(self.2)]
    }

    pub(super) fn from_linear([r, g, b]: [f64; 3]) -> Rgb {
        Rgb(gam(r), gam(g), gam(b))
    }

    /// `self` at `alpha` over `under`, in sRGB as GTK composites.
    pub fn over(self, alpha: f64, under: Rgb) -> Rgb {
        let m = |a: f64, b: f64| a * alpha + b * (1.0 - alpha);
        Rgb(m(self.0, under.0), m(self.1, under.1), m(self.2, under.2))
    }
}

fn lin(c: f64) -> f64 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn gam(v: f64) -> f64 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.003_130_8 {
        12.92 * v
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

/// OKLCH: lightness 0..1, chroma, hue in degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Oklch(pub f64, pub f64, pub f64);

impl From<Rgb> for Oklch {
    fn from(c: Rgb) -> Oklch {
        let [r, g, b] = c.to_linear();
        let l = (0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b).cbrt();
        let m = (0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b).cbrt();
        let s = (0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b).cbrt();
        let ll = 0.210_454_255_3 * l + 0.793_617_785 * m - 0.004_072_046_8 * s;
        let a = 1.977_998_495_1 * l - 2.428_592_205 * m + 0.450_593_709_9 * s;
        let bb = 0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766 * s;
        Oklch(ll, a.hypot(bb), bb.atan2(a).to_degrees().rem_euclid(360.0))
    }
}

/// `o` in linear sRGB, unclamped: a channel outside 0..1 is out of gamut.
pub(super) fn oklch_linear(Oklch(l, c, h): Oklch) -> [f64; 3] {
    let (a, b) = (c * h.to_radians().cos(), c * h.to_radians().sin());
    let l_ = (l + 0.396_337_777_4 * a + 0.215_803_757_3 * b).powi(3);
    let m_ = (l - 0.105_561_345_8 * a - 0.063_854_172_8 * b).powi(3);
    let s_ = (l - 0.089_484_177_5 * a - 1.291_485_548 * b).powi(3);
    [
        4.076_741_662_1 * l_ - 3.307_711_591_3 * m_ + 0.230_969_929_2 * s_,
        -1.268_438_004_6 * l_ + 2.609_757_401_1 * m_ - 0.341_319_396_5 * s_,
        -0.004_196_086_3 * l_ - 0.703_418_614_7 * m_ + 1.707_614_701 * s_,
    ]
}

impl From<Oklch> for Rgb {
    fn from(o: Oklch) -> Rgb {
        Rgb::from_linear(oklch_linear(o))
    }
}

/// `a` toward `b` by `t`, in OKLCH, hue along the shorter arc. A grey end
/// takes the other end's hue, so a neutral fading to grey does not swing
/// through the wheel.
pub(super) fn mix(a: Oklch, b: Oklch, t: f64) -> Oklch {
    let (ha, hb) = match (a.1 < 0.002, b.1 < 0.002) {
        (true, false) => (b.2, b.2),
        (false, true) => (a.2, a.2),
        _ => (a.2, b.2),
    };
    let dh = ((hb - ha + 540.0) % 360.0) - 180.0;
    Oklch(
        a.0 + (b.0 - a.0) * t,
        a.1 + (b.1 - a.1) * t,
        (ha + dh * t).rem_euclid(360.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oklch_round_trips() {
        for c in [
            Rgb::hex(0x32302f),
            Rgb::hex(0xebdbb2),
            Rgb::hex(0x689d6a),
            Rgb::hex(0x076678),
        ] {
            let back: Rgb = Oklch::from(c).into();
            assert_eq!(back.css(), c.css());
        }
    }
}
