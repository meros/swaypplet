//! Jump: `Super+Tab` walks back through the workspaces you came from.
//!
//! Tap and release goes back one. Keep Super held and tap again to walk
//! further, through a list that is at most eight rows and names, for each
//! place, the one chord that reaches it directly without this surface at all.
//! It is meant to make itself unnecessary.
//!
//! It replaces a grid of live window thumbnails. That grid answered "which
//! window", which on this desktop is nearly never the question: the reference
//! session runs about fifteen windows over eleven workspaces, so a workspace
//! holds one or two, and eight of the fifteen are near-identical terminals
//! that a thumbnail cannot tell apart any better than a title can. The
//! question that is actually asked many times an hour is "which workspace",
//! and it needs no pixels to answer - which is why nothing here captures
//! anything, opens a second Wayland connection, or allocates a texture.
//!
//! The parts that can go wrong live in [`gesture`] and [`rows`], with no GTK
//! in them. This file translates GTK events into [`gesture::Ev`] and applies
//! [`gesture::Action`]s to widgets. It decides nothing.

pub mod gesture;
pub mod place;
pub mod rows;

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::glib;
use gtk4::prelude::*;

use crate::layer_shell::{self, LayerShellConfig};
use gesture::{Action, Ev, Gesture};
use rows::Row;

/// How long a gesture may sit with no event before the surface assumes the
/// release was lost and lets go. A grab that outlives its keypress holds the
/// keyboard against every other window on the machine.
const WATCHDOG_MS: u64 = 3_000;

struct State {
    gesture: Gesture,
    /// Commands per row, captured at gesture start.
    commands: Vec<String>,
    /// The workspace we were on when the gesture began. If something else
    /// moves us mid-gesture, committing on top of it would move us twice.
    origin: String,
    watchdog: Option<glib::SourceId>,
}

pub struct Jump {
    window: gtk4::Window,
    card: gtk4::Box,
    list: gtk4::Box,
    state: RefCell<State>,
}

impl Jump {
    pub fn new(app: &gtk4::Application) -> Rc<Self> {
        static CONFIG: LayerShellConfig = LayerShellConfig {
            namespace: "swaypplet-jump",
            layer: gtk4_layer_shell::Layer::Overlay,
            exclusive: false,
            default_width: None,
            default_height: None,
            anchors: &[],
            margins: &[],
            // Exclusive, unlike the keybind sheet: this surface has to see the
            // modifier come up, and that only arrives at whoever holds the
            // keyboard. sway still evaluates its own bindings first, so
            // `Super+Tab` keeps reaching us and every other chord keeps
            // working while the card is up.
            keyboard_mode: gtk4_layer_shell::KeyboardMode::Exclusive,
        };

        let window = layer_shell::create_layer_window(app, &CONFIG);
        window.set_resizable(false);
        window.set_decorated(false);

        let card = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .halign(gtk4::Align::Center)
            .valign(gtk4::Align::Center)
            .build();
        card.add_css_class("glass-card");
        card.add_css_class("jump-card");

        let list = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .build();
        list.add_css_class("jump-list");
        card.append(&list);

        let wrapper = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .halign(gtk4::Align::Center)
            .valign(gtk4::Align::Center)
            .build();
        wrapper.append(&card);
        window.set_child(Some(&wrapper));

        let this = Rc::new(Jump {
            window,
            card,
            list,
            state: RefCell::new(State {
                gesture: Gesture::new(),
                commands: Vec::new(),
                origin: String::new(),
                watchdog: None,
            }),
        });
        this.wire();
        this
    }

