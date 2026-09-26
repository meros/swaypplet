//! The stylesheet, as GTK gets it.
//!
//! One provider, one document, in this order: the design tokens
//! (`crate::tokens`, generated from the theme inputs; docs/design-system.md),
//! then the rules, one file per surface in `data/css/`, joined in the order
//! of [`RULES`]. The order is the cascade: the files are contiguous pieces of
//! what was one stylesheet.
//!
//! [`load_css`] runs in all eight processes that draw something. Only the
//! long-lived ones ([`watch`]) follow the inputs after startup: the Look
//! settings, the sun, and the wallpaper's hue (`wallpaper`), which the panel
//! samples and every process reads.
//!
//! The theme is the runtime around the pure generator in `crate::tokens`:
//!
//! | file | holds |
//! |---|---|
//! | `mod.rs` | the stylesheet, [`reload`], [`observe`], [`watch`] |
//! | `fade.rs` | the colours fading from one set of inputs to the next |
//! | `inputs.rs` | [`inputs`] and [`shown`]: the one `Inputs` builder |
//! | `sun.rs` | the sun's elevation, for `auto` (§2.1) |
//! | `wallpaper.rs` | the wallpaper's hue, for the tint (§2.2) |
//! | `locked.rs` | logind's LockedHint, to time a sun switch |
//! | `paint.rs` | [`Paint`]: the token colours for Cairo |
//! | `sway.rs` | sway's window borders from the tokens |
//!
//! It sits below `crate::settings::glass`: the material is sent through the
//! callback [`watch`] is given, not by a call up.

use std::cell::RefCell;

use gdk4::Display;
use gtk4::CssProvider;

thread_local! {
    /// The provider this process installed, so an input change can reparse
    /// into it instead of stacking a second one on the display.
    static PROVIDER: RefCell<Option<CssProvider>> = const { RefCell::new(None) };
    /// The inputs last parsed, so the watch can tell a real change from a
    /// tick where nothing moved.
    static LOADED: RefCell<Option<crate::tokens::Inputs>> = const { RefCell::new(None) };
    /// Called after every reload that changed the stylesheet.
    static WATCHERS: RefCell<Vec<Box<dyn Fn()>>> = const { RefCell::new(Vec::new()) };
}

/// The rules, one file per surface, in cascade order.
pub const RULES: &[(&str, &str)] = &[
    (
        "components/surface.css",
        include_str!("../../data/css/components/surface.css"),
    ),
    (
        "components/text.css",
        include_str!("../../data/css/components/text.css"),
    ),
    (
        "components/layout.css",
        include_str!("../../data/css/components/layout.css"),
    ),
    (
        "components/card.css",
        include_str!("../../data/css/components/card.css"),
    ),
    (
        "components/button.css",
        include_str!("../../data/css/components/button.css"),
    ),
    (
        "components/chip.css",
        include_str!("../../data/css/components/chip.css"),
    ),
    (
        "components/row.css",
        include_str!("../../data/css/components/row.css"),
    ),
    (
        "components/expander.css",
        include_str!("../../data/css/components/expander.css"),
    ),
    (
        "components/tile.css",
        include_str!("../../data/css/components/tile.css"),
    ),
    (
        "components/slider.css",
        include_str!("../../data/css/components/slider.css"),
    ),
    (
        "components/field.css",
        include_str!("../../data/css/components/field.css"),
    ),
    (
        "components/menu.css",
        include_str!("../../data/css/components/menu.css"),
    ),
    (
        "components/avatar.css",
        include_str!("../../data/css/components/avatar.css"),
    ),
    (
        "components/progress.css",
        include_str!("../../data/css/components/progress.css"),
    ),
    (
        "components/bar.css",
        include_str!("../../data/css/components/bar.css"),
    ),
    (
        "components/popover.css",
        include_str!("../../data/css/components/popover.css"),
    ),
    (
        "components/media.css",
        include_str!("../../data/css/components/media.css"),
    ),
    (
        "components/face.css",
        include_str!("../../data/css/components/face.css"),
    ),
    (
        "components/motion.css",
        include_str!("../../data/css/components/motion.css"),
    ),
    (
        "components/gtk.css",
        include_str!("../../data/css/components/gtk.css"),
    ),
    ("02-bar.css", include_str!("../../data/css/02-bar.css")),
    ("03-panel.css", include_str!("../../data/css/03-panel.css")),
    (
        "04-network.css",
        include_str!("../../data/css/04-network.css"),
    ),
    ("05-osd.css", include_str!("../../data/css/05-osd.css")),
    (
        "06-notifications.css",
        include_str!("../../data/css/06-notifications.css"),
    ),
    (
        "07-launcher.css",
        include_str!("../../data/css/07-launcher.css"),
    ),
    ("08-auth.css", include_str!("../../data/css/08-auth.css")),
    ("09-helm.css", include_str!("../../data/css/09-helm.css")),
    ("10-lock.css", include_str!("../../data/css/10-lock.css")),
    (
        "11-keybinds.css",
        include_str!("../../data/css/11-keybinds.css"),
    ),
    (
        "12-screenshot.css",
        include_str!("../../data/css/12-screenshot.css"),
    ),
    (
        "14-elevation-face.css",
        include_str!("../../data/css/14-elevation-face.css"),
    ),
    (
        "15-settings.css",
        include_str!("../../data/css/15-settings.css"),
    ),
    ("16-jump.css", include_str!("../../data/css/16-jump.css")),
];

