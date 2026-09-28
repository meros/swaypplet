//! The theme inputs right now (docs/design-system.md §2): the Look
//! settings, with `auto` resolved by the sun and held back until a switch
//! cannot happen in front of someone.

use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

use super::wallpaper::Sample;
use crate::settings::schema::{Look, ThemeMode, Tint as Reach};
use crate::tokens::{Backdrop, Inputs, Mode, Tint};

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
    /// The tint last resolved.
    static TINT: Cell<Tint> = const { Cell::new(Tint::Off) };
    /// The wallpaper text's backdrop last read.
    static BACKDROP: Cell<Option<Backdrop>> = const { Cell::new(None) };
    /// The Look mode setting last resolved, to tell a choice made just now
    /// from the sun moving.
    static SETTING: Cell<Option<ThemeMode>> = const { Cell::new(None) };
    /// Set by [`own_mode`]: this process resolves `auto` and publishes it.
    static OWNER: Cell<bool> = const { Cell::new(false) };
    /// The mode last written to [`mode_file`], so a tick that moved nothing
    /// writes nothing.
    static PUBLISHED: Cell<Option<Mode>> = const { Cell::new(None) };
}

/// Make this process the one that resolves `auto` (the panel).
///
/// The sun's band holds whatever mode a process already shows, and a sun
/// switch waits for the lock or for `PATIENCE`, so eight processes that each
/// resolved `auto` from their own start time could disagree: the panel sent
/// light glass while another process kept dark mode's white text on it. Now
/// one process resolves, writes the answer to [`mode_file`], and every other
/// process draws what that file says. The glass is sent from the same
/// resolution, so the text and the glass cannot differ.
pub fn own_mode() {
    OWNER.with(|o| o.set(true));
}

/// Where the owner writes the mode `auto` resolved to: one word, `dark` or
/// `light`. In the runtime directory, so a new session never reads the last
/// one's answer.
pub fn mode_file() -> std::path::PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").unwrap_or_else(|| "/tmp".into());
    std::path::PathBuf::from(dir).join("swaypplet").join("mode")
}

/// The mode the owner published, or `None` before it has (the panel is not
/// up yet, or not running).
fn published() -> Option<Mode> {
    match std::fs::read_to_string(mode_file()).ok()?.trim() {
        "dark" => Some(Mode::Dark),
        "light" => Some(Mode::Light),
        _ => None,
    }
}

/// Write `mode` for the other processes, when it moved. Next to the file and
/// renamed over it, so a reader sees the old word or the new one.
fn publish(mode: Mode) {
    if PUBLISHED.with(|p| p.replace(Some(mode))) == Some(mode) {
        return;
    }
    let path = mode_file();
    let word = match mode {
        Mode::Dark => "dark\n",
        Mode::Light => "light\n",
    };
    let tmp = path.with_extension("tmp");
    let written = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&tmp, word))
        .and_then(|()| std::fs::rename(&tmp, &path));
    if let Err(e) = written {
        log::warn!("theme: cannot write {}: {e}", path.display());
        PUBLISHED.with(|p| p.set(None));
    }
}

/// The mode last resolved.
fn shown_mode() -> Mode {
    SHOWN.with(Cell::get)
}

/// The inputs with the mode and the tint as they were last resolved rather
/// than resolved again: see `theme::shown`.
pub(super) fn shown() -> Inputs {
    let look = crate::settings::store::with(|s| s.look());
    build(
        &look,
        SHOWN.with(Cell::get),
        TINT.with(Cell::get),
        BACKDROP.with(Cell::get),
    )
}

/// The one place outside tests that an `Inputs` is made: the Look settings,
/// with a mode and a tint already resolved. [`inputs`] resolves them and
/// [`shown`] reuses the last resolution; both end here, so the two cannot
/// map a Look setting differently.
fn build(look: &Look, mode: Mode, tint: Tint, backdrop: Option<Backdrop>) -> Inputs {
    Inputs {
        mode,
        accent: look.accent,
        neutral: look.neutral,
        contrast: look.contrast,
        motion: (look.motion.scale() * 100.0).round() as u8,
        tint,
        backdrop,
        lifted: crate::settings::glass::compositor_lifts(),
    }
}

