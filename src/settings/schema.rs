//! The settings file's shape: every section, its defaults, and what can be
//! done to a `Settings` without touching a file. `store.rs` owns the files
//! and the panel's live copy; this owns the types they move around.
//!
//! Every section is a struct with a `Default` that is the binary's own
//! fallback, `#[serde(default …)]` on each field so an older file still
//! loads, and a `sanitized` where a bad value is worse than ugly. The
//! [`Section`] trait is what lets the panes and the CLI edit a section by
//! type rather than by a copy of the same six lines.

use std::fmt::Write as _;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::store::system;

// ── Sections ────────────────────────────────────────────────────────────

/// How sway scales the image onto the output. Spelled as `output … bg`
/// takes it, so [`Mode::as_str`] is the wire format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WallpaperMode {
    #[default]
    Fill,
    Fit,
    Stretch,
    Center,
    Tile,
}

impl WallpaperMode {
    pub const ALL: [WallpaperMode; 5] = [
        WallpaperMode::Fill,
        WallpaperMode::Fit,
        WallpaperMode::Stretch,
        WallpaperMode::Center,
        WallpaperMode::Tile,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            WallpaperMode::Fill => "fill",
            WallpaperMode::Fit => "fit",
            WallpaperMode::Stretch => "stretch",
            WallpaperMode::Center => "center",
            WallpaperMode::Tile => "tile",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            WallpaperMode::Fill => "Fill — crop to cover",
            WallpaperMode::Fit => "Fit — letterbox",
            WallpaperMode::Stretch => "Stretch",
            WallpaperMode::Center => "Center — no scaling",
            WallpaperMode::Tile => "Tile",
        }
    }

    pub fn parse(s: &str) -> Option<WallpaperMode> {
        WallpaperMode::ALL.into_iter().find(|m| m.as_str() == s)
    }
}

/// The wallpaper the user picked. No default: the default is whatever the
/// sway config says, which `wallpaper::system_default` reads back from the
/// compositor rather than guessing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Wallpaper {
    pub path: PathBuf,
    #[serde(default)]
    pub mode: WallpaperMode,
}

/// The idle manager's timers, in seconds. Zero is "never" for every tier.
///
/// The defaults are the numbers the old swayidle config carried and
/// `idle/mod.rs` documents the incident history behind; a field missing
/// from a hand-edited file lands on them too.
///
/// The `night_*` fields are a second, shorter set of three tiers for a
/// time window. They are inert until `night` is on, and [`Idle::resolve`]
/// is the only thing that reads them: it hands back the tiers in force at
/// a given minute, so nothing downstream knows the window exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Idle {
    /// Fade the backlight after this much idle time.
    #[serde(default = "Idle::default_dim_after")]
    pub dim_after_s: u32,
    /// What the fade goes to, as a backlight percentage.
    #[serde(default = "Idle::default_dim_level")]
    pub dim_level: u8,
    /// Lock the session after this much idle time. Locking leaves the
    /// screen lit; see `blank_after_s`.
    #[serde(default = "Idle::default_lock_after")]
    pub lock_after_s: u32,
    /// Power the outputs off after this much idle time *while locked*.
    #[serde(default = "Idle::default_blank_after")]
    pub blank_after_s: u32,
    /// Suspend after this much idle time, on battery only.
    #[serde(default = "Idle::default_suspend_after")]
    pub suspend_after_s: u32,
    /// Lock when the presence sensor sees you leave. Off, walking away is
    /// no different from sitting still, and the idle tiers do the locking.
    #[serde(default = "yes")]
    pub walk_away_lock: bool,
    /// Try the camera while the lock screen is up. Off, the password and
    /// the fingerprint remain; nothing here can make unlocking easier.
    #[serde(default = "yes")]
    pub face_unlock: bool,
    /// Run the night window below. Off, the timers above hold all day and
    /// every `night_*` field is inert.
    #[serde(default)]
    pub night: bool,
    /// When the window opens, local time. The window is half-open,
    /// `[from, to)`, and wraps past midnight when the end is at or before
    /// the start.
    #[serde(default = "Idle::default_night_from_h")]
    pub night_from_h: u8,
    #[serde(default)]
    pub night_from_m: u8,
    /// When the window closes.
    #[serde(default = "Idle::default_night_to_h")]
    pub night_to_h: u8,
    #[serde(default)]
    pub night_to_m: u8,
    /// What `dim_after_s` becomes inside the window.
    #[serde(default = "Idle::default_night_dim_after")]
    pub night_dim_after_s: u32,
    /// What `lock_after_s` becomes inside the window.
    #[serde(default = "Idle::default_night_lock_after")]
    pub night_lock_after_s: u32,
    /// What `blank_after_s` becomes inside the window.
    #[serde(default = "Idle::default_night_blank_after")]
    pub night_blank_after_s: u32,
}

/// Minutes since local midnight, 0–1439. Noon on a clock the platform
/// cannot read, which is outside the shipped window and so is the tier set
/// a user is least surprised by.
pub fn local_minute_of_day() -> u16 {
    glib::DateTime::now_local()
        .map(|t| (t.hour() as u16) * 60 + t.minute() as u16)
        .unwrap_or(12 * 60)
}

impl Idle {
    fn default_dim_after() -> u32 {
        240
    }
    fn default_dim_level() -> u8 {
        10
    }
    fn default_lock_after() -> u32 {
        300
    }
    fn default_blank_after() -> u32 {
        15 * 60
    }
    fn default_suspend_after() -> u32 {
        1200
    }
    fn default_night_from_h() -> u8 {
        21
    }
    fn default_night_to_h() -> u8 {
        7
    }
    fn default_night_dim_after() -> u32 {
        60
    }
    fn default_night_lock_after() -> u32 {
        300
    }
    fn default_night_blank_after() -> u32 {
        120
    }
}

impl Idle {
    /// The file is hand-editable, and a dim level of 0 is a screen that
    /// goes black on the first idle tick and stays that way for anyone who
    /// does not know why. Bound it; `blank_after_s` and the timers already
    /// mean "never" at zero, which is a valid ask.
    ///
    /// The window bounds are clamped rather than rejected, for the same
    /// reason `Alerts` clamps its quiet hours: an hour of 40 is a typo, and
    /// a typo should cost the user a wrong window, not the whole section.
    fn sanitized(self) -> Idle {
        Idle {
            dim_level: self.dim_level.clamp(1, 100),
            night_from_h: self.night_from_h.min(23),
            night_from_m: self.night_from_m.min(59),
            night_to_h: self.night_to_h.min(23),
            night_to_m: self.night_to_m.min(59),
            ..self
        }
    }

    /// The window's bounds as minutes since midnight.
    fn night_bounds(&self) -> (u16, u16) {
        (
            u16::from(self.night_from_h) * 60 + u16::from(self.night_from_m),
            u16::from(self.night_to_h) * 60 + u16::from(self.night_to_m),
        )
    }

