//! What a query asks for, and where its results come from.
//!
//! The launcher has two kinds of source. **Remote** results come from
//! elephant over its socket, after the debounce; which providers are asked is
//! [`providers`], from the query's prefix and the Launcher settings.
//! **Local** results are made here, on the main thread, in the frame the key
//! was typed in: the calculator, the command row and the pages of the panel.
//! They cost microseconds and need nothing but the query. The settings
//! rows are local too: a walk over the static index (`settings::search`).

use crate::services::elephant::SearchResult;
use crate::settings::store::Launcher;

use super::calc;

/// The providers of the rows the launcher makes itself.
pub const CALC: &str = "swaypplet-calc";
pub const RUN: &str = "swaypplet-run";
pub const PAGE: &str = "swaypplet-page";
pub const SETTING: &str = "swaypplet-setting";

/// What was typed, read by its prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Query<'a> {
    Empty,
    /// `=expr`.
    Calc(&'a str),
    /// `>command`.
    Command(&'a str),
    Plain(&'a str),
}

pub fn parse(text: &str) -> Query<'_> {
    let t = text.trim_start();
    if let Some(rest) = t.strip_prefix('=') {
        Query::Calc(rest.trim())
    } else if let Some(rest) = t.strip_prefix('>') {
        Query::Command(rest.trim())
    } else if t.trim().is_empty() {
        Query::Empty
    } else {
        Query::Plain(t.trim_end())
    }
}

impl Query<'_> {
    /// What elephant is asked, with the prefix taken off.
    pub fn text(&self) -> &str {
        match self {
            Query::Empty => "",
            Query::Calc(s) | Query::Command(s) | Query::Plain(s) => s,
        }
    }
}

/// The elephant providers for `q` under the settings `l`. Empty means
/// elephant is not asked at all.
pub fn providers(l: &Launcher, q: Query) -> Vec<&'static str> {
    match q {
        Query::Empty => {
            if l.apps {
                vec!["desktopapplications"]
            } else {
                Vec::new()
            }
        }
        // Only what the local calculator cannot read goes to elephant.
        Query::Calc(expr) => {
            if l.calculator && !expr.is_empty() && calc::eval(expr).is_none() {
                vec!["calc"]
            } else {
                Vec::new()
            }
        }
        Query::Command(cmd) => {
            if l.commands && !cmd.is_empty() {
                vec!["runner"]
            } else {
                Vec::new()
            }
        }
        Query::Plain(_) => [
            (l.apps, "desktopapplications"),
            (l.calculator, "calc"),
            (l.commands, "runner"),
            (l.windows, "windows"),
            (l.clipboard, "clipboard"),
            (l.menus, "menus"),
            (l.web_search, "websearch"),
            (l.files, "files"),
            (l.bookmarks, "bookmarks"),
            (l.symbols, "symbols"),
            (l.symbols, "unicode"),
            // Elephant's own list of what it can search; always on, it is
            // how a provider not listed here is found at all.
            (true, "providerlist"),
        ]
        .into_iter()
        .filter_map(|(on, p)| on.then_some(p))
        .collect(),
    }
}

/// Whether a remembered row of `provider` may show under `l`: the empty
/// query's frecent rows follow the same switches as the search.
pub fn allows(l: &Launcher, provider: &str) -> bool {
    match provider {
        "desktopapplications" => l.apps,
        "runner" => l.commands,
        "menus" => l.menus,
        "websearch" => l.web_search,
        "files" => l.files,
        "bookmarks" => l.bookmarks,
        "symbols" | "unicode" => l.symbols,
        "clipboard" => l.clipboard,
        "windows" => l.windows,
        "calc" => l.calculator,
        _ => true,
    }
}

/// A page the launcher can open by name: a deck page or a settings tab of
/// the Helm card, and the omnibox prefix that routes to it.
#[derive(Debug, Clone)]
pub struct Page {
    pub title: String,
    pub prefix: String,
    /// Lowercased words a query may start: the title's words and the
    /// prefixes without their colon.
    pub words: Vec<String>,
}

impl Page {
    pub fn new(title: &str, prefixes: &[&str]) -> Page {
        let mut words: Vec<String> = title
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.len() > 1)
            .map(str::to_string)
            .collect();
        words.extend(
            prefixes
                .iter()
                .map(|p| p.trim_start_matches(':').to_lowercase()),
        );
        Page {
            title: title.to_string(),
            prefix: prefixes.first().copied().unwrap_or_default().to_string(),
            words,
        }
    }

    /// Every query word starts one of the page's words. Two characters at
    /// least, so a single letter does not list half the panel.
    fn matches(&self, query: &str) -> bool {
        let q = query.to_lowercase();
        if q.chars().count() < 2 {
            return false;
        }
        q.split_whitespace()
            .all(|w| self.words.iter().any(|pw| pw.starts_with(w)))
    }
}

