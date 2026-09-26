//! The theme inputs right now (docs/design-system.md §2): the Look
//! settings, with `auto` resolved by the sun and held back until a switch
//! cannot happen in front of someone.

use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

use crate::settings::schema::ThemeMode;
use crate::tokens::{Inputs, Mode};

/// Light mode waits until every surface is on the tokens: a legacy rule
/// still naming a dark palette colour would sit on light glass. Until then
/// `auto` and `light` resolve to dark, except through `SWAYPPLET_MODE`,
/// which the render harness uses to check the light work in progress.
const LIGHT_READY: bool = false;

/// How long a sun-driven switch may wait for the session to be locked
/// before it happens anyway (§2.1).
const PATIENCE: Duration = Duration::from_secs(600);

thread_local! {
    /// The mode on screen, for the sun's hysteresis band.
    static SHOWN: Cell<Mode> = const { Cell::new(Mode::Dark) };
    /// Whether a mode has been resolved in this process yet: the first one
    /// applies at once, since nothing is on screen to switch under anyone.
    static STARTED: Cell<bool> = const { Cell::new(false) };
    /// A mode the sun asks for that is not on screen yet, and since when.
    static PENDING: RefCell<Option<(Mode, Instant)>> = const { RefCell::new(None) };
}

/// The mode last resolved.
pub(super) fn shown() -> Mode {
    SHOWN.with(Cell::get)
}

/// Where the sun is computed for: `/etc/swaypplet/theme.json`
/// (`{"latitude": …, "longitude": …}`, written by Nix from the night light's
/// location), or `SWAYPPLET_THEME_CONFIG`.
fn location() -> Option<(f64, f64)> {
    let path = std::env::var("SWAYPPLET_THEME_CONFIG")
        .unwrap_or_else(|_| "/etc/swaypplet/theme.json".to_string());
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    Some((v["latitude"].as_f64()?, v["longitude"].as_f64()?))
}

fn now_unix() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// The mode the sun asks for, or dark where no location is known.
fn sun_mode() -> Mode {
    location()
        .map(|(lat, lon)| {
            crate::tokens::sun::mode(crate::tokens::sun::elevation(lat, lon, now_unix()), shown())
        })
        .unwrap_or(Mode::Dark)
}

/// `wanted` if it may be shown now: at startup, while the session is
/// locked, or once it has waited `PATIENCE`. Otherwise the mode on screen,
/// with `wanted` remembered.
fn when_unseen(wanted: Mode) -> Mode {
    let now = shown();
    if wanted == now || !STARTED.with(Cell::get) {
        PENDING.with(|p| *p.borrow_mut() = None);
        return wanted;
    }
    let since = PENDING.with(|p| {
        let mut p = p.borrow_mut();
        match *p {
            Some((m, t)) if m == wanted => t,
            _ => {
                *p = Some((wanted, Instant::now()));
                Instant::now()
            }
        }
    });
    if super::locked::is_locked() || since.elapsed() >= PATIENCE {
        PENDING.with(|p| *p.borrow_mut() = None);
        wanted
    } else {
        now
    }
}

/// The theme inputs right now.
pub fn inputs() -> Inputs {
    let look = crate::settings::store::with(|s| s.look());
    let forced = match std::env::var("SWAYPPLET_MODE").as_deref() {
        Ok("light") => Some(Mode::Light),
        Ok("dark") => Some(Mode::Dark),
        _ => None,
    };
    let mode = forced.unwrap_or_else(|| {
        if !LIGHT_READY {
            return Mode::Dark;
        }
        match look.mode {
            // A choice made in the pane applies at once: the person made it.
            ThemeMode::Dark => Mode::Dark,
            ThemeMode::Light => Mode::Light,
            ThemeMode::Auto => when_unseen(sun_mode()),
        }
    });
    SHOWN.with(|s| s.set(mode));
    STARTED.with(|s| s.set(true));
    Inputs {
        mode,
        accent: look.accent,
        neutral: look.neutral,
        contrast: look.contrast,
        motion: (look.motion.scale() * 100.0).round() as u8,
    }
}
