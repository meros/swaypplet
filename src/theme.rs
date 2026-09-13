//! The stylesheet, as GTK gets it.
//!
//! Two files in one provider: `data/palette.css` names every colour and
//! `data/style.css` is every rule, concatenated in that order and parsed as
//! one document. They cannot be two providers. GTK4 resolves `@define-color`
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

/// The rules. Dev override: SWAYPPLET_CSS=<path> loads them from disk at
/// runtime instead of the baked-in copy, so the render harness can iterate
/// on style.css without recompiling. Production runs leave it unset.
fn rules() -> String {
    match std::env::var_os("SWAYPPLET_CSS") {
        Some(path) => std::fs::read_to_string(&path).unwrap_or_else(|e| {
            log::warn!("theme: cannot read SWAYPPLET_CSS={path:?}: {e}");
            include_str!("../data/style.css").to_string()
        }),
        None => include_str!("../data/style.css").to_string(),
    }
}

pub fn load_css() {
    let provider = CssProvider::new();
    let palette = crate::palette::current();
    provider.load_from_string(&format!("{palette}\n{}", rules()));
    LOADED.with(|l| *l.borrow_mut() = palette);

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
    if LOADED.with(|l| *l.borrow() == palette) {
        return false;
    }
    PROVIDER.with(|p| {
        if let Some(provider) = p.borrow().as_ref() {
            provider.load_from_string(&format!("{palette}\n{}", rules()));
        }
    });
    LOADED.with(|l| *l.borrow_mut() = palette);
    true
}

/// Follow the palette for as long as this process lives.
///
/// One second, the same tick and for the same reason as `settings::watch`:
/// the palette is a file one process writes and the others read, and there
/// is no bus between them. A wallpaper change is not a hot path.
pub fn watch() {
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
        for (name, css) in [
            ("data/style.css", include_str!("../data/style.css")),
            ("data/palette.css", include_str!("../data/palette.css")),
            // What GTK actually parses: the two of them, joined. A file that
            // is whole on its own and breaks the other one at the seam is
            // the failure the split introduced.
            (
                "the joined stylesheet",
                &*format!(
                    "{}\n{}",
                    include_str!("../data/palette.css"),
                    include_str!("../data/style.css")
                ),
            ),
        ] {
            structurally_whole(name, css);
        }

        // The accent classes the notification card hands out by number
        // (`notifications::popup::accent_for`) have to exist, or a sender
        // silently falls back to @fg_dim and the hue channel is dead.
        let css = include_str!("../data/style.css");
        for n in 1..=crate::notifications::popup::ACCENTS {
            for rule in [
                format!(".notification-rail.a{n}"),
                format!(".notification-app-name.a{n}"),
            ] {
                assert!(css.contains(&rule), "data/style.css has no `{rule}`");
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
        let css = include_str!("../data/style.css");
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
                    "data/style.css:{} has the colour literal {} in a rule. \n\
                     Name it in data/palette.css and reference it with @name, or the \n\
                     wallpaper-derived palette cannot reach it.\n  {line}",
                    n + 1,
                    &code[i..i + 7]
                );
            }
        }
    }
}