    /// Whether `minute_of_day` (0–1439, local) is inside the night window.
    /// False whenever the window is off, so no caller has to check both.
    ///
    /// Half-open, `[from, to)`, wrapping past midnight when `to <= from`.
    /// `from == to` is the whole day, which follows `in_quiet_hours`: a
    /// switch that is on and does nothing is worse than one that does what
    /// it says.
    pub fn in_night(&self, minute_of_day: u16) -> bool {
        if !self.night {
            return false;
        }
        let (from, to) = self.night_bounds();
        if from < to {
            (from..to).contains(&minute_of_day)
        } else {
            minute_of_day >= from || minute_of_day < to
        }
    }

    /// The timers in force at `minute_of_day`: this set outside the window,
    /// and this set with the three night tiers substituted inside it.
    ///
    /// Returning an `Idle` rather than three numbers is what keeps the
    /// night window out of every consumer — `wayland::Timeouts::from`, the
    /// blank deadline and the log line all take the resolved struct and
    /// never learn that a window exists. Suspend is deliberately not
    /// substituted: it is battery-only, and a shorter night suspend would
    /// stop an overnight job on a machine the user left running on purpose.
    ///
    /// Idempotent, so a resolved struct may be resolved again.
    pub fn resolve(&self, minute_of_day: u16) -> Idle {
        if !self.in_night(minute_of_day) {
            return *self;
        }
        Idle {
            dim_after_s: self.night_dim_after_s,
            lock_after_s: self.night_lock_after_s,
            blank_after_s: self.night_blank_after_s,
            ..*self
        }
    }
}

impl Default for Idle {
    fn default() -> Self {
        Idle {
            dim_after_s: Self::default_dim_after(),
            dim_level: Self::default_dim_level(),
            lock_after_s: Self::default_lock_after(),
            blank_after_s: Self::default_blank_after(),
            suspend_after_s: Self::default_suspend_after(),
            walk_away_lock: true,
            face_unlock: true,
            night: false,
            night_from_h: Self::default_night_from_h(),
            night_from_m: 0,
            night_to_h: Self::default_night_to_h(),
            night_to_m: 0,
            night_dim_after_s: Self::default_night_dim_after(),
            night_lock_after_s: Self::default_night_lock_after(),
            night_blank_after_s: Self::default_night_blank_after(),
        }
    }
}

/// What the bar does that is a matter of taste rather than of state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bar {
    /// `14:05` rather than `2:05 PM`.
    #[serde(default = "yes")]
    pub clock_24h: bool,
    /// Put the date beside the time. Clicking the clock still flips to the
    /// ISO date on its own.
    #[serde(default)]
    pub clock_date: bool,
    /// Volume and brightness render in the bar's decision slot instead of
    /// as the center-screen card. Which one reads better depends on the
    /// material, which is why this is a setting and not a build flag.
    #[serde(default)]
    pub osd_in_bar: bool,
    /// The four-bay task board in the bar's right track (`bar/board.rs`).
    #[serde(default)]
    pub board: bool,
    /// The other segments of the right cluster. Hidden, not removed: the
    /// widget is still built and its service still runs.
    #[serde(default = "yes")]
    pub media: bool,
    #[serde(default = "yes")]
    pub tray: bool,
    #[serde(default = "yes")]
    pub battery: bool,
    #[serde(default = "yes")]
    pub presence: bool,
    /// The nightly backup's one-glyph verdict, beside the clock.
    #[serde(default = "yes")]
    pub backup: bool,
}

fn yes() -> bool {
    true
}

impl Default for Bar {
    fn default() -> Self {
        Bar {
            clock_24h: true,
            clock_date: false,
            osd_in_bar: false,
            board: false,
            media: true,
            tray: true,
            battery: true,
            presence: true,
            backup: true,
        }
    }
}

/// How much the shell moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Motion {
    #[default]
    Full,
    /// Half the duration: still a direction, half the wait.
    Reduced,
    /// One frame. The state flow still runs (`anim::duration`).
    Off,
}

impl Motion {
    pub const ALL: [Motion; 3] = [Motion::Full, Motion::Reduced, Motion::Off];

    pub fn label(self) -> &'static str {
        match self {
            Motion::Full => "Full",
            Motion::Reduced => "Reduced — half as long",
            Motion::Off => "Off — jump to the end",
        }
    }

    /// What a duration is multiplied by. Zero means "one frame" to
    /// `anim::duration`, which is why it is not literally zero here.
    pub fn scale(self) -> f64 {
        match self {
            Motion::Full => 1.0,
            Motion::Reduced => 0.5,
            Motion::Off => 0.0,
        }
    }
}

/// How much of the theme the wallpaper colours.
///
/// A token input (docs/design-system.md §2.2): each colour keeps its
/// lightness and only its hue moves, so the contrast the tokens are tested
/// at survives whatever is on the desktop. `src/tokens/tint.rs` has the
/// rule, and the contrast tests in `src/tokens/apca.rs` the proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Tint {
    /// The shipped tokens. The default: a theme that follows the wallpaper
    /// is a taste, and a taste is opted into.
    #[default]
    Off,
    /// The accents follow the wallpaper; the greys stay gruvbox.
    Accents,
    /// The greys pick up a cast of the wallpaper's hue as well.
    Full,
}

impl Tint {
    pub const ALL: [Tint; 3] = [Tint::Off, Tint::Accents, Tint::Full];

    pub fn label(self) -> &'static str {
        match self {
            Tint::Off => "Off — the shipped colours",
            Tint::Accents => "Accents — hues from the wallpaper",
            Tint::Full => "Full — surfaces tinted too",
        }
    }
}

/// Light, dark, or whichever the sun says (docs/design-system.md §2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    /// Dark from dusk to dawn, light in between, at the location in
    /// /etc/swaypplet/theme.json. Dark when no location is known.
    #[default]
    Auto,
    Dark,
    Light,
}

impl ThemeMode {
    pub const ALL: [ThemeMode; 3] = [ThemeMode::Auto, ThemeMode::Dark, ThemeMode::Light];

    pub fn label(self) -> &'static str {
        match self {
            ThemeMode::Auto => "Auto — light by day, dark by night",
            ThemeMode::Dark => "Dark",
            ThemeMode::Light => "Light",
        }
    }
}

/// The Look tab's second group. The wallpaper is the first and has its own
/// section, since it has no system layer in this file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Look {
    /// The theme inputs (docs/design-system.md §2).
    #[serde(default)]
    pub mode: ThemeMode,
    #[serde(default)]
    pub accent: crate::tokens::Accent,
    #[serde(default)]
    pub neutral: crate::tokens::Neutral,
    #[serde(default)]
    pub contrast: crate::tokens::Contrast,
    #[serde(default)]
    pub motion: Motion,
    #[serde(default)]
    pub tint: Tint,
    /// A launched app grows out of its launcher row (sway's `handoff open`).
    /// Off by default: on a real desktop the window's first frames rarely
    /// match the icon it grows from, and it read as a glitch more than as a
    /// transition.
    #[serde(default)]
    pub launch_zoom: bool,
    /// Apps follow the shell's appearance: the panel publishes the mode, the
    /// accent, contrast and motion to the GNOME settings that the desktop
    /// portal serves (`theme::apps`). Off, it leaves them as they are.
    #[serde(default = "yes")]
    pub apps_follow: bool,
}

