//! App launcher powered by the elephant search daemon.
//!
//! [`LauncherView`] is an embeddable widget (search entry + results list +
//! search/keyboard wiring) with no window of its own. Both the standalone
//! full-screen [`Launcher`] and the start-menu popup mount the same view.
//!
//! | module | holds |
//! |---|---|
//! | `sources.rs` | the query's prefix (`=`, `>`), which elephant providers the Launcher settings ask, the rows made locally (settings rows among them, from `settings::search`) |
//! | `calc.rs` | the `=` calculator |
//! | `frecency.rs` | what you launch, and the ranking it gives |
//! | `windows.rs` | open windows as rows, from sway's tree |
//!
//! A keystroke costs one frame, not one round trip. The local rows (a
//! calculator result, the command row, a page by name) are made on the key's
//! own frame, and the rows elephant gave for the query before are narrowed
//! to the ones that still match, so the list follows the typing at once.
//! Elephant's answer for the new query replaces them [`DEBOUNCE_MS`] later,
//! on a worker, ranked by frecency (a hash lookup per row).

mod calc;
pub mod frecency;
mod sources;
mod windows;

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;

use crate::services::elephant::{self, SearchResult};
use crate::shell::{Namespace, Surface};

pub use sources::Page;
use sources::Query;

const MAX_VISIBLE_RESULTS: usize = 10;

/// Coalesces a burst of typing into one elephant query. Short, because the
/// list no longer waits for it: the local pass has already redrawn.
const DEBOUNCE_MS: u64 = 40;

/// Frecent rows the empty query opens with, above elephant's app list.
const MAX_FRECENT_ROWS: usize = 6;

/// How tall the results list stands on a screen with room for it.
const RESULTS_HEIGHT: i32 = 360;

/// Slack left above and below the selected row when the list scrolls to it,
/// so the neighbour in the direction of travel peeks into view and the
/// selection never sits flush against the edge of the viewport.
const SCROLL_MARGIN: f64 = 8.0;

/// What the standalone launcher card asks for on a screen with room for it.
const LAUNCHER_CARD_SIZE: crate::shell::fit::CardSize = crate::shell::fit::CardSize {
    width: 560,
    height: Some(520),
};

/// What the host does when a row is activated.
#[derive(Default)]
struct Hooks {
    /// Runs after an activation, so a host popup can hide itself.
    done: Option<Box<dyn Fn()>>,
    /// Opens a settings row or a panel section a result names, given the
    /// row's identifier (`settings::search::Target::id`). Set by the host
    /// that can open them; without one the launcher lists no settings rows.
    setting: Option<OpenSetting>,
}

type OpenSetting = Box<dyn Fn(&str)>;

type OnActivate = Rc<RefCell<Hooks>>;

struct LauncherState {
    /// What is on screen: `local`, `remote`, then `tail`.
    results: Vec<SearchResult>,
    /// Rows made on the key's frame (`sources::local`), and the settings
    /// the query names.
    local: Vec<SearchResult>,
    /// Settings rows the query only might mean, below elephant's.
    tail: Vec<SearchResult>,
    /// Whether the host opens settings rows (`set_on_setting`).
    settings_host: bool,
    /// Rows elephant and the window search gave, for the last query they
    /// answered.
    remote: Vec<SearchResult>,
    /// The pages a query can open by name; empty in a host that cannot
    /// route a prefix.
    pages: Vec<Page>,
    /// The list shows one app's windows (Tab), not the query's results.
    tabbed: bool,
    selected: usize,
    query_generation: u64,
    /// The pictures of the running-window rows, and their capture. The
    /// capture runs only while such rows are on screen (see `start_live`).
    live: RefCell<crate::jump::card::Live>,
    stream: RefCell<Option<crate::jump::live::Stream>>,
    /// The windows `stream` captures, sorted, so a rebuild that shows the
    /// same windows keeps the capture it has.
    stream_ids: RefCell<Vec<String>>,
}

/// The provider of the rows this launcher adds itself: a window of the app
/// being searched for that is already open. Activating one goes there
/// instead of starting another.
const WINDOW_PROVIDER: &str = "swaypplet-window";
/// At most this many running-window rows, above the results.
const MAX_WINDOW_ROWS: usize = 3;

// ── Embeddable launcher view ────────────────────────────────────────────────

/// Search entry + scrolled results list, wired to elephant. Mountable inside
/// any container. Calls the registered `on_activate` callback (if any) right
/// after firing the activation, so a host popup can hide itself. Cloning
/// shares the same view.
#[derive(Clone)]
pub struct LauncherView {
    root: gtk4::Box,
    entry: gtk4::SearchEntry,
    results_box: gtk4::Box,
    /// The results list. Held because it is the one part of the view a short
    /// output is allowed to shrink (see `install_monitor_fit`).
    scroller: gtk4::ScrolledWindow,
    state: Rc<RefCell<LauncherState>>,
    on_activate: OnActivate,
}

