//! App launcher powered by the elephant search daemon.
//!
//! [`LauncherView`] is an embeddable widget (search entry + results list +
//! search/keyboard wiring) with no window of its own. Both the standalone
//! full-screen [`Launcher`] and the start-menu popup mount the same view.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4_layer_shell::Edge;

use crate::anim;
use crate::elephant::{self, SearchResult};
use crate::layer_shell::{self, LayerShellConfig};

const MAX_VISIBLE_RESULTS: usize = 10;
const DEBOUNCE_MS: u64 = 100;

/// How tall the results list stands on a screen with room for it.
const RESULTS_HEIGHT: i32 = 360;

/// Slack left above and below the selected row when the list scrolls to it,
/// so the neighbour in the direction of travel peeks into view and the
/// selection never sits flush against the edge of the viewport.
const SCROLL_MARGIN: f64 = 8.0;

/// What the standalone launcher card asks for on a screen with room for it.
const LAUNCHER_CARD_SIZE: CardSize = CardSize {
    width: 560,
    height: Some(520),
};

static LAUNCHER_CONFIG: LayerShellConfig = LayerShellConfig {
    namespace: "swaypplet-launcher",
    layer: gtk4_layer_shell::Layer::Overlay,
    exclusive: false,
    default_width: None,
    default_height: None,
    anchors: &[
        (Edge::Top, true),
        (Edge::Bottom, true),
        (Edge::Left, true),
        (Edge::Right, true),
    ],
    margins: &[],
    keyboard_mode: gtk4_layer_shell::KeyboardMode::Exclusive,
};

// Default providers matching the walker config
const DEFAULT_PROVIDERS: &[&str] = &[
    "desktopapplications",
    "calc",
    "runner",
    "windows",
    "clipboard",
    "providerlist",
    "menus",
    "websearch",
];

/// What runs after an activation, so a host popup can hide itself.
type OnActivate = Rc<RefCell<Option<Box<dyn Fn()>>>>;

struct LauncherState {
    results: Vec<SearchResult>,
    selected: usize,
    query_generation: u64,
    /// The pictures of the running-window rows, and their capture. The
    /// capture runs only while such rows are on screen (see `start_live`).
    live: RefCell<crate::jump::card::Live>,
    stream: RefCell<Option<crate::jump::live::Stream>>,
    /// The windows `stream` captures, sorted, so a rebuild that shows the
    /// same windows keeps the capture it has.
    stream_ids: RefCell<Vec<String>>,
    /// The last picture of each window shown, put on a rebuilt row at once.
    /// The compositor sends a frame only when a window has damage, so a
    /// row rebuilt on the next keystroke for an idle window would otherwise
    /// stay an empty grey box until that window next draws.
    textures: RefCell<std::collections::HashMap<String, gtk4::gdk::Texture>>,
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
/// after firing the activation, so a host popup can hide itself.
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
        let root = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(0)
            .build();
        root.add_css_class("launcher-view");

        let entry = gtk4::SearchEntry::builder()
            .placeholder_text("Search")
            .hexpand(true)
            .build();
        entry.add_css_class("launcher-entry");

