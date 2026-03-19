//! Full-screen launcher powered by the elephant search daemon.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use gtk4::prelude::*;
use gtk4_layer_shell::Edge;

use crate::elephant::{self, SearchResult};
use crate::layer_shell::{self, LayerShellConfig};

const MAX_VISIBLE_RESULTS: usize = 10;
const DEBOUNCE_MS: u64 = 100;

static LAUNCHER_CONFIG: LayerShellConfig = LayerShellConfig {
    namespace: "swaypplet-launcher",
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

pub struct Launcher {
    window: gtk4::Window,
    entry: gtk4::SearchEntry,
    results_box: gtk4::Box,
    state: Rc<RefCell<LauncherState>>,
}

impl Launcher {
    pub fn new(app: &gtk4::Application) -> Self {
        let window = layer_shell::create_layer_window(app, &LAUNCHER_CONFIG);
        window.add_css_class("launcher");

        // Semi-transparent backdrop — fills entire screen
        let backdrop = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .halign(gtk4::Align::Fill)
            .valign(gtk4::Align::Fill)
            .hexpand(true)
            .vexpand(true)
            .build();
        backdrop.add_css_class("launcher-backdrop");

        // Top spacer — positions content at ~25% from top (Spotlight-style)
        let top_offset = monitor_top_offset();
        let top_spacer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        top_spacer.set_height_request(top_offset);

        // Content container — centered horizontally
        let container = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(0)
            .halign(gtk4::Align::Center)
            .build();
        container.add_css_class("launcher-container");

        // Search entry
        let entry = gtk4::SearchEntry::builder()
            .placeholder_text("Search")
            .hexpand(true)
            .build();
        entry.add_css_class("launcher-entry");

        // Results list
        let results_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(0)
            .build();
        results_box.add_css_class("launcher-results");

        container.append(&entry);
        container.append(&results_box);
        backdrop.append(&top_spacer);
        backdrop.append(&container);
        window.set_child(Some(&backdrop));

        let state = Rc::new(RefCell::new(LauncherState {
            results: Vec::new(),
            selected: 0,
            query_generation: 0,
        }));

        let launcher = Launcher {
            window,
            entry,
            results_box,
            state,
        };

        launcher.wire_search();
        launcher.wire_keyboard();
        launcher.wire_backdrop_click();

        launcher
    }

    pub fn toggle(&self) {
        if self.window.is_visible() {
            self.hide();
        } else {
            self.show();
        }
    }

    pub fn show(&self) {
        self.entry.set_text("");
        {
            let mut s = self.state.borrow_mut();
            s.results.clear();
            s.selected = 0;
        }
        self.clear_results_ui();
        self.window.set_visible(true);
        self.entry.grab_focus();
    }

    pub fn hide(&self) {
        self.window.set_visible(false);
    }

    fn wire_search(&self) {
        let results_box = self.results_box.clone();
        let state = self.state.clone();
        let entry = self.entry.clone();

        // Debounced search
        let debounce_id: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));

        entry.connect_search_changed(move |entry| {
            let query = entry.text().to_string();

            // Cancel previous debounce timer
            if let Some(id) = debounce_id.borrow_mut().take() {
                id.remove();
            }

            let results_box_c = results_box.clone();
            let state_c = state.clone();

            // Bump generation to discard stale results
            {
                let mut s = state_c.borrow_mut();
                s.query_generation += 1;
            }
            let generation = state_c.borrow().query_generation;

            if query.is_empty() {
                // Clear results immediately
                let mut s = state_c.borrow_mut();
                s.results.clear();
                s.selected = 0;
                clear_results_box(&results_box_c);
                return;
            }

            let debounce_id_c = debounce_id.clone();
            let id = glib::timeout_add_local_once(
                std::time::Duration::from_millis(DEBOUNCE_MS),
                move || {
                    *debounce_id_c.borrow_mut() = None;
                    run_search(query, generation, state_c, results_box_c);
                },
            );
            *debounce_id.borrow_mut() = Some(id);
        });
    }

    fn wire_keyboard(&self) {
        let key_controller = gtk4::EventControllerKey::new();
        // Capture phase: intercept Enter/Escape/arrows before SearchEntry consumes them
        key_controller.set_propagation_phase(gtk4::PropagationPhase::Capture);
        let state = self.state.clone();
        let results_box = self.results_box.clone();
        let window = self.window.clone();
        let entry = self.entry.clone();

        key_controller.connect_key_pressed(move |_, key, _, _| {
            match key {
                gtk4::gdk::Key::Escape => {
                    window.set_visible(false);
                    glib::Propagation::Stop
                }
                gtk4::gdk::Key::Down => {
                    let mut s = state.borrow_mut();
                    if !s.results.is_empty() && s.selected < s.results.len() - 1 {
                        let old = s.selected;
                        s.selected += 1;
                        let new = s.selected;
                        drop(s);
                        update_selection(&results_box, old, new);
                    }
                    glib::Propagation::Stop
                }
                gtk4::gdk::Key::Up => {
                    let mut s = state.borrow_mut();
                    if s.selected > 0 {
                        let old = s.selected;
                        s.selected -= 1;
                        let new = s.selected;
                        drop(s);
                        update_selection(&results_box, old, new);
                    }
                    glib::Propagation::Stop
                }
                gtk4::gdk::Key::Return | gtk4::gdk::Key::KP_Enter => {
                    let s = state.borrow();
                    if let Some(item) = s.results.get(s.selected) {
                        let provider = item.provider.clone();
                        let identifier = item.identifier.clone();
                        let action = default_action(item);
                        let query = entry.text().to_string();
                        drop(s);

                        window.set_visible(false);

                        std::thread::spawn(move || {
                            if let Err(e) = elephant::activate(&provider, &identifier, &action, &query) {
                                log::warn!("Elephant activate failed: {}", e);
                            }
                        });
                    }
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        self.window.add_controller(key_controller);
    }

    fn wire_backdrop_click(&self) {
        let gesture = gtk4::GestureClick::new();
        let window = self.window.clone();
        gesture.connect_released(move |_, _, _, _| {
            window.set_visible(false);
        });
        self.window.add_controller(gesture);
    }

    fn clear_results_ui(&self) {
        clear_results_box(&self.results_box);
    }
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
) {
    let result_holder: Arc<Mutex<Option<Vec<SearchResult>>>> = Arc::new(Mutex::new(None));
    let result_writer = result_holder.clone();

    let query_c = query.clone();
    std::thread::spawn(move || {
        match elephant::query(&query_c, DEFAULT_PROVIDERS, MAX_VISIBLE_RESULTS as i32) {
            Ok(results) => {
                *result_writer.lock().unwrap() = Some(results);
            }
            Err(e) => {
                log::warn!("Elephant query failed: {}", e);
                *result_writer.lock().unwrap() = Some(Vec::new());
            }
        }
    });

    // Poll for results
    glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
        let done = result_holder.lock().unwrap().is_some();
        if !done {
            return glib::ControlFlow::Continue;
        }

        // Check if this is still the current query generation
        let current_gen = state.borrow().query_generation;
        if generation != current_gen {
            return glib::ControlFlow::Break;
        }

        let results = result_holder.lock().unwrap().take().unwrap();

        {
            let mut s = state.borrow_mut();
            s.results = results;
            s.selected = 0;
        }

        rebuild_results_ui(&results_box, &state, &query);

        glib::ControlFlow::Break
    });
}

fn rebuild_results_ui(
    results_box: &gtk4::Box,
    state: &Rc<RefCell<LauncherState>>,
    query: &str,
) {
    clear_results_box(results_box);

    let s = state.borrow();
    let selected = s.selected;

    for (i, result) in s.results.iter().enumerate() {
        let row = build_result_row(result, i == selected, i, state, query);
        results_box.append(&row);
    }
}

fn build_result_row(
    result: &SearchResult,
    selected: bool,
    _index: usize,
    _state: &Rc<RefCell<LauncherState>>,
    query: &str,
) -> gtk4::Box {
    let row = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(12)
        .build();
    row.add_css_class("launcher-result");
    if selected {
        row.add_css_class("selected");
    }

    // Icon
    let icon_label = gtk4::Label::builder()
        .label(provider_icon(&result.provider))
        .halign(gtk4::Align::Center)
        .valign(gtk4::Align::Center)
        .build();
    icon_label.add_css_class("launcher-result-icon");

    // Try to load the icon as a GTK icon if it looks like an icon name
    if !result.icon.is_empty() && !result.icon.contains('/') {
        let theme = gtk4::IconTheme::for_display(&gtk4::gdk::Display::default().unwrap());
        if theme.has_icon(&result.icon) {
            let image = gtk4::Image::builder()
                .icon_name(&result.icon)
                .pixel_size(24)
                .build();
            image.add_css_class("launcher-result-icon-img");
            row.append(&image);
        } else {
            row.append(&icon_label);
        }
    } else {
        row.append(&icon_label);
    }

    // Text content
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

    // Provider badge
    let badge = gtk4::Label::builder()
        .label(&result.provider)
        .halign(gtk4::Align::End)
        .valign(gtk4::Align::Center)
        .build();
    badge.add_css_class("launcher-result-badge");
    row.append(&badge);

    // Click to activate
    let gesture = gtk4::GestureClick::new();
    let provider = result.provider.clone();
    let identifier = result.identifier.clone();
    let action = default_action(result);
    let query_str = query.to_string();
    gesture.connect_released(move |gesture, _, _, _| {
        let provider = provider.clone();
        let identifier = identifier.clone();
        let action = action.clone();
        let query = query_str.clone();

        if let Some(widget) = gesture.widget() {
            if let Some(root) = widget.root() {
                if let Ok(window) = root.downcast::<gtk4::Window>() {
                    window.set_visible(false);
                }
            }
        }

        std::thread::spawn(move || {
            if let Err(e) = elephant::activate(&provider, &identifier, &action, &query) {
                log::warn!("Elephant activate failed: {}", e);
            }
        });
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

/// Get the default action for a search result — use the first action from elephant,
/// or fall back to "start" for desktop apps.
fn default_action(result: &SearchResult) -> String {
    result
        .actions
        .first()
        .cloned()
        .unwrap_or_else(|| "start".to_string())
}

/// Calculate top offset as ~25% of the primary monitor height (Spotlight-style positioning).
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
    // Fallback for 1080p
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
