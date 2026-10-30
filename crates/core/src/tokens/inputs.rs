//! The theme inputs the tokens are generated from (docs/design-system.md
//! §2): mode, accent, neutral, contrast, the Motion setting and the tint.
//!
//! Plain values. Resolving them at runtime (the Look settings, the sun, the
//! wallpaper's hue) is `theme::inputs`, which is the one place an
//! [`Inputs`] is built outside tests.

use super::{Backdrop, Rgb, Tint};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
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
    #[cfg(test)]
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
    pub(super) fn anchors(self, mode: Mode) -> [Rgb; 3] {
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

/// Serialisable because the panel publishes the inputs it resolved and every
/// other process draws those (`theme::inputs`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Inputs {
    pub mode: Mode,
    pub accent: Accent,
    pub neutral: Neutral,
    pub contrast: Contrast,
    /// The Motion setting as a percentage of every duration: 100, 50 or 0.
    pub motion: u8,
    /// The wallpaper's hue and how far it reaches (§2.2).
    pub tint: Tint,
    /// What the wallpaper is behind text that stands on it, when the panel
    /// has measured it (§3.3, `backdrop.rs`).
    pub backdrop: Option<Backdrop>,
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
            backdrop: None,
        }
    }
}

/// Every input combination the colour tests run over, and the tints they
/// run at.
#[cfg(test)]
pub(super) mod every {
    use super::*;
    use crate::tokens::tint::Palette;

    /// The wallpaper hues the tinted tests run at: every 5° round the
    /// circle. The tint is whole degrees, so every hue it can take is one
    /// edit away from exhaustive (`(0..360)`), which passes too; 5° keeps a
    /// debug build's run short.
    fn hues() -> impl Iterator<Item = u16> {
        (0..72).map(|i| i * 5)
    }

    /// Off, and both reaches at every hue of [`hues`].
    pub fn tint() -> Vec<Tint> {
        let mut v = vec![Tint::Off];
        for h in hues() {
            v.push(Tint::Accents(Palette::single(h)));
            v.push(Tint::Full(Palette::single(h)));
            // A second colour a third of the way round, and a ground that is
            // not the accent: the sets a real wallpaper gives.
            let two = Palette {
                primary: h,
                ground: (h + 180) % 360,
                secondary: Some((h + 120) % 360),
            };
            v.push(Tint::Accents(two));
            v.push(Tint::Full(two));
        }
        v
    }

    /// Every combination of the inputs that decide a colour: 72 untinted
    /// (2 modes, 6 accents, 3 neutrals, 2 contrasts), each at every tint.
    pub fn input() -> Vec<Inputs> {
        let mut v = Vec::new();
        for mode in Mode::ALL {
            for accent in Accent::ALL {
                for neutral in Neutral::ALL {
                    for contrast in Contrast::ALL {
                        for tint in tint() {
                            v.push(Inputs {
                                mode,
                                accent,
                                neutral,
                                contrast,
                                motion: 100,
                                tint,
                                backdrop: None,
                            });
                        }
                    }
                }
            }
        }
        v
    }
}