        let results_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(0)
            .build();
        results_box.add_css_class("launcher-results");

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
                selected: 0,
                query_generation: 0,
                live: RefCell::default(),
                stream: RefCell::default(),
                stream_ids: RefCell::default(),
                textures: RefCell::default(),
            })),
            on_activate: Rc::new(RefCell::new(None)),
        };

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

    /// Register a callback invoked right after an item is activated (used by
    /// the start menu to hide itself).
    pub fn set_on_activate<F: Fn() + 'static>(&self, f: F) {
        *self.on_activate.borrow_mut() = Some(Box::new(f));
    }

    /// Reset to the empty-query state (cleared input + default app list).
    pub fn reset(&self) {
        self.entry.set_text("");
        {
            let mut s = self.state.borrow_mut();
            s.results.clear();
            s.selected = 0;
        }
        // Empty query shows the default desktop-application list.
        run_search(
            String::new(),
            bump_generation(&self.state),
            self.state.clone(),
            self.results_box.clone(),
            self.scroller.clone(),
            self.on_activate.clone(),
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
                let s = view_state.borrow();
                if let Some(item) = s.results.get(s.selected) {
                    let provider = item.provider.clone();
                    let identifier = item.identifier.clone();
                    let action = default_action(item);
                    let query = entry.text().to_string();
                    drop(s);
                    activate_async(provider, identifier, action, query);
                    if let Some(cb) = on_activate.borrow().as_ref() {
                        cb();
                    }
                }
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

            let results_box_c = results_box.clone();
            let scroller_c = scroller.clone();
            let state_c = state.clone();
            let on_activate_c = on_activate.clone();

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
    window: gtk4::Window,
    view: LauncherView,
    reveal: anim::Reveal,
}

impl Launcher {
    pub fn new(app: &gtk4::Application) -> Self {
        let window = layer_shell::create_layer_window(app, &LAUNCHER_CONFIG);
        window.add_css_class("launcher");

        let backdrop = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .halign(gtk4::Align::Fill)
            .valign(gtk4::Align::Fill)
            .hexpand(true)
            .vexpand(true)
            .build();
        backdrop.add_css_class("launcher-backdrop");

        let top_spacer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

        // Size requests come from install_monitor_fit below, which clamps
        // them to the output the launcher opens on.
        let container = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(0)
            .halign(gtk4::Align::Center)
            .build();
        container.add_css_class("glass-card");
        container.add_css_class("launcher-container");

        let view = LauncherView::new();
        container.append(view.widget());

        backdrop.append(&top_spacer);
        backdrop.append(&container);
        window.set_child(Some(&backdrop));

        install_monitor_fit(&window, &top_spacer, &container, LAUNCHER_CARD_SIZE, None);

        // Enter/exit transition (motion on glass, anim.rs): the container is
        // the pane, the launcher view the content. Pure crossfade.
        let reveal = anim::Reveal::new(&window, &container).content(view.widget());

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

        Launcher {
            window,
            view,
            reveal,
        }
    }

    pub fn toggle(&self) {
        if self.reveal.is_shown() && self.window.is_visible() {
            self.reveal.hide();
        } else {
            self.view.reset();
            self.reveal.show();
            self.view.focus_entry();
            // Harness hook: the nested session in dev/render.sh has no
            // keyboard, so `SWAYPPLET_LAUNCHER_QUERY` types a query on open.
            // Steps separated by `|` are typed 1.5 s apart, for what the
            // rows do while a query grows ("spo|spot").
            if let Ok(query) = std::env::var("SWAYPPLET_LAUNCHER_QUERY")
                && !query.is_empty()
            {
                for (i, step) in query.split('|').enumerate() {
                    let entry = self.view.entry().clone();
                    let step = step.to_string();
                    glib::timeout_add_local_once(
                        std::time::Duration::from_millis(1500 * i as u64),
                        move || entry.set_text(&step),
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

fn activate_async(provider: String, identifier: String, action: String, query: String) {
    if provider == WINDOW_PROVIDER {
        if let Some(con_id) = identifier.split_whitespace().next() {
            crate::sway_ipc::run_command(&format!("[con_id={con_id}] focus"));
        }
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

fn run_search(
    query: String,
    generation: u64,
    state: Rc<RefCell<LauncherState>>,
    results_box: gtk4::Box,
    scroller: gtk4::ScrolledWindow,
    on_activate: OnActivate,
) {
    // Empty query → default desktop-application list only.
    let providers: Vec<&str> = if query.is_empty() {
        vec!["desktopapplications"]
    } else {
        DEFAULT_PROVIDERS.to_vec()
    };

    let query_c = query.clone();
    crate::spawn::spawn_work(
        move || {
            let results = match elephant::query(&query_c, &providers, MAX_VISIBLE_RESULTS as i32) {
                Ok(results) => results,
                Err(e) => {
                    log::warn!("Elephant query failed: {}", e);
                    Vec::new()
                }
            };
            // An app already open offers its windows first.
            let mut running = if query_c.is_empty() {
                Vec::new()
            } else {
                running_windows(&query_c, &results)
            };
            running.extend(results);
            running
        },
        move |results| {
            // A newer query superseded this one while it ran — drop the results.
            if generation != state.borrow().query_generation {
                return;
            }
            {
                let mut s = state.borrow_mut();
                s.results = results;
                s.selected = 0;
            }
            rebuild_results_ui(&results_box, &state, &query, &on_activate);
            // A fresh result set selects its first row, so the list has to go
            // back to the top with it — otherwise a search run from halfway
            // down the previous results opens scrolled past the best match.
            scroller.vadjustment().set_value(0.0);
        },
    );
}

fn rebuild_results_ui(
    results_box: &gtk4::Box,
    state: &Rc<RefCell<LauncherState>>,
    query: &str,
    on_activate: &OnActivate,
) {
    clear_results_box(results_box);

    let s = state.borrow();
    let selected = s.selected;
    *s.live.borrow_mut() = crate::jump::card::Live::default();

    for (i, result) in s.results.iter().enumerate() {
        let row = if result.provider == WINDOW_PROVIDER {
            window_row(result, i == selected, &mut s.live.borrow_mut(), on_activate)
        } else {
            build_result_row(result, i == selected, query, on_activate)
        };
        results_box.append(&row);
    }
    drop(s);
    start_live(state);
}

/// Capture the windows the running-window rows show, or stop capturing
/// when there are none. The rows are rebuilt on every keystroke; while they
/// show the same windows, the capture carries on and each new row gets the
/// window's last picture straight away.
fn start_live(state: &Rc<RefCell<LauncherState>>) {
    let s = state.borrow();
    let mut ids = s.live.borrow().window_ids();
    ids.sort();
    {
        let live = s.live.borrow();
        let mut textures = s.textures.borrow_mut();
        textures.retain(|id, _| ids.contains(id));
        for (id, texture) in textures.iter() {
            live.show(id, texture);
        }
    }
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
            let id = frame.id.clone();
            let s = state.borrow();
            let texture = s.live.borrow().frame(frame);
            if let Some(texture) = texture {
                s.textures.borrow_mut().insert(id, texture);
            }
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

/// The windows already open for the best application match, as rows. Their
/// identifier is `<con_id> <foreign toplevel identifier>`: the first to go
/// there, the second to show it.
fn running_windows(query: &str, results: &[SearchResult]) -> Vec<SearchResult> {
    let app = results.iter().find(|r| r.provider == "desktopapplications");
    let names = app.map(app_names).unwrap_or_default();
    let words = title_words(query);
    if names.is_empty() && words.is_empty() {
        return Vec::new();
    }
    let Some(tree) = crate::sway_ipc::connect()
        .ok()
        .and_then(|mut c| c.get_tree().ok())
    else {
        return Vec::new();
    };
    let windows: Vec<_> = crate::jump::scene::all_windows(&tree)
        .into_iter()
        .filter(|(w, _, _)| w.id.is_some())
        .collect();
    // A window whose title has the query in it comes first: that is the one
    // being asked for by name. Then the windows of the app that matched.
    let by_title = windows
        .iter()
        .filter(|(_, _, title)| title_matches(&words, title));
    let by_app = windows.iter().filter(|(w, _, title)| {
        !title_matches(&words, title) && names.iter().any(|n| same_app(n, &w.app))
    });
    by_title
        .chain(by_app)
        .take(MAX_WINDOW_ROWS)
        .map(|(w, ws, title)| SearchResult {
            identifier: format!("{} {}", w.con_id, w.id.clone().unwrap_or_default()),
            text: format!("Go to {}", if title.is_empty() { &w.app } else { title }),
            subtext: format!(
                "{} \u{00b7} open on {}",
                app.filter(|a| app_names(a).iter().any(|n| same_app(n, &w.app)))
                    .map_or(w.app.as_str(), |a| a.text.as_str()),
                ws
            ),
            icon: String::new(),
            provider: WINDOW_PROVIDER.to_string(),
            score: 0,
            actions: Vec::new(),
        })
        .collect()
}

/// The words a window title has to contain, lowercased. None for a query
/// under two characters, which would match nearly every title.
fn title_words(query: &str) -> Vec<String> {
    let query = query.trim().to_lowercase();
    if query.chars().count() < 2 {
        return Vec::new();
    }
    query.split_whitespace().map(str::to_string).collect()
}

/// Whether every query word is in the title, in any case.
fn title_matches(words: &[String], title: &str) -> bool {
    if words.is_empty() {
        return false;
    }
    let title = title.to_lowercase();
    words.iter().all(|w| title.contains(w.as_str()))
}

/// What an application result could be called as a window's app_id: its
/// desktop entry and its icon name, lowercased, without `.desktop`.
fn app_names(result: &SearchResult) -> Vec<String> {
    let mut names = Vec::new();
    for raw in [&result.identifier, &result.icon] {
        let base = raw.rsplit('/').next().unwrap_or(raw);
        let name = base.trim_end_matches(".desktop").to_lowercase();
        if !name.is_empty() && !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

/// A desktop-entry name and an app_id name the same app: exactly, or by the
/// last part of a reverse-DNS id (`org.mozilla.firefox` is `firefox`).
fn same_app(name: &str, app_id: &str) -> bool {
    let id = app_id.to_lowercase();
    let tail = |s: &str| s.rsplit('.').next().unwrap_or(s).to_string();
    id == name || tail(&id) == tail(name)
}

/// A running-window row: the window live where the icon would be, "Go to"
/// its title, and where it is open.
fn window_row(
    result: &SearchResult,
    selected: bool,
    live: &mut crate::jump::card::Live,
    on_activate: &OnActivate,
) -> gtk4::Box {
    let row = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(12)
        .build();
    row.add_css_class("launcher-result");
    row.add_css_class("launcher-window");
    if selected {
        row.add_css_class("selected");
    }
    let picture = crate::jump::card::LivePicture::new();
    picture.set_size_request(WINDOW_THUMB_W, WINDOW_THUMB_H);
    picture.add_css_class("launcher-window-picture");
    if let Some(id) = result.identifier.split_whitespace().nth(1) {
        live.add(id.to_string(), picture.clone());
    }
    row.append(&picture);

    let text_box = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .valign(gtk4::Align::Center)
        .build();
    let name = gtk4::Label::builder()
        .label(&result.text)
        .halign(gtk4::Align::Start)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .css_classes(["launcher-result-name"])
        .build();
    text_box.append(&name);
    let sub = gtk4::Label::builder()
        .label(&result.subtext)
        .halign(gtk4::Align::Start)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .css_classes(["launcher-result-sub"])
        .build();
    text_box.append(&sub);
    row.append(&text_box);

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
        if let Some(cb) = on_activate.borrow().as_ref() {
            cb();
        }
    });
    row.add_controller(gesture);
    row
}

fn build_result_row(
    result: &SearchResult,
    selected: bool,
    query: &str,
    on_activate: &OnActivate,
) -> gtk4::Box {
    let row = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(12)
        .build();
    row.add_css_class("launcher-result");
    if selected {
        row.add_css_class("selected");
    }

    let icon_label = gtk4::Label::builder()
        .label(provider_icon(&result.provider))
        .halign(gtk4::Align::Center)
        .valign(gtk4::Align::Center)
        .build();
    icon_label.add_css_class("launcher-result-icon");

    let mut used_themed_icon = false;
    if !result.icon.is_empty() && !result.icon.contains('/') {
        if let Some(display) = gtk4::gdk::Display::default() {
            let theme = gtk4::IconTheme::for_display(&display);
            if theme.has_icon(&result.icon) {
                let image = gtk4::Image::builder()
                    .icon_name(&result.icon)
                    .pixel_size(24)
                    .build();
                image.add_css_class("launcher-result-icon-img");
                row.append(&image);
                used_themed_icon = true;
            }
        }
    }
    if !used_themed_icon {
        row.append(&icon_label);
    }

    let text_box = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .valign(gtk4::Align::Center)
        .build();

    let name_label = gtk4::Label::builder()
        .label(&result.text)
        .halign(gtk4::Align::Start)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .build();
    name_label.add_css_class("launcher-result-name");
    text_box.append(&name_label);

    if !result.subtext.is_empty() {
        let sub_label = gtk4::Label::builder()
            .label(&result.subtext)
            .halign(gtk4::Align::Start)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .build();
        sub_label.add_css_class("launcher-result-sub");
        text_box.append(&sub_label);
    }

    row.append(&text_box);

    // Only badge non-default providers (websearch, calc, …). The dominant
    // "desktopapplications" source is implied by the surface, so badging every
    // row with it is pure visual noise.
    if result.provider != "desktopapplications" {
        let badge = gtk4::Label::builder()
            .label(&result.provider)
            .halign(gtk4::Align::End)
            .valign(gtk4::Align::Center)
            .build();
        badge.add_css_class("launcher-result-badge");
        row.append(&badge);
    }

    // Click to activate.
    let gesture = gtk4::GestureClick::new();
    let provider = result.provider.clone();
    let identifier = result.identifier.clone();
    let action = default_action(result);
    let query_str = query.to_string();
    let on_activate = on_activate.clone();
    gesture.connect_released(move |_, _, _, _| {
        activate_async(
            provider.clone(),
            identifier.clone(),
            action.clone(),
            query_str.clone(),
        );
        if let Some(cb) = on_activate.borrow().as_ref() {
            cb();
        }
    });
    row.add_controller(gesture);

    row
}

/// Move the `selected` class from row `old` to row `new`, returning the row
/// that now carries it so the caller can scroll it into view.
fn update_selection(results_box: &gtk4::Box, old: usize, new: usize) -> Option<gtk4::Widget> {
    let mut selected = None;
    let mut child = results_box.first_child();
    let mut i = 0;
    while let Some(widget) = child {
        if i == old {
            widget.remove_css_class("selected");
        }
        if i == new {
            widget.add_css_class("selected");
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

/// Breathing room kept between the card and the screen edges when the card's
/// preferred size does not fit the output it opened on.
const CARD_SIDE_MARGIN: i32 = 16;
const CARD_BOTTOM_MARGIN: i32 = 24;

/// Marks a card that had to give up vertical density to fit its output.
/// data/style.css answers it by dropping the fixed minimum heights that would
/// otherwise hold the card taller than the screen it is on.
const COMPACT_CLASS: &str = "card-compact";

/// Offset used until the compositor tells us which output the surface landed
/// on. A quarter of 1080p, so the card is roughly in place on the first frame
/// of a display we have not measured yet.
const FALLBACK_TOP_OFFSET: i32 = 270;

/// The size a card asks for when the screen has room, and the sizes it is
/// willing to shrink to when the screen has not.
///
/// `height` is `None` for a card that sizes itself to its content, which is
/// then never given a height request at all. The panel is that case: its
/// sections decide how tall it is.
pub(crate) struct CardSize {
    pub width: i32,
    pub height: Option<i32>,
}

/// Everything the fit needs about one card: its preferred size, and how to
/// ask it to give up vertical density when the output is too short for it.
///
/// The density hook is a callback rather than a CSS class because the fit
/// measures the card immediately after asking it to thin out, and GTK
/// validates style lazily. A class added here does not reach `measure` until
/// a later frame, so the fit would go on placing a card of the height it used
/// to have. Widget properties change the measurement on the spot, so the
/// caller sets those and the fit stays the only thing that reads geometry.
struct Fit {
    size: CardSize,
    on_compact: Option<Rc<dyn Fn(bool)>>,
    /// The output size the card was last fitted to.
    ///
    /// Deciding whether the card has to thin out means asking it for full
    /// density and measuring, which queues a resize, which brings us straight
    /// back here through the `layout` hook. GDK cuts that off with "layout
    /// continuously requested, giving up after 4 tries" and the panel never
    /// appears. Repeating the fit for an output whose size has not changed
    /// cannot reach a different answer, so it is skipped.
    fitted_to: std::cell::Cell<Option<(i32, i32)>>,
}

impl Fit {
    fn set_compact(&self, compact: bool) {
        if let Some(on_compact) = &self.on_compact {
            on_compact(compact);
        }
    }
}

/// Fit `card` to the monitor the window is actually on: clamp its size
/// request to what the output can show, then size `top_spacer` so the card
/// sits at the optical foveal sweet spot, a quarter of the way down.
///
/// Every part of this fixes the same class of bug, a fixed number that
/// silently assumes one screen's geometry. The offset came from
/// `monitors().item(0)`, whatever GDK happened to enumerate first, so a
/// 2560x1440 primary handed a 360 px spacer to a card opening on a 1440x900
/// secondary and pushed its lower half past the bottom edge. The sizes came
/// from literals in the builders and from CSS minimums taller than the screen
/// the card had to fit, with no way for anyone to scroll or drag the card
/// back into view.
///
/// Everything is recomputed on map, whenever the surface enters another
/// output, whenever the compositor sends a new size, and whenever the
/// output's geometry changes.
///
/// The card is not re-fitted while it stays mapped and its content grows.
/// Doing that would mean queueing a resize from inside layout, and both cards
/// settle their content before they are shown, so map time is late enough.
pub(crate) fn install_monitor_fit(
    window: &gtk4::Window,
    top_spacer: &gtk4::Box,
    card: &impl IsA<gtk4::Widget>,
    size: CardSize,
    on_compact: Option<Rc<dyn Fn(bool)>>,
) {
    let card = card.as_ref().clone();
    let fit = Rc::new(Fit {
        size,
        on_compact,
        fitted_to: std::cell::Cell::new(None),
    });

    // The preferred size and offset stand in until an output is known, so the
    // card looks exactly as designed on a screen big enough to hold it and
    // never flashes at some placeholder size on the way there.
    card.set_width_request(fit.size.width);
    if let Some(height) = fit.size.height {
        card.set_height_request(height);
    }
    top_spacer.set_height_request(FALLBACK_TOP_OFFSET);

    // The geometry watch has to move with the surface: a mode change on an
    // output we already left must not resize this card.
    let tracked: Rc<RefCell<Option<(gtk4::gdk::Monitor, glib::SignalHandlerId)>>> =
        Rc::new(RefCell::new(None));

    // Wayland delivers wl_surface.enter after the surface is mapped, so the
    // map pass usually has no output to measure yet and the enter below is
    // what places the card. Both paths are wired because a remap onto the
    // same output emits no enter.
    window.connect_map({
        let top_spacer = top_spacer.clone();
        let card = card.clone();
        let fit = fit.clone();
        let tracked = tracked.clone();
        move |window| refresh_monitor_fit(window, &top_spacer, &card, &fit, &tracked)
    });

    window.connect_realize({
        let top_spacer = top_spacer.clone();
        let card = card.clone();
        let fit = fit.clone();
        let tracked = tracked.clone();
        move |window| {
            let Some(surface) = window.surface() else {
                return;
            };
            surface.connect_enter_monitor({
                let window = window.downgrade();
                let top_spacer = top_spacer.clone();
                let card = card.clone();
                let fit = fit.clone();
                let tracked = tracked.clone();
                move |_, _| {
                    if let Some(window) = window.upgrade() {
                        refresh_monitor_fit(&window, &top_spacer, &card, &fit, &tracked);
                    }
                }
            });

            // The compositor's configure is the only place the surface's real
            // height comes from, and it arrives after both the map and the
            // enter that precede it. Without this the fit runs against a
            // height that is either uninitialised or left over from the
            // output the surface just left. `Fit::fitted_to` is what keeps
            // this from becoming a resize loop.
            surface.connect_layout({
                let window = window.downgrade();
                let top_spacer = top_spacer.clone();
                let card = card.clone();
                let fit = fit.clone();
                let tracked = tracked.clone();
                move |_, _, _| {
                    if let Some(window) = window.upgrade() {
                        refresh_monitor_fit(&window, &top_spacer, &card, &fit, &tracked);
                    }
                }
            });
        }
    });
}

/// Resolve the current output, re-hang the geometry watch if it changed, then
/// re-fit. Does nothing while the surface has no output yet, leaving whatever
/// size and offset were set last.
fn refresh_monitor_fit(
    window: &gtk4::Window,
    top_spacer: &gtk4::Box,
    card: &gtk4::Widget,
    fit: &Rc<Fit>,
    tracked: &Rc<RefCell<Option<(gtk4::gdk::Monitor, glib::SignalHandlerId)>>>,
) {
    let Some(monitor) = surface_monitor(window) else {
        return;
    };

    let unchanged = tracked
        .borrow()
        .as_ref()
        .is_some_and(|(watched, _)| watched == &monitor);
    if !unchanged {
        if let Some((previous, handler)) = tracked.borrow_mut().take() {
            previous.disconnect(handler);
        }
        // Weak on the widgets, because the monitor outlives the window and a
        // strong capture would keep a closed launcher alive for the session.
        let handler = monitor.connect_geometry_notify({
            let window = window.downgrade();
            let top_spacer = top_spacer.downgrade();
            let card = card.downgrade();
            let fit = fit.clone();
            let tracked = Rc::downgrade(tracked);
            move |_| {
                if let (Some(window), Some(top_spacer), Some(card), Some(tracked)) = (
                    window.upgrade(),
                    top_spacer.upgrade(),
                    card.upgrade(),
                    tracked.upgrade(),
                ) {
                    refresh_monitor_fit(&window, &top_spacer, &card, &fit, &tracked);
                }
            }
        });
        *tracked.borrow_mut() = Some((monitor.clone(), handler));
    }

    fit_card(window, &monitor, top_spacer, card, fit);
}

/// The height the card actually has to play with.
///
/// The monitor is the wrong number on an output carrying a bar. The panel's
/// layer surface is anchored to all four edges, so the compositor hands it
/// the output minus the bar's exclusive zone, measured here at 61 logical px,
/// and a card fitted to the full monitor puts its bottom row behind the bar.
///
/// Only the surface knows the real figure, and it knows it late: before its
/// first configure it reports a height of 1, and just after it moves output
/// it still reports the size the previous output gave it.
///
/// So it is believed only when it is at least half the output, which no bar
/// eats into, and capped at the output, which no configure can exceed. A
/// height of 1 taken at face value would fit the card to nothing and squeeze
/// its rows below their minimum for a frame, which GTK complains about by the
/// screenful. Whatever this returns early is corrected by the `layout` hook
/// in [`install_monitor_fit`] as soon as the real size arrives.
fn usable_height(window: &gtk4::Window, monitor: &gtk4::gdk::Monitor) -> i32 {
    let screen = monitor.geometry().height();
    window
        .surface()
        .map(|surface| surface.height())
        .filter(|height| height * 2 >= screen)
        .map_or(screen, |height| height.min(screen))
}

/// The output this window is displayed on.
///
/// `monitor_at_surface` answers from the output the compositor sent
/// wl_surface.enter for, which is also the right answer when a layer surface
/// was pinned to an output explicitly. It is `None` until that enter arrives.
fn surface_monitor(window: &gtk4::Window) -> Option<gtk4::gdk::Monitor> {
    let surface = window.surface()?;
    gtk4::gdk::Display::default()?.monitor_at_surface(&surface)
}

/// Clamp the card's size request to the output, thin it out if it still does
/// not fit, then place it.
///
/// The order matters. Width is settled first because the card's minimum
/// height is a function of its width: rows wrap and grow taller as the card
/// narrows, and an offset computed against the wide measurement would put the
/// card back off the bottom of the screen.
fn fit_card(
    window: &gtk4::Window,
    monitor: &gtk4::gdk::Monitor,
    top_spacer: &gtk4::Box,
    card: &gtk4::Widget,
    fit: &Fit,
) {
    // Monitor geometry is in logical pixels, the same space size requests and
    // the spacer's height are in, so a scaled output needs no conversion.
    let screen_width = monitor.geometry().width();
    let screen_height = usable_height(window, monitor);

    if fit.fitted_to.get() == Some((screen_width, screen_height)) {
        return;
    }
    fit.fitted_to.set(Some((screen_width, screen_height)));

    // The clamp lowers the request, which is a floor, so it cannot pull a
    // card below the minimum width its own content demands: GTK allocates the
    // larger of the two. Rows that wrap are what let the content minimum fall
    // far enough for this to bite.
    let usable_width = (screen_width - 2 * CARD_SIDE_MARGIN).max(0);
    card.set_width_request(fit.size.width.min(usable_width));

    // Ask the card what it will really be given rather than assuming it got
    // the request: for a card whose content is wider than the clamp, the two
    // differ, and measuring a height for a width the card has already refused
    // is both an over-estimate and a GTK warning.
    let (width, _, _, _) = card.measure(gtk4::Orientation::Horizontal, -1);

    let room = (screen_height - CARD_BOTTOM_MARGIN).max(0);
    if let Some(height) = fit.size.height {
        // A fixed-height card flush against the top is the tallest it can
        // ever be here, so that is the ceiling. The offset below then decides
        // where in the remaining room it actually sits.
        card.set_height_request(height.min(room));
    }

    // A short output needs the card thinned before it is placed, because the
    // lists inside it hold a floor tall enough on its own to outgrow the whole
    // screen, and no offset can rescue a card that does not fit at any offset.
    //
    // Full density is asked for first every time, so a card that visits a
    // small output once does not stay thin for the rest of the session.
    fit.set_compact(false);
    card.remove_css_class(COMPACT_CLASS);
    let mut card_min = card.measure(gtk4::Orientation::Vertical, width).0;
    if card_min > room {
        fit.set_compact(true);
        // The class only carries padding and margin trims, which the callback
        // above cannot express. It lands a frame late, and that is harmless
        // here: everything it changes makes the card shorter, so measuring
        // without it errs towards leaving the card more room than it needs.
        card.add_css_class(COMPACT_CLASS);
        card_min = card.measure(gtk4::Orientation::Vertical, width).0;
    }

    // A card taller than three quarters of the screen has to start above the
    // sweet spot, or its bottom rows, the launcher's last results among them,
    // sit off-screen where nothing can reach them.
    let limit = (room - card_min).max(0);
    top_spacer.set_height_request((screen_height / 4).min(limit));
}

fn provider_icon(provider: &str) -> &'static str {
    match provider {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_title_matches_every_query_word_in_any_case() {
        let words = title_words("dreaded Board");
        assert!(title_matches(&words, "The dreaded board view · The Norban project - Google Chrome"));
        assert!(!title_matches(&words, "The dreaded list view"));
        assert!(title_matches(&title_words("youtube"), "(238) YouTube - Google Chrome"));
    }

    #[test]
    fn a_one_letter_query_matches_no_title() {
        assert!(!title_matches(&title_words("y"), "(238) YouTube - Google Chrome"));
        assert!(!title_matches(&title_words(" "), "anything"));
    }

    fn app(identifier: &str, icon: &str) -> SearchResult {
        SearchResult {
            identifier: identifier.into(),
            text: String::new(),
            subtext: String::new(),
            icon: icon.into(),
            provider: "desktopapplications".into(),
            score: 0,
            actions: Vec::new(),
        }
    }

    #[test]
    fn an_application_is_named_by_its_entry_and_its_icon() {
        let names = app_names(&app(
            "/usr/share/applications/org.mozilla.firefox.desktop",
            "firefox",
        ));
        assert_eq!(names, ["org.mozilla.firefox", "firefox"]);
        assert_eq!(
            app_names(&app("Alacritty.desktop", "Alacritty")),
            ["alacritty"]
        );
    }

    #[test]
    fn a_window_matches_by_app_id_or_its_reverse_dns_tail() {
        assert!(same_app("alacritty", "Alacritty"));
        assert!(same_app("org.mozilla.firefox", "firefox"));
        assert!(same_app("firefox", "org.mozilla.firefox"));
        assert!(!same_app("code", "claude"));
    }
}