impl LauncherView {
    pub fn new() -> Self {
        let root = crate::ui::vbox(0);
        root.add_css_class("launcher-view");

        let entry = gtk4::SearchEntry::builder()
            .placeholder_text("Search")
            .hexpand(true)
            .build();
        crate::ui::entry::adopt(&entry, crate::ui::FieldSize::Large);
        entry.add_css_class("launcher-entry");

        let results_box = crate::ui::vbox(0);

        let scroller = gtk4::ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .vexpand(true)
            .child(&results_box)
            .build();
        scroller.add_css_class("launcher-scroller");
        // The floor is a property rather than a CSS `min-height` because GTK
        // takes the larger of the two, so a rule here would override any host
        // that has to shrink the list to fit a short output.
        scroller.set_min_content_height(RESULTS_HEIGHT);

        root.append(&entry);
        root.append(&scroller);

        let view = LauncherView {
            root,
            entry,
            results_box,
            scroller,
            state: Rc::new(RefCell::new(LauncherState {
                results: Vec::new(),
                local: Vec::new(),
                tail: Vec::new(),
                settings_host: false,
                remote: Vec::new(),
                pages: Vec::new(),
                tabbed: false,
                selected: 0,
                query_generation: 0,
                live: RefCell::default(),
                stream: RefCell::default(),
                stream_ids: RefCell::default(),
            })),
            on_activate: Rc::default(),
        };

        frecency::load();
        view.wire_search();
        // Whatever hosts the view, a launcher off screen captures nothing.
        let weak = Rc::downgrade(&view.state);
        view.root.connect_unmap(move |_| {
            if let Some(state) = weak.upgrade() {
                let s = state.borrow();
                s.stream.replace(None);
                s.stream_ids.borrow_mut().clear();
            }
        });
        view
    }

    /// What Enter does: activate the selected row.
    pub fn activate_selected(&self) {
        activate_selected(
            &self.state,
            &self.results_box,
            &self.entry,
            &self.on_activate,
        );
    }

    pub fn entry(&self) -> &gtk4::SearchEntry {
        &self.entry
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    /// The results list, for a host that has to shrink it to fit its output.
    pub fn scroller(&self) -> &gtk4::ScrolledWindow {
        &self.scroller
    }

    /// The pages this host can open by prefix, for the launcher to offer by
    /// name. Activating one types its prefix into the entry, which the host
    /// routes on its own `search-changed`.
    pub fn set_pages(&self, pages: Vec<Page>) {
        self.state.borrow_mut().pages = pages;
    }

    /// Offer the settings and the panel sections by what they do, and open
    /// one through `f` when its row is activated. `f` gets the row's
    /// identifier, a `settings::search::Target` id.
    pub fn set_on_setting<F: Fn(&str) + 'static>(&self, f: F) {
        self.on_activate.borrow_mut().setting = Some(Box::new(f));
        self.state.borrow_mut().settings_host = true;
    }

