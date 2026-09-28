//! Settings: a surface of its own (`window.rs`), a sidebar of panes and the
//! chosen pane at full height beside it.
//!
//! Settings used to be a deck page in the Helm card, reached from a button
//! among 25 others and read through a 420 px scroller under ten tab chips.
//! The Helm is for what is done now; this is for what is configured once,
//! and it gets the room for it. It wears the Helm's glass (the same layer
//! namespace), so the Glass pane still changes the surface it is drawn on,
//! live.
//!
//! Nine panes in two groups. Settings: Appearance, Glass, Idle & Lock,
//! Bar, Input, Alerts, Launcher and Displays edit `store::Settings`, one
//! file with one to three sections each (Glass keeps its own file,
//! `glass.rs`; Displays keeps its profiles in the `displays` section,
//! `displays_pane.rs`). This machine: System shows this host's build
//! against origin/main and runs nx (`system_pane.rs`), and Quality shows
//! the open issues and the fixes for them (`quality_pane.rs`); they edit
//! nothing, and say so by where they sit. Every settings pane applies live
//! and saves after the fact, and each has one Reset that puts the defaults
//! back and removes its sections from the file, so there is always a way
//! out of a setting that turned out to be wrong.
//!
//! What is deliberately not here: the bar's position and height (a layout
//! the whole stylesheet is built around), and anything done now rather than
//! configured, which is the Helm's. A setting earns a row when it is a
//! matter of taste that a rebuild is too slow a loop for.

mod alerts_pane;
mod arrange;
mod bar_pane;
pub mod cli;
mod displays_pane;
mod form;
pub mod glass;
mod glass_fade;
mod glass_pane;
mod daylight_group;
mod idle_pane;
mod location_map;
mod input_pane;
mod keep;
mod launcher_pane;
mod look_pane;
pub mod preset;
mod quality_pane;
pub mod schema;
pub mod search;
pub mod store;
mod system_info;
mod system_job;
mod system_pane;
pub mod wallpaper;
pub mod window;
pub mod xkb;

use gtk4::prelude::*;

/// Which sidebar group a pane sits in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Group {
    /// Edits a setting.
    Settings,
    /// Shows this host and acts on it; edits nothing.
    Machine,
}

impl Group {
    fn title(self) -> &'static str {
        match self {
            Group::Settings => "Settings",
            Group::Machine => "This machine",
        }
    }
}

/// A pane: its stack name, its sidebar title and glyph, its group, and the
/// omnibox prefixes that open it.
struct Pane {
    name: &'static str,
    title: &'static str,
    glyph: &'static str,
    group: Group,
    prefixes: &'static [&'static str],
}

const PANES: [Pane; 10] = [
    Pane {
        name: "look",
        title: "Appearance",
        glyph: "󰏘",
        group: Group::Settings,
        prefixes: &[
            ":look", ":appear", ":wall", ":bg", ":paper", ":motion", ":theme",
        ],
    },
    Pane {
        name: "glass",
        title: "Glass",
        glyph: "󰂵",
        group: Group::Settings,
        prefixes: &[":glass", ":material"],
    },
    Pane {
        name: "idle",
        title: "Idle & Lock",
        glyph: "󰌾",
        group: Group::Settings,
        prefixes: &[
            ":idle", ":lock", ":timeout", ":sleep", ":sudo", ":elevate", ":admin",
        ],
    },
    Pane {
        name: "bar",
        title: "Bar",
        glyph: "󰍜",
        group: Group::Settings,
        prefixes: &[":bar", ":clock", ":osd", ":pins"],
    },
    Pane {
        name: "input",
        title: "Input",
        glyph: crate::ui::icons::KEYBOARD,
        group: Group::Settings,
        prefixes: &[
            ":input",
            ":keyboard",
            ":layout",
            ":touchpad",
            ":mouse",
            ":keys",
        ],
    },
    Pane {
        name: "alerts",
        title: "Alerts",
        glyph: crate::ui::icons::NOTIFICATION,
        group: Group::Settings,
        prefixes: &[":alerts", ":quiet", ":shot", ":capture"],
    },
    Pane {
        name: "launcher",
        title: "Launcher",
        glyph: "󰍉",
        group: Group::Settings,
        prefixes: &[":launch", ":search"],
    },
    Pane {
        name: "displays",
        title: "Displays",
        glyph: crate::ui::icons::DISPLAY,
        group: Group::Settings,
        prefixes: &[":monitor", ":output", ":arrange"],
    },
    Pane {
        name: "system",
        title: "System",
        glyph: "󱄅",
        group: Group::Machine,
        prefixes: &[":nixos", ":nx", ":update", ":host"],
    },
    Pane {
        name: "quality",
        title: "Quality",
        glyph: "󰃤",
        group: Group::Machine,
        prefixes: &[":quality", ":report", ":crash", ":autofix"],
    },
];

