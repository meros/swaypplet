//! The settings as the launcher finds them: every row of every tab, by its
//! title, a one-line subtitle and the words people type for it ("dark mode"
//! for Appearance › Mode, "blur" for Glass › Material › Frost).
//!
//! The index is static. Each pane keeps its own table, `SEARCH`, beside the
//! code that builds its rows, and [`TABLES`] lists the tables by tab. Nothing
//! is built or read until a query asks, and then only this table is walked:
//! no widget, no file, no process.
//!
//! An entry is anchored by what the pane already shows: the group's overline
//! and the row's gutter label. `SettingsSection::reveal` finds the row by
//! those two strings, so a pane needs no ids of its own. What keeps the two
//! from drifting apart is the test below: every entry's group and title must
//! be a string literal in its pane's source, so renaming a row without its
//! entry fails `cargo test`.
//!
//! The panel's quick sections that are the natural place for a setting
//! (Wi-Fi, Bluetooth, the audio output) are in [`QUICK`]; they open by their
//! omnibox prefix.

/// One searchable row of a settings tab.
#[derive(Debug, Clone, Copy)]
pub struct Entry {
    /// The group's overline, as `form::section_box` was given it.
    pub group: &'static str,
    /// The row's gutter label; empty for the group as a whole (a grid, a
    /// canvas: something with no one row).
    pub title: &'static str,
    /// What the row does, in a line. Shown under the result and searched,
    /// at a lower weight than the title.
    pub subtitle: &'static str,
    /// Other words for it. A phrase counts as its words, and as a phrase
    /// when the whole query is that phrase.
    pub keywords: &'static [&'static str],
    /// The settings the row edits, by dotted key (`schema::Settings::get`),
    /// so settings can mark it when the file changes it from the system's
    /// and put the system's back. Empty for a row that edits nothing in the
    /// settings file: glass (its own file), the displays (sway's state), a
    /// grid or a list.
    pub keys: &'static [&'static str],
}

/// An entry, in the tables' own shorthand.
pub const fn row(
    group: &'static str,
    title: &'static str,
    subtitle: &'static str,
    keywords: &'static [&'static str],
) -> Entry {
    Entry {
        group,
        title,
        subtitle,
        keywords,
        keys: &[],
    }
}

impl Entry {
    /// The row edits `keys` (see [`Entry::keys`]).
    pub const fn keys(self, keys: &'static [&'static str]) -> Entry {
        Entry { keys, ..self }
    }
}

/// Every pane's table, by the pane's stack name (`settings::PANES`). A new pane
/// adds its pane's `SEARCH` here.
pub const TABLES: &[(&str, &[Entry])] = &[
    ("look", super::look_pane::SEARCH),
    ("idle", super::idle_pane::SEARCH),
    ("bar", super::bar_pane::SEARCH),
    ("input", super::input_pane::SEARCH),
    ("alerts", super::alerts_pane::SEARCH),
    ("launcher", super::launcher_pane::SEARCH),
    ("displays", super::displays_pane::SEARCH),
    ("glass", super::glass_pane::SEARCH),
    ("system", super::system_pane::SEARCH),
    ("quality", super::quality_pane::SEARCH),
];

/// A section of the panel a query can land on, by its omnibox prefix
/// (`panel::ROUTES`).
#[derive(Debug, Clone, Copy)]
pub struct Quick {
    pub prefix: &'static str,
    pub title: &'static str,
    pub subtitle: &'static str,
    pub keywords: &'static [&'static str],
}

pub const QUICK: &[Quick] = &[
    Quick {
        prefix: ":wifi",
        title: "Wi-Fi",
        subtitle: "Networks, VPN and the connection in use",
        keywords: &[
            "wifi",
            "wireless",
            "wlan",
            "network",
            "internet",
            "vpn",
            "wireguard",
            "hotspot",
            "ssid",
        ],
    },
    Quick {
        prefix: ":audio",
        title: "Audio output",
        subtitle: "Speakers, headphones and the microphone",
        keywords: &[
            "audio",
            "sound",
            "speaker",
            "speakers",
            "headphones",
            "output device",
            "sink",
            "microphone",
            "mic",
            "input",
            "volume",
        ],
    },
    Quick {
        prefix: ":bt",
        title: "Bluetooth",
        subtitle: "Pair and connect devices",
        keywords: &[
            "bluetooth",
            "pair",
            "pairing",
            "headphones",
            "earbuds",
            "headset",
            "mouse",
            "keyboard",
            "devices",
        ],
    },
];

/// The pane's title, for the breadcrumb.
fn tab_title(tab: &str) -> &'static str {
    super::pane_title(tab).unwrap_or("Settings")
}