/// The Look setting's reach with the wallpaper's hues, as the panel last
/// sampled it (`super::wallpaper`), the accent on the colour the Look pane
/// picked. Off until a sample exists: a wallpaper with no usable colour, or
/// one not sampled yet, leaves the tokens shipped.
fn tint(reach: Reach, sample: Option<&Sample>, pick: u8) -> Tint {
    if reach == Reach::Off {
        return Tint::Off;
    }
    match (reach, sample.map(|s| s.palette(usize::from(pick)))) {
        (Reach::Accents, Some(p)) => Tint::Accents(p),
        (Reach::Full, Some(p)) => Tint::Full(p),
        _ => Tint::Off,
    }
}

/// The mode the sun asks for, or dark where no location is known.
fn sun_mode() -> Mode {
    super::sun::elevation_now()
        .map(|e| super::sun::mode(e, shown_mode()))
        .unwrap_or(Mode::Dark)
}

/// `wanted` if it may be shown now: at startup, while the session is
/// locked, or once it has waited `PATIENCE`. Otherwise the mode on screen,
/// with `wanted` remembered.
fn when_unseen(wanted: Mode) -> Mode {
    let now = shown_mode();
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

/// The mode Auto shows: the sun's, at once when Auto was chosen just now
/// (the person asked, and is looking at it), else when nobody is looking
/// ([`when_unseen`]).
fn auto_mode(sun: Mode, chosen_now: bool) -> Mode {
    if chosen_now {
        PENDING.with(|p| *p.borrow_mut() = None);
        return sun;
    }
    when_unseen(sun)
}

/// The theme inputs right now.
pub fn inputs() -> Inputs {
    let look = crate::settings::store::with(|s| s.look());
    // Whether the mode setting moved since the last resolution: then the
    // person just chose it, and Auto shows the sun's answer at once.
    let chosen_now = SETTING.with(|c| c.replace(Some(look.mode))) != Some(look.mode);
    let forced = match std::env::var("SWAYPPLET_MODE").as_deref() {
        Ok("light") => Some(Mode::Light),
        Ok("dark") => Some(Mode::Dark),
        _ => None,
    };
    // `SWAYPPLET_MODE` forces a mode: the render harness checks both.
    let mode = forced.unwrap_or_else(|| {
        match look.mode {
            // A choice made in the pane applies at once: the person made it.
            ThemeMode::Dark => Mode::Dark,
            ThemeMode::Light => Mode::Light,
            // The owner resolves; everyone else draws what it resolved,
            // and resolves alone only while it has published nothing.
            ThemeMode::Auto if !OWNER.with(Cell::get) => {
                published().unwrap_or_else(|| auto_mode(sun_mode(), chosen_now))
            }
            ThemeMode::Auto => auto_mode(sun_mode(), chosen_now),
        }
    });
    if OWNER.with(Cell::get) {
        publish(mode);
    }
    // One read of the one-line cache for both of the wallpaper's inputs.
    let (sample, backdrop) = match super::wallpaper::read() {
        Some((sample, backdrop)) => (sample, Some(backdrop)),
        None => (None, None),
    };
    let tint = tint(look.tint, sample.as_ref(), look.tint_colour);
    SHOWN.with(|s| s.set(mode));
    TINT.with(|t| t.set(tint));
    BACKDROP.with(|b| b.set(backdrop));
    STARTED.with(|s| s.set(true));
    build(&look, mode, tint, backdrop)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Choosing Auto shows the sun's mode at once; the sun moving while Auto
    /// is already set waits until nobody is looking.
    #[test]
    fn auto_applies_at_once_when_chosen_and_waits_when_the_sun_moves() {
        STARTED.with(|s| s.set(true));
        SHOWN.with(|s| s.set(Mode::Dark));
        PENDING.with(|p| *p.borrow_mut() = None);
        // Chosen just now: the sun says light, light it is.
        assert_eq!(auto_mode(Mode::Light, true), Mode::Light);
        // Already on Auto, the sun turns: the dark on screen stays for now.
        SHOWN.with(|s| s.set(Mode::Dark));
        assert_eq!(auto_mode(Mode::Light, false), Mode::Dark);
        assert!(PENDING.with(|p| p.borrow().is_some()));
        // Choosing Auto again clears the wait and applies.
        assert_eq!(auto_mode(Mode::Light, true), Mode::Light);
        assert!(PENDING.with(|p| p.borrow().is_none()));
    }
}