/// Every prefix and the pane it opens, for the omnibox's help page and the
/// launcher's page rows.
pub fn prefixes() -> impl Iterator<Item = (&'static [&'static str], String)> {
    PANES
        .iter()
        .map(|p| (p.prefixes, format!("Settings · {}", p.title)))
}

/// A pane's title, by its stack name.
fn pane_title(name: &str) -> Option<&'static str> {
    PANES.iter().find(|p| p.name == name).map(|p| p.title)
}

/// The pane an omnibox prefix opens, if it names one. `:set` and `:pref`
/// open settings on whatever pane it was last on and are not in this table.
pub fn tab_for_prefix(prefix: &str) -> Option<&'static str> {
    PANES
        .iter()
        .find(|p| p.prefixes.iter().any(|x| prefix.starts_with(x)))
        .map(|p| p.name)
}

/// What to open settings on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Open {
    /// Wherever it was last.
    Last,
    /// A pane, by its stack name.
    Pane(&'static str),
    /// A row of the search index, by its identifier (`search::Target::id`).
    Row(String),
}

impl Open {
    /// What an omnibox prefix opens here: a pane, or settings as it was
    /// (`:set`, `:pref`). `None` for a prefix settings does not own.
    pub fn for_prefix(prefix: &str) -> Option<Open> {
        if let Some(pane) = tab_for_prefix(prefix) {
            return Some(Open::Pane(pane));
        }
        (prefix.starts_with(":set") || prefix.starts_with(":pref")).then_some(Open::Last)
    }
}

type OnPage = std::rc::Rc<std::cell::RefCell<Option<Box<dyn Fn(&str)>>>>;

