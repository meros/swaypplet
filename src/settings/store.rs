//! The settings file: what the pane changes that is not the glass material.
//!
//! Two files, three optional sections each, and the same shape as glass:
//!
//! - `/etc/swaypplet/settings.json` is the *system default*, written by Nix
//!   (`users/modules/theme/settings.nix` in the nixos repo). Optional; a
//!   host without one gets the constants this binary ships, which are the
//!   same numbers.
//! - `~/.config/swaypplet/settings.json` is the user's override. A section
//!   that is present replaces the system's section whole; one that is
//!   absent means "the system default", which is what Reset returns to.
//!
//! So the user file is a record of what was changed rather than a dump of
//! every value, and a fresh account has no file at all. The wallpaper has no
//! system section here: its system default is the sway config's own `bg`
//! line, which `wallpaper::system_default` reads back from the compositor.
//!
//! Three processes touch the user file and none of them share memory. The
//! panel (`app::run`) writes on every edit, coalesced by [`SAVE_DEBOUNCE_MS`],
//! and publishes the new value in-process through [`observe`] so the bar's
//! clock and the OSD route follow without a restart. `swaypplet settings`
//! (`cli.rs`) writes it from a keybind or a script. The idle manager
//! (`idle::run`) only reads. So every reader watches the file's mtime on a
//! one-second tick — the idle loop on its own (`idle/mod.rs`), the panel
//! through [`watch`] — and reloads when it moves. Nothing here talks to
//! systemd or sway.
//!
//! Glass is deliberately not in this file. `glass.json` is an override of a
//! *system* material that Nix ships and the pane exports back to Nix; the
//! values here have no Nix side and no export. Two files with two meanings
//! read better than one file with a footnote.

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::SystemTime;

use serde_json::Value;

use crate::service::Observed;

pub use super::schema::*;

/// How long after the last edit the file is written. Long enough that a
/// slider drag is one write, short enough that a switch flipped just before
/// the panel is closed still lands.
const SAVE_DEBOUNCE_MS: u64 = 800;

/// Where Nix leaves the system defaults. `SWAYPPLET_SETTINGS_CONFIG`
/// overrides it, for the render harness and for trying a set of defaults
/// without a rebuild.
const SYSTEM_CONFIG: &str = "/etc/swaypplet/settings.json";

/// Which file is being read, which decides how loud an unknown key is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Layer {
    /// Nix wrote it, from a table `cross-repo-guard.nix` checks against
    /// `data/settings-defaults.json`; a key that still gets here is a bug,
    /// and it is logged as one.
    System,
    /// A person or the CLI wrote it; a stale or misspelled key is a warning.
    User,
}

/// One settings file, either layer. Missing is empty; unparseable is empty
/// with a warning; a key the structs do not have is reported and dropped,
/// which is what serde would have done silently; a value out of range is
/// clamped.
fn read(path: &std::path::Path, layer: Layer) -> Settings {
    let Ok(raw) = std::fs::read(path) else {
        return Settings::default();
    };
    let value: Value = match serde_json::from_slice(&raw) {
        Ok(v) => v,
        Err(e) => {
            log::warn!("settings: ignoring unreadable {}: {e}", path.display());
            return Settings::default();
        }
    };
    for key in unknown_keys(&value) {
        match layer {
            Layer::System => log::error!(
                "settings: {} has `{key}`, which this build does not know — ignored; \
                 theme/settings.nix and data/settings-defaults.json disagree",
                path.display()
            ),
            Layer::User => log::warn!(
                "settings: {} has `{key}`, which this build does not know — ignored",
                path.display()
            ),
        }
    }
    match serde_json::from_value::<Settings>(value) {
        Ok(settings) => settings.sanitized(),
        Err(e) => {
            log::warn!("settings: ignoring {}: {e}", path.display());
            Settings::default()
        }
    }
}

impl Settings {
    /// The user's file, or nothing overridden when there is none or it does
    /// not parse. A stale file is logged and ignored rather than failing the
    /// caller: the system default is always a safe answer.
    pub fn load() -> Settings {
        read(&path(), Layer::User)
    }

    /// Write the file, or remove it when everything is back at the default.
    pub fn save(&self) {
        let path = path();
        if self.is_default() {
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => log::warn!("settings: cannot remove {}: {e}", path.display()),
            }
            return;
        }
        if let Some(parent) = path.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            log::warn!("settings: cannot create {}: {e}", parent.display());
            return;
        }
        match serde_json::to_vec_pretty(self) {
            // With a final newline: the file is meant to be opened in an
            // editor as well as by this module.
            Ok(mut json) => {
                json.push(b'\n');
                if let Err(e) = std::fs::write(&path, json) {
                    log::warn!("settings: cannot write {}: {e}", path.display());
                }
            }
            Err(e) => log::warn!("settings: cannot serialise: {e}"),
        }
    }
}

/// The system defaults, read once per process. Nix rewrites the file only
/// on a switch, which restarts every unit that reads it.
pub fn system() -> &'static Settings {
    static SYSTEM: OnceLock<Settings> = OnceLock::new();
    SYSTEM.get_or_init(|| {
        let path = std::env::var("SWAYPPLET_SETTINGS_CONFIG")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(SYSTEM_CONFIG));
        read(&path, Layer::System)
    })
}

/// `~/.config/swaypplet/settings.json`.
pub fn path() -> PathBuf {
    let dir = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    dir.join("swaypplet").join("settings.json")
}

/// The file's mtime, or `None` when there is no file. An appearance or a
/// removal is a change too.
pub fn mtime() -> Option<SystemTime> {
    std::fs::metadata(path()).ok()?.modified().ok()
}

