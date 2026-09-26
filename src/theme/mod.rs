//! The stylesheet, as GTK gets it.
//!
//! One provider, one document, in this order: the design tokens
//! (`crate::tokens`, generated from the theme inputs; docs/design-system.md),
//! `data/palette.css` (the `@define-color` names the rules still use while
//! they move onto the tokens), then the rules, one file per surface in
//! `data/css/`, joined in the order of [`RULES`]. The order is the cascade:
//! the files are contiguous pieces of what was one stylesheet.
//!
//! The palette and the rules are one provider on purpose. They cannot be two providers. GTK4 resolves `@define-color`
//! per provider at parse time, so a second provider that redefines @accent
//! recolours nothing the first one already parsed — which is why the
//! wallpaper-derived palette (`crate::palette`) is swapped in *here*, in
//! front of the rules, rather than layered over them.
//!
//! [`load_css`] runs in all eight processes that draw something. Only the
//! long-lived ones ([`watch`]) follow the palette after startup.

use std::cell::RefCell;

use gdk4::Display;
use gtk4::CssProvider;

thread_local! {
    /// The provider this process installed, so a palette change can reparse
    /// into it instead of stacking a second one on the display.
    static PROVIDER: RefCell<Option<CssProvider>> = const { RefCell::new(None) };
    /// The palette last parsed, so the watch can tell a real change from a
    /// cache file that was rewritten with the same contents.
    static LOADED: RefCell<String> = const { RefCell::new(String::new()) };
}

/// The rules, one file per surface, in cascade order.
pub const RULES: &[(&str, &str)] = &[
    (
        "00-components.css",
        include_str!("../../data/css/00-components.css"),
    ),
    ("01-base.css", include_str!("../../data/css/01-base.css")),
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
        "13-switcher.css",
        include_str!("../../data/css/13-switcher.css"),
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

mod inputs;
mod locked;

pub use inputs::inputs;

/// The whole document: tokens, palette, rules.
fn document(inputs: crate::tokens::Inputs, palette: &str) -> String {
    let tokens = crate::tokens::css(inputs);
    format!("{tokens}\n{palette}\n{}", rules())
}

/// What was last parsed: the palette and the theme inputs.
fn key(inputs: crate::tokens::Inputs, palette: &str) -> String {
    format!("{inputs:?}\n{palette}")
}

pub fn load_css() {
    let provider = CssProvider::new();
    let palette = crate::palette::current();
    let inputs = inputs();
    provider.load_from_string(&document(inputs, &palette));
    LOADED.with(|l| *l.borrow_mut() = key(inputs, &palette));

    gtk4::style_context_add_provider_for_display(
        &Display::default().expect("Could not get default display"),
        &provider,
        gtk4::STYLE_PROVIDER_PRIORITY_USER,
    );
    PROVIDER.with(|p| *p.borrow_mut() = Some(provider));
}

/// Reparse the stylesheet with whatever palette is current. Every widget
/// already on screen restyles itself; nothing is rebuilt.
///
/// Returns whether the palette had in fact moved.
pub fn reload() -> bool {
    let palette = crate::palette::current();
    let inputs = inputs();
    let k = key(inputs, &palette);
    if LOADED.with(|l| *l.borrow() == k) {
        return false;
    }
    PROVIDER.with(|p| {
        if let Some(provider) = p.borrow().as_ref() {
            provider.load_from_string(&document(inputs, &palette));
        }
    });
    LOADED.with(|l| *l.borrow_mut() = k);
    true
}

/// Follow the palette for as long as this process lives.
///
/// One second, the same tick and for the same reason as `settings::watch`:
/// the palette is a file one process writes and the others read, and there
/// is no bus between them. A wallpaper change is not a hot path.
pub fn watch() {
    locked::follow();
    glib::timeout_add_local(std::time::Duration::from_secs(1), || {
        let mode = inputs::shown();
        if reload() && inputs::shown() != mode {
            // The glass follows the mode (§4). Only this long-lived process
            // watches, so the material is sent once, not once per process.
            crate::settings::glass::apply_saved();
        }
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
        structurally_whole("data/palette.css", include_str!("../../data/palette.css"));
        // What GTK actually parses: all of them, joined. A file that is
        // whole on its own and breaks the next one at the seam is the
        // failure a split introduces.
        let tokens = crate::tokens::css(crate::tokens::Inputs::default());
        structurally_whole(
            "the joined stylesheet",
            &format!(
                "{tokens}\n{}\n{}",
                include_str!("../../data/palette.css"),
                super::joined_rules()
            ),
        );

        // The categorical classes the notification card hands out by number
        // (`notifications::popup::accent_for`, through `ui::rail` and
        // `ui::set_category`) have to exist, or a sender silently falls back to
        // plain ink and the hue channel is dead.
        let css = super::joined_rules();
        for n in 1..=crate::notifications::popup::ACCENTS {
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

    /// A hex value inside a rule is a colour the wallpaper cannot reach: the
    /// derived palette rewrites definitions, so a literal in a rule stays
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
