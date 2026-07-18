//! `swaypplet dmenu` — dmenu-style picker with the launcher's styling.
//!
//! Reads newline-separated items on stdin, shows a filterable list in a
//! layer-shell window, prints the chosen item to stdout and exits 0.
//! Esc or a backdrop click exits 1 with no output. When the query matches
//! no item, Enter prints the query itself — so a single piped line plus
//! `--prompt` doubles as an editable text prompt (used by `settask`).
//!
//! Cold-starting a GTK process per keypress costs several hundred ms, so the
//! CLI is a thin client first: when the panel process is listening on the
//! dmenu socket (see [`start_server`]) the request is forwarded there and the
//! warm panel presents the picker. The standalone GTK path stays as the
//! fallback for sessions without a panel (greeter, lock, first start).

use std::cell::RefCell;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4_layer_shell::Edge;
use serde::{Deserialize, Serialize};

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

/// One JSON line client → server; the reply is a [`Response`] line back.
#[derive(Serialize, Deserialize)]
struct Request {
    prompt: String,
    items: Vec<String>,
}

/// `text: Some` is a selection (or raw query), `None` a cancel.
#[derive(Serialize, Deserialize)]
struct Response {
    text: Option<String>,
}

/// Socket lives in the per-user runtime dir (mode 0700) like the pid file,
/// so only the owning user can reach the picker.
fn socket_path() -> PathBuf {
    let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
    std::path::Path::new(&dir).join("swaypplet-dmenu.sock")
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

    // Warm path: a running panel presents the picker; we just relay.
    if let Some(reply) = remote_pick(&placeholder, &items) {
        match reply {
            Some(text) => {
                println!("{text}");
                std::process::exit(0);
            }
            None => std::process::exit(1),
        }
    }

    // No panel listening — cold-start our own GTK instance.
    let app = gtk4::Application::builder()
        .application_id("dev.swaypplet.dmenu")
        .flags(gtk4::gio::ApplicationFlags::NON_UNIQUE)
        .build();

    app.connect_activate(move |app| {
        theme::load_css();
        present_picker(app, &placeholder, items.clone(), |reply| match reply {
            Some(text) => {
                println!("{text}");
                std::process::exit(0);
            }
            None => std::process::exit(1),
        });
    });
    // Don't let GApplication parse our argv (it contains dmenu options).
    app.run_with_args(&["swaypplet"]);

    // Main loop ended without a selection (window closed externally).
    std::process::exit(1);
}

/// Hand the request to the panel's picker server. `None` means no usable
/// server (fall back to standalone GTK); `Some(reply)` means it was handled.
fn remote_pick(prompt: &str, items: &[String]) -> Option<Option<String>> {
    let mut stream = UnixStream::connect(socket_path()).ok()?;
    let request = serde_json::to_string(&Request {
        prompt: prompt.to_string(),
        items: items.to_vec(),
    })
    .ok()?;
    stream.write_all(request.as_bytes()).ok()?;
    stream.write_all(b"\n").ok()?;
    // No read timeout: the reply arrives whenever the user picks something.
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).ok()?;
    let response: Response = serde_json::from_str(line.trim()).ok()?;
    Some(response.text)
}

/// Panel-side picker server: presents dmenu requests from the warm process,
/// making `swaypplet dmenu` effectively instant. One picker at a time; a
/// newer request supersedes the current one (the latest hotkey press wins)
/// and the superseded client gets a cancel reply.
pub fn start_server(app: &gtk4::Application) {
    let path = socket_path();
    // A stale socket file only outlives a crashed panel — GApplication
    // uniqueness guarantees no live listener exists.
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(listener) => listener,
        Err(e) => {
            log::warn!("dmenu server: bind {} failed: {e}", path.display());
            return;
        }
    };

    let (tx, rx) = async_channel::unbounded::<(Request, UnixStream)>();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(stream) = conn else { continue };
            let tx = tx.clone();
            // Read + parse off-main so a slow client can't stall the panel.
            std::thread::spawn(move || {
                let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
                let Ok(reader) = stream.try_clone() else {
                    return;
                };
                let mut line = String::new();
                if BufReader::new(reader).read_line(&mut line).is_err() {
                    return;
                }
                let Ok(request) = serde_json::from_str::<Request>(line.trim()) else {
                    return;
                };
                let _ = tx.send_blocking((request, stream));
            });
        }
    });

    let app = app.clone();
    glib::MainContext::default().spawn_local(async move {
        let active: RefCell<Option<Rc<Picker>>> = RefCell::new(None);
        while let Ok((request, mut stream)) = rx.recv().await {
            if let Some(prev) = active.borrow_mut().take() {
                prev.finish(None);
            }
            let picker = present_picker(&app, &request.prompt, request.items, move |reply| {
                // Tiny write; the timeout only guards against a dead client
                // with a full socket buffer.
                let _ = stream.set_write_timeout(Some(std::time::Duration::from_secs(1)));
                if let Ok(json) = serde_json::to_string(&Response { text: reply }) {
                    let _ = stream.write_all(json.as_bytes());
                    let _ = stream.write_all(b"\n");
                }
            });
            *active.borrow_mut() = Some(picker);
        }
    });
}

