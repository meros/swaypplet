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

struct LauncherState {
    results: Vec<SearchResult>,
    selected: usize,
    query_generation: u64,
}

// ── Embeddable launcher view ────────────────────────────────────────────────

/// Search entry + scrolled results list, wired to elephant. Mountable inside
/// any container. Calls the registered `on_activate` callback (if any) right
/// after firing the activation, so a host popup can hide itself.
pub struct LauncherView {
    root: gtk4::Box,
    entry: gtk4::SearchEntry,
    results_box: gtk4::Box,
    state: Rc<RefCell<LauncherState>>,
    on_activate: Rc<RefCell<Option<Box<dyn Fn()>>>>,
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

        root.append(&entry);
        root.append(&scroller);

        let view = LauncherView {
            root,
            entry,
            results_box,
            state: Rc::new(RefCell::new(LauncherState {
                results: Vec::new(),
                selected: 0,
                query_generation: 0,
            })),
            on_activate: Rc::new(RefCell::new(None)),
        };

        view.wire_search();
        view
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.root
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
        let entry = self.entry.clone();
        let on_activate = self.on_activate.clone();

        key_controller.connect_key_pressed(move |_, key, _, _| match key {
            gtk4::gdk::Key::Escape => {
                on_escape();
                glib::Propagation::Stop
            }
            gtk4::gdk::Key::Down => {
                move_selection_state(&view_state, &results_box, 1);
                glib::Propagation::Stop
            }
            gtk4::gdk::Key::Up => {
                move_selection_state(&view_state, &results_box, -1);
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
        let state = self.state.clone();
        let entry = self.entry.clone();
        let on_activate = self.on_activate.clone();

        let debounce_id: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));

        entry.connect_search_changed(move |entry| {
            let query = entry.text().to_string();

            if let Some(id) = debounce_id.borrow_mut().take() {
                id.remove();
            }

            let results_box_c = results_box.clone();
            let state_c = state.clone();
            let on_activate_c = on_activate.clone();

            let generation = bump_generation(&state_c);

            let debounce_id_c = debounce_id.clone();
            let id = glib::timeout_add_local_once(
                std::time::Duration::from_millis(DEBOUNCE_MS),
                move || {
                    *debounce_id_c.borrow_mut() = None;
                    run_search(query, generation, state_c, results_box_c, on_activate_c);
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

        let top_offset = monitor_top_offset();
        let top_spacer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        top_spacer.set_height_request(top_offset);

        let container = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(0)
            .halign(gtk4::Align::Center)
            .width_request(560)
            .height_request(520)
            .build();
        container.add_css_class("glass-card");
        container.add_css_class("launcher-container");

        let view = LauncherView::new();
        container.append(view.widget());

        backdrop.append(&top_spacer);
        backdrop.append(&container);
        window.set_child(Some(&backdrop));

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
        }
    }
}

// ── Shared helpers ──────────────────────────────────────────────────────────

fn bump_generation(state: &Rc<RefCell<LauncherState>>) -> u64 {
    let mut s = state.borrow_mut();
    s.query_generation += 1;
    s.query_generation
}

fn move_selection_state(state: &Rc<RefCell<LauncherState>>, results_box: &gtk4::Box, delta: i32) {
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
        update_selection(results_box, old, new);
    }
}

fn activate_async(provider: String, identifier: String, action: String, query: String) {
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
    on_activate: Rc<RefCell<Option<Box<dyn Fn()>>>>,
) {
    // Empty query → default desktop-application list only.
    let providers: Vec<&str> = if query.is_empty() {
        vec!["desktopapplications"]
    } else {
        DEFAULT_PROVIDERS.to_vec()
    };

    let query_c = query.clone();
    crate::spawn::spawn_work(
        move || match elephant::query(&query_c, &providers, MAX_VISIBLE_RESULTS as i32) {
            Ok(results) => results,
            Err(e) => {
                log::warn!("Elephant query failed: {}", e);
                Vec::new()
            }
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
        },
    );
}

fn rebuild_results_ui(
    results_box: &gtk4::Box,
    state: &Rc<RefCell<LauncherState>>,
    query: &str,
    on_activate: &Rc<RefCell<Option<Box<dyn Fn()>>>>,
) {
    clear_results_box(results_box);

    let s = state.borrow();
    let selected = s.selected;

    for (i, result) in s.results.iter().enumerate() {
        let row = build_result_row(result, i == selected, query, on_activate);
        results_box.append(&row);
    }
}

fn build_result_row(
    result: &SearchResult,
    selected: bool,
    query: &str,
    on_activate: &Rc<RefCell<Option<Box<dyn Fn()>>>>,
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

fn update_selection(results_box: &gtk4::Box, old: usize, new: usize) {
    let mut child = results_box.first_child();
    let mut i = 0;
    while let Some(widget) = child {
        if i == old {
            widget.remove_css_class("selected");
        }
        if i == new {
            widget.add_css_class("selected");
        }
        child = widget.next_sibling();
        i += 1;
    }
}

/// Default action for a result — the first elephant action, or "start".
fn default_action(result: &SearchResult) -> String {
    result
        .actions
        .first()
        .cloned()
        .unwrap_or_else(|| "start".to_string())
}

/// Top offset ~25% of the primary monitor height (Spotlight-style).
fn monitor_top_offset() -> i32 {
    if let Some(display) = gtk4::gdk::Display::default() {
        let monitors = display.monitors();
        if let Some(obj) = monitors.item(0) {
            if let Ok(monitor) = obj.downcast::<gtk4::gdk::Monitor>() {
                let height = monitor.geometry().height();
                return height / 4;
            }
        }
    }
    270
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