impl Default for Look {
    fn default() -> Self {
        Look {
            mode: ThemeMode::default(),
            accent: crate::tokens::Accent::default(),
            neutral: crate::tokens::Neutral::default(),
            contrast: crate::tokens::Contrast::default(),
            motion: Motion::default(),
            tint: Tint::default(),
            launch_zoom: false,
            apps_follow: true,
        }
    }
}

/// The volume and brightness keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Keys {
    /// Percent per press.
    #[serde(default = "Keys::default_step")]
    pub volume_step: u8,
    #[serde(default = "Keys::default_step")]
    pub brightness_step: u8,
    /// Let the volume keys go past 100 %, to the 150 % the sound server
    /// allows. Off, both the keys and the panel's slider stop at 100.
    #[serde(default = "yes")]
    pub volume_boost: bool,
}

impl Keys {
    fn default_step() -> u8 {
        5
    }

    /// The ceiling as a fraction, which is what the audio path speaks.
    pub fn volume_ceiling(&self) -> f64 {
        if self.volume_boost {
            crate::services::audio::VOLUME_CEILING
        } else {
            1.0
        }
    }

    fn sanitized(self) -> Keys {
        Keys {
            volume_step: self.volume_step.clamp(1, 25),
            brightness_step: self.brightness_step.clamp(1, 25),
            ..self
        }
    }
}

impl Default for Keys {
    fn default() -> Self {
        Keys {
            volume_step: 5,
            brightness_step: 5,
            volume_boost: true,
        }
    }
}

/// How long a popup with no timeout of its own stays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Linger {
    Short,
    #[default]
    Normal,
    Long,
}

impl Linger {
    pub const ALL: [Linger; 3] = [Linger::Short, Linger::Normal, Linger::Long];

    pub fn label(self) -> &'static str {
        match self {
            Linger::Short => "Short — 3 s",
            Linger::Normal => "Normal — 5 s",
            Linger::Long => "Long — 9 s",
        }
    }

    /// Base milliseconds, and milliseconds per character of text on top.
    pub fn ms(self) -> (u64, u64) {
        match self {
            Linger::Short => (3000, 25),
            Linger::Normal => (5000, 40),
            Linger::Long => (9000, 60),
        }
    }
}

/// Which corner the popup stack grows from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Corner {
    #[default]
    TopRight,
    TopLeft,
    BottomRight,
    BottomLeft,
}

impl Corner {
    pub const ALL: [Corner; 4] = [
        Corner::TopRight,
        Corner::TopLeft,
        Corner::BottomRight,
        Corner::BottomLeft,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Corner::TopRight => "Top right",
            Corner::TopLeft => "Top left",
            Corner::BottomRight => "Bottom right",
            Corner::BottomLeft => "Bottom left",
        }
    }

    pub fn is_bottom(self) -> bool {
        matches!(self, Corner::BottomRight | Corner::BottomLeft)
    }

    pub fn is_left(self) -> bool {
        matches!(self, Corner::TopLeft | Corner::BottomLeft)
    }
}

/// Notifications: the popup stack, and the hours it keeps quiet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Alerts {
    #[serde(default)]
    pub linger: Linger,
    #[serde(default)]
    pub corner: Corner,
    /// Cards shown at full size before older ones collapse behind them.
    #[serde(default = "Alerts::default_stack")]
    pub stack: u8,
    /// Arm Do Not Disturb between `quiet_from_h` and `quiet_to_h`, and
    /// disarm it after. A manual toggle inside the window is left alone.
    #[serde(default)]
    pub quiet: bool,
    /// Whole hours, 0–23. A window that ends before it starts crosses
    /// midnight.
    #[serde(default = "Alerts::default_quiet_from")]
    pub quiet_from_h: u8,
    #[serde(default = "Alerts::default_quiet_to")]
    pub quiet_to_h: u8,
}

impl Alerts {
    fn default_stack() -> u8 {
        3
    }
    fn default_quiet_from() -> u8 {
        22
    }
    fn default_quiet_to() -> u8 {
        7
    }

    fn sanitized(self) -> Alerts {
        Alerts {
            stack: self.stack.clamp(1, 5),
            quiet_from_h: self.quiet_from_h.min(23),
            quiet_to_h: self.quiet_to_h.min(23),
            ..self
        }
    }

    /// Whether `hour` (0–23) is inside the quiet window. The window is
    /// half-open, `[from, to)`, and wraps past midnight when `to <= from`;
    /// `from == to` is the whole day, since a switch that is on and does
    /// nothing is worse than one that does what it says.
    pub fn in_quiet_hours(&self, hour: u8) -> bool {
        let (from, to) = (self.quiet_from_h, self.quiet_to_h);
        if from < to {
            (from..to).contains(&hour)
        } else {
            hour >= from || hour < to
        }
    }
}

impl Default for Alerts {
    fn default() -> Self {
        Alerts {
            linger: Linger::Normal,
            corner: Corner::TopRight,
            stack: 3,
            quiet: false,
            quiet_from_h: 22,
            quiet_to_h: 7,
        }
    }
}

/// What a screenshot becomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum After {
    #[default]
    Both,
    Save,
    Copy,
}

impl After {
    pub const ALL: [After; 3] = [After::Both, After::Save, After::Copy];

    pub fn label(self) -> &'static str {
        match self {
            After::Both => "Save and copy",
            After::Save => "Save only",
            After::Copy => "Copy only",
        }
    }

    pub fn saves(self) -> bool {
        !matches!(self, After::Copy)
    }

    pub fn copies(self) -> bool {
        !matches!(self, After::Save)
    }
}

/// Screenshots and recordings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Capture {
    /// Where shots land. Empty is `~/Pictures/Screenshots`.
    #[serde(default)]
    pub folder: String,
    #[serde(default)]
    pub after: After,
    /// Open the annotation editor on every shot, instead of from the card.
    #[serde(default)]
    pub annotate: bool,
}

/// Administrator access: what `sudo` and `pkexec` may ask for besides the
/// password, and where they may ask it.
///
/// Read by the polkit agent, which is the process that draws the card for
/// both and answers pam_race's `begin` (`polkit/race.rs`). Every switch here
/// removes a way in or a surface; none can make elevation easier than the
/// password alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Elevate {
    /// Ask the camera. Off, `sudo` and `pkexec` never open the shutter; the
    /// reader and the password remain.
    #[serde(default = "yes")]
    pub face: bool,
    /// Draw the card for a `sudo` typed in a terminal. Off, the terminal
    /// keeps the prompt to itself, and the camera is not asked for it: the
    /// Allow press lives on the card.
    #[serde(default = "yes")]
    pub terminal_card: bool,
    /// The pill under the lens while the camera runs. Off, the card's
    /// caption reports the check instead.
    #[serde(default = "yes")]
    pub cue: bool,
    /// The first keystroke in the password field ends a face check that is
    /// still looking. Off, the camera runs out its window alongside the
    /// typing.
    #[serde(default = "yes")]
    pub typing_abandons_face: bool,
}

impl Default for Elevate {
    fn default() -> Self {
        Elevate {
            face: true,
            terminal_card: true,
            cue: true,
            typing_abandons_face: true,
        }
    }
}