    /// Escape cancels; the modifier coming up commits.
    fn wire(self: &Rc<Self>) {
        let keys = gtk4::EventControllerKey::new();
        {
            let this = self.clone();
            keys.connect_key_pressed(move |_, key, _, _| {
                if key == gtk4::gdk::Key::Escape {
                    this.feed(Ev::Escape);
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
        }
        {
            let this = self.clone();
            keys.connect_key_released(move |_, key, _, _| {
                if matches!(key, gtk4::gdk::Key::Super_L | gtk4::gdk::Key::Super_R) {
                    this.feed(Ev::SuperReleased);
                }
            });
        }
        self.window.add_controller(keys);
    }

    /// `Super+Tab`.
    pub fn step(self: &Rc<Self>) {
        if !self.state.borrow().gesture.is_live() {
            self.begin();
        }
        self.feed(Ev::Step);
    }

    /// `Super+Shift+Tab`.
    pub fn step_back(self: &Rc<Self>) {
        self.feed(Ev::StepBack);
    }

    /// Read the session and build the list, before the first event.
    ///
    /// Synchronous on the GTK thread on purpose. It is one `get_tree` plus one
    /// `get_config` round trip over a unix socket, single-digit milliseconds,
    /// and the alternative is a worker thread whose result arrives after the
    /// user has already released the key.
    fn begin(self: &Rc<Self>) {
        let Some(session) = read_session() else {
            return;
        };
        let (places, bindings, focused_output, tree) = session;
        let apps = |ws: &str| apps_on(&tree, ws);
        let built = rows::rows(&places, &bindings, &apps, &focused_output);

        self.rebuild(&built);
        {
            let mut st = self.state.borrow_mut();
            st.commands = built.iter().map(|r| r.command.clone()).collect();
            st.origin = places.first().map(|p| p.name.clone()).unwrap_or_default();
        }
    }

    fn feed(self: &Rc<Self>, ev: Ev) {
        let actions = {
            let mut st = self.state.borrow_mut();
            let commands = st.commands.clone();
            st.gesture.on(ev, &commands)
        };
        for action in actions {
            self.apply(action);
        }
    }

    fn apply(self: &Rc<Self>, action: Action) {
        match action {
            Action::Map => {
                self.window.present();
                self.arm_watchdog();
            }
            Action::Select(i) => self.select(i),
            Action::Unmap => {
                self.disarm_watchdog();
                self.window.set_visible(false);
            }
            Action::Run(command) => {
                // Refuse to move if something already did. `Super+g` while the
                // card is up runs sway's own binding, and committing on top of
                // that would move you twice - once where you asked, once where
                // this surface still thought you were.
                let origin = self.state.borrow().origin.clone();
                if focused_workspace().is_some_and(|now| now != origin) {
                    log::debug!("jump: cancelled, something else moved us to {origin:?}");
                    return;
                }
                crate::sway_ipc::run_command(&command);
            }
        }
    }

    fn arm_watchdog(self: &Rc<Self>) {
        self.disarm_watchdog();
        let this = self.clone();
        let id = glib::timeout_add_local_once(
            std::time::Duration::from_millis(WATCHDOG_MS),
            move || {
                this.state.borrow_mut().watchdog = None;
                this.feed(Ev::Watchdog);
            },
        );
        self.state.borrow_mut().watchdog = Some(id);
    }

    fn disarm_watchdog(&self) {
        if let Some(id) = self.state.borrow_mut().watchdog.take() {
            crate::spawn::remove_source(id);
        }
    }

    fn select(&self, index: usize) {
        let mut i = 0;
        let mut child = self.list.first_child();
        while let Some(row) = child {
            row.set_css_classes(if i == index {
                &["jump-row", "selected"]
            } else {
                &["jump-row"]
            });
            child = row.next_sibling();
            i += 1;
        }
    }

    fn rebuild(&self, built: &[Row]) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        for row in built {
            self.list.append(&row_widget(row));
        }
        // Both dimensions asked for explicitly, from the row count and nothing
        // else. Letting GTK derive the height from the children is precisely
        // how the old switcher came out a different shape on every open: the
        // size then depends on what happens to be in the list - a long title,
        // a thumbnail that came back at an odd aspect - and no two openings
        // can be compared. `rows::card_size` is the contract and this is where
        // it is enforced; the unit tests assert the same function.
        let (w, h) = rows::card_size(rows::row_count(built.len() + 1));
        self.card.set_size_request(w, h);
    }
}

/// One row: four columns, every width fixed, on every row.
fn row_widget(row: &Row) -> gtk4::Box {
    let b = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(0)
        .build();
    b.add_css_class("jump-row");
    b.set_size_request(-1, rows::ROW_H);

    let chord = gtk4::Label::builder()
        .label(row.chord.as_deref().unwrap_or("\u{2014}"))
        .xalign(1.0)
        .width_chars(3)
        .build();
    chord.add_css_class("jump-chord");
    b.append(&chord);

    let label = gtk4::Label::builder().label(&row.label).xalign(0.0).build();
    label.add_css_class("jump-label");
    label.set_size_request(120, -1);
    b.append(&label);

    let detail = gtk4::Label::builder()
        .label(&row.detail)
        .xalign(0.0)
        .hexpand(true)
        // Ellipsized, never wrapped: a wrapped name would make one row taller
        // than the rest and the card a different height every time it opened.
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .build();
    detail.add_css_class("jump-detail");
    b.append(&detail);

    let marker = gtk4::Label::builder()
        .label(if row.other_output { "\u{f0379}" } else { "" })
        .xalign(1.0)
        .width_chars(2)
        .build();
    marker.add_css_class("jump-output");
    b.append(&marker);

    b
}

/// Everything the list needs, in two round trips.
type Session = (
    Vec<place::Place>,
    Vec<crate::keybinds::Binding>,
    String,
    swayipc::Node,
);

fn read_session() -> Option<Session> {
    let mut conn = crate::sway_ipc::connect().ok()?;
    let tree = conn.get_tree().ok()?;
    let config = conn.get_config().ok().map(|c| c.config).unwrap_or_default();
    let places = place::mru(&tree);
    let focused_output = places.first().map(|p| p.output.clone()).unwrap_or_default();
    Some((
        places,
        crate::keybinds::parse(&config),
        focused_output,
        tree,
    ))
}

/// The workspace with focus right now, by name.
fn focused_workspace() -> Option<String> {
    let mut conn = crate::sway_ipc::connect().ok()?;
    conn.get_workspaces()
        .ok()?
        .into_iter()
        .find(|w| w.focused)
        .map(|w| w.name)
}

/// The app ids of every window on a workspace, in tree order.
fn apps_on(tree: &swayipc::Node, workspace: &str) -> Vec<String> {
    fn collect(node: &swayipc::Node, out: &mut Vec<String>) {
        if let Some(app) = node.app_id.as_deref() {
            out.push(app.to_string());
        } else if let Some(class) = node
            .window_properties
            .as_ref()
            .and_then(|p| p.class.as_deref())
        {
            out.push(class.to_string());
        }
        for child in node.nodes.iter().chain(node.floating_nodes.iter()) {
            collect(child, out);
        }
    }
    fn find<'a>(node: &'a swayipc::Node, name: &str) -> Option<&'a swayipc::Node> {
        if node.node_type == swayipc::NodeType::Workspace && node.name.as_deref() == Some(name) {
            return Some(node);
        }
        node.nodes
            .iter()
            .chain(node.floating_nodes.iter())
            .find_map(|c| find(c, name))
    }
    let mut out = Vec::new();
    if let Some(ws) = find(tree, workspace) {
        collect(ws, &mut out);
    }
    out
}