pub struct SettingsSection {
    root: gtk4::Box,
    search: gtk4::SearchEntry,
    nav: gtk4::ListBox,
    results: gtk4::ListBox,
    /// Each hit's target (`search::Target::id`), by its row's index.
    hits: std::cell::RefCell<Vec<String>>,
    nav_scroller: gtk4::ScrolledWindow,
    results_scroller: gtk4::ScrolledWindow,
    /// The sidebar row of each pane, by stack name.
    rows: Vec<(&'static str, gtk4::ListBoxRow)>,
    stack: gtk4::Stack,
    title: gtk4::Label,
    /// What a Helm page among the search hits opens: the Helm, on it.
    on_page: OnPage,
    /// Every row that edits the settings file, with its keys and its name,
    /// marked while the file changes it from the system's
    /// (`mark_changed`).
    marks: std::cell::RefCell<Vec<Mark>>,
    look: look_pane::LookPane,
    idle: idle_pane::IdlePane,
    bar: bar_pane::BarPane,
    input: input_pane::InputPane,
    alerts: alerts_pane::AlertsPane,
    launcher: launcher_pane::LauncherPane,
    displays: displays_pane::DisplaysPane,
    glass: glass_pane::GlassPane,
    system: system_pane::SystemPane,
    quality: quality_pane::QualityPane,
}

impl SettingsSection {
    pub fn new() -> std::rc::Rc<Self> {
        let look = look_pane::LookPane::new();
        let idle = idle_pane::IdlePane::new();
        let bar = bar_pane::BarPane::new();
        let input = input_pane::InputPane::new();
        let alerts = alerts_pane::AlertsPane::new();
        let launcher = launcher_pane::LauncherPane::new();
        let displays = displays_pane::DisplaysPane::new();
        let glass = glass_pane::GlassPane::new();
        let system = system_pane::SystemPane::new();
        let quality = quality_pane::QualityPane::new();

        let stack = crate::ui::page_stack(
            gtk4::StackTransitionType::Crossfade,
            crate::tokens::motion::EXPAND,
        );
        stack.set_vhomogeneous(false);
        stack.set_hexpand(true);
        stack.add_named(look.widget(), Some("look"));
        stack.add_named(glass.widget(), Some("glass"));
        stack.add_named(idle.widget(), Some("idle"));
        stack.add_named(bar.widget(), Some("bar"));
        stack.add_named(input.widget(), Some("input"));
        stack.add_named(alerts.widget(), Some("alerts"));
        stack.add_named(launcher.widget(), Some("launcher"));
        stack.add_named(displays.widget(), Some("displays"));
        stack.add_named(system.widget(), Some("system"));
        stack.add_named(quality.widget(), Some("quality"));

        // ── The sidebar: search over the panes' rows, then the panes ─────────
        let search = gtk4::SearchEntry::new();
        search.set_placeholder_text(Some("Search settings"));
        search.add_css_class("settings-search");

        let nav = crate::ui::list();
        nav.set_selection_mode(gtk4::SelectionMode::Single);
        let mut rows = Vec::new();
        let mut group = None;
        for pane in &PANES {
            if group != Some(pane.group) {
                group = Some(pane.group);
                nav.append(&group_header(pane.group));
            }
            let row = nav_row(pane);
            nav.append(&row);
            rows.push((pane.name, row));
        }
        let nav_scroller = sidebar_scroller(&nav);

        let results = crate::ui::list();
        results.set_selection_mode(gtk4::SelectionMode::Browse);
        results.add_css_class("settings-hits");
        let results_scroller = sidebar_scroller(&results);
        results_scroller.set_visible(false);

        let sidebar = crate::ui::vbox(3);
        sidebar.add_css_class("settings-sidebar");
        sidebar.append(&search);
        sidebar.append(&nav_scroller);
        sidebar.append(&results_scroller);

        // ── The pane: its title, and the pane at the surface's full height ──
        let title = crate::ui::text("", crate::ui::Text::Title, crate::ui::Tone::Fg);
        title.set_halign(gtk4::Align::Start);
        title.add_css_class("settings-title");
        title.set_xalign(0.0);
        // Horizontal External keeps a wide row from widening the card; the
        // pane scrolls on its own, whatever the sidebar does.
        let scroller = gtk4::ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .propagate_natural_width(false)
            .hexpand(true)
            .vexpand(true)
            .child(&stack)
            .build();
        let content = crate::ui::vbox(3);
        content.set_hexpand(true);
        content.append(&title);
        content.append(&scroller);

        let root = crate::ui::hbox(0);
        root.add_css_class("settings-pane");
        root.set_hexpand(true);
        root.set_vexpand(true);
        root.append(&sidebar);
        root.append(&crate::ui::separator(gtk4::Orientation::Vertical));
        root.append(&content);

        let this = std::rc::Rc::new(SettingsSection {
            root,
            search,
            nav,
            results,
            hits: std::cell::RefCell::default(),
            nav_scroller,
            results_scroller,
            rows,
            stack,
            title,
            on_page: OnPage::default(),
            marks: std::cell::RefCell::default(),
            look,
            idle,
            bar,
            input,
            alerts,
            launcher,
            displays,
            glass,
            system,
            quality,
        });
        this.wire();
        this.install_marks();
        this.show("look");
        this
    }