/// What the launcher's results include, and whether it learns.
///
/// One switch per kind of result rather than per elephant provider: the
/// windows switch covers both the running-window rows the launcher finds
/// itself and elephant's `windows`, the calculator switch both the `=` rows
/// and elephant's `calc`. `launcher::sources` maps these to providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Launcher {
    /// Installed applications.
    #[serde(default = "yes")]
    pub apps: bool,
    /// Open windows: "Go to" rows above an app's result, and Tab on one.
    #[serde(default = "yes")]
    pub windows: bool,
    /// `=` and arithmetic typed bare.
    #[serde(default = "yes")]
    pub calculator: bool,
    /// `>` runs a command in your shell; commands on `PATH` as results.
    #[serde(default = "yes")]
    pub commands: bool,
    /// The clipboard history.
    #[serde(default = "yes")]
    pub clipboard: bool,
    /// The panel's pages and the settings tabs, by name.
    #[serde(default = "yes")]
    pub settings: bool,
    /// Desktop actions and menus that apps publish.
    #[serde(default = "yes")]
    pub menus: bool,
    /// A web search for what was typed.
    #[serde(default = "yes")]
    pub web_search: bool,
    /// Files, by name, as elephant indexes them.
    #[serde(default)]
    pub files: bool,
    /// Browser bookmarks.
    #[serde(default)]
    pub bookmarks: bool,
    /// Emoji and symbols by name.
    #[serde(default)]
    pub symbols: bool,
    /// Rank by what you launch, and show it first with the query empty.
    #[serde(default = "yes")]
    pub frecency: bool,
}

impl Default for Launcher {
    fn default() -> Self {
        Launcher {
            apps: true,
            windows: true,
            calculator: true,
            commands: true,
            clipboard: true,
            settings: true,
            menus: true,
            web_search: true,
            files: false,
            bookmarks: false,
            symbols: false,
            frecency: true,
        }
    }
}

/// When the night light is on (`services::gamma`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum NightSchedule {
    /// Warm from dusk to dawn, across civil twilight, from the sun at
    /// `/etc/swaypplet/theme.json`'s location: the one the automatic mode
    /// uses.
    #[default]
    Sun,
    /// Warm all day.
    Always,
}

/// The night light: a colour temperature the compositor applies to every
/// output, in place of gammastep.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NightLight {
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub schedule: NightSchedule,
    /// The night's colour temperature in kelvin, 1700–6500. The day is
    /// 6500 K, which is no change at all.
    #[serde(default = "NightLight::default_night_k")]
    pub night_k: u32,
}

impl NightLight {
    pub const MIN_K: u32 = 1700;
    pub const DAY_K: u32 = 6500;

    fn default_night_k() -> u32 {
        3500
    }

    fn sanitized(self) -> NightLight {
        NightLight {
            night_k: self.night_k.clamp(Self::MIN_K, Self::DAY_K),
            ..self
        }
    }
}

impl Default for NightLight {
    fn default() -> Self {
        NightLight {
            enabled: true,
            schedule: NightSchedule::Sun,
            night_k: Self::default_night_k(),
        }
    }
}

/// Which connected output a profile output claims (`services::displays`).
/// Every field given must match, as a glob (`*`, `?`); a field left out
/// matches anything. An output that does not report a make, model or
/// serial matches it as `Unknown`, as kanshi does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct OutputMatch {
    /// The connector, `eDP-1`, `DP-3`: stable for a built-in panel, not
    /// for a monitor that moves between ports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub make: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial: Option<String>,
}

/// An output's rotation and flip, spelled as sway's `output … transform`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum OutputTransform {
    #[default]
    #[serde(rename = "normal")]
    Normal,
    #[serde(rename = "90")]
    R90,
    #[serde(rename = "180")]
    R180,
    #[serde(rename = "270")]
    R270,
    #[serde(rename = "flipped")]
    Flipped,
    #[serde(rename = "flipped-90")]
    Flipped90,
    #[serde(rename = "flipped-180")]
    Flipped180,
    #[serde(rename = "flipped-270")]
    Flipped270,
}

