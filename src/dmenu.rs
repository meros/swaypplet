//! `swaypplet dmenu` — dmenu-style picker with the launcher's styling.
//!
//! Reads newline-separated items on stdin, shows a filterable list in a
//! layer-shell window, prints the chosen item to stdout and exits 0.
//! Esc or a backdrop click exits 1 with no output. When the query matches
//! no item, Enter prints the query itself — so a single piped line plus
//! `--prompt` doubles as an editable text prompt (used by `settask`).
//!
//! Runs as a standalone (non-unique) process: it owns its stdin/stdout,
//! which can't be forwarded through the single-instance panel GApplication.

use std::cell::RefCell;
use std::io::BufRead;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4_layer_shell::Edge;

use crate::layer_shell::{self, LayerShellConfig};
use crate::theme;

// Reuses the launcher's namespace so swayfx layer_effects (blur) apply.
static DMENU_CONFIG: LayerShellConfig = LayerShellConfig {
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

/// Cap on rendered rows — stdin can be arbitrarily long, the screen isn't.
const MAX_ROWS: usize = 30;

struct State {
    items: Vec<String>,
    /// Indices into `items` currently shown, in order.
    visible: Vec<usize>,
    /// Index into `visible`.
    selected: usize,
}

pub fn run(mut args: impl Iterator<Item = String>) {
    let mut placeholder = String::from("Select");
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--prompt" | "--placeholder" | "-p" => {
                if let Some(text) = args.next() {
                    placeholder = text;
                }
            }
            other => {
                eprintln!("swaypplet dmenu: unknown option: {other}");
                std::process::exit(2);
            }
        }
    }

    // Drain stdin before GTK starts — both call sites pipe a finite list.
    let items: Vec<String> = std::io::stdin()
        .lock()
        .lines()
        .map_while(Result::ok)
        .filter(|line| !line.trim().is_empty())
        .collect();

    let app = gtk4::Application::builder()
        .application_id("dev.swaypplet.dmenu")
        .flags(gtk4::gio::ApplicationFlags::NON_UNIQUE)
        .build();

    app.connect_activate(move |app| {
        theme::load_css();
        build_ui(app, &placeholder, items.clone());
    });
    // Don't let GApplication parse our argv (it contains dmenu options).
    app.run_with_args(&["swaypplet"]);

    // Main loop ended without a selection (window closed externally).
    std::process::exit(1);
}

fn build_ui(app: &gtk4::Application, placeholder: &str, items: Vec<String>) {
    let window = layer_shell::create_layer_window(app, &DMENU_CONFIG);
    window.add_css_class("launcher");

    let state = Rc::new(RefCell::new(State {
        items,
        visible: Vec::new(),
        selected: 0,
    }));

    let backdrop = gtk4::Box::builder().hexpand(true).vexpand(true).build();
    backdrop.add_css_class("launcher-backdrop");

    let container = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .halign(gtk4::Align::Center)
        .valign(gtk4::Align::Center)
        .width_request(560)
        .build();
    container.add_css_class("launcher-container");
    container.add_css_class("dmenu");

    let view = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .build();
    view.add_css_class("launcher-view");

    let entry = gtk4::SearchEntry::builder()
        .placeholder_text(placeholder)
        .hexpand(true)
        .build();
    entry.add_css_class("launcher-entry");

    let results_box = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .build();
    results_box.add_css_class("launcher-results");

    let scroller = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .propagate_natural_height(true)
        .max_content_height(520)
        .child(&results_box)
        .build();
    scroller.add_css_class("launcher-scroller");

    view.append(&entry);
    view.append(&scroller);
    container.append(&view);

    let overlay = gtk4::Overlay::new();
    overlay.set_child(Some(&backdrop));
    overlay.add_overlay(&container);
    window.set_child(Some(&overlay));

    // Click on the backdrop (not the card) cancels.
    let backdrop_click = gtk4::GestureClick::new();
    backdrop_click.connect_released(|_, _, _, _| std::process::exit(1));
    backdrop.add_controller(backdrop_click);

    // Live filtering — local list, no debounce needed.
    {
        let state = state.clone();
        let results_box = results_box.clone();
        entry.connect_search_changed(move |entry| {
            refilter(&state, &results_box, &entry.text());
        });
    }

    // Capture-phase keys on the window: Esc cancels, arrows move the
    // selection, Enter accepts (before the SearchEntry can swallow it).
    let key = gtk4::EventControllerKey::new();
    key.set_propagation_phase(gtk4::PropagationPhase::Capture);
    {
        let state = state.clone();
        let results_box = results_box.clone();
        let entry = entry.clone();
        key.connect_key_pressed(move |_, keyval, _, _| match keyval {
            gdk4::Key::Escape => std::process::exit(1),
            gdk4::Key::Down | gdk4::Key::Tab => {
                move_selection(&state, &results_box, 1);
                glib::Propagation::Stop
            }
            gdk4::Key::Up | gdk4::Key::ISO_Left_Tab => {
                move_selection(&state, &results_box, -1);
                glib::Propagation::Stop
            }
            gdk4::Key::Return | gdk4::Key::KP_Enter => {
                accept(&state, &entry);
            }
            _ => glib::Propagation::Proceed,
        });
    }
    window.add_controller(key);

    refilter(&state, &results_box, "");
    window.present();
    entry.grab_focus();
}