    /// Mark every row the search index gives keys (`search::Entry`) while
    /// the settings file changes it from the system's, and give it a
    /// right-click that puts the system's back, for that row alone. The
    /// pane's Reset does the same for all of it.
    fn install_marks(self: &std::rc::Rc<Self>) {
        let mut marks = Vec::new();
        for (tab, entries) in search::TABLES {
            let Some(page) = self.stack.child_by_name(tab) else {
                continue;
            };
            for e in entries
                .iter()
                .filter(|e| !e.keys.is_empty() && !e.title.is_empty())
            {
                let Some(row) = find_row_in(&page, e.group, e.title, false) else {
                    log::warn!("settings: no row {} › {} to mark", e.group, e.title);
                    continue;
                };
                let Some(label) = descendants(&row).into_iter().find_map(|w| {
                    w.has_css_class("settings-row-label")
                        .then(|| w.downcast::<gtk4::Label>().ok())
                        .flatten()
                }) else {
                    continue;
                };
                let keys = e.keys;
                let click = gtk4::GestureClick::new();
                click.set_button(gtk4::gdk::BUTTON_SECONDARY);
                let weak = std::rc::Rc::downgrade(self);
                click.connect_released(move |_, _, _, _| {
                    reset_keys(keys);
                    // The panes read the store when told to, not on every
                    // change; this one shows the value just put back.
                    if let Some(this) = weak.upgrade() {
                        this.refresh_pane(tab);
                    }
                });
                row.add_controller(click);
                marks.push(Mark { label, keys });
            }
        }
        *self.marks.borrow_mut() = marks;
        let weak = std::rc::Rc::downgrade(self);
        store::observe(move || {
            if let Some(this) = weak.upgrade() {
                this.mark_changed();
            }
        });
        self.mark_changed();
    }

    /// Mark each row the settings file changes from the system's: its name
    /// in the accent and the strong weight, and a tooltip saying so and how
    /// to put it back. The name and not a dot beside it, so a marked row
    /// keeps its place in the pane's column; the weight as well as the
    /// colour, so the mark reads without the colour (the accent at body
    /// size is under the text tokens' contrast floor, §3.2).
    fn mark_changed(&self) {
        let values = |s: store::Settings| serde_json::to_value(s.effective()).ok();
        let (Some(now), Some(system)) =
            (values(store::current()), values(store::Settings::default()))
        else {
            return;
        };
        let at = |v: &serde_json::Value, key: &str| {
            key.split_once('.')
                .and_then(|(section, field)| v.get(section)?.get(field).cloned())
        };
        for mark in self.marks.borrow().iter() {
            let changed = mark.keys.iter().any(|k| at(&now, k) != at(&system, k));
            crate::ui::set_tone(
                &mark.label,
                if changed {
                    crate::ui::Tone::Accent
                } else {
                    crate::ui::Tone::Fg
                },
            );
            crate::ui::set_weight(
                &mark.label,
                if changed {
                    crate::ui::Weight::Strong
                } else {
                    crate::ui::Weight::Regular
                },
            );
            mark.label.set_tooltip_text(changed.then_some(
                "Changed from the system's default. Right-click the row to put it back.",
            ));
        }
    }

    fn wire(self: &std::rc::Rc<Self>) {
        {
            let weak = std::rc::Rc::downgrade(self);
            self.nav.connect_row_selected(move |_, row| {
                let (Some(this), Some(row)) = (weak.upgrade(), row) else {
                    return;
                };
                if let Some((name, _)) = this.rows.iter().find(|(_, r)| r == row) {
                    this.stack.set_visible_child_name(name);
                    if let Some(pane) = PANES.iter().find(|p| p.name == *name) {
                        this.title.set_label(pane.title);
                    }
                }
            });
        }
        {
            let weak = std::rc::Rc::downgrade(self);
            self.search.connect_search_changed(move |entry| {
                if let Some(this) = weak.upgrade() {
                    this.search_for(entry.text().trim());
                }
            });
        }
        {
            // Enter takes the first hit; Down walks into the hits.
            let weak = std::rc::Rc::downgrade(self);
            self.search.connect_activate(move |_| {
                if let Some(this) = weak.upgrade()
                    && let Some(row) = this.results.row_at_index(0)
                {
                    this.open_hit(&row);
                }
            });
            let keys = gtk4::EventControllerKey::new();
            let weak = std::rc::Rc::downgrade(self);
            keys.connect_key_pressed(move |_, key, _, _| {
                let Some(this) = weak.upgrade() else {
                    return glib::Propagation::Proceed;
                };
                if key != gtk4::gdk::Key::Down {
                    return glib::Propagation::Proceed;
                }
                let list = if this.results_scroller.is_visible() {
                    &this.results
                } else {
                    &this.nav
                };
                match list.selected_row().or_else(|| first_selectable(list)) {
                    Some(row) => {
                        row.grab_focus();
                        glib::Propagation::Stop
                    }
                    None => glib::Propagation::Proceed,
                }
            });
            self.search.add_controller(keys);
        }
        {
            let weak = std::rc::Rc::downgrade(self);
            self.results.connect_row_activated(move |_, row| {
                if let Some(this) = weak.upgrade() {
                    this.open_hit(row);
                }
            });
        }
    }