/// Where a hit goes when it is activated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A row (or a group) of a settings tab: `(tab, group, title)`.
    Row(&'static str, &'static str, &'static str),
    /// A panel section, by its omnibox prefix.
    Page(&'static str),
}

impl Target {
    /// The launcher row's identifier: a prefix for a page, and the three
    /// strings of a row joined by tabs (none of them holds one).
    pub fn id(&self) -> String {
        match self {
            Target::Page(prefix) => (*prefix).to_string(),
            Target::Row(tab, group, title) => format!("{tab}\t{group}\t{title}"),
        }
    }

    /// The target an identifier names, if it is one of the index's.
    pub fn from_id(id: &str) -> Option<Target> {
        if id.starts_with(':') {
            return QUICK
                .iter()
                .find(|q| q.prefix == id)
                .map(|q| Target::Page(q.prefix));
        }
        let mut parts = id.split('\t');
        let (tab, group, title) = (parts.next()?, parts.next()?, parts.next()?);
        TABLES
            .iter()
            .filter(|(t, _)| *t == tab)
            .flat_map(|(t, entries)| entries.iter().map(move |e| (*t, e)))
            .find(|(_, e)| e.group == group && e.title == title)
            .map(|(t, e)| Target::Row(t, e.group, e.title))
    }
}

/// A query's match: where it goes, what the row says, and how well it
/// matched.
#[derive(Debug, Clone)]
pub struct Hit {
    pub target: Target,
    /// "Appearance › Mode".
    pub path: String,
    pub subtitle: &'static str,
    pub score: u32,
    /// Every query word is a whole word of the title or the keywords: the
    /// query names this setting, and the launcher lists it above the apps.
    pub clear: bool,
}

/// Lowercased words of `s`: split on anything not a letter or a digit, so
/// "Wi-Fi" is "wi" and "fi" and "24-hour" is "24" and "hour".
fn words(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// What one entry is searched by, lowercased once.
struct Prepared {
    target: Target,
    path: String,
    subtitle: &'static str,
    title: Vec<String>,
    keywords: Vec<String>,
    /// Whole phrases: the title and each keyword, words joined by a space.
    phrases: Vec<String>,
    /// The tab's and the group's words.
    place: Vec<String>,
    /// The subtitle's words of three letters or more.
    subtitle_words: Vec<String>,
}

fn prepare() -> Vec<Prepared> {
    let mut out = Vec::new();
    for (tab, entries) in TABLES {
        let tab_title = tab_title(tab);
        for e in *entries {
            let title = if e.title.is_empty() { e.group } else { e.title };
            // A group named after its pane (Appearance › Appearance) is
            // said once: the path is where the row is, not a repetition.
            let mut path = tab_title.to_string();
            if !e.group.eq_ignore_ascii_case(tab_title) {
                path.push_str(" › ");
                path.push_str(e.group);
            }
            if !e.title.is_empty() {
                path.push_str(" › ");
                path.push_str(e.title);
            }
            let mut place = words(tab_title);
            place.extend(words(e.group));
            out.push(Prepared {
                target: Target::Row(tab, e.group, e.title),
                path,
                subtitle: e.subtitle,
                title: words(title),
                keywords: e.keywords.iter().flat_map(|k| words(k)).collect(),
                phrases: std::iter::once(title)
                    .chain(e.keywords.iter().copied())
                    .map(|p| words(p).join(" "))
                    .collect(),
                place,
                subtitle_words: words(e.subtitle)
                    .into_iter()
                    .filter(|w| w.chars().count() >= 3)
                    .collect(),
            });
        }
    }
    for q in QUICK {
        out.push(Prepared {
            target: Target::Page(q.prefix),
            path: format!("Panel › {}", q.title),
            subtitle: q.subtitle,
            title: words(q.title),
            keywords: q.keywords.iter().flat_map(|k| words(k)).collect(),
            phrases: std::iter::once(q.title)
                .chain(q.keywords.iter().copied())
                .map(|p| words(p).join(" "))
                .collect(),
            place: vec!["panel".to_string()],
            subtitle_words: words(q.subtitle)
                .into_iter()
                .filter(|w| w.chars().count() >= 3)
                .collect(),
        });
    }
    out
}

thread_local! {
    /// Built on the first query and kept: a few hundred short strings.
    static PREPARED: Vec<Prepared> = prepare();
}

/// How one query word meets a list of words: the best of an exact word
/// (`exact`) and the start of one (`prefix`), or nothing.
fn meet(q: &str, ws: &[String], exact: u32, prefix: u32) -> u32 {
    ws.iter()
        .map(|w| {
            if w == q {
                exact
            } else if w.starts_with(q) {
                prefix
            } else {
                0
            }
        })
        .max()
        .unwrap_or(0)
}

impl Prepared {
    /// The launcher's local rule, the one pages match by (`sources::Page`):
    /// every query word starts a word of the entry. Each word scores by
    /// where it landed, the title highest and the subtitle lowest.
    fn score(&self, qwords: &[String], phrase: &str) -> Option<(u32, bool)> {
        let mut total = 0;
        let mut clear = true;
        for q in qwords {
            let title = meet(q, &self.title, 8, 5);
            let keyword = meet(q, &self.keywords, 7, 4);
            let place = meet(q, &self.place, 3, 2);
            let subtitle = meet(q, &self.subtitle_words, 2, 1);
            let best = title.max(keyword).max(place).max(subtitle);
            if best == 0 {
                return None;
            }
            clear &= title == 8 || keyword == 7;
            total += best;
        }
        // The whole query is the title or one of the keyword phrases.
        if self.phrases.iter().any(|p| p == phrase) {
            total += 6;
        }
        Some((total, clear))
    }
}

/// The entries `query` finds, best first. Nothing under two characters, so a
/// single letter does not list half the settings.
pub fn find(query: &str) -> Vec<Hit> {
    let qwords = words(query);
    let phrase = qwords.join(" ");
    if phrase.chars().count() < 2 {
        return Vec::new();
    }
    let mut hits: Vec<Hit> = PREPARED.with(|all| {
        all.iter()
            .filter_map(|p| {
                let (score, clear) = p.score(&qwords, &phrase)?;
                Some(Hit {
                    target: p.target.clone(),
                    path: p.path.clone(),
                    subtitle: p.subtitle,
                    score,
                    clear: clear && phrase.chars().count() >= 3,
                })
            })
            .collect()
    });
    // Stable: equals keep the tables' order, which is the pane's.
    hits.sort_by_key(|h| std::cmp::Reverse(h.score));
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each tab's pane source, for the anchor check.
    const SOURCES: &[(&str, &str)] = &[
        ("look", include_str!("look_pane.rs")),
        // The Look tab's Day and night group lives in its own file.
        ("look", include_str!("daylight_group.rs")),
        ("idle", include_str!("idle_pane.rs")),
        ("bar", include_str!("bar_pane.rs")),
        ("input", include_str!("input_pane.rs")),
        ("alerts", include_str!("alerts_pane.rs")),
        ("launcher", include_str!("launcher_pane.rs")),
        ("displays", include_str!("displays_pane.rs")),
        ("glass", include_str!("glass_pane.rs")),
        ("system", include_str!("system_pane.rs")),
        ("quality", include_str!("quality_pane.rs")),
    ];

    /// The pane's source without its `SEARCH` table, so an entry cannot
    /// vouch for itself.
    fn code(src: &str) -> &str {
        match src.find("pub(super) const SEARCH") {
            Some(i) => &src[..i],
            None => src,
        }
    }

    #[test]
    fn every_tab_has_a_table_and_a_source() {
        for tab in &super::super::PANES {
            assert!(
                TABLES.iter().any(|(t, e)| *t == tab.name && !e.is_empty()),
                "the {} tab has no search table",
                tab.name
            );
            assert!(
                SOURCES.iter().any(|(t, _)| *t == tab.name),
                "the {} tab's source is not checked",
                tab.name
            );
        }
    }

    #[test]
    fn every_entry_names_a_group_and_a_row_its_pane_builds() {
        for (tab, entries) in TABLES {
            let src: String = SOURCES
                .iter()
                .filter(|(t, _)| t == tab)
                .map(|(_, s)| code(s))
                .collect();
            let src = src.as_str();
            for e in *entries {
                // The group: a section_box title (or, on Displays, the
                // selected output's group, whose overline is the output).
                if !(*tab == "displays" && e.group == "Display") {
                    assert!(
                        src.contains(&format!("\"{}\"", e.group)),
                        "{tab}: no group \"{}\" in the pane",
                        e.group
                    );
                }
                if !e.title.is_empty() {
                    assert!(
                        src.contains(&format!("\"{}\"", e.title)),
                        "{tab} › {}: no row \"{}\" in the pane",
                        e.group,
                        e.title
                    );
                }
                assert!(!e.subtitle.is_empty(), "{tab} › {}: no subtitle", e.title);
                assert!(
                    !e.group.contains('\t') && !e.title.contains('\t'),
                    "a tab would break the id"
                );
            }
        }
    }

    #[test]
    fn every_row_the_panes_build_is_in_the_index() {
        // The labelled rows: the first string argument of each row helper.
        // Rows built from a table (`timer.label`) are covered by the table's
        // `label: "…"` lines.
        for (tab, src) in SOURCES {
            let src = code(src);
            let (_, entries) = TABLES.iter().find(|(t, _)| t == tab).unwrap();
            let mut labels = Vec::new();
            for needle in [
                "switch_row(\n            \"",
                "switch_row(\"",
                "dropdown_row(\n            \"",
                "dropdown_row(\"",
                "scale_row(\n            \"",
                "kind_row(\"",
                "time_row(\n            \"",
                "label: \"",
            ] {
                let mut rest = src;
                while let Some(i) = rest.find(needle) {
                    rest = &rest[i + needle.len()..];
                    let end = rest.find('"').unwrap();
                    labels.push(&rest[..end]);
                }
            }
            // A page with no settings rows (System) indexes its groups only.
            let groups_only = entries.iter().all(|e| e.title.is_empty());
            assert!(!labels.is_empty() || groups_only, "{tab}: no rows found");
            for label in labels {
                assert!(
                    entries.iter().any(|e| e.title == label),
                    "{tab}: the row \"{label}\" is not in its SEARCH table"
                );
            }
        }
    }

    #[test]
    fn an_id_goes_back_to_its_target() {
        for hit in find("mode").into_iter().chain(find("wifi")) {
            assert_eq!(Target::from_id(&hit.target.id()), Some(hit.target));
        }
        assert_eq!(Target::from_id("look\tNope\tMode"), None);
        assert_eq!(Target::from_id(":nothing"), None);
    }

    fn top(query: &str) -> String {
        find(query)
            .first()
            .map(|h| h.path.clone())
            .unwrap_or_default()
    }

    #[test]
    fn synonyms_find_their_row() {
        for (query, path) in [
            ("dark mode", "Appearance › Mode"),
            ("dark", "Appearance › Mode"),
            ("light theme", "Appearance › Mode"),
            ("wallpaper", "Appearance › Wallpaper"),
            ("background image", "Appearance › Wallpaper"),
            ("night light", "Appearance › Night light › Night light"),
            ("blue light", "Appearance › Night light › Night light"),
            (
                "colour temperature",
                "Appearance › Night light › Night warmth",
            ),
            (
                "screen timeout",
                "Idle & Lock › Idle timers › Screen off after",
            ),
            ("auto lock", "Idle & Lock › Idle timers › Lock after"),
            ("sleep", "Idle & Lock › Idle timers › Suspend after"),
            ("blur", "Glass › Material › Frost"),
            ("transparency", "Glass › Material › Clarity"),
            ("resolution", "Displays › Display › Resolution"),
            ("scale", "Displays › Display › Scale"),
            ("hidpi", "Displays › Display › Scale"),
            ("hz", "Displays › Display › Refresh rate"),
            ("do not disturb", "Alerts › Quiet hours › Quiet hours"),
            ("screenshot folder", "Alerts › Capture › Folder"),
            ("24h", "Bar › Clock › 24-hour clock"),
            (
                "sudo",
                "Idle & Lock › Administrator access › Card for terminal sudo",
            ),
            ("wireless", "Panel › Wi-Fi"),
            ("headphones", "Panel › Audio output"),
            ("pair", "Panel › Bluetooth"),
            ("accent colour", "Appearance › Accent"),
        ] {
            assert_eq!(top(query), path, "{query:?}");
        }
    }

    #[test]
    fn titles_and_subtitles_find_their_row_too() {
        assert_eq!(top("launch zoom"), "Appearance › Motion › Launch zoom");
        assert_eq!(top("refraction"), "Glass › Material › Refraction");
        assert_eq!(top("adaptive"), "Displays › Display › Adaptive sync");
        // A word only the subtitle has.
        assert!(
            find("backlight")
                .iter()
                .any(|h| h.path == "Idle & Lock › Idle timers › Dim after")
        );
    }

    #[test]
    fn ranking_is_sane() {
        // A whole word of the title or a keyword names the setting; a
        // prefix of one only suggests it.
        assert!(find("dark")[0].clear);
        assert!(find("resolution")[0].clear);
        assert!(!find("reso")[0].clear);
        // Two letters are a query, one is not; "fi" alone is too short to
        // be clear.
        assert!(find("x").is_empty());
        assert!(find("fi").iter().all(|h| !h.clear));
        // An app's name finds no setting.
        assert!(find("firefox").is_empty());
        assert!(find("spotify").is_empty());
        // Every query word has to land.
        assert!(find("dark zebra").is_empty());
        // The title outranks a keyword, a keyword the tab's name.
        let lock = find("lock");
        assert!(lock[0].path.ends_with("Lock after"), "{:?}", lock[0].path);
        // Scores come out best first.
        let hits = find("screen");
        assert!(hits.windows(2).all(|w| w[0].score >= w[1].score));
    }

    #[test]
    fn every_key_a_row_names_is_a_setting() {
        let settings = crate::settings::store::Settings::default();
        for (tab, entries) in TABLES {
            for e in *entries {
                for key in e.keys {
                    assert!(
                        settings.get(key).is_some(),
                        "{tab} › {} › {} names {key}, which is no setting",
                        e.group,
                        e.title
                    );
                }
            }
        }
    }
}
