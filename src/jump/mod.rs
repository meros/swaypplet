//! Jump: `Super+Tab` walks back through the workspaces you came from.
//!
//! Tap and release goes back one. Keep Super held and tap again to walk
//! further, through at most eight places. Each is a tile with a live picture
//! of the workspace and the one chord that reaches it directly without this
//! surface at all.
//!
//! The question is "which workspace", so the unit is the workspace and not
//! the window. An earlier switcher (removed 2026-08-21) showed one still
//! thumbnail per window, captured once on open, and on this desktop that
//! was a grid of near-identical terminals. A workspace picture carries what a
//! title cannot: the layout, the mix of apps, and whatever is moving.
//!
//! The pictures are live. `live.rs` keeps one capture session per window
//! open while the card is up and the compositor sends a frame when a window
//! changes, including windows on workspaces nobody is looking at. `scene.rs`
//! says where each window goes in its tile, from the tree; `card.rs` puts the
//! frames there.
//!
//! The parts that can go wrong live in [`gesture`], [`rows`] and [`scene`],
//! with no GTK in them. This file translates GTK events into [`gesture::Ev`]
//! and applies [`gesture::Action`]s to widgets. It decides nothing.

pub mod card;
pub mod carousel;
pub mod gesture;
pub mod live;
pub mod peek;
pub mod pin;
pub mod place;
pub mod rows;
pub mod scene;

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::glib;
use gtk4::prelude::*;

use crate::layer_shell::{self, LayerShellConfig};
use gesture::{Action, Ev, Gesture};
use rows::Row;

/// The longer edge of a window frame, in pixels: the preview box's width
/// at scale 2. No window is drawn wider than the box.
const FRAME_EDGE: u32 = (rows::PREVIEW_W * 2) as u32;

/// Per window. A terminal scrolling or a video playing reads as live at this
/// rate, and the card is on screen for a second or two.
const FRAME_RATE: u32 = 20;

/// How long a gesture may sit with no event before the surface assumes the
/// release was lost and lets go. A grab that outlives its keypress holds the
/// keyboard against every other window on the machine.
///
/// Counted from the last step, not from the open: the pictures are live, and
/// someone who holds Super to watch one should not be moved away from under
/// it. Six seconds is longer than anyone looks at a switcher without pressing
/// anything, and short enough that a lost release does not strand the grab.
const WATCHDOG_MS: u64 = 6_000;

struct State {
    gesture: Gesture,
    /// Commands per row, captured at gesture start.
    commands: Vec<String>,
    /// The workspace we were on when the gesture began. If something else
    /// moves us mid-gesture, committing on top of it would move us twice.
    origin: String,
    /// The workspace per row, for `p` to pin the selected one.
    names: Vec<String>,
    selected: usize,
    watchdog: Option<glib::SourceId>,
    /// The selected place's picture and what it shows, taken as the card
    /// unmaps: the switch that follows grows out of it (`handoff`).
    handoff: Option<(crate::handoff::Rect, crate::handoff::Rect)>,
}

/// What `p` does with the selected workspace's name.
/// What `p` does with the selected workspace's name: pin or unpin it, and
/// say which.
type PinFn = Box<dyn Fn(String) -> bool>;

