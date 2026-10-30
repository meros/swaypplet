//! The Launcher tab: what the launcher's results include, and whether it
//! learns from what you launch.
//!
//! One `launcher` section. The launcher reads it on every query
//! (`launcher::sources`), so a switch here applies to the next keystroke;
//! nothing is rebuilt.

use std::rc::Rc;

use gtk4::prelude::*;

use swaypplet_model::{store, store::Launcher};
use swaypplet_settings_kit::{
    form,
    form::{section_box, switch_row},
};

/// One switch: its label, its hint, and the field it moves.
struct Switch {
    label: &'static str,
    hint: &'static str,
    get: fn(&Launcher) -> bool,
    set: fn(&mut Launcher, bool),
}

/// The kinds of result, in the order the list tends to show them.
static SOURCES: &[Switch] = &[
    Switch {
        label: "Applications",
        hint: "Installed apps.",
        get: |l| l.apps,
        set: |l, v| l.apps = v,
    },
    Switch {
        label: "Open windows",
        hint: "\u{201c}Go to\u{201d} rows above an app that is already open, and Tab on an app for all of its windows.",
        get: |l| l.windows,
        set: |l, v| l.windows = v,
    },
    Switch {
        label: "Calculator",
        hint: "Arithmetic as you type it, or after =. Enter copies the result.",
        get: |l| l.calculator,
        set: |l, v| l.calculator = v,
    },
    Switch {
        label: "Commands",
        hint: "> runs the rest in your shell; commands on your PATH show as results.",
        get: |l| l.commands,
        set: |l, v| l.commands = v,
    },
    Switch {
        label: "Pages and settings",
        hint: "The Helm's pages and these settings, by name.",
        get: |l| l.settings,
        set: |l, v| l.settings = v,
    },
    Switch {
        label: "Clipboard",
        hint: "Entries from the clipboard history that match.",
        get: |l| l.clipboard,
        set: |l, v| l.clipboard = v,
    },
    Switch {
        label: "App actions",
        hint: "Actions and menus that apps publish, such as a new private window.",
        get: |l| l.menus,
        set: |l, v| l.menus = v,
    },
    Switch {
        label: "Web search",
        hint: "A row that searches the web for what you typed.",
        get: |l| l.web_search,
        set: |l, v| l.web_search = v,
    },
    Switch {
        label: "Files",
        hint: "Files by name. A large home folder makes the list slower to fill.",
        get: |l| l.files,
        set: |l, v| l.files = v,
    },
    Switch {
        label: "Bookmarks",
        hint: "Browser bookmarks.",
        get: |l| l.bookmarks,
        set: |l, v| l.bookmarks = v,
    },
    Switch {
        label: "Emoji and symbols",
        hint: "Emoji and Unicode characters by name.",
        get: |l| l.symbols,
        set: |l, v| l.symbols = v,
    },
];

fn describe(l: &Launcher) -> String {
    let on: Vec<&str> = SOURCES
        .iter()
        .filter(|s| (s.get)(l))
        .map(|s| s.label)
        .collect();
    format!(
        "System default: {}; learning {}",
        on.join(", ").to_lowercase(),
        if l.frecency { "on" } else { "off" }
    )
}

struct State {
    switches: Vec<(gtk4::Switch, &'static Switch)>,
    frecency: gtk4::Switch,
    status: gtk4::Label,
    guard: form::Sync,
}

impl State {
    fn edit(&self, f: impl FnOnce(&mut Launcher)) {
        self.guard.edit(f, || self.sync());
    }

    fn sync(&self) {
        self.guard.writing(|| self.draw());
    }