    /// List the index's hits for `query` in place of the panes; the panes
    /// again when it is empty.
    fn search_for(&self, query: &str) {
        while let Some(child) = self.results.first_child() {
            self.results.remove(&child);
        }
        self.hits.borrow_mut().clear();
        let searching = !query.is_empty();
        self.nav_scroller.set_visible(!searching);
        self.results_scroller.set_visible(searching);
        if !searching {
            return;
        }
        let hits = search::find(query);
        if hits.is_empty() {
            let none = crate::ui::row("", "No setting matches", "Try another word");
            let row = gtk4::ListBoxRow::new();
            row.set_selectable(false);
            row.set_activatable(false);
            row.set_child(Some(&none.root));
            self.results.append(&row);
            return;
        }
        for hit in hits.iter().take(30) {
            // The pane first, as the sidebar names it; the row's path under
            // it.
            let (pane, rest) = hit
                .path
                .split_once(" › ")
                .unwrap_or((hit.path.as_str(), ""));
            let glyph = PANES
                .iter()
                .find(|p| p.title == pane)
                .map_or("", |p| p.glyph);
            let r = crate::ui::row(
                glyph,
                if rest.is_empty() { pane } else { rest },
                hit.subtitle,
            );
            crate::ui::glyph::adopt(&r.icon, crate::ui::Text::TitleSm, crate::ui::Tone::Muted);
            r.title.set_ellipsize(gtk4::pango::EllipsizeMode::End);
            r.title.set_tooltip_text(Some(&hit.path));
            let row = gtk4::ListBoxRow::new();
            row.set_child(Some(&r.root));
            self.results.append(&row);
            self.hits.borrow_mut().push(hit.target.id());
        }
        if let Some(first) = self.results.row_at_index(0) {
            self.results.select_row(Some(&first));
        }
    }

    /// Open a search hit: its row, lit, or the Helm page it names.
    fn open_hit(&self, row: &gtk4::ListBoxRow) {
        let Some(id) = usize::try_from(row.index())
            .ok()
            .and_then(|i| self.hits.borrow().get(i).cloned())
        else {
            return;
        };
        if id.starts_with(':') {
            if let Some(f) = self.on_page.borrow().as_ref() {
                f(&id);
            }
            return;
        }
        self.search.set_text("");
        self.reveal(&id);
    }

