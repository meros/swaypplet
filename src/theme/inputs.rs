//! The theme inputs right now (docs/design-system.md §2): the Look
//! settings, with `auto` resolved by the sun and held back until a switch
//! cannot happen in front of someone.

use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

use crate::settings::schema::{Look, ThemeMode, Tint as Reach};
use crate::tokens::{Backdrop, Inputs, Mode, Palette, Tint};

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
    }
}

/// The Look setting's reach with the wallpaper's hues, as the panel last
/// sampled it (`super::wallpaper`). Off until a sample exists: a wallpaper
/// with no usable colour, or one not sampled yet, leaves the tokens shipped.
fn tint(reach: Reach, palette: Option<Palette>) -> Tint {
    if reach == Reach::Off {
        return Tint::Off;
    }
    match (reach, palette) {
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

/// The theme inputs right now.
pub fn inputs() -> Inputs {
    let look = crate::settings::store::with(|s| s.look());
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
            ThemeMode::Auto => when_unseen(sun_mode()),
        }
    });
    // One read of the one-line cache for both of the wallpaper's inputs.
    let (palette, backdrop) = match super::wallpaper::read() {
        Some((palette, backdrop)) => (palette, Some(backdrop)),
        None => (None, None),
    };
    let tint = tint(look.tint, palette);
    SHOWN.with(|s| s.set(mode));
    TINT.with(|t| t.set(tint));
    BACKDROP.with(|b| b.set(backdrop));
    STARTED.with(|s| s.set(true));
    build(&look, mode, tint, backdrop)
}