/// Print the selection (or, with nothing matching, the raw query) and exit.
fn accept(state: &Rc<RefCell<State>>, entry: &gtk4::SearchEntry) -> ! {
    let s = state.borrow();
    if let Some(&idx) = s.visible.get(s.selected) {
        println!("{}", s.items[idx]);
        std::process::exit(0);
    }
    let query = entry.text().to_string();
    if query.is_empty() {
        std::process::exit(1);
    }
    println!("{query}");
    std::process::exit(0);
}

/// Case-insensitive word match: every whitespace-separated query term must
/// appear somewhere in the item.
fn matches(item: &str, query: &str) -> bool {
    let item = item.to_lowercase();
    query
        .split_whitespace()
        .all(|term| item.contains(&term.to_lowercase()))
}

fn refilter(state: &Rc<RefCell<State>>, results_box: &gtk4::Box, query: &str) {
    let mut s = state.borrow_mut();
    s.visible = s
        .items
        .iter()
        .enumerate()
        .filter(|(_, item)| matches(item, query))
        .map(|(i, _)| i)
        .take(MAX_ROWS)
        .collect();
    s.selected = 0;

    while let Some(child) = results_box.first_child() {
        results_box.remove(&child);
    }
    for (pos, &idx) in s.visible.iter().enumerate() {
        let row = build_row(&s.items[idx], pos == s.selected);
        // Click on a row accepts it directly.
        let click = gtk4::GestureClick::new();
        let text = s.items[idx].clone();
        click.connect_released(move |_, _, _, _| {
            println!("{text}");
            std::process::exit(0);
        });
        row.add_controller(click);
        results_box.append(&row);
    }
}

fn build_row(text: &str, selected: bool) -> gtk4::Box {
    let row = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .build();
    row.add_css_class("launcher-result");
    if selected {
        row.add_css_class("selected");
    }
    let name = gtk4::Label::builder()
        .label(text)
        .halign(gtk4::Align::Start)
        .hexpand(true)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .build();
    name.add_css_class("launcher-result-name");
    row.append(&name);
    row
}

fn move_selection(state: &Rc<RefCell<State>>, results_box: &gtk4::Box, delta: i32) {
    let mut s = state.borrow_mut();
    if s.visible.is_empty() {
        return;
    }
    let len = s.visible.len() as i32;
    let old = s.selected;
    s.selected = (s.selected as i32 + delta).rem_euclid(len) as usize;
    let new = s.selected;
    drop(s);

    let mut child = results_box.first_child();
    let mut pos = 0usize;
    while let Some(widget) = child {
        if pos == old {
            widget.remove_css_class("selected");
        }
        if pos == new {
            widget.add_css_class("selected");
        }
        child = widget.next_sibling();
        pos += 1;
    }
}