    /// Register a callback invoked right after an item is activated (used by
    /// the start menu to hide itself).
    pub fn set_on_activate<F: Fn() + 'static>(&self, f: F) {
        self.on_activate.borrow_mut().done = Some(Box::new(f));
    }

    /// Reset to the empty-query state (cleared input + default app list).
    pub fn reset(&self) {
        self.entry.set_text("");
        {
            let mut s = self.state.borrow_mut();
            s.local.clear();
            s.tail.clear();
            // What you use most, drawn now from memory; elephant's app list
            // follows below it.
            s.remote = frecent_rows();
            s.tabbed = false;
            s.selected = 0;
        }
        compose(&self.state);
        rebuild_results_ui(
            &self.results_box,
            &self.state,
            &self.on_activate,
            &self.entry,
        );
        run_search(
            String::new(),
            bump_generation(&self.state),
            self.state.clone(),
            self.results_box.clone(),
            self.scroller.clone(),
            self.on_activate.clone(),
            self.entry.clone(),
        );
    }

    pub fn focus_entry(&self) {
        self.entry.grab_focus();
    }

    /// Attach a Capture-phase key controller to `widget` so Enter/Escape/arrows
    /// reach the launcher before the SearchEntry consumes them. `on_escape` is
    /// called when Escape is pressed (e.g. to hide the host popup).
    pub fn install_key_controller<E: Fn() + 'static>(
        &self,
        widget: &impl IsA<gtk4::Widget>,
        on_escape: E,
    ) {
        let key_controller = gtk4::EventControllerKey::new();
        key_controller.set_propagation_phase(gtk4::PropagationPhase::Capture);

        let view_state = self.state.clone();
        let results_box = self.results_box.clone();
        let scroller = self.scroller.clone();
        let entry = self.entry.clone();
        let on_activate = self.on_activate.clone();

        key_controller.connect_key_pressed(move |_, key, _, _| match key {
            gtk4::gdk::Key::Tab | gtk4::gdk::Key::ISO_Left_Tab => {
                toggle_windows_of(&view_state, &results_box, &scroller, &entry, &on_activate);
                glib::Propagation::Stop
            }
            gtk4::gdk::Key::Escape => {
                on_escape();
                glib::Propagation::Stop
            }
            gtk4::gdk::Key::Down => {
                move_selection_state(&view_state, &results_box, &scroller, 1);
                glib::Propagation::Stop
            }
            gtk4::gdk::Key::Up => {
                move_selection_state(&view_state, &results_box, &scroller, -1);
                glib::Propagation::Stop
            }
            gtk4::gdk::Key::Return | gtk4::gdk::Key::KP_Enter => {
                activate_selected(&view_state, &results_box, &entry, &on_activate);
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        });
        widget.add_controller(key_controller);
    }

    fn wire_search(&self) {
        let results_box = self.results_box.clone();
        let scroller = self.scroller.clone();
        let state = self.state.clone();
        let entry = self.entry.clone();
        let on_activate = self.on_activate.clone();

        let debounce_id: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));

        entry.connect_search_changed(move |entry| {
            let query = entry.text().to_string();

            if let Some(id) = debounce_id.borrow_mut().take() {
                crate::spawn::remove_source(id);
            }

            // This frame: the local rows, and the last answer narrowed to
            // what still matches.
            let started = std::time::Instant::now();
            local_pass(&state, &query);
            rebuild_results_ui(&results_box, &state, &on_activate, entry);
            scroller.vadjustment().set_value(0.0);
            log::debug!(
                "launcher: {:?} drawn locally in {:?}",
                query,
                started.elapsed()
            );

            let results_box_c = results_box.clone();
            let scroller_c = scroller.clone();
            let state_c = state.clone();
            let on_activate_c = on_activate.clone();
            let entry_c = entry.clone();

            let generation = bump_generation(&state_c);

            let debounce_id_c = debounce_id.clone();
            let id = glib::timeout_add_local_once(
                std::time::Duration::from_millis(DEBOUNCE_MS),
                move || {
                    *debounce_id_c.borrow_mut() = None;
                    run_search(
                        query,
                        generation,
                        state_c,
                        results_box_c,
                        scroller_c,
                        on_activate_c,
                        entry_c,
                    );
                },
            );
            *debounce_id.borrow_mut() = Some(id);
        });
    }
}

impl Default for LauncherView {
    fn default() -> Self {
        Self::new()
    }
}

// ── Standalone full-screen launcher window ──────────────────────────────────

pub struct Launcher {
    surface: Surface,
    view: LauncherView,
}

impl Launcher {
    pub fn new(app: &gtk4::Application) -> Self {
        let surface = Surface::builder(app, Namespace::Launcher)
            .fill()
            .keyboard(gtk4_layer_shell::KeyboardMode::Exclusive)
            .card(crate::ui::Card::Floating)
            .build();
        let window = surface.window().clone();

        let backdrop = surface.root();
        backdrop.set_hexpand(true);
        backdrop.set_vexpand(true);

        let top_spacer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        backdrop.prepend(&top_spacer);

        // Size requests come from install_monitor_fit below, which clamps
        // them to the output the launcher opens on.
        let container = surface.card();
        container.set_halign(gtk4::Align::Center);
        container.add_css_class("launcher-container");

        let view = LauncherView::new();
        container.append(view.widget());

        crate::shell::fit::install_monitor_fit(
            &window,
            &top_spacer,
            container,
            LAUNCHER_CARD_SIZE,
            None,
        );

        // Enter/exit transition (motion on glass, anim.rs): the container is
        // the pane, the launcher view the content. Pure crossfade.
        surface.set_content(view.widget());
        let reveal = surface.reveal().expect("the launcher has a card").clone();

        // Hide the window after a result is activated.
        {
            let reveal_c = reveal.clone();
            view.set_on_activate(move || reveal_c.hide());
        }

        // Esc / arrows / Enter handled on the window in capture phase.
        {
            let reveal_c = reveal.clone();
            view.install_key_controller(&window, move || reveal_c.hide());
        }

        // Backdrop click → dismiss.
        let gesture = gtk4::GestureClick::new();
        {
            let reveal_c = reveal.clone();
            gesture.connect_released(move |_, _, _, _| {
                reveal_c.hide();
            });
        }
        window.add_controller(gesture);

        Launcher { surface, view }
    }

