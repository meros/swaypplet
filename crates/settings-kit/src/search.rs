//! The settings as the launcher finds them: every row of every tab, by its
//! title, a one-line subtitle and the words people type for it ("dark mode"
//! for Appearance › Mode, "blur" for Glass › Material › Frost).
//!
//! The index is static. Each pane keeps its own table, `SEARCH`, beside the
//! code that builds its rows, and the settings window installs the tables
//! by tab ([`install`], from `settings::PANES`). Nothing is built or read
//! until a query asks, and then only those tables are walked: no widget, no
//! file, no process. This module knows no pane: an [`Index`] is built from
//! whatever tabs it is given, so its tests use tables of their own.
//!
//! An entry is anchored by what the pane already shows: the group's overline
//! and the row's gutter label. `SettingsSection::reveal` finds the row by
//! those two strings, so a pane needs no ids of its own. What keeps the two
//! from drifting apart is each pane's test ([`anchors::check`]): every
//! entry's group and title must be a string literal in its pane's source,
//! and every row the pane builds must have an entry, so renaming a row
//! without its entry fails `cargo test`.
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

/// One settings tab as the index sees it.
#[derive(Debug, Clone, Copy)]
pub struct Tab {
    /// The pane's stack name (`settings::PANES`).
    pub name: &'static str,
    /// The pane's title, the first step of a hit's path.
    pub title: &'static str,
    /// The pane's `SEARCH` table.
    pub entries: &'static [Entry],
}

static TABS: std::sync::OnceLock<Vec<Tab>> = std::sync::OnceLock::new();

/// Install the settings tabs the index searches. The settings window does
/// it (`settings::install_search`) before anything queries; until then the
/// index holds the panel's sections only. A second call changes nothing.
pub fn install(tabs: Vec<Tab>) {
    let _ = TABS.set(tabs);
}