/// At most this many page rows, above elephant's.
const MAX_PAGE_ROWS: usize = 2;

fn row(provider: &str, identifier: String, text: String, subtext: String) -> SearchResult {
    SearchResult {
        identifier,
        text,
        subtext,
        icon: String::new(),
        provider: provider.to_string(),
        score: 0,
        actions: Vec::new(),
    }
}

/// The rows made here for `q`, best first.
pub fn local(l: &Launcher, q: Query, pages: &[Page], shell: &str) -> Vec<SearchResult> {
    let mut out = Vec::new();
    match q {
        Query::Empty => {}
        Query::Calc(expr) => {
            if let (true, Some(v)) = (l.calculator, calc::eval(expr)) {
                out.push(calc_row(expr, v));
            }
        }
        Query::Command(cmd) => {
            if l.commands && !cmd.is_empty() {
                let name = shell.rsplit('/').next().unwrap_or(shell);
                out.push(row(
                    RUN,
                    cmd.to_string(),
                    format!("Run {cmd}"),
                    format!("In {name}"),
                ));
            }
        }
        Query::Plain(text) => {
            if l.calculator
                && calc::looks_like_arithmetic(text)
                && let Some(v) = calc::eval(text)
            {
                out.push(calc_row(text, v));
            }
            if l.settings {
                out.extend(
                    pages
                        .iter()
                        .filter(|p| p.matches(text))
                        .take(MAX_PAGE_ROWS)
                        .map(|p| {
                            row(
                                PAGE,
                                p.prefix.clone(),
                                p.title.clone(),
                                format!("Open · {}", p.prefix),
                            )
                        }),
                );
            }
        }
    }
    out
}

/// At most this many settings rows above the apps, for a query that names a
/// setting, and below them, for one that only might.
const MAX_CLEAR_SETTINGS: usize = 3;
const MAX_OTHER_SETTINGS: usize = 4;

/// The settings rows for `q`: those that go above elephant's rows, and
/// those that go below them. A query goes above when every word of it is a
/// whole word of a setting's title or keywords ("dark", "blur",
/// "resolution"); the rest ("re", "fir") trail the apps, where a word that
/// happens to start a setting's name costs nothing.
pub fn settings(l: &Launcher, q: Query) -> (Vec<SearchResult>, Vec<SearchResult>) {
    let Query::Plain(text) = q else {
        return (Vec::new(), Vec::new());
    };
    if !l.settings {
        return (Vec::new(), Vec::new());
    }
    let (clear, other): (Vec<_>, Vec<_>) = crate::settings::search::find(text)
        .into_iter()
        .partition(|h| h.clear);
    let to_row = |h: crate::settings::search::Hit| {
        row(SETTING, h.target.id(), h.path, h.subtitle.to_string())
    };
    (
        clear
            .into_iter()
            .take(MAX_CLEAR_SETTINGS)
            .map(to_row)
            .collect(),
        other
            .into_iter()
            .take(MAX_OTHER_SETTINGS)
            .map(to_row)
            .collect(),
    )
}

fn calc_row(expr: &str, v: f64) -> SearchResult {
    let value = calc::format(v);
    row(
        CALC,
        value.clone(),
        format!("= {value}"),
        format!("{expr} · Enter copies"),
    )
}

/// Whether elephant's row `r` is the command row this launcher already
/// made for `cmd`: its `runner` echoes the typed line back as "Run: cmd".
pub fn duplicates_run_row(r: &SearchResult, cmd: &str) -> bool {
    let text = r.text.trim();
    let echoed = text
        .get(..4)
        .is_some_and(|p| p.eq_ignore_ascii_case("run:"))
        .then(|| text[4..].trim());
    r.provider == "runner" && (r.identifier == cmd || echoed == Some(cmd))
}

/// `cmd` as one word of a POSIX shell line: single-quoted, with each
/// single quote closed, escaped and reopened.
pub fn sh_quote(cmd: &str) -> String {
    format!("'{}'", cmd.replace('\'', r"'\''"))
}