/// One output of a display profile. A field left out keeps what the output
/// has, so a profile can say only where a screen goes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DisplayOutput {
    #[serde(rename = "match")]
    pub criteria: OutputMatch,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Width, height and refresh in mHz: `[3840, 2160, 60000]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<[u32; 3]>,
    /// The top-left corner in the layout, logical pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<[i32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transform: Option<OutputTransform>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adaptive_sync: Option<bool>,
}

/// A named layout for one set of connected outputs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DisplayProfile {
    pub name: String,
    #[serde(default)]
    pub outputs: Vec<DisplayOutput>,
}

/// Display profiles, kanshi's job in the panel (`services::displays`): the
/// first profile, in this order, whose outputs claim every connected output
/// is applied when the outputs change, and the one applied stays while it
/// still matches. No profile matching leaves the outputs as they are.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Displays {
    #[serde(default)]
    pub profiles: Vec<DisplayProfile>,
}

impl Displays {
    /// Drop what cannot be applied: a scale outside what sway accepts, an
    /// empty name, a profile with no outputs.
    fn sanitized(self) -> Displays {
        Displays {
            profiles: self
                .profiles
                .into_iter()
                .filter(|p| !p.name.trim().is_empty() && !p.outputs.is_empty())
                .map(|mut p| {
                    for o in &mut p.outputs {
                        o.scale = o.scale.filter(|s| s.is_finite()).map(|s| s.clamp(0.25, 8.0));
                    }
                    p
                })
                .collect(),
        }
    }
}

// ── Input ───────────────────────────────────────────────────────────────

/// libinput's acceleration curve, spelled as sway's `accel_profile` takes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccelProfile {
    Adaptive,
    Flat,
}

/// How a touchpad without buttons makes a right or middle click, spelled as
/// sway's `click_method` takes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClickMethod {
    None,
    ButtonAreas,
    Clickfinger,
}

/// How a touchpad scrolls, spelled as sway's `scroll_method` takes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScrollMethod {
    None,
    TwoFinger,
    Edge,
    OnButtonDown,
}

/// Keyboard, touchpad and mouse, sent to sway by device type
/// (`services::input`). Every field is optional, and `None` leaves the sway
/// config's own `input` block in force for that one knob: the section holds
/// only what was changed here, so it never fights the config over the rest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Input {
    /// Keyboard layouts in switch order, in xkb's spelling: `se`,
    /// `us(dvorak)` for a variant.
    #[serde(default)]
    pub layouts: Option<Vec<String>>,
    /// The `grp:` xkb option that switches layout, `grp:win_space_toggle`;
    /// empty for none.
    #[serde(default)]
    pub layout_switch: Option<String>,
    /// The xkb option for Caps Lock, `caps:escape`, `ctrl:nocaps`; empty for
    /// Caps Lock as itself.
    #[serde(default)]
    pub caps: Option<String>,
    /// Milliseconds a key is held before it repeats.
    #[serde(default)]
    pub repeat_delay_ms: Option<u32>,
    /// Repeats per second.
    #[serde(default)]
    pub repeat_rate: Option<u32>,
    #[serde(default)]
    pub touchpad_tap: Option<bool>,
    #[serde(default)]
    pub touchpad_natural_scroll: Option<bool>,
    /// libinput's pointer speed, −1 to 1.
    #[serde(default)]
    pub touchpad_speed: Option<f64>,
    #[serde(default)]
    pub touchpad_accel_profile: Option<AccelProfile>,
    /// Disable while typing.
    #[serde(default)]
    pub touchpad_dwt: Option<bool>,
    #[serde(default)]
    pub touchpad_click_method: Option<ClickMethod>,
    #[serde(default)]
    pub touchpad_scroll_method: Option<ScrollMethod>,
    #[serde(default)]
    pub mouse_speed: Option<f64>,
    #[serde(default)]
    pub mouse_accel_profile: Option<AccelProfile>,
    #[serde(default)]
    pub mouse_natural_scroll: Option<bool>,
}

impl Input {
    pub const REPEAT_DELAY_MS: (u32, u32) = (100, 1000);
    pub const REPEAT_RATE: (u32, u32) = (1, 100);

    /// An xkb name as sway will be handed it inside double quotes: letters,
    /// digits and the punctuation xkb names use, nothing that could end the
    /// quote or the command.
    pub fn is_xkb_name(s: &str) -> bool {
        s.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '(' | ')' | ':' | '+' | '.')
        })
    }

    fn sanitized(self) -> Input {
        let speed = |s: Option<f64>| s.filter(|v| v.is_finite()).map(|v| v.clamp(-1.0, 1.0));
        let name = |s: Option<String>| s.filter(|v| Self::is_xkb_name(v));
        Input {
            layouts: self
                .layouts
                .map(|l| {
                    l.into_iter()
                        .filter(|n| !n.is_empty() && Self::is_xkb_name(n))
                        .collect::<Vec<_>>()
                })
                .filter(|l| !l.is_empty()),
            layout_switch: name(self.layout_switch),
            caps: name(self.caps),
            repeat_delay_ms: self
                .repeat_delay_ms
                .map(|v| v.clamp(Self::REPEAT_DELAY_MS.0, Self::REPEAT_DELAY_MS.1)),
            repeat_rate: self
                .repeat_rate
                .map(|v| v.clamp(Self::REPEAT_RATE.0, Self::REPEAT_RATE.1)),
            touchpad_speed: speed(self.touchpad_speed),
            mouse_speed: speed(self.mouse_speed),
            ..self
        }
    }
}

// ── The file ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Settings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wallpaper: Option<Wallpaper>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub look: Option<Look>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle: Option<Idle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bar: Option<Bar>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keys: Option<Keys>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alerts: Option<Alerts>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture: Option<Capture>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elevate: Option<Elevate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launcher: Option<Launcher>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub night_light: Option<NightLight>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub displays: Option<Displays>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<Input>,
}

impl Settings {
    /// The section names, in the order the file and the pane list them.
    pub const SECTIONS: [&'static str; 12] = [
        "wallpaper",
        "look",
        "idle",
        "bar",
        "keys",
        "alerts",
        "capture",
        "elevate",
        "launcher",
        "night_light",
        "displays",
        "input",
    ];

    /// The sections with a system layer, which is every one but the
    /// wallpaper: its system default is the sway config's `bg` line.
    pub const NIX_SECTIONS: [&'static str; 11] = [
        "look",
        "idle",
        "bar",
        "keys",
        "alerts",
        "capture",
        "elevate",
        "launcher",
        "night_light",
        "displays",
        "input",
    ];

    /// The section in force: the user's, else the system's, else the
    /// binary's. One per section, so a reader names what it wants.
    pub fn look(&self) -> Look {
        self.look.or(system().look).unwrap_or_default()
    }
    pub fn idle(&self) -> Idle {
        self.idle.or(system().idle).unwrap_or_default()
    }
    pub fn bar(&self) -> Bar {
        self.bar.or(system().bar).unwrap_or_default()
    }
    pub fn keys(&self) -> Keys {
        self.keys.or(system().keys).unwrap_or_default()
    }
    pub fn alerts(&self) -> Alerts {
        self.alerts.or(system().alerts).unwrap_or_default()
    }
    pub fn capture(&self) -> Capture {
        self.capture
            .clone()
            .or_else(|| system().capture.clone())
            .unwrap_or_default()
    }
    pub fn elevate(&self) -> Elevate {
        self.elevate.or(system().elevate).unwrap_or_default()
    }
    pub fn launcher(&self) -> Launcher {
        self.launcher.or(system().launcher).unwrap_or_default()
    }
    pub fn night_light(&self) -> NightLight {
        self.night_light
            .or(system().night_light)
            .unwrap_or_default()
    }
    pub fn displays(&self) -> Displays {
        self.displays
            .clone()
            .or_else(|| system().displays.clone())
            .unwrap_or_default()
    }
    pub fn input(&self) -> Input {
        self.input
            .clone()
            .or_else(|| system().input.clone())
            .unwrap_or_default()
    }

    /// True when nothing is overridden, which is when the file should not
    /// exist.
    pub fn is_default(&self) -> bool {
        *self == Settings::default()
    }

    /// What is in force, every section filled in.
    pub fn effective(&self) -> Settings {
        Settings {
            wallpaper: self.wallpaper.clone(),
            look: Some(self.look()),
            idle: Some(self.idle()),
            bar: Some(self.bar()),
            keys: Some(self.keys()),
            alerts: Some(self.alerts()),
            capture: Some(self.capture()),
            elevate: Some(self.elevate()),
            launcher: Some(self.launcher()),
            night_light: Some(self.night_light()),
            displays: Some(self.displays()),
            input: Some(self.input()),
        }
    }

    /// Every section at the binary's defaults, the wallpaper included with
    /// an empty path: the shape of the file, for checking keys against.
    ///
    /// Built from the `Default`s directly, never through the accessors:
    /// this runs inside `system()`'s own initialisation (`read` checks the
    /// system file's keys against it), and an accessor would call
    /// `system()` again from within its `OnceLock`, which hangs the
    /// process on the first read of a system file with any section in it.
    fn probe() -> Settings {
        Settings {
            wallpaper: Some(Wallpaper {
                path: PathBuf::new(),
                mode: WallpaperMode::default(),
            }),
            look: Some(Look::default()),
            idle: Some(Idle::default()),
            bar: Some(Bar::default()),
            keys: Some(Keys::default()),
            alerts: Some(Alerts::default()),
            capture: Some(Capture::default()),
            elevate: Some(Elevate::default()),
            launcher: Some(Launcher::default()),
            night_light: Some(NightLight::default()),
            displays: Some(Displays::default()),
            input: Some(Input::default()),
        }
    }

    /// Clamp what a hand-edited file may have put out of range. Zero on a
    /// timer is a valid "never" and is left alone.
    pub(super) fn sanitized(self) -> Settings {
        Settings {
            idle: self.idle.map(Idle::sanitized),
            keys: self.keys.map(Keys::sanitized),
            alerts: self.alerts.map(Alerts::sanitized),
            night_light: self.night_light.map(NightLight::sanitized),
            displays: self.displays.map(Displays::sanitized),
            input: self.input.map(Input::sanitized),
            ..self
        }
    }

    /// The section names and field names a file may carry.
    fn known(section: &str) -> Option<Vec<String>> {
        let probe = serde_json::to_value(Self::probe()).ok()?;
        Some(probe.get(section)?.as_object()?.keys().cloned().collect())
    }

    /// One value by dotted key, `idle.lock_after_s`, out of the effective
    /// settings. `None` for a key that is not a field.
    pub fn get(&self, key: &str) -> Option<Value> {
        let (section, field) = key.split_once('.')?;
        let all = serde_json::to_value(self.effective()).ok()?;
        all.get(section)?.get(field).cloned()
    }

    /// Set one field by dotted key. The section is taken from what is in
    /// force, so setting `idle.lock_after_s` on a fresh account produces an
    /// idle section with every other timer at the system default rather than
    /// at zero. A wrong key or a value of the wrong type is an error, and
    /// nothing changes.
    pub fn set(&mut self, key: &str, value: Value) -> Result<(), String> {
        let (section, field) = key
            .split_once('.')
            .ok_or_else(|| format!("`{key}` is not <section>.<field>"))?;
        let known = Self::known(section).ok_or_else(|| {
            format!(
                "no section `{section}`; one of {}",
                Self::SECTIONS.join(", ")
            )
        })?;
        // A field the struct does not have would be dropped silently by
        // serde, which reads as success. Check by name first.
        if !known.iter().any(|k| k == field) {
            return Err(format!(
                "no field `{field}` in `{section}`; one of {}",
                known.join(", ")
            ));
        }
        let all = serde_json::to_value(self.effective()).map_err(|e| e.to_string())?;
        let with = |value: Value| {
            let mut all = all.clone();
            let mut obj = all[section].as_object().cloned().unwrap_or_default();
            obj.insert(field.to_string(), value);
            all[section] = Value::Object(obj);
            serde_json::from_value::<Settings>(all)
        };
        // The command line hands every value over as a string unless it is a
        // number or a boolean. A field that takes a list or an object
        // (`displays.profiles`) gets the string read as JSON, and only then:
        // a path that happens to be valid JSON stays a path.
        let next = match with(value.clone()) {
            Ok(next) => next,
            Err(e) => match value
                .as_str()
                .and_then(|s| serde_json::from_str::<Value>(s).ok())
                .filter(|v| v.is_array() || v.is_object())
            {
                Some(parsed) => with(parsed).map_err(|e| format!("`{key}`: {e}"))?,
                None => return Err(format!("`{key}`: {e}")),
            },
        };
        let next = next.sanitized();
        // Only the section named moves; the rest of `next` is the effective
        // copy, which must not become an override.
        let mut mine = serde_json::to_value(&*self).map_err(|e| e.to_string())?;
        let mut moved = serde_json::to_value(&next).map_err(|e| e.to_string())?;
        mine[section] = moved[section].take();
        *self = serde_json::from_value(mine).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Drop one section, or every section, from the override.
    pub fn reset(&mut self, section: Option<&str>) -> Result<(), String> {
        let Some(section) = section else {
            *self = Settings::default();
            return Ok(());
        };
        if !Self::SECTIONS.contains(&section) {
            return Err(format!(
                "no section `{section}`; one of {}",
                Self::SECTIONS.join(", ")
            ));
        }
        let mut mine = serde_json::to_value(&*self).map_err(|e| e.to_string())?;
        mine[section] = Value::Null;
        *self = serde_json::from_value(mine).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// One section as the `theme/settings.nix` attrset it would be, for
    /// promoting a keeper into the Nix side by hand. `None` for the
    /// wallpaper, which the sway config owns, and for a name that is not a
    /// section.
    pub fn section_as_nix(&self, section: &str) -> Option<String> {
        if !Self::NIX_SECTIONS.contains(&section) {
            return None;
        }
        let all = serde_json::to_value(self.effective()).ok()?;
        let obj = all.get(section)?.as_object()?;
        let mut out = format!(
            "# swaypplet settings pane, live values. Into theme/settings.nix:\n{section} = {{\n"
        );
        for (key, value) in obj {
            let _ = writeln!(out, "  {key} = {};", nix_literal(value));
        }
        out.push_str("};\n");
        Some(out)
    }
}

/// A JSON value as Nix would have it written: a scalar, `null`, or a list
/// of them (`input.layouts`).
fn nix_literal(value: &Value) -> String {
    match value {
        Value::String(s) => format!("\"{}\"", s.replace('"', "\\\"")),
        Value::Array(items) => {
            let items: Vec<String> = items.iter().map(nix_literal).collect();
            format!("[ {} ]", items.join(" "))
        }
        other => other.to_string(),
    }
}

/// One section of [`Settings`], as a type: the accessor that resolves it
/// through the layers, and the slot in the user's override it lives in.
/// `store::edit` and `store::reset` are generic over this, which is what
/// keeps six panes from carrying the same edit-and-normalise dance.
pub trait Section: Clone + PartialEq + Default {
    /// The section in force: the user's, else the system's, else the
    /// binary's.
    fn in_force(settings: &Settings) -> Self;
    /// The user's override for it.
    fn slot(settings: &mut Settings) -> &mut Option<Self>;
}

macro_rules! section {
    ($ty:ident, $field:ident, $accessor:ident) => {
        impl Section for $ty {
            fn in_force(settings: &Settings) -> Self {
                settings.$accessor()
            }
            fn slot(settings: &mut Settings) -> &mut Option<Self> {
                &mut settings.$field
            }
        }
    };
}

section!(Look, look, look);
section!(Idle, idle, idle);
section!(Bar, bar, bar);
section!(Keys, keys, keys);
section!(Alerts, alerts, alerts);
section!(Capture, capture, capture);
section!(Elevate, elevate, elevate);
section!(Launcher, launcher, launcher);
section!(NightLight, night_light, night_light);
section!(Displays, displays, displays);
section!(Input, input, input);

/// Every `section` or `section.field` in `value` that the structs do not
/// have.
pub(crate) fn unknown_keys(value: &Value) -> Vec<String> {
    let Some(sections) = value.as_object() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (section, body) in sections {
        let Some(known) = Settings::known(section) else {
            out.push(section.clone());
            continue;
        };
        if let Some(fields) = body.as_object() {
            for field in fields.keys() {
                if !known.contains(field) {
                    out.push(format!("{section}.{field}"));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_launcher_section_round_trips_and_an_old_file_loads() {
        let mut s = Settings::default();
        s.set("launcher.files", Value::Bool(true)).unwrap();
        s.set("launcher.frecency", Value::Bool(false)).unwrap();
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
        let l = back.launcher();
        assert!(l.files && !l.frecency && l.apps && l.calculator);
        // A section written before a switch existed takes that switch's
        // default rather than failing to load.
        let old: Settings = serde_json::from_str(r#"{"launcher":{"apps":false}}"#).unwrap();
        let l = old.launcher();
        assert!(!l.apps && l.windows && !l.files && l.frecency);
        assert!(s.set("launcher.nope", Value::Bool(true)).is_err());
    }

    #[test]
    fn an_empty_file_is_the_defaults() {
        let s: Settings = serde_json::from_str("{}").unwrap();
        assert!(s.is_default());
        // The test process has no system file (SYSTEM_CONFIG is not on a
        // build sandbox), so the section in force is the binary's.
        assert_eq!(s.idle(), Idle::default());
        assert_eq!(s.bar(), Bar::default());
        assert_eq!(s.keys(), Keys::default());
        assert_eq!(s.alerts(), Alerts::default());
        assert_eq!(s.capture(), Capture::default());
        assert_eq!(s.look(), Look::default());
        assert_eq!(s.elevate(), Elevate::default());
        assert_eq!(s.night_light(), NightLight::default());
    }

    #[test]
    fn a_list_from_the_command_line_sets_the_profiles() {
        let mut s = Settings::default();
        let raw = r#"[{"name": "desk", "outputs": [{"match": {"name": "eDP-1"}}]}]"#;
        s.set("displays.profiles", Value::String(raw.into())).unwrap();
        assert_eq!(s.displays().profiles[0].name, "desk");
        // A string field keeps a JSON-looking string as the string it is.
        s.set("wallpaper.path", Value::String("[1]".into())).ok();
        assert!(s.set("displays.profiles", Value::String("desk".into())).is_err());
    }

    #[test]
    fn the_displays_section_round_trips_in_kanshis_terms() {
        let text = r#"{"displays": {"profiles": [
            {"name": "desk", "outputs": [
                {"match": {"name": "eDP-1"}, "mode": [2880, 1800, 60000],
                 "position": [2560, 149], "scale": 2.0},
                {"match": {"make": "NON", "model": "28H2U"}, "scale": 1.5,
                 "transform": "90"}]},
            {"name": "", "outputs": [{"match": {}}]},
            {"name": "empty"}]}}"#;
        let s: Settings = serde_json::from_str(text).unwrap();
        let s = s.sanitized();
        let d = s.displays();
        // The nameless and the outputless profiles cannot be applied.
        assert_eq!(d.profiles.len(), 1);
        let p = &d.profiles[0];
        assert_eq!(p.outputs[0].criteria.name.as_deref(), Some("eDP-1"));
        assert!(p.outputs[0].enabled);
        assert_eq!(p.outputs[1].transform, Some(OutputTransform::R90));
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
        // An unset field is left out of the file, not written as null.
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("null"), "{json}");
    }

    #[test]
    fn the_input_sanitizer_keeps_anything_that_could_end_the_quote_out() {
        let s: Settings = serde_json::from_str(
            r#"{"input": {"layouts": ["se", "us\"; exec rm", ""], "caps": "caps:escape\"",
                          "touchpad_speed": 4.0, "repeat_rate": 0}}"#,
        )
        .unwrap();
        let input = s.sanitized().input.unwrap();
        assert_eq!(input.layouts, Some(vec!["se".to_string()]));
        assert_eq!(input.caps, None);
        assert_eq!(input.touchpad_speed, Some(1.0));
        assert_eq!(input.repeat_rate, Some(1));
        assert_eq!(input.touchpad_tap, None);
    }

    #[test]
    fn the_input_section_sets_by_key_and_exports_a_nix_list() {
        let mut s = Settings::default();
        s.set(
            "input.layouts",
            Value::String(r#"["se", "us(dvorak)"]"#.into()),
        )
        .unwrap();
        s.set("input.touchpad_tap", Value::Bool(true)).unwrap();
        assert_eq!(s.input().touchpad_tap, Some(true));
        let nix = s.section_as_nix("input").unwrap();
        assert!(
            nix.contains(r#"  layouts = [ "se" "us(dvorak)" ];"#),
            "{nix}"
        );
        assert!(nix.contains("  mouse_speed = null;"), "{nix}");
        assert!(
            s.set(
                "input.touchpad_click_method",
                Value::String("sideways".into())
            )
            .is_err()
        );
    }

    #[test]
    fn a_night_temperature_out_of_range_is_clamped() {
        let s: Settings = serde_json::from_str(r#"{"night_light": {"night_k": 900}}"#).unwrap();
        let s = s.sanitized();
        assert_eq!(s.night_light().night_k, NightLight::MIN_K);
        assert!(s.night_light().enabled);
    }

    #[test]
    fn a_partial_section_lands_on_the_defaults_for_the_rest() {
        // A hand-edited file that names one timer must not zero the others:
        // zero is "never", and a lock that silently became "never" is the
        // failure mode the idle manager exists to prevent.
        let s: Settings = serde_json::from_str(r#"{"idle": {"lock_after_s": 600}}"#).unwrap();
        let idle = s.idle();
        assert_eq!(idle.lock_after_s, 600);
        assert_eq!(idle.dim_after_s, 240);
        assert_eq!(idle.blank_after_s, 900);
        assert_eq!(idle.suspend_after_s, 1200);
        assert_eq!(idle.dim_level, 10);
        assert!(idle.walk_away_lock && idle.face_unlock);
    }

    #[test]
    fn only_overridden_sections_are_written() {
        let s = Settings {
            bar: Some(Bar {
                clock_24h: false,
                ..Bar::default()
            }),
            ..Settings::default()
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"bar\""));
        assert!(!json.contains("\"idle\""));
        assert!(!json.contains("\"wallpaper\""));
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn set_by_key_fills_the_rest_of_the_section_from_what_is_in_force() {
        let mut s = Settings::default();
        s.set("idle.lock_after_s", serde_json::json!(600)).unwrap();
        let idle = s.idle.unwrap();
        assert_eq!(idle.lock_after_s, 600);
        assert_eq!(idle.dim_after_s, Idle::default().dim_after_s);
        assert_eq!(s.get("idle.lock_after_s"), Some(serde_json::json!(600)));
        // The other sections are untouched and still read as the default.
        assert!(s.bar.is_none() && s.keys.is_none() && s.alerts.is_none());
        assert_eq!(s.get("bar.clock_24h"), Some(serde_json::json!(true)));
        s.set("look.motion", serde_json::json!("off")).unwrap();
        assert_eq!(s.look().motion, Motion::Off);
        s.set("capture.after", serde_json::json!("copy")).unwrap();
        assert_eq!(s.capture().after, After::Copy);
    }

    #[test]
    fn set_refuses_what_the_struct_would_silently_drop() {
        let mut s = Settings::default();
        assert!(s.set("idle.lock_after", serde_json::json!(1)).is_err());
        assert!(s.set("lock_after_s", serde_json::json!(1)).is_err());
        assert!(s.set("night.temp", serde_json::json!(1)).is_err());
        // The wrong type is an error too, and nothing changed.
        assert!(s.set("bar.clock_24h", serde_json::json!("yes")).is_err());
        assert!(s.set("idle.dim_level", serde_json::json!(-4)).is_err());
        assert!(s.set("look.motion", serde_json::json!("fast")).is_err());
        assert!(s.is_default());
        // A wallpaper needs a path before a mode means anything.
        assert!(s.set("wallpaper.mode", serde_json::json!("fit")).is_err());
        s.set("wallpaper.path", serde_json::json!("/tmp/a.png"))
            .unwrap();
        s.set("wallpaper.mode", serde_json::json!("fit")).unwrap();
        assert_eq!(s.wallpaper.as_ref().unwrap().mode, WallpaperMode::Fit);
        assert!(s.set("wallpaper.mode", serde_json::json!("cover")).is_err());
    }

    #[test]
    fn reset_drops_one_section_or_all() {
        let mut s = Settings::default();
        s.set("idle.dim_level", serde_json::json!(40)).unwrap();
        s.set("bar.board", serde_json::json!(true)).unwrap();
        s.reset(Some("idle")).unwrap();
        assert!(s.idle.is_none() && s.bar.is_some());
        assert!(s.reset(Some("glass")).is_err());
        s.reset(None).unwrap();
        assert!(s.is_default());
    }

    #[test]
    fn the_nix_export_is_the_section_as_settings_nix_holds_it() {
        let mut s = Settings::default();
        s.set("bar.clock_date", serde_json::json!(true)).unwrap();
        let nix = s.section_as_nix("bar").unwrap();
        assert!(nix.contains("bar = {\n"), "{nix}");
        assert!(nix.contains("  clock_24h = true;\n"), "{nix}");
        assert!(nix.contains("  clock_date = true;\n"), "{nix}");
        assert!(nix.ends_with("};\n"));
        let nix = s.section_as_nix("idle").unwrap();
        assert!(nix.contains("  dim_after_s = 240;\n"), "{nix}");
        assert!(s.section_as_nix("wallpaper").is_none());
        assert_eq!(nix_literal(&serde_json::json!("a\"b")), "\"a\\\"b\"");
    }

    /// 21:30–07:00, the shape the window is for: it wraps midnight, it is
    /// half-open at both ends, and the minute is part of the bound.
    #[test]
    fn the_night_window_wraps_past_midnight_and_is_half_open() {
        let at = |h: u16, m: u16| h * 60 + m;
        let night = Idle {
            night: true,
            night_from_h: 21,
            night_from_m: 30,
            night_to_h: 7,
            night_to_m: 0,
            ..Idle::default()
        };
        assert!(night.in_night(at(21, 30)) && night.in_night(at(23, 59)));
        assert!(night.in_night(at(0, 0)) && night.in_night(at(6, 59)));
        assert!(!night.in_night(at(21, 29)) && !night.in_night(at(7, 0)));
        assert!(!night.in_night(at(12, 0)));

        // A window inside one day, and the whole-day case.
        let day = Idle {
            night_from_h: 9,
            night_from_m: 0,
            night_to_h: 17,
            night_to_m: 15,
            ..night
        };
        assert!(day.in_night(at(9, 0)) && day.in_night(at(17, 14)));
        assert!(!day.in_night(at(17, 15)) && !day.in_night(at(2, 0)));
        let whole = Idle {
            night_from_h: 5,
            night_from_m: 0,
            night_to_h: 5,
            night_to_m: 0,
            ..night
        };
        assert!(whole.in_night(at(4, 59)) && whole.in_night(at(5, 0)));

        // The switch is the gate: the same bounds with `night` off are never
        // inside, so no caller has to test both.
        let off = Idle {
            night: false,
            ..night
        };
        assert!(!off.in_night(at(22, 0)));
    }

    #[test]
    fn resolve_swaps_three_tiers_inside_the_window_and_none_outside() {
        let cfg = Idle {
            night: true,
            night_from_h: 21,
            night_from_m: 0,
            night_to_h: 7,
            night_to_m: 0,
            night_dim_after_s: 60,
            night_lock_after_s: 300,
            night_blank_after_s: 120,
            dim_after_s: 240,
            lock_after_s: 1800,
            blank_after_s: 900,
            suspend_after_s: 1200,
            ..Idle::default()
        };
        assert_eq!(cfg.resolve(12 * 60), cfg);

        let night = cfg.resolve(22 * 60);
        assert_eq!(
            (night.dim_after_s, night.lock_after_s, night.blank_after_s),
            (60, 300, 120)
        );
        // Suspend is battery-only and never substituted; an overnight job on
        // a machine left running must not be cut short by the window.
        assert_eq!(night.suspend_after_s, cfg.suspend_after_s);
        // The window's own fields survive, so resolving again is a no-op.
        assert_eq!(night.resolve(22 * 60), night);
    }

    #[test]
    fn a_typed_window_bound_is_clamped_not_rejected() {
        let bad = Idle {
            night_from_h: 40,
            night_from_m: 99,
            night_to_h: 25,
            night_to_m: 60,
            ..Idle::default()
        }
        .sanitized();
        assert_eq!(
            (
                bad.night_from_h,
                bad.night_from_m,
                bad.night_to_h,
                bad.night_to_m
            ),
            (23, 59, 23, 59)
        );
    }

    #[test]
    fn quiet_hours_wrap_past_midnight() {
        let a = Alerts {
            quiet_from_h: 22,
            quiet_to_h: 7,
            ..Alerts::default()
        };
        assert!(a.in_quiet_hours(22) && a.in_quiet_hours(23) && a.in_quiet_hours(3));
        assert!(!a.in_quiet_hours(7) && !a.in_quiet_hours(12) && !a.in_quiet_hours(21));
        let day = Alerts {
            quiet_from_h: 9,
            quiet_to_h: 17,
            ..Alerts::default()
        };
        assert!(day.in_quiet_hours(9) && day.in_quiet_hours(16));
        assert!(!day.in_quiet_hours(17) && !day.in_quiet_hours(2));
        let whole = Alerts {
            quiet_from_h: 5,
            quiet_to_h: 5,
            ..Alerts::default()
        };
        assert!(whole.in_quiet_hours(4) && whole.in_quiet_hours(5) && whole.in_quiet_hours(6));
    }

    #[test]
    fn wallpaper_modes_spell_themselves_the_way_sway_reads_them() {
        for mode in WallpaperMode::ALL {
            assert_eq!(WallpaperMode::parse(mode.as_str()), Some(mode));
            let json = serde_json::to_string(&mode).unwrap();
            assert_eq!(json, format!("\"{}\"", mode.as_str()));
        }
        assert_eq!(WallpaperMode::parse("cover"), None);
    }

    /// `data/settings-defaults.json` is what `cross-repo-guard.nix` checks
    /// `theme/settings.nix` against, so it has to be exactly the shape and
    /// the defaults this build carries. Regenerate it with
    /// `cargo test -- --ignored write_settings_defaults` when a field is
    /// added.
    fn defaults_json() -> String {
        let mut probe = Settings::probe();
        probe.wallpaper = None;
        let mut json = serde_json::to_string_pretty(&probe).unwrap();
        json.push('\n');
        json
    }

    #[test]
    fn the_shipped_defaults_file_matches_the_structs() {
        let shipped = include_str!("../../data/settings-defaults.json");
        assert_eq!(
            shipped,
            defaults_json(),
            "data/settings-defaults.json is stale: cargo test -- --ignored write_settings_defaults"
        );
    }

    #[test]
    #[ignore]
    fn write_settings_defaults() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/data/settings-defaults.json");
        std::fs::write(path, defaults_json()).unwrap();
    }
}