    fn draw(&self) {
        let settings = store::current();
        let l = settings.launcher();
        for (w, s) in &self.switches {
            w.set_active((s.get)(&l));
        }
        self.frecency.set_active(l.frecency);
        form::set_source(&self.status, settings.launcher.is_some(), &describe(&l));
    }
}

pub struct LauncherPane {
    root: gtk4::Box,
    state: Rc<State>,
}

impl LauncherPane {
    pub fn new() -> Self {
        let root = form::pane();
        let l = store::current().launcher();

        let results = section_box(
            "Results",
            "What a search lists. A switch applies to the next key you type.",
        );
        let mut switches = Vec::new();
        for s in SOURCES {
            let (row, w) = switch_row(s.label, s.hint, (s.get)(&l));
            if let Some(line) = match s.label {
                "Commands" => Some("Type > to run a command"),
                "Pages and settings" => Some("Helm pages and these settings"),
                "App actions" => Some("Such as a new private window"),
                _ => None,
            } {
                form::subtitle(&row, line);
            }
            results.append(&row);
            switches.push((w, s));
        }

        let learning = section_box(
            "Learning",
            "What you launch rises in the results and opens the list when nothing is typed. The history stays on this machine.",
        );
        let (row_frecency, frecency) = switch_row(
            "Rank by use",
            "Off, results come in the order the search gives them.",
            l.frecency,
        );
        form::subtitle(&row_frecency, "What you open often comes first");
        learning.append(&row_frecency);
        let forget = form::action_button(
            "Forget history",
            "Clear what the launcher has learned. Your settings stay.",
        );
        learning.append(&form::kind_row("History", &forget));

        let reset = form::ResetButtons::new(
            "Put the system's choices back and drop the launcher section from the settings file.",
        );
        let (footer, status) = form::reset_footer(
            &reset,
            "The launcher section as theme/settings.nix holds it.",
            &["launcher"],
        );

        let state = Rc::new(State {
            switches,
            frecency: frecency.clone(),
            status,
            guard: form::Sync::default(),
        });

        for (w, s) in &state.switches {
            let state_c = state.clone();
            let set = s.set;
            w.connect_active_notify(move |w| {
                let on = w.is_active();
                state_c.edit(|l| set(l, on));
            });
        }
        {
            let state = state.clone();
            frecency.connect_active_notify(move |w| {
                let on = w.is_active();
                state.edit(|l| l.frecency = on);
            });
        }
        {
            // Asks first: what was learned cannot be put back.
            let status = state.status.clone();
            swaypplet_ui::ui::arm(&forget, "Forget all of it?", move |_| {
                swaypplet_launcher::frecency::forget();
                status.set_text("History forgotten: results start from scratch.");
                form::mark_source(&status, false);
            });
        }
        {
            let state = state.clone();
            let undone = state.clone();
            reset.connect(
                move || {
                    store::reset::<Launcher>();
                    state.sync();
                },
                move || undone.sync(),
            );
        }

        root.append(&results);
        root.append(&learning);
        root.append(&footer);
        state.sync();

        LauncherPane { root, state }
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    pub fn refresh(&self) {
        self.state.sync();
    }
}

// ── Search ──────────────────────────────────────────────────────────────

use swaypplet_settings_kit::search::{Entry, row};

/// This tab's rows as the launcher finds them (`search.rs`). A row added
/// to the tab gets a line here; the test there fails until it does.
#[rustfmt::skip]
pub const SEARCH: &[Entry] = &[
    row("Results", "Applications", "Installed apps in a search", &["apps", "programs", "desktop"]).keys(&["launcher.apps"]),
    row("Results", "Open windows", "Windows already open in a search", &["windows", "switch", "go to"]).keys(&["launcher.windows"]),
    row("Results", "Calculator", "Arithmetic as you type it", &["calc", "math", "maths", "qalc"]).keys(&["launcher.calculator"]),
    row("Results", "Commands", "> runs a line in your shell", &["shell", "run", "terminal", "command"]).keys(&["launcher.commands"]),
    row("Results", "Pages and settings", "The Helm's pages and settings in a search", &["settings search", "pages"]).keys(&["launcher.settings"]),
    row("Results", "Clipboard", "Clipboard history in a search", &["clipboard history", "paste", "copied"]).keys(&["launcher.clipboard"]),
    row("Results", "App actions", "What apps publish, such as a private window", &["actions", "menus", "desktop actions"]).keys(&["launcher.menus"]),
    row("Results", "Web search", "Search the web for what you typed", &["google", "duckduckgo", "browser"]).keys(&["launcher.web_search"]),
    row("Results", "Files", "Files by name", &["documents", "find files"]).keys(&["launcher.files"]),
    row("Results", "Bookmarks", "Browser bookmarks", &["browser", "favourites", "favorites"]).keys(&["launcher.bookmarks"]),
    row("Results", "Emoji and symbols", "Emoji and Unicode characters by name", &["emoji", "unicode", "characters", "symbols", "emoticons"]).keys(&["launcher.symbols"]),
    row("Learning", "Rank by use", "What you launch rises in the results", &["frecency", "history", "ranking", "recent"]).keys(&["launcher.frecency"]),
    row("Learning", "History", "Forget what you launched", &["forget", "clear history", "privacy", "frecency"]),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Every switch moves its own field and only that one.
    #[test]
    fn each_switch_owns_one_field() {
        for s in SOURCES {
            let mut l = Launcher::default();
            let before = (s.get)(&l);
            (s.set)(&mut l, !before);
            assert_eq!((s.get)(&l), !before, "{}", s.label);
            let changed = SOURCES
                .iter()
                .filter(|o| (o.get)(&l) != (o.get)(&Launcher::default()))
                .count();
            assert_eq!(changed, 1, "{} moved another switch", s.label);
        }
    }
}

#[cfg(test)]
mod search_anchors {
    /// Every entry of `SEARCH` names a row this pane builds, and every row
    /// it builds has an entry.
    #[test]
    fn the_search_table_matches_the_rows() {
        swaypplet_settings_kit::search::anchors::check(
            "launcher",
            super::SEARCH,
            &[include_str!("lib.rs")],
            &[],
        );
    }
}