    /// What a Helm page among the search hits opens (`:wifi` and the rest).
    pub fn set_on_page(&self, f: impl Fn(&str) + 'static) {
        *self.on_page.borrow_mut() = Some(Box::new(f));
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    /// Type `query` into the search, for the render harness.
    pub fn search_text(&self, query: &str) {
        self.search.set_text(query);
    }

    /// Put the keyboard in the search field.
    pub fn focus_search(&self) {
        self.search.grab_focus();
    }

    /// Clear the search, and say whether there was one: Esc clears a search
    /// before it closes settings.
    pub fn clear_search(&self) -> bool {
        let had = !self.search.text().is_empty();
        if had {
            self.search.set_text("");
        }
        had
    }

    /// Open `what`.
    pub fn open(&self, what: &Open) {
        match what {
            Open::Last => {}
            Open::Pane(name) => self.show(name),
            Open::Row(id) => {
                self.reveal(id);
            }
        }
    }

    /// Switch to a pane by its stack name. Unknown names are ignored.
    pub fn show(&self, name: &str) {
        if let Some((_, row)) = self.rows.iter().find(|(n, _)| *n == name) {
            // Selecting the row drives the stack through its handler, so the
            // sidebar and the pane cannot disagree.
            self.nav.select_row(Some(row));
        } else {
            log::warn!("settings: no pane named {name}");
        }
    }

    /// Open the pane a search result names, bring its row into view and
    /// light it up for a moment (`search.rs`). False when `id` names no
    /// row of the index.
    pub fn reveal(&self, id: &str) -> bool {
        let Some(search::Target::Row(tab, group, title)) = search::Target::from_id(id) else {
            return false;
        };
        self.show(tab);
        let Some(page) = self.stack.child_by_name(tab) else {
            return false;
        };
        match find_row(&page, group, title) {
            Some(row) => {
                reveal_when_laid_out(&row);
                true
            }
            None => {
                log::warn!("settings: no row {group} › {title} on the {tab} pane");
                false
            }
        }
    }

    /// The Displays pane's apply, for the render harness.
    pub fn demo_displays(&self) {
        self.displays.demo_apply();
    }

    /// The Input pane's layout picker, open on `query`, for the render
    /// harness.
    pub fn demo_layout_pick(&self, query: &str) {
        self.input.demo_pick(query);
    }

    /// Re-read one pane from what it edits, by its stack name.
    fn refresh_pane(&self, name: &str) {
        match name {
            "look" => self.look.refresh(),
            "idle" => self.idle.refresh(),
            "bar" => self.bar.refresh(),
            "input" => self.input.refresh(),
            "alerts" => self.alerts.refresh(),
            "launcher" => self.launcher.refresh(),
            "displays" => self.displays.refresh(),
            "glass" => self.glass.refresh(),
            "system" => self.system.refresh(),
            "quality" => self.quality.refresh(),
            _ => {}
        }
    }

    /// Re-read every pane from what it edits. Settings refreshes every pane
    /// when it opens.
    pub fn refresh(&self) {
        self.look.refresh();
        self.idle.refresh();
        self.bar.refresh();
        self.input.refresh();
        self.alerts.refresh();
        self.launcher.refresh();
        self.displays.refresh();
        self.glass.refresh();
        self.system.refresh();
        self.quality.refresh();
    }
}

/// A row that edits the settings file: its name, which is marked while the
/// file changes it, and its keys.
struct Mark {
    label: gtk4::Label,
    keys: &'static [&'static str],
}

/// Put the system's value back for `keys`, in the settings file. A
/// section that is then the system's again leaves the file, as the pane's
/// Reset removes it: kept, it would hold the values of today's system
/// against a later change to the system's defaults.
fn reset_keys(keys: &[&str]) {
    let system = store::Settings::default();
    store::update(|s| {
        for key in keys {
            if let Some(value) = system.get(key)
                && let Err(e) = s.set(key, value)
            {
                log::warn!("settings: reset {key}: {e}");
            }
        }
        *s = without_system_sections(s, &system, keys);
    });
}

/// `user` without the sections of `keys` that say what the system says.
fn without_system_sections(
    user: &store::Settings,
    system: &store::Settings,
    keys: &[&str],
) -> store::Settings {
    let (Ok(mut file), Ok(effective)) = (
        serde_json::to_value(user),
        serde_json::to_value(system.effective()),
    ) else {
        return user.clone();
    };
    for section in keys
        .iter()
        .filter_map(|k| k.split_once('.').map(|(s, _)| s))
    {
        if file
            .get(section)
            .is_some_and(|v| Some(v) == effective.get(section))
            && let Some(slot) = file.get_mut(section)
        {
            *slot = serde_json::Value::Null;
        }
    }
    serde_json::from_value(file).unwrap_or_else(|_| user.clone())
}

/// A group's heading in the sidebar: not selectable, not a stop for the
/// keyboard.
fn group_header(group: Group) -> gtk4::ListBoxRow {
    let row = gtk4::ListBoxRow::new();
    row.set_selectable(false);
    row.set_activatable(false);
    row.set_focusable(false);
    let label = crate::ui::overline(group.title(), crate::ui::Tone::Muted);
    label.set_halign(gtk4::Align::Start);
    label.add_css_class("settings-nav-group");
    row.set_child(Some(&label));
    row
}

/// A pane's row in the sidebar: its glyph and its title.
fn nav_row(pane: &Pane) -> gtk4::ListBoxRow {
    let r = crate::ui::row(pane.glyph, pane.title, "");
    crate::ui::glyph::adopt(&r.icon, crate::ui::Text::TitleSm, crate::ui::Tone::Muted);
    let row = gtk4::ListBoxRow::new();
    row.set_child(Some(&r.root));
    row
}

fn sidebar_scroller(list: &gtk4::ListBox) -> gtk4::ScrolledWindow {
    gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .vexpand(true)
        .child(list)
        .build()
}

/// The first row of `list` the keyboard can land on.
fn first_selectable(list: &gtk4::ListBox) -> Option<gtk4::ListBoxRow> {
    let mut i = 0;
    while let Some(row) = list.row_at_index(i) {
        if row.is_selectable() {
            return Some(row);
        }
        i += 1;
    }
    None
}

/// Every widget under `root`, depth first, `root` included.
fn descendants(root: &gtk4::Widget) -> Vec<gtk4::Widget> {
    let mut out = vec![root.clone()];
    let mut i = 0;
    while i < out.len() {
        let mut child = out[i].first_child();
        while let Some(c) = child {
            child = c.next_sibling();
            out.push(c);
        }
        i += 1;
    }
    out
}

/// The row a search entry names on a pane: inside the group whose overline
/// is `group` (anywhere on the page when no group has that name, as on
/// Displays, whose group is named after the selected output), the row
/// whose gutter label is `title`. An empty `title` is the group itself.
fn find_row(page: &gtk4::Widget, group: &str, title: &str) -> Option<gtk4::Widget> {
    find_row_in(page, group, title, true)
}

/// [`find_row`], and whether a row may be looked for on the whole page
/// when no group has that name. The marks may not: two groups of a pane
/// have rows of the same name (Dim after by day and at night), and the
/// page's first would be marked for the other's key.
fn find_row_in(
    page: &gtk4::Widget,
    group: &str,
    title: &str,
    anywhere: bool,
) -> Option<gtk4::Widget> {
    let text_is = |w: &gtk4::Widget, text: &str| {
        w.downcast_ref::<gtk4::Label>()
            .is_some_and(|l| l.text().as_str() == text)
    };
    // An overline is written in capitals (`ui::overline`); the tables name
    // the group as it is written in the pane's code. Compared as written,
    // no group ever matched, and a row whose name two groups share was
    // found in whichever came first on the page.
    let group_box = descendants(page).into_iter().find(|w| {
        w.has_css_class("settings-group")
            && w.first_child().is_some_and(|o| {
                o.downcast_ref::<gtk4::Label>()
                    .is_some_and(|l| l.text().to_uppercase() == group.to_uppercase())
            })
    });
    if title.is_empty() {
        return group_box;
    }
    let scope = match group_box {
        Some(g) => g,
        None if anywhere => page.clone(),
        None => return None,
    };
    descendants(&scope)
        .into_iter()
        .find(|w| w.has_css_class("settings-row-label") && text_is(w, title))
        .and_then(|label| label.parent())
}

/// Scroll `row` into the middle of the settings sheet's scroller and light
/// it, once it has a place on screen. The tab and the deck page were only
/// just switched, so the row is laid out a frame or two from now: the tick
/// callback waits for it, for at most a second of frames.
fn reveal_when_laid_out(row: &gtk4::Widget) {
    let frames = std::cell::Cell::new(0u32);
    row.add_tick_callback(move |row, _| {
        frames.set(frames.get() + 1);
        let laid_out = row.is_mapped() && row.height() > 0;
        if !laid_out && frames.get() < 60 {
            return glib::ControlFlow::Continue;
        }
        if laid_out {
            scroll_to(row);
            crate::ui::highlight(row);
        }
        glib::ControlFlow::Break
    });
}

/// Centre `row` in the nearest scroller above it that scrolls vertically.
fn scroll_to(row: &gtk4::Widget) {
    let mut up = row.parent();
    while let Some(w) = up {
        if let Some(scroller) = w.downcast_ref::<gtk4::ScrolledWindow>()
            && scroller.vscrollbar_policy() != gtk4::PolicyType::Never
        {
            // The adjustment measures in the scrolled content's space, which
            // is the viewport's child when GTK wrapped one in.
            let content = scroller
                .child()
                .map(|c| match c.downcast_ref::<gtk4::Viewport>() {
                    Some(v) => v.child().unwrap_or(c.clone()),
                    None => c,
                });
            let Some(bounds) = content.and_then(|c| row.compute_bounds(&c)) else {
                return;
            };
            let adj = scroller.vadjustment();
            let middle =
                f64::from(bounds.y()) - (adj.page_size() - f64::from(bounds.height())) / 2.0;
            adj.set_value(middle.clamp(
                adj.lower(),
                (adj.upper() - adj.page_size()).max(adj.lower()),
            ));
            return;
        }
        up = w.parent();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_prefix_opens_exactly_one_tab() {
        let mut seen = std::collections::HashSet::new();
        for pane in &PANES {
            for prefix in pane.prefixes {
                assert_eq!(tab_for_prefix(prefix), Some(pane.name), "{prefix}");
                assert!(seen.insert(*prefix), "{prefix} is claimed twice");
            }
        }
        assert_eq!(tab_for_prefix(":set"), None);
        assert_eq!(tab_for_prefix(":wallpaper"), Some("look"));
        assert_eq!(prefixes().count(), PANES.len());
    }

    #[test]
    fn a_prefix_opens_its_pane_or_settings_as_it_was() {
        assert_eq!(Open::for_prefix(":glass"), Some(Open::Pane("glass")));
        assert_eq!(Open::for_prefix(":sleep"), Some(Open::Pane("idle")));
        assert_eq!(Open::for_prefix(":settings"), Some(Open::Last));
        assert_eq!(Open::for_prefix(":pref"), Some(Open::Last));
        assert_eq!(Open::for_prefix(":wifi"), None);
    }

    #[test]
    fn every_search_table_names_a_pane() {
        for (tab, _) in search::TABLES {
            assert!(PANES.iter().any(|p| p.name == *tab), "{tab}");
        }
        // Machine panes after settings panes, so the sidebar's groups are
        // each one run.
        let groups: Vec<Group> = PANES.iter().map(|p| p.group).collect();
        assert!(
            groups
                .windows(2)
                .all(|w| !(w[0] == Group::Machine && w[1] == Group::Settings))
        );
    }

    #[test]
    fn a_section_the_system_holds_again_leaves_the_file() {
        let system = store::Settings::default();
        let mut user = store::Settings::default();
        user.set("idle.lock_after_s", serde_json::json!(4242))
            .unwrap();
        user.set("bar.clock_24h", system.get("bar.clock_24h").unwrap())
            .unwrap();
        // Put back to the system's: the idle section leaves, bar was never
        // different and leaves too; one still different stays.
        let mut back = user.clone();
        back.set(
            "idle.lock_after_s",
            system.get("idle.lock_after_s").unwrap(),
        )
        .unwrap();
        let cleaned =
            without_system_sections(&back, &system, &["idle.lock_after_s", "bar.clock_24h"]);
        assert!(cleaned.is_default(), "{cleaned:?}");
        let kept = without_system_sections(&user, &system, &["idle.lock_after_s"]);
        assert_eq!(kept.get("idle.lock_after_s"), Some(serde_json::json!(4242)));
    }
}
