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
}

impl Idle {
    /// The file is hand-editable, and a dim level of 0 is a screen that
    /// goes black on the first idle tick and stays that way for anyone who
    /// does not know why. Bound it; `blank_after_s` and the timers already
    /// mean "never" at zero, which is a valid ask.
    fn sanitized(self) -> Idle {
        Idle {
            dim_level: self.dim_level.clamp(1, 100),
            ..self
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

/// The Look tab's second group. The wallpaper is the first and has its own
/// section, since it has no system layer in this file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Look {
    #[serde(default)]
    pub motion: Motion,
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
            crate::audio::VOLUME_CEILING
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
}

impl Settings {
    /// The section names, in the order the file and the pane list them.
    pub const SECTIONS: [&'static str; 7] = [
        "wallpaper",
        "look",
        "idle",
        "bar",
        "keys",
        "alerts",
        "capture",
    ];

    /// The sections with a system layer, which is every one but the
    /// wallpaper: its system default is the sway config's `bg` line.
    pub const NIX_SECTIONS: [&'static str; 6] =
        ["look", "idle", "bar", "keys", "alerts", "capture"];

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
        }
    }

    /// Clamp what a hand-edited file may have put out of range. Zero on a
    /// timer is a valid "never" and is left alone.
    pub(super) fn sanitized(self) -> Settings {
        Settings {
            idle: self.idle.map(Idle::sanitized),
            keys: self.keys.map(Keys::sanitized),
            alerts: self.alerts.map(Alerts::sanitized),
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
        let mut all = serde_json::to_value(self.effective()).map_err(|e| e.to_string())?;
        let mut obj = all[section].as_object().cloned().unwrap_or_default();
        obj.insert(field.to_string(), value);
        all[section] = Value::Object(obj);
        let next: Settings = serde_json::from_value(all).map_err(|e| format!("`{key}`: {e}"))?;
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

/// A JSON scalar as Nix would have it written. Every field here is a
/// number, a bool or a string, so this is the whole translation.
fn nix_literal(value: &Value) -> String {
    match value {
        Value::String(s) => format!("\"{}\"", s.replace('"', "\\\"")),
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