/// The installed tabs.
pub fn tabs() -> &'static [Tab] {
    TABS.get().map(Vec::as_slice).unwrap_or(&[])
}

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

    /// The target an identifier names, if it is one of the installed
    /// index's.
    pub fn from_id(id: &str) -> Option<Target> {
        Self::from_id_in(tabs(), id)
    }

    /// The target an identifier names among `tabs` and the panel's sections.
    pub fn from_id_in(tabs: &[Tab], id: &str) -> Option<Target> {
        if id.starts_with(':') {
            return QUICK
                .iter()
                .find(|q| q.prefix == id)
                .map(|q| Target::Page(q.prefix));
        }
        let mut parts = id.split('\t');
        let (tab, group, title) = (parts.next()?, parts.next()?, parts.next()?);
        tabs.iter()
            .filter(|t| t.name == tab)
            .flat_map(|t| t.entries.iter().map(move |e| (t.name, e)))
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

fn prepare(tabs: &[Tab]) -> Vec<Prepared> {
    let mut out = Vec::new();
    for tab in tabs {
        let tab_title = tab.title;
        for e in tab.entries {
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
                target: Target::Row(tab.name, e.group, e.title),
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

/// The searchable form of some tabs and the panel's sections.
pub struct Index {
    prepared: Vec<Prepared>,
}

impl Index {
    pub fn new(tabs: &[Tab]) -> Index {
        Index {
            prepared: prepare(tabs),
        }
    }

    /// The entries `query` finds, best first. Nothing under two characters,
    /// so a single letter does not list half the settings.
    pub fn find(&self, query: &str) -> Vec<Hit> {
        let qwords = words(query);
        let phrase = qwords.join(" ");
        if phrase.chars().count() < 2 {
            return Vec::new();
        }
        let mut hits: Vec<Hit> = self
            .prepared
            .iter()
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
            .collect();
        // Stable: equals keep the tables' order, which is the pane's.
        hits.sort_by_key(|h| std::cmp::Reverse(h.score));
        hits
    }
}

thread_local! {
    /// The installed tabs' index, built on the first query after
    /// [`install`] and kept: a few hundred short strings.
    static INSTALLED: std::cell::RefCell<Option<(usize, std::rc::Rc<Index>)>> =
        const { std::cell::RefCell::new(None) };
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

/// What the search says when `query` finds nothing: a pointer for the
/// words that name no one row, or the plain "try another word".
pub fn no_match_hint(query: &str) -> &'static str {
    let q = words(query).join(" ");
    if ["reset", "default", "defaults", "system default", "factory"]
        .iter()
        .any(|w| q.starts_with(w))
    {
        "Every pane has Reset to system at its foot"
    } else {
        "Try another word"
    }
}

/// The entries `query` finds in the installed index, best first.
pub fn find(query: &str) -> Vec<Hit> {
    let tabs = tabs();
    let index = INSTALLED.with(|cell| {
        let mut cell = cell.borrow_mut();
        match &*cell {
            Some((n, index)) if *n == tabs.len() => index.clone(),
            _ => {
                let index = std::rc::Rc::new(Index::new(tabs));
                *cell = Some((tabs.len(), index.clone()));
                index
            }
        }
    });
    index.find(query)
}

/// The check each pane's test runs on its own table and source: every
/// entry names a group and a row the pane builds, and every row the pane
/// builds has an entry. `loose_groups` are groups whose overline is not a
/// literal in the source (Displays titles a group by its output).
pub mod anchors {
    use super::Entry;

    /// The source before its `SEARCH` table, so an entry cannot vouch for
    /// itself.
    fn code(src: &str) -> &str {
        match src.find("const SEARCH") {
            Some(i) => &src[..i],
            None => src,
        }
    }

    pub fn check(tab: &str, entries: &[Entry], sources: &[&str], loose_groups: &[&str]) {
        assert!(!entries.is_empty(), "the {tab} tab has no search table");
        let src: String = sources.iter().map(|s| code(s)).collect();
        let src = src.as_str();
        for e in entries {
            if !loose_groups.contains(&e.group) {
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
        // The labelled rows: the first string argument of each row helper.
        // Rows built from a table (`timer.label`) are covered by the table's
        // `label: "…"` lines.
        let mut labels = Vec::new();
        for source in sources {
            let src = code(source);
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

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &[Tab] = &[Tab {
        name: "look",
        title: "Appearance",
        entries: &[
            row(
                "Appearance",
                "Mode",
                "Dark, light or by the sun",
                &["dark", "theme"],
            ),
            row(
                "Motion",
                "Launch zoom",
                "Windows grow from where they start",
                &[],
            ),
        ],
    }];

    #[test]
    fn an_id_goes_back_to_its_target() {
        let index = Index::new(FIXTURE);
        for hit in index.find("mode").into_iter().chain(index.find("wifi")) {
            assert_eq!(
                Target::from_id_in(FIXTURE, &hit.target.id()),
                Some(hit.target)
            );
        }
        assert_eq!(Target::from_id_in(FIXTURE, "look\tNope\tMode"), None);
        assert_eq!(Target::from_id_in(FIXTURE, ":nothing"), None);
    }

    #[test]
    fn a_group_named_after_its_pane_is_said_once() {
        let index = Index::new(FIXTURE);
        assert_eq!(index.find("dark")[0].path, "Appearance › Mode");
        assert_eq!(
            index.find("zoom")[0].path,
            "Appearance › Motion › Launch zoom"
        );
    }

    #[test]
    fn a_query_needs_two_characters_and_every_word() {
        let index = Index::new(FIXTURE);
        assert!(index.find("x").is_empty());
        assert!(index.find("dark zebra").is_empty());
        assert!(index.find("dark")[0].clear);
        assert!(!index.find("dar")[0].clear);
    }

    /// "reset" and "default" name no row: every pane has one. The empty
    /// search says where it is instead of "try another word".
    #[test]
    fn reset_says_where_it_is() {
        assert_eq!(
            no_match_hint("Reset"),
            "Every pane has Reset to system at its foot"
        );
        assert_eq!(
            no_match_hint("default"),
            "Every pane has Reset to system at its foot"
        );
        assert_eq!(no_match_hint("zzzq"), "Try another word");
    }
}