/// The sway command that runs `cmd` in `shell`. Sway leaves an `exec`
/// line's quotes alone and hands it to `sh -c`, so the quoting above is the
/// only quoting there is; the command is then the compositor's child, not
/// the panel's, and outlives a panel restart.
pub fn exec_line(shell: &str, cmd: &str) -> String {
    format!("exec {} -c {}", sh_quote(shell), sh_quote(cmd))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_prefix_picks_the_kind() {
        assert_eq!(parse(""), Query::Empty);
        assert_eq!(parse("   "), Query::Empty);
        assert_eq!(parse("=2+2"), Query::Calc("2+2"));
        assert_eq!(parse(" = 2 + 2 "), Query::Calc("2 + 2"));
        assert_eq!(parse(">ls -la"), Query::Command("ls -la"));
        assert_eq!(parse("firefox"), Query::Plain("firefox"));
    }

    #[test]
    fn the_switches_choose_the_providers() {
        let all = Launcher::default();
        let p = providers(&all, Query::Plain("x"));
        assert!(p.contains(&"desktopapplications") && p.contains(&"websearch"));
        assert!(!p.contains(&"files"));
        let only_apps = Launcher {
            calculator: false,
            commands: false,
            windows: false,
            clipboard: false,
            menus: false,
            web_search: false,
            ..all
        };
        assert_eq!(
            providers(&only_apps, Query::Plain("x")),
            ["desktopapplications", "providerlist"]
        );
        // Arithmetic the local calculator reads never reaches elephant.
        assert!(providers(&all, Query::Calc("2+2")).is_empty());
        assert_eq!(providers(&all, Query::Calc("5 usd in sek")), ["calc"]);
        assert_eq!(providers(&all, Query::Command("ls")), ["runner"]);
        let no_apps = Launcher { apps: false, ..all };
        assert!(providers(&no_apps, Query::Empty).is_empty());
    }

    #[test]
    fn local_rows_answer_in_the_same_call() {
        let l = Launcher::default();
        let pages = [Page::new("Settings · Glass", &[":glass", ":material"])];
        let r = local(&l, Query::Calc("2*21"), &pages, "zsh");
        assert_eq!(r[0].text, "= 42");
        assert_eq!(r[0].identifier, "42");
        let r = local(&l, Query::Plain("2+2"), &pages, "zsh");
        assert_eq!(r[0].provider, CALC);
        let r = local(&l, Query::Plain("mater"), &pages, "zsh");
        assert_eq!(
            (r[0].provider.as_str(), r[0].identifier.as_str()),
            (PAGE, ":glass")
        );
        assert!(local(&l, Query::Plain("g"), &pages, "zsh").is_empty());
        let r = local(
            &l,
            Query::Command("make -j"),
            &pages,
            "/run/current-system/sw/bin/zsh",
        );
        assert_eq!(r[0].text, "Run make -j");
        assert_eq!(r[0].subtext, "In zsh");
        let echo = SearchResult {
            provider: "runner".into(),
            text: "Run: make -j".into(),
            ..r[0].clone()
        };
        assert!(duplicates_run_row(&echo, "make -j"));
        let lower = SearchResult {
            text: "run: make -j".into(),
            ..echo.clone()
        };
        assert!(duplicates_run_row(&lower, "make -j"));
        assert!(!duplicates_run_row(&echo, "make"));
        let off = Launcher {
            settings: false,
            calculator: false,
            ..l
        };
        assert!(local(&off, Query::Plain("glass"), &pages, "zsh").is_empty());
        assert!(local(&off, Query::Calc("1+1"), &pages, "zsh").is_empty());
    }

    #[test]
    fn settings_rows_go_above_the_apps_only_when_the_query_names_one() {
        let l = Launcher::default();
        let (above, below) = settings(&l, Query::Plain("dark"));
        assert_eq!(above[0].text, "Appearance › Appearance › Mode");
        assert_eq!(above[0].provider, SETTING);
        assert!(below.len() <= MAX_OTHER_SETTINGS);
        let (above, below) = settings(&l, Query::Plain("reso"));
        assert!(above.is_empty());
        assert_eq!(below[0].text, "Displays › Display › Resolution");
        let (above, below) = settings(&l, Query::Plain("firefox"));
        assert!(above.is_empty() && below.is_empty());
        // The Launcher tab's switch for pages and settings covers these.
        let off = Launcher {
            settings: false,
            ..l
        };
        assert!(settings(&off, Query::Plain("dark")).0.is_empty());
        assert!(settings(&l, Query::Command("dark")).0.is_empty());
    }

    #[test]
    fn a_command_reaches_the_shell_as_typed() {
        assert_eq!(sh_quote("it's"), r"'it'\''s'");
        assert_eq!(
            exec_line("/bin/zsh", "echo 'a; b' && ls"),
            r"exec '/bin/zsh' -c 'echo '\''a; b'\'' && ls'"
        );
        // What sh makes of the quoted word is the command, byte for byte.
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("printf %s {}", sh_quote("a 'b' $HOME \"c\"")))
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "a 'b' $HOME \"c\"");
    }
}