pub struct Jump {
    window: gtk4::Window,
    /// Holds the card, which is rebuilt for every gesture.
    wrapper: gtk4::Box,
    card: RefCell<Option<card::Card>>,
    /// Running while the card is mapped; dropping it stops every capture.
    stream: RefCell<Option<live::Stream>>,
    pin: RefCell<Option<PinFn>>,
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
            // A strip across the output: the places float over the desktop,
            // and a wider screen shows more of them.
            anchors: &[
                (gtk4_layer_shell::Edge::Left, true),
                (gtk4_layer_shell::Edge::Right, true),
            ],
            margins: &[],
            // Exclusive, unlike the keybind sheet: this surface has to see the
            // modifier come up, and that only arrives at whoever holds the
            // keyboard. sway still evaluates its own bindings first, so
            // `Super+Tab` keeps reaching us and every other chord keeps
            // working while the card is up.
            keyboard_mode: gtk4_layer_shell::KeyboardMode::Exclusive,
        };

        let window = layer_shell::create_layer_window(app, &CONFIG);
        // Resizable, unlike the other surfaces: a non-resizable GTK window
        // keeps its natural size, and the strip must take the output's width
        // from its left and right anchors.
        window.set_decorated(false);

        let wrapper = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .hexpand(true)
            .valign(gtk4::Align::Center)
            .build();
        window.set_child(Some(&wrapper));

        let this = Rc::new(Jump {
            window,
            wrapper,
            card: RefCell::new(None),
            stream: RefCell::new(None),
            pin: RefCell::new(None),
            state: RefCell::new(State {
                gesture: Gesture::new(),
                commands: Vec::new(),
                origin: String::new(),
                names: Vec::new(),
                selected: 0,
                watchdog: None,
                handoff: None,
            }),
        });
        this.wire();
        // Presented once here, then shown and hidden with `set_visible`.
        // That is the pattern every other surface in this process follows
        // (panel.rs, and anim::Reveal for the rest), and it is not a style
        // choice: gtk4-layer-shell configures the surface as it is realized,
        // so a window first presented later comes up without ever having been
        // laid out - it maps, the compositor gives it a surface, and nothing
        // is drawn on it. That failure is silent and looks exactly like the
        // gesture not firing.
        this.window.present();
        this.window.set_visible(false);
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
                // `p` pins the selected place and keeps the card up.
                if key == gtk4::gdk::Key::p {
                    this.pin_selected();
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
            log::warn!("jump: could not read the session; nothing to show");
            return;
        };
        let (places, bindings, focused_output, tree) = session;
        let apps = |ws: &str| apps_on(&tree, ws);
        let built = rows::rows(&places, &bindings, &apps, &focused_output);
        // The same places `rows` kept, in the same order.
        let scenes: Vec<_> = places
            .iter()
            .skip(1)
            .take(built.len())
            .map(|p| scene::scene(&tree, &p.name))
            .collect();

        log::info!(
            "jump: {} places, {} rows, focused output {:?}",
            places.len(),
            built.len(),
            focused_output
        );
        self.rebuild(&built, &scenes);
        {
            let mut st = self.state.borrow_mut();
            st.commands = built.iter().map(|r| r.command.clone()).collect();
            st.origin = places.first().map(|p| p.name.clone()).unwrap_or_default();
            st.names = places
                .iter()
                .skip(1)
                .take(built.len())
                .map(|p| p.name.clone())
                .collect();
            st.selected = 0;
            if let Some(card) = &*self.card.borrow() {
                for (i, name) in st.names.iter().enumerate() {
                    card.set_pinned(i, pin::is_pinned(name));
                }
            }
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
                // No material: the places float over the desktop, and glass
                // on a strip as wide as the output would be a card again.
                self.window.set_visible(true);
                self.arm_watchdog();
                self.start_stream();
                // Harness hook: the nested session has no keyboard to let go
                // of, so `SWAYPPLET_JUMP_RELEASE_MS=<ms>` releases Super that
                // long after the card maps, down the same path.
                if let Some(ms) = std::env::var("SWAYPPLET_JUMP_RELEASE_MS")
                    .ok()
                    .and_then(|v| v.parse::<u64>().ok())
                {
                    let this = self.clone();
                    glib::timeout_add_local_once(std::time::Duration::from_millis(ms), move || {
                        this.feed(Ev::SuperReleased)
                    });
                }
            }
            Action::Select(i) => {
                self.select(i);
                self.arm_watchdog();
            }
            Action::Unmap => {
                // Measured while the card is still on screen.
                let selected = self.state.borrow().selected;
                let handoff = self.card.borrow().as_ref().and_then(|c| c.handoff(selected));
                self.state.borrow_mut().handoff = handoff;
                self.stream.replace(None);
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
                let handoff = self.state.borrow_mut().handoff.take();
                crate::handoff::run_workspace_switch(
                    handoff.map(|h| h.0),
                    handoff.map(|h| h.1),
                    &command,
                );
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

    /// What `p` does: set by the app, which owns the pins.
    pub fn set_pin(&self, pin: impl Fn(String) -> bool + 'static) {
        self.pin.replace(Some(Box::new(pin)));
    }

    fn pin_selected(&self) {
        let name = {
            let st = self.state.borrow();
            st.names.get(st.selected).cloned()
        };
        let selected = self.state.borrow().selected;
        if let (Some(name), Some(pin)) = (name, &*self.pin.borrow()) {
            let pinned = pin(name);
            if let Some(card) = &*self.card.borrow() {
                card.set_pinned(selected, pinned);
            }
        }
    }

    fn select(&self, index: usize) {
        self.state.borrow_mut().selected = index;
        if let Some(card) = &*self.card.borrow() {
            card.select(index);
        }
    }

    fn rebuild(&self, built: &[Row], scenes: &[Option<scene::Scene>]) {
        if let Some(old) = self.card.replace(None) {
            self.wrapper.remove(&old.root);
        }
        let card = card::Card::new(built, scenes);
        self.wrapper.append(&card.root);
        self.card.replace(Some(card));
    }

    /// Capture every window on the card until the card goes away.
    fn start_stream(self: &Rc<Self>) {
        let ids = match &*self.card.borrow() {
            Some(card) => card.window_ids(),
            None => return,
        };
        if ids.is_empty() {
            return;
        }
        let (tx, rx) = async_channel::unbounded::<live::Frame>();
        self.stream
            .replace(Some(live::Stream::start(ids, FRAME_EDGE, FRAME_RATE, tx)));

        // Ends when the worker drops its sender, which it does when the
        // stream is dropped at unmap.
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            while let Ok(frame) = rx.recv().await {
                let Some(this) = weak.upgrade() else { break };
                if let Some(card) = &*this.card.borrow() {
                    card.frame(frame);
                }
            }
        });
    }
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
pub(crate) fn apps_on(tree: &swayipc::Node, workspace: &str) -> Vec<String> {
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