struct State {
    items: Vec<String>,
    /// Indices into `items` currently shown, in order.
    visible: Vec<usize>,
    /// Index into `visible`.
    selected: usize,
}

struct Picker {
    window: gtk4::Window,
    entry: gtk4::SearchEntry,
    results_box: gtk4::Box,
    state: RefCell<State>,
    /// Taken on the first `finish`; later calls are no-ops.
    done: RefCell<Option<Box<dyn FnOnce(Option<String>)>>>,
}

/// Build and present a picker window on `app`. `on_done` runs exactly once —
/// on selection, cancel, or supersession — after the window is destroyed.
fn present_picker(
    app: &gtk4::Application,
    placeholder: &str,
    items: Vec<String>,
    on_done: impl FnOnce(Option<String>) + 'static,
) -> Rc<Picker> {
    let window = layer_shell::create_layer_window(app, &DMENU_CONFIG);
    window.add_css_class("launcher");

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

    let picker = Rc::new(Picker {
        window: window.clone(),
        entry: entry.clone(),
        results_box,
        state: RefCell::new(State {
            items,
            visible: Vec::new(),
            selected: 0,
        }),
        done: RefCell::new(Some(Box::new(on_done))),
    });

    // Click on the backdrop (not the card) cancels.
    let backdrop_click = gtk4::GestureClick::new();
    {
        let picker = picker.clone();
        backdrop_click.connect_released(move |_, _, _, _| picker.finish(None));
    }
    backdrop.add_controller(backdrop_click);

    // Live filtering — local list, no debounce needed.
    {
        let picker = picker.clone();
        entry.connect_search_changed(move |entry| picker.refilter(&entry.text()));
    }

    // Capture-phase keys on the window: Esc cancels, arrows move the
    // selection, Enter accepts (before the SearchEntry can swallow it).
    let key = gtk4::EventControllerKey::new();
    key.set_propagation_phase(gtk4::PropagationPhase::Capture);
    {
        let picker = picker.clone();
        key.connect_key_pressed(move |_, keyval, _, _| match keyval {
            gdk4::Key::Escape => {
                picker.finish(None);
                glib::Propagation::Stop
            }
            gdk4::Key::Down | gdk4::Key::Tab => {
                picker.move_selection(1);
                glib::Propagation::Stop
            }
            gdk4::Key::Up | gdk4::Key::ISO_Left_Tab => {
                picker.move_selection(-1);
                glib::Propagation::Stop
            }
            gdk4::Key::Return | gdk4::Key::KP_Enter => {
                picker.accept();
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        });
    }
    window.add_controller(key);

    picker.refilter("");
    window.present();
    entry.grab_focus();
    picker
}

impl Picker {
    /// Resolve once: destroy the window (releasing the exclusive keyboard
    /// grab immediately) and deliver the reply.
    fn finish(&self, reply: Option<String>) {
        let Some(done) = self.done.borrow_mut().take() else {
            return;
        };
        self.window.destroy();
        done(reply);
    }

    /// Accept the selection (or, with nothing matching, the raw query).
    fn accept(&self) {
        let reply = {
            let s = self.state.borrow();
            if let Some(&idx) = s.visible.get(s.selected) {
                Some(s.items[idx].clone())
            } else {
                let query = self.entry.text().to_string();
                (!query.is_empty()).then_some(query)
            }
        };
        self.finish(reply);
    }

    fn refilter(self: &Rc<Self>, query: &str) {
        let mut s = self.state.borrow_mut();
        s.visible = s
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| matches(item, query))
            .map(|(i, _)| i)
            .take(MAX_ROWS)
            .collect();
        s.selected = 0;

        while let Some(child) = self.results_box.first_child() {
            self.results_box.remove(&child);
        }
        for (pos, &idx) in s.visible.iter().enumerate() {
            let row = build_row(&s.items[idx], pos == s.selected);
            // Click on a row accepts it directly.
            let click = gtk4::GestureClick::new();
            let text = s.items[idx].clone();
            let picker = self.clone();
            click.connect_released(move |_, _, _, _| picker.finish(Some(text.clone())));
            row.add_controller(click);
            self.results_box.append(&row);
        }
    }

    fn move_selection(&self, delta: i32) {
        let mut s = self.state.borrow_mut();
        if s.visible.is_empty() {
            return;
        }
        let len = s.visible.len() as i32;
        let old = s.selected;
        s.selected = (s.selected as i32 + delta).rem_euclid(len) as usize;
        let new = s.selected;
        drop(s);

        let mut child = self.results_box.first_child();
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
}

/// Case-insensitive word match: every whitespace-separated query term must
/// appear somewhere in the item.
fn matches(item: &str, query: &str) -> bool {
    let item = item.to_lowercase();
    query
        .split_whitespace()
        .all(|term| item.contains(&term.to_lowercase()))
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
