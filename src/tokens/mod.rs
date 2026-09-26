//! The design tokens, generated from the theme inputs (docs/design-system.md).
//!
//! Every colour swaypplet shows comes from here: two 12-step scales per mode
//! (neutral and accent), a fixed status set, and the semantic tier the
//! stylesheet is allowed to use, emitted as CSS custom properties on
//! `:root`. The same numbers are available to Rust (Cairo drawing, the
//! glass material values), so nothing reads a colour back out of the CSS.
//!
//! The scales are built in OKLCH. Neutral is interpolated between three
//! anchors per preset and mode, so a preset keeps its exact identity (the
//! gruvbox ground `#32302f`, its text `#ebdbb2`); accent is a pair per name,
//! the bright gruvbox tone for dark and the deep one for light. The
//! wallpaper tint is one more input (`tint`, docs/design-system.md §2.2):
//! it moves hues and never lightness.
//!
//! The contrast requirements of §5 are tests at the bottom: every text token
//! over the glass body, computed through the material the way
//! `liquid_glass.frag` does it, over white, mid-grey and black behind.

pub mod motion;
pub mod sun;
pub mod tint;

pub use tint::Tint;

use std::fmt::Write as _;

// ── Colour ──────────────────────────────────────────────────────────────

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

    fn to_linear(self) -> [f64; 3] {
        [lin(self.0), lin(self.1), lin(self.2)]
    }

    fn from_linear([r, g, b]: [f64; 3]) -> Rgb {
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
fn oklch_linear(Oklch(l, c, h): Oklch) -> [f64; 3] {
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
fn mix(a: Oklch, b: Oklch, t: f64) -> Oklch {
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

// ── Inputs ──────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Dark,
    Light,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Contrast {
    #[default]
    Standard,
    High,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Accent {
    #[default]
    Aqua,
    Yellow,
    Blue,
    Purple,
    Orange,
    Red,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Neutral {
    #[default]
    Gruvbox,
    Slate,
    Pure,
}

impl Mode {
    pub const ALL: [Mode; 2] = [Mode::Dark, Mode::Light];
}
impl Contrast {
    pub const ALL: [Contrast; 2] = [Contrast::Standard, Contrast::High];
}
impl Accent {
    pub const ALL: [Accent; 6] = [
        Accent::Aqua,
        Accent::Yellow,
        Accent::Blue,
        Accent::Purple,
        Accent::Orange,
        Accent::Red,
    ];

    /// Step 9 in each mode: gruvbox's bright tone for dark, its deep tone
    /// for light.
    pub fn pair(self) -> (Rgb, Rgb) {
        match self {
            Accent::Aqua => (Rgb::hex(0x689d6a), Rgb::hex(0x427b58)),
            Accent::Yellow => (Rgb::hex(0xd79921), Rgb::hex(0xb57614)),
            Accent::Blue => (Rgb::hex(0x458588), Rgb::hex(0x076678)),
            Accent::Purple => (Rgb::hex(0xb16286), Rgb::hex(0x8f3f71)),
            Accent::Orange => (Rgb::hex(0xd65d0e), Rgb::hex(0xaf3a03)),
            Accent::Red => (Rgb::hex(0xcc241d), Rgb::hex(0x9d0006)),
        }
    }
}
impl Neutral {
    pub const ALL: [Neutral; 3] = [Neutral::Gruvbox, Neutral::Slate, Neutral::Pure];

    /// Steps 1, 3 and 12 in a mode.
    fn anchors(self, mode: Mode) -> [Rgb; 3] {
        match (self, mode) {
            (Neutral::Gruvbox, Mode::Dark) => {
                [Rgb::hex(0x1d2021), Rgb::hex(0x32302f), Rgb::hex(0xebdbb2)]
            }
            (Neutral::Gruvbox, Mode::Light) => {
                // Not gruvbox's own cream (#fbf1c7 / #ebdbb2): as glass that
                // read as yellowed paper. The same warm hue at a fraction of
                // the chroma: white glass with a trace of warmth.
                [Rgb::hex(0xf8f7f4), Rgb::hex(0xebe9e4), Rgb::hex(0x282624)]
            }
            (Neutral::Slate, Mode::Dark) => {
                [Rgb::hex(0x15181c), Rgb::hex(0x262b31), Rgb::hex(0xe3e8ee)]
            }
            (Neutral::Slate, Mode::Light) => {
                [Rgb::hex(0xf7f9fb), Rgb::hex(0xe6ebf0), Rgb::hex(0x1f252b)]
            }
            (Neutral::Pure, Mode::Dark) => {
                [Rgb::hex(0x141414), Rgb::hex(0x262626), Rgb::hex(0xebebeb)]
            }
            (Neutral::Pure, Mode::Light) => {
                [Rgb::hex(0xfafafa), Rgb::hex(0xe8e8e8), Rgb::hex(0x1f1f1f)]
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Inputs {
    pub mode: Mode,
    pub accent: Accent,
    pub neutral: Neutral,
    pub contrast: Contrast,
    /// The Motion setting as a percentage of every duration: 100, 50 or 0.
    pub motion: u8,
    /// The wallpaper's hue and how far it reaches (§2.2).
    pub tint: Tint,
}

impl Default for Inputs {
    fn default() -> Inputs {
        Inputs {
            mode: Mode::Dark,
            accent: Accent::Aqua,
            neutral: Neutral::Gruvbox,
            contrast: Contrast::Standard,
            motion: 100,
            tint: Tint::Off,
        }
    }
}

// ── Scales ──────────────────────────────────────────────────────────────

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

// ── Status ──────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug)]
pub struct Status {
    pub success: Rgb,
    pub success_bg: Rgb,
    pub warning: Rgb,
    pub warning_bg: Rgb,
    pub danger: Rgb,
    pub danger_bg: Rgb,
}

/// The status set for `inputs`: the shipped one per mode, harmonised toward
/// the wallpaper by at most `tint::STATUS_CAP` under a tint.
pub fn status(inputs: Inputs) -> Status {
    let st = shipped_status(inputs.mode);
    let Some(h) = inputs.tint.hue() else {
        return st;
    };
    let t = |c: Rgb| tint::harmonized(c, h);
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

// ── Categorical ─────────────────────────────────────────────────────────

/// Identity colours: which task a workspace belongs to (t1–t4), which app a
/// notification came from (a1–a6). Never state, never decoration. Per slot:
/// the readable tone (text, dots, rails) and the fill (solid chips).
///
/// Under a tint all six turn by the one angle that puts slot 1 on the
/// wallpaper's hue, so they stay as far apart as they were (§2.2).
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
    let raw = match inputs.tint.hue() {
        Some(h) => {
            let delta = tint::difference(Oklch::from(raw[0].1).2, h);
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

// ── Text levels ─────────────────────────────────────────────────────────

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

// ── The glass material per mode (§4) ────────────────────────────────────

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

// ── APCA 0.0.98G ────────────────────────────────────────────────────────

/// Lc of `text` on `bg`, signed as APCA signs it (positive: dark text).
pub fn apca(text: Rgb, bg: Rgb) -> f64 {
    let y = |c: Rgb| {
        0.212_672_9 * c.0.powf(2.4) + 0.715_152_2 * c.1.powf(2.4) + 0.072_175 * c.2.powf(2.4)
    };
    let clamp = |y: f64| {
        if y < 0.022 {
            y + (0.022 - y).powf(1.414)
        } else {
            y
        }
    };
    let (yt, yb) = (clamp(y(text)), clamp(y(bg)));
    if (yb - yt).abs() < 0.0005 {
        return 0.0;
    }
    let out = if yb > yt {
        let s = (yb.powf(0.56) - yt.powf(0.57)) * 1.14;
        if s < 0.1 { 0.0 } else { s - 0.027 }
    } else {
        let s = (yb.powf(0.65) - yt.powf(0.62)) * 1.14;
        if s > -0.1 { 0.0 } else { s + 0.027 }
    };
    out * 100.0
}

// ── Space, type, radius, motion ─────────────────────────────────────────

/// `--space-1` … `--space-7`. Box spacing and margins in Rust use these.
pub const SPACE: [i32; 7] = [2, 4, 8, 12, 16, 24, 32];

/// `--space-n`, 1-based as in the stylesheet; 0 is no space at all.
pub const fn space(n: usize) -> i32 {
    if n == 0 { 0 } else { SPACE[n - 1] }
}

pub const TYPE: [(&str, u32); 8] = [
    ("caption", 11),
    ("label", 12),
    ("body", 13),
    ("title-sm", 15),
    ("title", 18),
    ("display-sm", 28),
    ("display", 36),
    ("hero", 96),
];

pub const RADIUS: [(&str, u32); 5] = [
    ("control", 6),
    ("tile", 10),
    ("thin", 14),
    ("card", 18),
    ("pill", 999),
];

pub const DURATION: [(&str, u32); 5] = [
    ("fast", 150),
    ("standard", 200),
    ("emphasis", 300),
    ("spatial", 400),
    ("long", 500),
];

/// The fill key: what a glass card paints so the compositor finds its
/// shape, and drops. `glass.nix` `fillKey`; the cross-repo guard checks it.
pub const SURFACE_KEY: (Rgb, f64) = (Rgb::hex(0x32302f), 0.5);

// ── Emission ────────────────────────────────────────────────────────────

fn mixed(color: Rgb, share: f64) -> String {
    format!(
        "color-mix(in srgb, {} {}%, transparent)",
        color.css(),
        (share * 100.0).round()
    )
}

/// `tokens.css` for `inputs`: one `:root` block holding the primitives and
/// the semantic tier. The rules in `data/style.css` use only the latter.
pub fn css(inputs: Inputs) -> String {
    let s = scales(inputs);
    let st = status(inputs);
    let lv = levels(inputs.mode, inputs.contrast);
    let m = material(inputs);
    let fg = s.neutral[11];
    let mut out = String::from(
        "/* Generated by src/tokens.rs from the theme inputs; see \
         docs/design-system.md. Do not edit. */\n:root {\n",
    );
    let mut put = |name: &str, value: String| {
        let _ = writeln!(out, "  --{name}: {value};");
    };

    // Primitives: private to this block.
    for (i, c) in s.neutral.iter().enumerate() {
        put(&format!("neutral-{}", i + 1), c.css());
    }
    for (i, c) in s.accent.iter().enumerate() {
        put(&format!("accent-{}", i + 1), c.css());
    }

    // Surfaces.
    put("surface-key", mixed(SURFACE_KEY.0, SURFACE_KEY.1));
    put("surface-raised", s.neutral[3].css());
    put("scrim", "rgb(0 0 0 / 0.2)".into());

    // Text.
    put("fg", fg.css());
    put("fg-muted", mixed(fg, lv.muted));
    put("fg-faint", mixed(fg, lv.faint));
    put("fg-disabled", mixed(fg, lv.disabled));

    // Fills on glass, and the state overlays: currentColor at the use site.
    put(
        "fill-1",
        "color-mix(in srgb, currentColor 6%, transparent)".into(),
    );
    put(
        "fill-2",
        "color-mix(in srgb, currentColor 10%, transparent)".into(),
    );
    put(
        "fill-3",
        "color-mix(in srgb, currentColor 16%, transparent)".into(),
    );
    put(
        "state-hover",
        "color-mix(in srgb, currentColor 7%, transparent)".into(),
    );
    put(
        "state-pressed",
        "color-mix(in srgb, currentColor 16%, transparent)".into(),
    );
    put(
        "state-selected",
        "color-mix(in srgb, currentColor 10%, transparent)".into(),
    );

    // Lines.
    put("border-subtle", mixed(fg, lv.border_subtle));
    put("border-strong", mixed(fg, lv.border_strong));
    put("focus-ring", mixed(s.accent[10], 0.7));

    // Accent.
    put("accent-bg", s.accent_bg.css());
    put("accent-bg-hover", s.accent[9].css());
    put("on-accent", s.on_accent.css());
    // The second line on an accent fill (a tile's state under its name).
    put("on-accent-muted", mixed(s.on_accent, 0.8));
    put("accent", s.accent[10].css());
    put("accent-tint", mixed(s.accent[8], 0.16));

    // Status.
    put("success", st.success.css());
    put("success-bg", st.success_bg.css());
    put("warning", st.warning.css());
    put("warning-bg", st.warning_bg.css());
    put("danger", st.danger.css());
    put("danger-bg", st.danger_bg.css());
    put("on-status", ON_STATUS.css());
    put("danger-tint", mixed(st.danger_bg, 0.16));
    put("warning-tint", mixed(st.warning_bg, 0.16));
    put("success-tint", mixed(st.success_bg, 0.16));

    // Categorical: identity only (§3.2).
    for (i, (text, fill)) in categorical(inputs).iter().enumerate() {
        put(&format!("cat-{}", i + 1), text.css());
        put(&format!("cat-{}-bg", i + 1), fill.css());
        put(&format!("cat-{}-tint", i + 1), mixed(*fill, 0.30));
    }

    // Elevation.
    put(
        "shadow-float",
        match inputs.mode {
            Mode::Dark => "none".into(),
            Mode::Light => "0 10px 30px rgb(20 16 10 / 0.16)".into(),
        },
    );
    put("shadow-solid", "0 8px 32px rgb(0 0 0 / 0.45)".into());

    // Type, space, shape, motion.
    for (name, px) in TYPE {
        put(&format!("type-{name}"), format!("{px}px"));
    }
    put("w-light", "300".into());
    put("w-regular", "400".into());
    put("w-strong", "600".into());
    put("w-heavy", "700".into());
    for (i, px) in SPACE.iter().enumerate() {
        put(&format!("space-{}", i + 1), format!("{px}px"));
    }
    for (name, px) in RADIUS {
        put(&format!("radius-{name}"), format!("{px}px"));
    }
    // Motion, scaled by the Motion setting. Zero is one frame's worth, not
    // zero: GTK skips a transition of 0 but the end state must still land.
    let scaled = |ms: f64| (ms * f64::from(inputs.motion) / 100.0).max(1.0).round();
    for (name, ms) in DURATION {
        put(
            &format!("dur-{name}"),
            format!("{}ms", scaled(f64::from(ms))),
        );
    }
    put("ease-standard", motion::STANDARD.css());
    put("ease-decelerate", motion::DECELERATE.css());
    put("ease-accelerate", motion::ACCELERATE.css());
    for m in motion::ALL {
        put(
            &format!("motion-{}", m.name),
            format!("{}ms {}", scaled(m.ms), m.curve.css()),
        );
    }

    // Families (§3.4).
    put(
        "font",
        "\"UbuntuSans Nerd Font\", \"Ubuntu Sans\", \"Noto Sans\", sans-serif".into(),
    );
    put(
        "mono",
        "\"JetBrainsMono Nerd Font\", \"JetBrains Mono\", monospace".into(),
    );

    // Component tokens (§3.9).
    put("control-height", "30px".into());
    put("control-height-small", "24px".into());
    put("field-height", "34px".into());
    put("chip-height", "26px".into());
    put("menu-item-height", "32px".into());
    put("row-height", "40px".into());
    put("tile-height", "52px".into());
    put("track-height", "6px".into());
    put(
        "knob",
        match inputs.mode {
            Mode::Dark => fg.css(),
            Mode::Light => Rgb::WHITE.css(),
        },
    );
    let _ = m;
    out.push_str("}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wallpaper hues the tinted tests run at: every 5° round the
    /// circle. The tint is whole degrees, so every hue it can take is one
    /// edit away from exhaustive (`(0..360)`), which passes too; 5° keeps a
    /// debug build's run short.
    fn hues() -> impl Iterator<Item = u16> {
        (0..72).map(|i| i * 5)
    }

    /// Off, and both reaches at every hue of [`hues`].
    fn every_tint() -> Vec<Tint> {
        let mut v = vec![Tint::Off];
        for h in hues() {
            v.push(Tint::Accents(h));
            v.push(Tint::Full(h));
        }
        v
    }

    /// Every combination of the inputs that decide a colour: 144 untinted
    /// (2 modes, 6 accents, 3 neutrals, 2 contrasts), each at every tint.
    fn every_input() -> Vec<Inputs> {
        let mut v = Vec::new();
        for mode in Mode::ALL {
            for accent in Accent::ALL {
                for neutral in Neutral::ALL {
                    for contrast in Contrast::ALL {
                        for tint in every_tint() {
                            v.push(Inputs {
                                mode,
                                accent,
                                neutral,
                                contrast,
                                motion: 100,
                                tint,
                            });
                        }
                    }
                }
            }
        }
        v
    }

    const BEHIND: [Rgb; 3] = [Rgb::WHITE, Rgb(0.5, 0.5, 0.5), Rgb::BLACK];

    #[test]
    fn space_zero_is_nothing() {
        assert_eq!(space(0), 0);
        assert_eq!(space(1), 2);
        assert_eq!(space(7), 32);
    }

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
        for inputs in every_input() {
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

    /// §5: every text token, through the material, over what can be behind
    /// the glass. Grey and black backdrops (a desktop, a terminal) are held
    /// to the full targets; a pure white page, which the dark material
    /// cannot fully overcome, to 15 less, and to 5 less at high contrast.
    #[test]
    fn text_meets_its_contrast_over_every_backdrop() {
        let mut failures = Vec::new();
        for inputs in every_input() {
            let s = scales(inputs);
            let lv = levels(inputs.mode, inputs.contrast);
            let m = material(inputs);
            let fg = s.neutral[11];
            for behind in BEHIND {
                let ground = glass_body(behind, &m);
                let slack = match (behind == Rgb::WHITE, inputs.contrast) {
                    (false, _) => 0.0,
                    (true, Contrast::Standard) => 15.0,
                    (true, Contrast::High) => 5.0,
                };
                for (name, color, alpha, need) in [
                    ("fg", fg, 1.0, 75.0 - slack),
                    ("fg-muted", fg, lv.muted, 60.0 - slack),
                    ("fg-faint", fg, lv.faint, 45.0 - slack),
                    ("accent", s.accent[10], 1.0, 60.0 - slack),
                ] {
                    let lc = apca(color.over(alpha, ground), ground).abs();
                    if lc < need {
                        failures.push(format!(
                            "{inputs:?} {name} over {} behind: Lc {lc:.0} < {need}",
                            behind.css()
                        ));
                    }
                }
            }
        }
        assert!(
            failures.is_empty(),
            "{} failures:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }

    #[test]
    fn on_colours_meet_their_contrast() {
        let mut failures = Vec::new();
        for inputs in every_input() {
            let s = scales(inputs);
            let lc = apca(s.on_accent, s.accent_bg).abs();
            if lc < 60.0 {
                failures.push(format!("{inputs:?} on-accent: Lc {lc:.0}"));
            }
        }
        for inputs in every_input() {
            let st = status(inputs);
            for (name, bg) in [
                ("success", st.success_bg),
                ("warning", st.warning_bg),
                ("danger", st.danger_bg),
            ] {
                let lc = apca(ON_STATUS, bg).abs();
                if lc < 60.0 {
                    failures.push(format!("{inputs:?} on-status on {name}: Lc {lc:.0}"));
                }
            }
        }
        assert!(
            failures.is_empty(),
            "{} failures:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }

    /// A categorical colour marks identity with a dot, a rail or a short
    /// label; Lc 45 over grey and black behind, the large-text level.
    #[test]
    fn categorical_colours_read_on_the_glass() {
        let mut failures = Vec::new();
        // The accent does not reach the categorical set; the neutral does,
        // through the glass body.
        for inputs in every_input()
            .into_iter()
            .filter(|i| i.accent == Accent::Aqua)
        {
            let m = material(inputs);
            let ink = scales(inputs).neutral[0];
            for behind in [Rgb(0.5, 0.5, 0.5), Rgb::BLACK] {
                let ground = glass_body(behind, &m);
                for (i, (text, fill)) in categorical(inputs).iter().enumerate() {
                    let lc = apca(*text, ground).abs();
                    if lc < 45.0 {
                        failures.push(format!(
                            "{inputs:?} cat-{} over {}: Lc {lc:.0}",
                            i + 1,
                            behind.css()
                        ));
                    }
                    let on = apca(Rgb::WHITE, *fill).abs().max(apca(ink, *fill).abs());
                    if on < 45.0 {
                        failures.push(format!("{inputs:?} cat-{}-bg label: Lc {on:.0}", i + 1));
                    }
                }
            }
        }
        assert!(
            failures.is_empty(),
            "{} failures:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }

    fn hue_of(c: Rgb) -> f64 {
        Oklch::from(c).2
    }

    /// §2.2: the accent is the wallpaper's colour, and keeps the lightness
    /// its contrast was measured at.
    #[test]
    fn a_tinted_accent_takes_the_wallpapers_hue() {
        for inputs in every_input() {
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

    /// Red is a word, not a decoration: a status colour turns toward the
    /// wallpaper by at most the cap, whatever the hue.
    #[test]
    fn status_stays_what_it_says() {
        for mode in Mode::ALL {
            let shipped = shipped_status(mode);
            for tint in every_tint() {
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
            for tint in every_tint() {
                let got = closest(at(tint));
                assert!(
                    got >= floor - 2.0,
                    "{mode:?} {tint:?}: two categorical hues {got:.1}° apart, shipped {floor:.1}°"
                );
            }
        }
    }

    /// Accents leaves the greys exactly alone; Full casts them toward the
    /// wallpaper at their own lightness.
    #[test]
    fn only_full_reaches_the_neutrals() {
        for inputs in every_input() {
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

    #[test]
    fn the_css_names_every_semantic_token() {
        let css = css(Inputs::default());
        for name in [
            "surface-key",
            "surface-raised",
            "scrim",
            "fg",
            "fg-muted",
            "fg-faint",
            "fg-disabled",
            "fill-1",
            "fill-2",
            "fill-3",
            "state-hover",
            "state-pressed",
            "state-selected",
            "border-subtle",
            "border-strong",
            "focus-ring",
            "accent-bg",
            "accent-bg-hover",
            "on-accent",
            "accent",
            "accent-tint",
            "success",
            "warning",
            "danger",
            "on-status",
            "type-body",
            "space-5",
            "radius-card",
            "dur-fast",
            "ease-standard",
            "motion-state",
            "motion-enter",
            "motion-exit",
            "motion-travel",
        ] {
            assert!(css.contains(&format!("  --{name}: ")), "missing --{name}");
        }
    }
}