    /// Offer the settings and the panel sections, and open one through `f`
    /// (the panel, which holds them) once this card has gone.
    pub fn set_on_setting(&self, f: impl Fn(&str) + 'static) {
        let surface = self.surface.clone();
        self.view.set_on_setting(move |id| {
            surface.hide();
            f(id);
        });
    }

    pub fn toggle(&self) {
        if self.surface.is_shown() && self.surface.window().is_visible() {
            self.surface.hide();
        } else {
            self.view.reset();
            self.surface.show();
            self.view.focus_entry();
            // Harness hook: the nested session in dev/render.sh has no
            // keyboard, so `SWAYPPLET_LAUNCHER_QUERY` types a query on open.
            // Steps separated by `|` are typed 1.5 s apart, for what the
            // rows do while a query grows ("spo|spot").
            if let Ok(query) = std::env::var("SWAYPPLET_LAUNCHER_QUERY")
                && !query.is_empty()
            {
                let steps: Vec<String> = query.split('|').map(str::to_string).collect();
                for (i, step) in steps.iter().enumerate() {
                    let entry = self.view.entry().clone();
                    let step = step.clone();
                    glib::timeout_add_local_once(
                        std::time::Duration::from_millis(1500 * i as u64),
                        move || entry.set_text(&step),
                    );
                }
                // `SWAYPPLET_LAUNCHER_ACTIVATE=1` then presses Enter, 1.5 s
                // after the last step: the launch hand-off, end to end.
                if std::env::var("SWAYPPLET_LAUNCHER_ACTIVATE").is_ok_and(|v| !v.is_empty()) {
                    let view = self.view.clone();
                    glib::timeout_add_local_once(
                        std::time::Duration::from_millis(1500 * steps.len() as u64),
                        move || view.activate_selected(),
                    );
                }
            }
        }
    }
}

// ── Shared helpers ──────────────────────────────────────────────────────────

fn bump_generation(state: &Rc<RefCell<LauncherState>>) -> u64 {
    let mut s = state.borrow_mut();
    s.query_generation += 1;
    s.query_generation
}

fn move_selection_state(
    state: &Rc<RefCell<LauncherState>>,
    results_box: &gtk4::Box,
    scroller: &gtk4::ScrolledWindow,
    delta: i32,
) {
    let mut s = state.borrow_mut();
    if s.results.is_empty() {
        return;
    }
    let old = s.selected;
    let len = s.results.len();
    let new = if delta < 0 {
        old.saturating_sub((-delta) as usize)
    } else {
        (old + delta as usize).min(len - 1)
    };
    if new != old {
        s.selected = new;
        drop(s);
        if let Some(row) = update_selection(results_box, old, new) {
            scroll_row_into_view(scroller, results_box, &row);
        }
    }
}

fn activate_selected(
    state: &Rc<RefCell<LauncherState>>,
    results_box: &gtk4::Box,
    entry: &gtk4::SearchEntry,
    on_activate: &OnActivate,
) {
    let s = state.borrow();
    let Some(item) = s.results.get(s.selected).cloned() else {
        return;
    };
    let selected = s.selected;
    drop(s);
    if let Some(row) = nth_child(results_box, selected) {
        hand_off_launch(&item.provider, &row);
    }
    activate(&item, entry, on_activate);
}

/// Run `item`, count it, and let the host hide, from Enter or a click.
fn activate(item: &SearchResult, entry: &gtk4::SearchEntry, on_activate: &OnActivate) {
    if item.provider == sources::PAGE {
        // The host routes the prefix and stays open on the page.
        entry.set_text(&item.identifier);
        entry.set_position(-1);
        return;
    }
    if item.provider == sources::SETTING {
        // The host opens it and stays open on it; nothing is remembered,
        // as for a page.
        if let Some(open) = on_activate.borrow().setting.as_ref() {
            open(&item.identifier);
        }
        return;
    }
    if launcher_settings().frecency {
        frecency::record(item);
    }
    let query = sources::parse(&entry.text()).text().to_string();
    activate_async(
        item.provider.clone(),
        item.identifier.clone(),
        default_action(item),
        query,
    );
    if let Some(cb) = on_activate.borrow().done.as_ref() {
        cb();
    }
}

fn launcher_settings() -> crate::settings::store::Launcher {
    crate::settings::store::with(|s| s.launcher())
}