/// The user file followed by mtime, one `stat` per `changed`. Both readers
/// of the file poll it this way: the idle loop on its own tick
/// (`idle/mod.rs`), the panel through [`watch`].
pub struct Watch {
    last: Option<SystemTime>,
}

impl Watch {
    /// Start from the file as it is now, so the first `changed` is a real
    /// change and not the initial read.
    pub fn new() -> Watch {
        Watch { last: mtime() }
    }

    /// Whether the file moved since the last call.
    pub fn changed(&mut self) -> bool {
        let now = mtime();
        if now == self.last {
            return false;
        }
        self.last = now;
        true
    }
}

impl Default for Watch {
    fn default() -> Self {
        Self::new()
    }
}

// ── The panel's live copy ───────────────────────────────────────────────
//
// Main thread only, like every widget that reads it. The idle manager never
// touches this half: it reads the file.

thread_local! {
    static LIVE: Observed<Settings> = Observed::new(Settings::default());
    static SAVE_TIMER: RefCell<Option<glib::SourceId>> = const { RefCell::new(None) };
}

/// Read the file into the live copy. Once, at panel startup, before anything
/// that observes it is built.
pub fn init() {
    LIVE.with(|live| live.set(Settings::load()));
}

/// Follow the file for the life of the process: a write by `swaypplet
/// settings` (or by hand) lands in the live copy within a second. The
/// panel's own saves move the mtime too, and reload to what is already
/// live, which `set_if_changed` drops.
pub fn watch() {
    let mut watch = Watch::new();
    glib::timeout_add_local(std::time::Duration::from_secs(1), move || {
        if watch.changed() {
            LIVE.with(|live| live.set_if_changed(Settings::load()));
        }
        glib::ControlFlow::Continue
    });
}

/// A snapshot of the live copy. For one field on a hot path, [`with`]
/// reads without the clone.
pub fn current() -> Settings {
    LIVE.with(|live| live.with(Clone::clone))
}

/// Read the live copy in place. Do not call [`update`] from inside `f`.
pub fn with<R>(f: impl FnOnce(&Settings) -> R) -> R {
    LIVE.with(|live| live.with(f))
}

/// Run when the live copy changes. The callback sees the new value through
/// [`current`].
pub fn observe(cb: impl Fn() + 'static) {
    LIVE.with(|live| live.connect_change(cb));
}

/// Change the live copy, notify observers now, and write the file after the
/// debounce. A no-op edit is dropped before it reaches either.
pub fn update(edit: impl FnOnce(&mut Settings)) {
    let mut next = current();
    edit(&mut next);
    LIVE.with(|live| {
        if live.with(|cur| *cur == next) {
            return;
        }
        live.set(next);
        schedule_save();
    });
}

/// Change one section of the live copy: take what is in force, let `f`
/// change it, and store it as an override — or as no override, when it is
/// back at the binary's default, which keeps the file honest about what
/// was changed.
pub fn edit<S: Section>(f: impl FnOnce(&mut S)) {
    update(|s| {
        let mut section = S::in_force(s);
        f(&mut section);
        *S::slot(s) = (section != S::default()).then_some(section);
    });
}

/// Drop one section's override from the live copy.
pub fn reset<S: Section>() {
    update(|s| *S::slot(s) = None);
}

fn schedule_save() {
    SAVE_TIMER.with(|timer| {
        if let Some(id) = timer.borrow_mut().take() {
            crate::spawn::remove_source(id);
        }
        let id = glib::timeout_add_local_once(
            std::time::Duration::from_millis(SAVE_DEBOUNCE_MS),
            || {
                SAVE_TIMER.with(|timer| timer.borrow_mut().take());
                current().save();
            },
        );
        *timer.borrow_mut() = Some(id);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("swaypplet-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn out_of_range_values_are_clamped_and_unknown_keys_reported() {
        let file = scratch("clamp.json");
        std::fs::write(
            &file,
            r#"{"idle": {"dim_level": 0, "lock_after_s": 0}, "keys": {"volume_step": 90},
                "alerts": {"stack": 9, "quiet_to_h": 40}, "night": {"temp": 1}, "bar": {"clok": true}}"#,
        )
        .unwrap();
        let value: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(unknown_keys(&value), vec!["bar.clok", "night"]);
        let s = read(&file, Layer::User);
        let idle = s.idle.unwrap();
        assert_eq!(idle.dim_level, 1);
        // Zero on a timer is a valid "never" and is left alone.
        assert_eq!(idle.lock_after_s, 0);
        assert_eq!(s.keys.unwrap().volume_step, 25);
        assert_eq!(s.alerts.unwrap().stack, 5);
        assert_eq!(s.alerts.unwrap().quiet_to_h, 23);
        // The unknown key is dropped; the section it sat in still loads.
        assert!(s.bar.is_some());
        // A file that is not JSON is the same as no file.
        std::fs::write(&file, "not json").unwrap();
        assert!(read(&file, Layer::System).is_default());
    }

    #[test]
    fn the_system_layer_reads_a_full_file_without_consulting_itself() {
        // `read(_, System)` is what `system()` runs inside its OnceLock. If
        // anything on this path reached for `system()` the real process
        // would hang; here it would recurse into `read` and, at worst,
        // overflow. Either way the shape of the call is what is guarded.
        let file = scratch("system.json");
        std::fs::write(&file, include_str!("../../data/settings-defaults.json")).unwrap();
        let s = read(&file, Layer::System);
        assert_eq!(s.idle, Some(Idle::default()));
        assert_eq!(s.alerts, Some(Alerts::default()));
        assert!(s.wallpaper.is_none());
    }
}