/// Every rule file joined, as GTK parses them.
pub fn joined_rules() -> String {
    RULES
        .iter()
        .map(|(_, css)| *css)
        .collect::<Vec<_>>()
        .join("\n")
}

/// The rules. Dev override: SWAYPPLET_CSS=<dir> loads the same files from
/// that directory at runtime instead of the baked-in copies, so the render
/// harness can iterate on them without recompiling. Production runs leave
/// it unset.
fn rules() -> String {
    match std::env::var_os("SWAYPPLET_CSS") {
        Some(dir) => {
            let dir = std::path::PathBuf::from(dir);
            RULES
                .iter()
                .map(|(name, baked)| {
                    std::fs::read_to_string(dir.join(name)).unwrap_or_else(|e| {
                        log::warn!("theme: cannot read {name} in SWAYPPLET_CSS: {e}");
                        (*baked).to_string()
                    })
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
        None => joined_rules(),
    }
}

mod fade;
mod inputs;
mod locked;
mod paint;
mod sun;
mod sway;
pub mod wallpaper;

pub use inputs::inputs;
pub use paint::{Paint, paint};

/// The inputs the stylesheet on screen was generated from: the Look
/// settings with the mode and the tint as they were last resolved rather
/// than resolved again. For Cairo drawing, which has to match the CSS beside
/// it, and which must neither move the sun's pending switch nor read the
/// wallpaper cache on every draw.
pub fn shown() -> crate::tokens::Inputs {
    inputs::shown()
}

/// The whole document: tokens, then rules.
fn document(inputs: crate::tokens::Inputs) -> String {
    let tokens = crate::tokens::css(inputs);
    format!("{tokens}\n{}", rules())
}

pub fn load_css() {
    let provider = CssProvider::new();
    let inputs = inputs();
    provider.load_from_string(&document(inputs));
    LOADED.with(|l| *l.borrow_mut() = Some(inputs));

    gtk4::style_context_add_provider_for_display(
        &Display::default().expect("Could not get default display"),
        &provider,
        gtk4::STYLE_PROVIDER_PRIORITY_USER,
    );
    PROVIDER.with(|p| *p.borrow_mut() = Some(provider));
}

/// Reparse the stylesheet with the inputs as they are now. Every widget
/// already on screen restyles itself; nothing is rebuilt. The colours that
/// changed fade to their new values over `page` (`fade`), when a window is
/// on screen to fade them on and motion is on.
///
/// Returns whether the inputs had in fact moved.
pub fn reload() -> bool {
    let inputs = inputs();
    let before = LOADED.with(|l| *l.borrow());
    if before == Some(inputs) {
        return false;
    }
    PROVIDER.with(|p| {
        if let Some(provider) = p.borrow().as_ref() {
            let clock = std::time::Instant::now();
            let tokens = crate::tokens::css(inputs);
            let text = format!("{tokens}\n{}", rules());
            let built = clock.elapsed();
            if let Some(before) = before {
                fade::start(&crate::tokens::css(before), &tokens);
            }
            let clock = std::time::Instant::now();
            provider.load_from_string(&text);
            log::debug!(
                "theme: reload: document {:.2} ms, parse {:.2} ms",
                built.as_secs_f64() * 1000.0,
                clock.elapsed().as_secs_f64() * 1000.0
            );
        }
    });
    LOADED.with(|l| *l.borrow_mut() = Some(inputs));
    WATCHERS.with(|w| {
        for cb in w.borrow().iter() {
            cb();
        }
    });
    true
}

/// Run `cb` after every reload that changed the stylesheet, for Cairo
/// drawing that shows the tokens (the Look tab's scales, its accent dots).
pub fn observe(cb: impl Fn() + 'static) {
    WATCHERS.with(|w| w.borrow_mut().push(Box::new(cb)));
}

/// Follow the inputs for as long as this process lives, and keep what the
/// compositor draws from them in step: sway's window borders here, and the
/// glass material through `on_material`, called with the inputs now on
/// screen whenever the material they make has changed (§4). The caller owns
/// what "send the material" means (`settings::glass`, which sits above the
/// theme), so the theme does not reach up into the settings to do it.
///
/// One second, the same tick and for the same reason as `settings::watch`:
/// the wallpaper's hue is a file one process writes and the others read,
/// and there is no bus between them. A wallpaper change is not a hot path.
pub fn watch(on_material: impl Fn(crate::tokens::Inputs) + 'static) {
    locked::follow();
    // The borders are the compositor's and outlive this process, so they are
    // put up at start as well as on every change: a fresh session has the
    // sway config's on them.
    sway::apply_borders(LOADED.with(|l| *l.borrow()).unwrap_or_else(inputs));
    glib::timeout_add_local(std::time::Duration::from_secs(1), move || {
        let before = LOADED.with(|l| *l.borrow());
        if reload() {
            let now = LOADED.with(|l| *l.borrow()).unwrap_or_default();
            // The glass follows the mode and, under a full tint, the
            // neutral (§4).
            if before.map(crate::tokens::material) != Some(crate::tokens::material(now)) {
                on_material(now);
            }
            sway::apply_borders(now);
        }
        glib::ControlFlow::Continue
    });
}

/// [`watch`] for the lock screen, from the moment it locks: the stylesheet
/// only. The panel sends the glass material and the borders; a second
/// sender would race it.
///
/// The lock process is started ahead of time and parks until the lock, so
/// its stylesheet is from whenever it started. A sun switch waits for the
/// lock (§2.1), which is exactly when the panel moves the glass to the new
/// mode; a lock screen that kept its old stylesheet then drew dark mode's
/// white text on light glass. So it reloads here, before the surfaces are
/// built, and follows every second while it is up. Each tick compares the
/// inputs and reparses only when they moved.
pub fn follow_while_locked() {
    locked::assume_locked();
    reload();
    glib::timeout_add_local(std::time::Duration::from_secs(1), || {
        reload();
        glib::ControlFlow::Continue
    });
}

#[cfg(test)]
mod tests {
    /// The stylesheet, structurally.
    ///
    /// GTK does not fail on a malformed one: it logs the parse error to
    /// stderr, drops what it could not read, and shows the shell with pieces
    /// of itself missing. An unclosed comment costs every rule after it,
    /// which on 2026-09-09 was the notification cards' radius, their rail and
    /// their accents at once — a whole surface silently unstyled, and nothing
    /// but a screenshot said so.
    ///
    /// Braces and comment markers, counted outside string literals, catch
    /// that class without a display to init GTK against. This is not a CSS
    /// parser and is not trying to be one; it is the check that the file is
    /// still made of the pieces it says it is.
    #[test]
    fn the_stylesheet_is_structurally_whole() {
        for (name, css) in super::RULES {
            structurally_whole(name, css);
        }
        // What GTK actually parses: all of them, joined. A file that is
        // whole on its own and breaks the next one at the seam is the
        // failure a split introduces.
        let tokens = crate::tokens::css(crate::tokens::Inputs::default());
        structurally_whole(
            "the joined stylesheet",
            &format!("{tokens}\n{}", super::joined_rules()),
        );

        // The categorical classes the notification card hands out by number
        // (`notifications::card::accent_for`, through `ui::rail` and
        // `ui::set_category`) have to exist, or a sender silently falls back to
        // plain ink and the hue channel is dead.
        let css = super::joined_rules();
        for n in 1..=crate::notifications::card::ACCENTS {
            for rule in [format!(".ui-rail.cat-{n}"), format!(".ui-cat-{n}")] {
                assert!(css.contains(&rule), "data/css/ has no `{rule}`");
            }
        }
    }

    fn structurally_whole(name: &str, css: &str) {
        let bytes = css.as_bytes();

        let (mut depth, mut min_depth, mut comments) = (0i32, 0i32, 0i32);
        let (mut in_comment, mut in_string, mut quote) = (false, false, b'"');
        let mut i = 0;
        while i < bytes.len() {
            let b = bytes[i];
            let next = bytes.get(i + 1).copied();
            if in_comment {
                if b == b'*' && next == Some(b'/') {
                    in_comment = false;
                    comments -= 1;
                    i += 2;
                    continue;
                }
            } else if in_string {
                if b == b'\\' {
                    i += 2;
                    continue;
                }
                if b == quote {
                    in_string = false;
                }
            } else {
                match b {
                    b'/' if next == Some(b'*') => {
                        in_comment = true;
                        comments += 1;
                        i += 2;
                        continue;
                    }
                    b'*' if next == Some(b'/') => {
                        panic!("{name}: `*/` at byte {i} closes a comment nothing opened")
                    }
                    b'"' | b'\'' => {
                        in_string = true;
                        quote = b;
                    }
                    b'{' => depth += 1,
                    b'}' => {
                        depth -= 1;
                        min_depth = min_depth.min(depth);
                    }
                    _ => {}
                }
            }
            i += 1;
        }

        assert!(!in_comment, "{name}: a /* comment is never closed");
        assert!(!in_string, "{name}: a string literal is never closed");
        assert_eq!(comments, 0, "{name}: unbalanced comment markers");
        assert_eq!(min_depth, 0, "{name}: a }} closes a block nothing opened");
        assert_eq!(depth, 0, "{name}: {depth} block(s) left open");
    }

    /// A hex value inside a rule is a colour the inputs cannot reach: the
    /// tokens move with the mode and the tint, so a literal in a rule stays
    /// gruvbox while everything around it moves. One went in as
    /// `alpha(#32302f, 0.72)` on the notification card and took the card's
    /// ground out of the theme; this is why that class of edit now fails.
    ///
    /// Comments are exempt. They cite the values a colour was derived from,
    /// and that is the documentation the derivation depends on.
    #[test]
    fn the_rules_carry_no_colour_literals() {
        for (file, css) in super::RULES {
            let mut in_comment = false;
            for (n, line) in css.lines().enumerate() {
                let mut rest = line;
                let mut code = String::new();
                loop {
                    if in_comment {
                        match rest.find("*/") {
                            Some(i) => {
                                in_comment = false;
                                rest = &rest[i + 2..];
                            }
                            None => break,
                        }
                    } else {
                        match rest.find("/*") {
                            Some(i) => {
                                code.push_str(&rest[..i]);
                                in_comment = true;
                                rest = &rest[i + 2..];
                            }
                            None => {
                                code.push_str(rest);
                                break;
                            }
                        }
                    }
                }
                let bytes = code.as_bytes();
                for (i, b) in bytes.iter().enumerate() {
                    if *b != b'#' {
                        continue;
                    }
                    let hex = &bytes[i + 1..bytes.len().min(i + 7)];
                    assert!(
                        hex.len() < 6 || !hex.iter().all(u8::is_ascii_hexdigit),
                        "data/css/{file}:{} has the colour literal {} in a rule. \n\
                     Use a token (docs/design-system.md §3), or the theme cannot reach it.\n  {line}",
                        n + 1,
                        &code[i..i + 7]
                    );
                }
            }
        }
    }
}