/// The shell a `>` command runs in: `$SHELL`, else `sh`.
fn user_shell() -> String {
    std::env::var("SHELL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "sh".to_string())
}

/// The empty query's first rows: what you launch most, from memory, as far
/// as the settings still show its kind.
fn frecent_rows() -> Vec<SearchResult> {
    let l = launcher_settings();
    if !l.frecency {
        return Vec::new();
    }
    frecency::with(|f| f.top(MAX_FRECENT_ROWS * 2, frecency::now()))
        .unwrap_or_default()
        .into_iter()
        .filter(|r| sources::allows(&l, &r.provider))
        .take(MAX_FRECENT_ROWS)
        .collect()
}

/// `local`, `remote` and `tail` into `results`, the selection back on the
/// first.
fn compose(state: &Rc<RefCell<LauncherState>>) {
    let mut s = state.borrow_mut();
    let mut results = s.local.clone();
    results.extend(s.remote.iter().cloned());
    results.extend(s.tail.iter().cloned());
    s.results = results;
    s.selected = 0;
}

/// The keystroke's own frame: the local rows for `text`, and the previous
/// remote rows narrowed to those whose name still has every query word in
/// it. No IO, and nothing but string compares over a dozen rows.
fn local_pass(state: &Rc<RefCell<LauncherState>>, text: &str) {
    let l = launcher_settings();
    let q = sources::parse(text);
    {
        let mut s = state.borrow_mut();
        s.tabbed = false;
        s.local = sources::local(&l, q, &s.pages, &user_shell());
        (s.tail, s.local) = if s.settings_host {
            // A setting the query names goes above the page rows too: "sudo"
            // means the sudo row before the Idle & Lock tab its prefix opens.
            let (clear, other) = sources::settings(&l, q);
            let mut local = std::mem::take(&mut s.local);
            let at = local.iter().take_while(|r| r.provider == sources::CALC).count();
            local.splice(at..at, clear);
            (other, local)
        } else {
            (Vec::new(), std::mem::take(&mut s.local))
        };
        let words: Vec<String> = q
            .text()
            .to_lowercase()
            .split_whitespace()
            .map(str::to_string)
            .collect();
        s.remote.retain(|r| match q {
            Query::Plain(_) => {
                let hay = format!("{} {}", r.text, r.subtext).to_lowercase();
                words.iter().all(|w| hay.contains(w.as_str()))
            }
            // A prefix changes what is asked: nothing of the old answer
            // stands.
            _ => false,
        });
    }
    compose(state);
}

/// Tab: the selected app's open windows in place of the results, or back
/// to the results. On a row that is not an app with windows, nothing
/// happens but a shake of the row.
fn toggle_windows_of(
    state: &Rc<RefCell<LauncherState>>,
    results_box: &gtk4::Box,
    scroller: &gtk4::ScrolledWindow,
    entry: &gtk4::SearchEntry,
    on_activate: &OnActivate,
) {
    let (tabbed, item, selected) = {
        let s = state.borrow();
        (s.tabbed, s.results.get(s.selected).cloned(), s.selected)
    };
    if tabbed {
        // Back to the query's results, as a fresh search.
        let query = entry.text().to_string();
        local_pass(state, &query);
        rebuild_results_ui(results_box, state, on_activate, entry);
        run_search(
            query,
            bump_generation(state),
            state.clone(),
            results_box.clone(),
            scroller.clone(),
            on_activate.clone(),
            entry.clone(),
        );
        return;
    }
    let shake = || {
        if let Some(row) = nth_child(results_box, selected) {
            crate::ui::shake(&row);
        }
    };
    let Some(app) = item.filter(|r| r.provider == "desktopapplications") else {
        shake();
        return;
    };
    if !launcher_settings().windows {
        shake();
        return;
    }
    let generation = bump_generation(state);
    let (state, results_box, scroller, entry, on_activate) = (
        state.clone(),
        results_box.clone(),
        scroller.clone(),
        entry.clone(),
        on_activate.clone(),
    );
    crate::spawn::spawn_work(
        move || windows::windows_of_app(&app),
        move |rows| {
            if generation != state.borrow().query_generation {
                return;
            }
            if rows.is_empty() {
                if let Some(row) = nth_child(&results_box, selected) {
                    crate::ui::shake(&row);
                }
                return;
            }
            {
                let mut s = state.borrow_mut();
                s.local.clear();
                s.tail.clear();
                s.remote = rows;
                s.tabbed = true;
            }
            compose(&state);
            rebuild_results_ui(&results_box, &state, &on_activate, &entry);
            scroller.vadjustment().set_value(0.0);
        },
    );
}

/// An application about to start opens out of its row's icon (swayfx
/// `handoff open`). Only for applications: every other provider either opens
/// no window or, for the running-window rows, goes to one that exists.
fn hand_off_launch(provider: &str, row: &gtk4::Widget) {
    // Off unless the Look tab's "Launch zoom" is on.
    if !crate::settings::store::with(|s| s.look().launch_zoom) {
        return;
    }
    if provider == "desktopapplications"
        && let Some(icon) = row.first_child()
    {
        crate::handoff::open_from(&icon);
    }
}

/// The `n`th child of `parent`.
fn nth_child(parent: &impl IsA<gtk4::Widget>, n: usize) -> Option<gtk4::Widget> {
    let mut child = parent.first_child();
    for _ in 0..n {
        child = child?.next_sibling();
    }
    child
}

fn activate_async(provider: String, identifier: String, action: String, query: String) {
    if provider == sources::CALC {
        if let Some(display) = gtk4::gdk::Display::default() {
            display.clipboard().set_text(&identifier);
        }
        return;
    }
    if provider == sources::RUN {
        crate::sway::ipc::run_command(&sources::exec_line(&user_shell(), &identifier));
        return;
    }
    if provider == WINDOW_PROVIDER {
        if let Some(con_id) = identifier.split_whitespace().next() {
            crate::sway::ipc::run_command(&format!("[con_id={con_id}] focus"));
        }
        return;
    }
    // Harness hook: elephant runs in the live session and would start the
    // app there. `SWAYPPLET_LAUNCH_EXEC=<cmd>` starts `cmd` through the sway
    // this process talks to instead, after the same hand-off.
    if provider == "desktopapplications"
        && let Ok(cmd) = std::env::var("SWAYPPLET_LAUNCH_EXEC")
        && !cmd.is_empty()
    {
        crate::sway::ipc::run_command(&format!("exec {cmd}"));
        return;
    }
    std::thread::spawn(move || {
        if let Err(e) = elephant::activate(&provider, &identifier, &action, &query) {
            log::warn!("Elephant activate failed: {}", e);
        }
    });
}

fn clear_results_box(results_box: &gtk4::Box) {
    while let Some(child) = results_box.first_child() {
        results_box.remove(&child);
    }
}

#[allow(clippy::too_many_arguments)]
fn run_search(
    query: String,
    generation: u64,
    state: Rc<RefCell<LauncherState>>,
    results_box: gtk4::Box,
    scroller: gtk4::ScrolledWindow,
    on_activate: OnActivate,
    entry: gtk4::SearchEntry,
) {
    let l = launcher_settings();
    let q = sources::parse(&query);
    let providers = sources::providers(&l, q);
    let asked = q.text().to_string();
    let plain = matches!(q, Query::Plain(_));
    let windows = l.windows && plain;
    // Nothing remote to ask: the local pass already drew all there is.
    if providers.is_empty() && !windows {
        let mut s = state.borrow_mut();
        if !matches!(q, Query::Empty) {
            s.remote.clear();
        }
        drop(s);
        compose(&state);
        rebuild_results_ui(&results_box, &state, &on_activate, &entry);
        return;
    }
    crate::spawn::spawn_work(
        move || {
            let results = if providers.is_empty() {
                Vec::new()
            } else {
                // Twice what shows, so a frecent row just below the cut can
                // rise into it.
                match elephant::query(&asked, &providers, (MAX_VISIBLE_RESULTS * 2) as i32) {
                    Ok(results) => results,
                    Err(e) => {
                        log::warn!("Elephant query failed: {}", e);
                        Vec::new()
                    }
                }
            };
            // An app already open offers its windows first.
            let running = if windows {
                windows::running_windows(&asked, &results)
            } else {
                Vec::new()
            };
            (running, results)
        },
        move |(mut running, results)| {
            // A newer query superseded this one while it ran — drop the results.
            if generation != state.borrow().query_generation {
                return;
            }
            let l = launcher_settings();
            let now = frecency::now();
            let ranked = if l.frecency {
                frecency::with(|f| f.rank(results.clone(), now)).unwrap_or(results)
            } else {
                results
            };
            let mut remote = if matches!(q_kind(&query), Kind::Empty) {
                // The frecent rows first, then elephant's list without them.
                let frecent = frecent_rows();
                let seen: std::collections::HashSet<String> = frecent
                    .iter()
                    .map(|r| frecency::key(&r.provider, &r.identifier))
                    .collect();
                let mut rows = frecent;
                rows.extend(
                    ranked
                        .into_iter()
                        .filter(|r| !seen.contains(&frecency::key(&r.provider, &r.identifier))),
                );
                rows
            } else {
                ranked
            };
            if let Query::Command(cmd) = sources::parse(&query) {
                remote.retain(|r| !sources::duplicates_run_row(r, cmd));
            }
            remote.truncate(MAX_VISIBLE_RESULTS);
            running.extend(remote);
            {
                let mut s = state.borrow_mut();
                s.remote = running;
                s.tabbed = false;
            }
            compose(&state);
            rebuild_results_ui(&results_box, &state, &on_activate, &entry);
            log::debug!("launcher: {query:?} answered by elephant");
            // A fresh result set selects its first row, so the list has to go
            // back to the top with it — otherwise a search run from halfway
            // down the previous results opens scrolled past the best match.
            scroller.vadjustment().set_value(0.0);
        },
    );
}

/// `Query` borrows the text; the worker's callback needs only the kind.
enum Kind {
    Empty,
    Other,
}

fn q_kind(query: &str) -> Kind {
    match sources::parse(query) {
        Query::Empty => Kind::Empty,
        _ => Kind::Other,
    }
}

fn rebuild_results_ui(
    results_box: &gtk4::Box,
    state: &Rc<RefCell<LauncherState>>,
    on_activate: &OnActivate,
    entry: &gtk4::SearchEntry,
) {
    clear_results_box(results_box);

    let s = state.borrow();
    let selected = s.selected;
    *s.live.borrow_mut() = crate::jump::card::Live::default();

    for (i, result) in s.results.iter().enumerate() {
        let row = if result.provider == WINDOW_PROVIDER {
            window_row(result, i == selected, &mut s.live.borrow_mut(), on_activate)
        } else {
            build_result_row(result, i == selected, on_activate, entry)
        };
        results_box.append(&row);
    }
    drop(s);
    start_live(state);
}

/// Capture the windows the running-window rows show, or stop capturing
/// when there are none. The rows are rebuilt on every keystroke; while they
/// show the same windows, the capture carries on, and `Live::add` has
/// already given each new row the window's last picture.
fn start_live(state: &Rc<RefCell<LauncherState>>) {
    let s = state.borrow();
    let mut ids = s.live.borrow().window_ids();
    ids.sort();
    if s.stream.borrow().is_some() && *s.stream_ids.borrow() == ids {
        return;
    }
    s.stream.replace(None);
    *s.stream_ids.borrow_mut() = ids.clone();
    if ids.is_empty() {
        return;
    }
    let (tx, rx) = async_channel::unbounded::<crate::jump::live::Frame>();
    let weak = Rc::downgrade(state);
    glib::spawn_future_local(async move {
        while let Ok(frame) = rx.recv().await {
            let Some(state) = weak.upgrade() else { break };
            state.borrow().live.borrow().frame(frame);
        }
    });
    s.stream.replace(Some(crate::jump::live::Stream::start(
        ids,
        (WINDOW_THUMB_W * 2) as u32,
        20,
        tx,
    )));
}

/// A running-window row's picture, at most this box.
const WINDOW_THUMB_W: i32 = 112;
const WINDOW_THUMB_H: i32 = 70;

/// A running-window row: the window live where the icon would be, "Go to"
/// its title, and where it is open.
fn window_row(
    result: &SearchResult,
    selected: bool,
    live: &mut crate::jump::card::Live,
    on_activate: &OnActivate,
) -> gtk4::Box {
    let r = crate::ui::row("", &result.text, &result.subtext);
    let row = r.root;
    result_row_style(&row, selected);
    let picture = crate::jump::card::LivePicture::new();
    picture.set_size_request(WINDOW_THUMB_W, WINDOW_THUMB_H);
    crate::ui::thumb::adopt(&picture);
    if let Some(id) = result.identifier.split_whitespace().nth(1) {
        live.add(id.to_string(), picture.clone());
    }
    // The window live where the icon would be.
    r.icon.set_visible(false);
    row.prepend(&picture);

    let gesture = gtk4::GestureClick::new();
    let identifier = result.identifier.clone();
    let on_activate = on_activate.clone();
    gesture.connect_released(move |_, _, _, _| {
        activate_async(
            WINDOW_PROVIDER.to_string(),
            identifier.clone(),
            String::new(),
            String::new(),
        );
        if let Some(cb) = on_activate.borrow().done.as_ref() {
            cb();
        }
    });
    row.add_controller(gesture);
    row
}

fn build_result_row(
    result: &SearchResult,
    selected: bool,
    on_activate: &OnActivate,
    entry: &gtk4::SearchEntry,
) -> gtk4::Box {
    let r = crate::ui::row(
        provider_icon(&result.provider),
        &result.text,
        &result.subtext,
    );
    let row = r.root.clone();
    result_row_style(&row, selected);
    crate::ui::glyph::adopt(&r.icon, crate::ui::Text::Title, crate::ui::Tone::Muted);

    // The app's own icon in the glyph's place, when the theme has it.
    if !result.icon.is_empty()
        && !result.icon.contains('/')
        && let Some(display) = gtk4::gdk::Display::default()
    {
        let theme = gtk4::IconTheme::for_display(&display);
        if theme.has_icon(&result.icon) {
            let image = gtk4::Image::builder()
                .icon_name(&result.icon)
                .pixel_size(24)
                .build();
            r.set_icon_image(&image);
        }
    }

    // Only badge non-default providers (websearch, calc, …). The dominant
    // "desktopapplications" source is implied by the surface, so badging every
    // row with it is pure visual noise.
    if result.provider != "desktopapplications" {
        let badge = crate::ui::text(
            provider_label(&result.provider),
            crate::ui::Text::Caption,
            crate::ui::Tone::Faint,
        );
        r.end.append(&badge);
    }

    // Click to activate.
    let gesture = gtk4::GestureClick::new();
    let item = result.clone();
    let entry = entry.clone();
    let on_activate = on_activate.clone();
    let row_weak = row.downgrade();
    gesture.connect_released(move |_, _, _, _| {
        if let Some(row) = row_weak.upgrade() {
            hand_off_launch(&item.provider, row.upcast_ref());
        }
        activate(&item, &entry, &on_activate);
    });
    row.add_controller(gesture);

    row
}

/// A result row's look: the row component, its selection moving in one frame
/// (`ui::set_instant`: the card's fill is the compositor's key, and a fade from
/// the selected fill back to it spends most of its frames as a dark ghost of
/// the old row).
fn result_row_style(row: &gtk4::Box, selected: bool) {
    crate::ui::set_instant(row, true);
    crate::ui::set_selected(row, selected);
}

/// Move the `selected` class from row `old` to row `new`, returning the row
/// that now carries it so the caller can scroll it into view.
fn update_selection(results_box: &gtk4::Box, old: usize, new: usize) -> Option<gtk4::Widget> {
    let mut selected = None;
    let mut child = results_box.first_child();
    let mut i = 0;
    while let Some(widget) = child {
        if i == old {
            crate::ui::set_selected(&widget, false);
        }
        if i == new {
            crate::ui::set_selected(&widget, true);
            selected = Some(widget.clone());
        }
        child = widget.next_sibling();
        i += 1;
    }
    selected
}

/// Scroll the results list the least amount that puts `row` on screen.
///
/// The rows are plain boxes rather than focusable list rows, and the
/// selection is a CSS class rather than keyboard focus — focus stays in the
/// search entry so typing keeps working while the arrows move through the
/// results. That is the design, but it means GTK's own scroll-to-focus never
/// fires and the viewport used to sit still while the selection walked off
/// the bottom of it.
///
/// `clamp_page` is the same adjustment call GTK's focus handling makes: it
/// scrolls only far enough to bring the range into the page, and does nothing
/// at all when the row is already visible.
fn scroll_row_into_view(
    scroller: &gtk4::ScrolledWindow,
    results_box: &gtk4::Box,
    row: &gtk4::Widget,
) {
    // Bounds in the coordinate space of the scroller's child, which is the
    // space the vertical adjustment measures in.
    let Some(bounds) = row.compute_bounds(results_box) else {
        return;
    };
    let vadj = scroller.vadjustment();
    let top = f64::from(bounds.y()) - SCROLL_MARGIN;
    let bottom = f64::from(bounds.y() + bounds.height()) + SCROLL_MARGIN;
    vadj.clamp_page(top.max(vadj.lower()), bottom.min(vadj.upper()));
}

/// Default action for a result — the first elephant action, or "start".
fn default_action(result: &SearchResult) -> String {
    result
        .actions
        .first()
        .cloned()
        .unwrap_or_else(|| "start".to_string())
}

/// The badge a row of `provider` carries.
fn provider_label(provider: &str) -> &str {
    match provider {
        sources::CALC => "calc",
        sources::RUN => "run",
        sources::PAGE => "open",
        sources::SETTING => "setting",
        other => other,
    }
}

fn provider_icon(provider: &str) -> &'static str {
    match provider {
        sources::CALC => "󰃬",
        sources::RUN => "",
        sources::PAGE | sources::SETTING => "󰒓",
        "desktopapplications" => "󰀻",
        "runner" => "",
        "windows" => "󰖯",
        "clipboard" => "󰅌",
        "calc" | "calculator" => "󰃬",
        "websearch" => "󰖟",
        "files" => "󰈔",
        "menus" => "󰍜",
        "bookmarks" => "󰃃",
        _ => "󰍉",
    }
}
