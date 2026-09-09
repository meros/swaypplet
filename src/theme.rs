use gdk4::Display;
use gtk4::CssProvider;

pub fn load_css() {
    let provider = CssProvider::new();
    // Dev override: SWAYPPLET_CSS=<path> loads the stylesheet from disk at
    // runtime instead of the baked-in copy, so the render harness can iterate
    // on style.css without recompiling. Production runs leave it unset.
    match std::env::var_os("SWAYPPLET_CSS") {
        Some(path) => provider.load_from_path(path),
        None => provider.load_from_string(include_str!("../data/style.css")),
    }

    gtk4::style_context_add_provider_for_display(
        &Display::default().expect("Could not get default display"),
        &provider,
        gtk4::STYLE_PROVIDER_PRIORITY_USER,
    );
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
        let css = include_str!("../data/style.css");
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
                        panic!("data/style.css: `*/` at byte {i} closes a comment nothing opened")
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

        assert!(!in_comment, "data/style.css: a /* comment is never closed");
        assert!(
            !in_string,
            "data/style.css: a string literal is never closed"
        );
        assert_eq!(comments, 0, "data/style.css: unbalanced comment markers");
        assert_eq!(
            min_depth, 0,
            "data/style.css: a }} closes a block nothing opened"
        );
        assert_eq!(depth, 0, "data/style.css: {depth} block(s) left open");

        // The accent classes the notification card hands out by number
        // (`notifications::popup::accent_for`) have to exist, or a sender
        // silently falls back to @fg_dim and the hue channel is dead.
        for n in 1..=crate::notifications::popup::ACCENTS {
            for rule in [
                format!(".notification-rail.a{n}"),
                format!(".notification-app-name.a{n}"),
            ] {
                assert!(css.contains(&rule), "data/style.css has no `{rule}`");
            }
        }
    }
}
