//! Where the omnibox sends what was typed into it.
//!
//! One reading for every caller: the panel switches its deck page as a
//! prefix is typed and opens settings on Enter, and a launcher result that
//! names a prefix is opened the same way. The tables come from their
//! owners, the panel's deck pages and settings' panes, so this module knows
//! the shape of a prefix and nothing about any page.

use crate::sources::{self, Query};

/// The prefixes that open settings as it was, on whatever pane it was on.
pub const SETTINGS: &[&str] = &[":set", ":pref"];

/// A table of prefixes and the page each row opens.
pub type Table<'t> = &'t [(&'static [&'static str], &'static str)];

/// What typed text asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route<'a> {
    /// A bare `:`: the list of every prefix.
    Prefixes,
    /// A deck page of the panel, by its stack name; shown as it is typed.
    Deck(&'static str),
    /// A settings pane, by its stack name; opened on Enter, since that
    /// takes the panel away.
    Pane(&'static str),
    /// [`SETTINGS`]: settings as it was, on Enter.
    Settings,
    /// A `:` word that names no page; searched like any other text.
    Unknown,
    /// Not a page at all: a search, read by its own prefix (`=`, `>`).
    Query(Query<'a>),
}

/// Whether `text` starts with the ASCII `prefix`, in any case.
fn starts_with(text: &str, prefix: &str) -> bool {
    text.get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

fn find(text: &str, table: Table<'_>) -> Option<&'static str> {
    table
        .iter()
        .find(|(prefixes, _)| prefixes.iter().any(|p| starts_with(text, p)))
        .map(|(_, page)| *page)
}

/// Read `text` against the panel's deck pages and settings' panes. A
/// prefix matches the start of a word, so `:wifi` and `:wifi home` both
/// open the Wi-Fi page, and `:settings` is `:set`.
pub fn resolve<'a>(text: &'a str, decks: Table<'_>, panes: Table<'_>) -> Route<'a> {
    let t = text.trim();
    if !t.starts_with(':') {
        return Route::Query(sources::parse(text));
    }
    if t == ":" {
        return Route::Prefixes;
    }
    if let Some(page) = find(t, decks) {
        return Route::Deck(page);
    }
    if let Some(pane) = find(t, panes) {
        return Route::Pane(pane);
    }
    if SETTINGS.iter().any(|p| starts_with(t, p)) {
        return Route::Settings;
    }
    Route::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    const DECKS: Table<'static> = &[(&[":wifi", ":net"], "wifi"), (&[":bt"], "bluetooth")];
    const PANES: Table<'static> = &[(&[":glass"], "glass"), (&[":idle", ":sleep"], "idle")];

    #[test]
    fn typed_text_goes_where_its_prefix_says() {
        for (text, route) in [
            (":", Route::Prefixes),
            (" : ", Route::Prefixes),
            (":wifi", Route::Deck("wifi")),
            (":network", Route::Deck("wifi")),
            (":WiFi home", Route::Deck("wifi")),
            ("  :bt", Route::Deck("bluetooth")),
            (":glass", Route::Pane("glass")),
            (":Sleep", Route::Pane("idle")),
            (":set", Route::Settings),
            (":settings", Route::Settings),
            (":pref", Route::Settings),
            (":nothing", Route::Unknown),
            ("", Route::Query(Query::Empty)),
            ("   ", Route::Query(Query::Empty)),
            ("firefox", Route::Query(Query::Plain("firefox"))),
            ("=2+2", Route::Query(Query::Calc("2+2"))),
            (">ls -l", Route::Query(Query::Command("ls -l"))),
            // A colon later in the text is not a prefix.
            ("a:wifi", Route::Query(Query::Plain("a:wifi"))),
        ] {
            assert_eq!(resolve(text, DECKS, PANES), route, "{text:?}");
        }
    }

    #[test]
    fn a_deck_page_is_read_before_a_pane() {
        let panes: Table<'static> = &[(&[":wi"], "look")];
        assert_eq!(resolve(":wifi", DECKS, panes), Route::Deck("wifi"));
        assert_eq!(resolve(":wire", DECKS, panes), Route::Pane("look"));
    }

    #[test]
    fn text_shorter_than_a_prefix_or_split_inside_a_character_matches_nothing() {
        assert_eq!(resolve(":w", DECKS, PANES), Route::Unknown);
        assert_eq!(resolve(":å", DECKS, PANES), Route::Unknown);
    }
}
