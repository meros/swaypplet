//! Jump: `Super+Tab` walks back through the workspaces you came from.
//!
//! Tap and release goes back one. Keep Super held and tap again to walk
//! further, through at most eight places on this output.
//!
//! The workspaces themselves are the pictures. While Super is held the one
//! you are on shrinks to [`row::SCALE`] and moves left, and the one you came
//! from slides in to the middle at the same size; the next one peeks in from
//! the right edge. Tab moves the row a place left, Shift+Tab a place right.
//! Releasing Super grows the middle one to full size and switches to it;
//! Escape brings back the one you were on. sway draws and animates all of it
//! (`workspace_transform`, nixos patches/swayfx-ws-transform.patch), so the
//! windows are the real ones, not captured frames, and this process sends
//! one short command per workspace per step.
//!
//! The surface here is a transparent layer over the output. It holds the
//! keyboard (the release edge only reaches whoever does), catches the
//! pointer so a click cannot land on a window that is only a picture, and
//! draws two things: a ring round the middle workspace and its caption.
//!
//! The parts that can go wrong live in [`gesture`] and [`row`], with no GTK
//! in them. This file translates GTK events into [`gesture::Ev`], and
//! [`gesture::Action`]s into sway commands and widgets. It decides nothing.
//!
//! An earlier version (removed 2026-09-26) drew a strip of tiles with live
//! captures of every window, and a card before it one still thumbnail per
//! window. Both were pictures of the workspace; this is the workspace.

pub mod card;
pub mod gesture;
pub mod live;
pub mod peek;
pub mod pin;
pub mod place;
pub mod row;
pub mod rows;
pub mod scene;

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::glib;
use gtk4::prelude::*;
use gtk4_layer_shell::LayerShell;

use crate::layer_shell::{self, LayerShellConfig};
use gesture::{Action, Ev, Gesture};
use rows::Row;

/// How long a gesture may sit with no event before the surface assumes the
/// release was lost and lets go. A grab that outlives its keypress holds the
/// keyboard against every other window on the machine.
///
/// Counted from the last step, not from the open: someone who holds Super
/// to look at a workspace should not be moved away from under it. Six
/// seconds is longer than anyone looks at a switcher without pressing
/// anything, and short enough that a lost release does not strand the grab.
const WATCHDOG_MS: u64 = 6_000;

/// The sway binding mode that is active while the switcher is up. The keys
/// you press here are pressed with Super still held, and sway runs its own
/// bindings before any client sees a key: `Super+Escape` closed the focused
/// window and `Super+p` went to workspace p. The mode (users/modules/sway.nix
/// in the nixos repo) binds both to `swaypplet-jump cancel` and
/// `swaypplet-jump pin`, and `Escape` in it always goes back to the default
/// mode, so a panel that dies with the switcher up cannot strand the
/// keyboard. On a sway config without the mode, sway rejects the command and
/// nothing changes.
const MODE: &str = "swaypplet-jump";

struct State {
    gesture: Gesture,
    /// The switch command per selectable workspace, captured at the start.
    commands: Vec<String>,
    /// The workspaces in the row: `[0]` the one you were on, then one per
    /// entry in `commands`.
    names: Vec<String>,
    /// Their captions, one per entry in `commands`.
    rows: Vec<Row>,
    /// The output's geometry, read when the gesture starts.
    row: Option<row::Row>,
    /// Where the output sits in the layout: the surface's origin.
    origin_xy: (f64, f64),
    cursor: usize,
    watchdog: Option<glib::SourceId>,
}

/// What `p` does with the selected workspace's name: pin or unpin it, and
/// say which.
type PinFn = Box<dyn Fn(String) -> bool>;

pub struct Jump {
    window: gtk4::Window,
    stage: gtk4::Fixed,
    /// The ring round the middle workspace.
    ring: gtk4::Box,
    caption: gtk4::Box,
    chord: gtk4::Label,
    label: gtk4::Label,
    detail: gtk4::Label,
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
            // The whole output: the row is drawn across all of it, and the
            // pointer must not reach the windows in it.
            anchors: &[
                (gtk4_layer_shell::Edge::Left, true),
                (gtk4_layer_shell::Edge::Right, true),
                (gtk4_layer_shell::Edge::Top, true),
                (gtk4_layer_shell::Edge::Bottom, true),
            ],
            margins: &[],
            // Exclusive: this surface has to see the modifier come up, and
            // that only arrives at whoever holds the keyboard. sway still
            // evaluates its own bindings first, which is why the switcher
            // puts sway in [`MODE`] while it is up.
            keyboard_mode: gtk4_layer_shell::KeyboardMode::Exclusive,
        };

        let window = layer_shell::create_layer_window(app, &CONFIG);
        // Over the bar too: the row's geometry is the output's, and the ring
        // is placed in output coordinates.
        window.set_exclusive_zone(-1);
        window.set_decorated(false);
        window.add_css_class("jump-surface");

        let stage = gtk4::Fixed::new();
        let ring = gtk4::Box::builder().css_classes(["jump-ring"]).build();
        ring.set_can_target(false);
        let chord = gtk4::Label::builder().css_classes(["jump-chord"]).build();
        let label = gtk4::Label::builder().css_classes(["jump-label"]).build();
        let detail = gtk4::Label::builder()
            .css_classes(["jump-detail"])
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .build();
        // Centred under the middle workspace: the outer box is as wide as
        // the workspace (`place_ring`), the line itself only as wide as it is.
        let line = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .halign(gtk4::Align::Center)
            .css_classes(["jump-caption"])
            .build();
        line.append(&chord);
        line.append(&label);
        line.append(&detail);
        let caption = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        caption.append(&line);
        line.set_hexpand(true);
        caption.set_can_target(false);
        stage.put(&ring, 0.0, 0.0);
        stage.put(&caption, 0.0, 0.0);
        window.set_child(Some(&stage));

        let this = Rc::new(Jump {
            window,
            stage,
            ring,
            caption,
            chord,
            label,
            detail,
            pin: RefCell::new(None),
            state: RefCell::new(State {
                gesture: Gesture::new(),
                commands: Vec::new(),
                names: Vec::new(),
                rows: Vec::new(),
                row: None,
                origin_xy: (0.0, 0.0),
                cursor: 0,
                watchdog: None,
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

    /// Escape cancels, `p` pins, the modifier coming up commits. A click
    /// left or right of the middle steps that way; on it, commits.
    fn wire(self: &Rc<Self>) {
        let keys = gtk4::EventControllerKey::new();
        {
            let this = self.clone();
            keys.connect_key_pressed(move |_, key, _, _| {
                if key == gtk4::gdk::Key::Escape {
                    this.feed(Ev::Escape);
                    return glib::Propagation::Stop;
                }
                if key == gtk4::gdk::Key::p {
                    this.pin();
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

        let click = gtk4::GestureClick::new();
        {
            let this = self.clone();
            click.connect_released(move |_, _, x, _| {
                let (row, (ox, _)) = {
                    let st = this.state.borrow();
                    (st.row, st.origin_xy)
                };
                let Some(row) = row else { return };
                let (mx, _, mw, _) = row.middle();
                let x = x + ox;
                let ev = if x < mx {
                    Ev::StepBack
                } else if x > mx + mw {
                    Ev::Step
                } else {
                    Ev::SuperReleased
                };
                this.feed(ev);
            });
        }
        self.window.add_controller(click);
    }

    /// `Super+Tab`.
    pub fn step(self: &Rc<Self>) {
        if !self.state.borrow().gesture.is_live() {
            self.begin();
        }
        self.feed(Ev::Step);
    }

    /// `Escape` or `Super+Escape` while the switcher is up.
    pub fn cancel(self: &Rc<Self>) {
        self.feed(Ev::Escape);
    }

    /// `p` or `Super+p` while the switcher is up.
    pub fn pin(self: &Rc<Self>) {
        if self.state.borrow().gesture.is_live() {
            self.pin_selected();
        }
    }

    /// `Super+Shift+Tab`.
    pub fn step_back(self: &Rc<Self>) {
        self.feed(Ev::StepBack);
    }

    /// Read the session and build the row, before the first event.
    ///
    /// Synchronous on the GTK thread on purpose. It is a few round trips over
    /// a unix socket, single-digit milliseconds, and the alternative is a
    /// worker thread whose result arrives after the user has already
    /// released the key.
    fn begin(self: &Rc<Self>) {
        let Some(session) = read_session() else {
            log::warn!("jump: could not read the session; nothing to show");
            return;
        };
        let Session {
            places,
            bindings,
            output,
            geometry,
            tree,
        } = session;
        // This output's workspaces only: the row is drawn on it, and a
        // workspace shown on another output cannot be drawn here too.
        let places: Vec<_> = places.into_iter().filter(|p| p.output == output).collect();
        let apps = |ws: &str| apps_on(&tree, ws);
        let built = rows::rows(&places, &bindings, &apps, &output);

        log::info!(
            "jump: {} places, {} in the row, output {output:?}",
            places.len(),
            built.len()
        );
        let mut st = self.state.borrow_mut();
        st.commands = built.iter().map(|r| r.command.clone()).collect();
        st.names = places
            .iter()
            .take(built.len() + 1)
            .map(|p| p.name.clone())
            .collect();
        st.rows = built;
        st.row = geometry.map(|(o, a)| row::Row::new(o, a));
        st.origin_xy = geometry.map(|(o, _)| (o.0, o.1)).unwrap_or_default();
        st.cursor = 0;
    }

    fn feed(self: &Rc<Self>, ev: Ev) {
        log::debug!("jump: {ev:?}");
        let actions = {
            let mut st = self.state.borrow_mut();
            let commands = st.commands.clone();
            st.gesture.on(ev, &commands)
        };
        // An unmap with no switch after it is a cancel. Known only once every
        // action is in.
        let committed = actions.iter().any(|a| matches!(a, Action::Run(_)));
        let unmapped = actions.iter().any(|a| matches!(a, Action::Unmap));
        for action in actions {
            self.apply(action);
        }
        if unmapped && !committed {
            let st = self.state.borrow();
            if let Some(r) = &st.row {
                send(row::cancel(r, &st.names));
            }
        }
    }

    fn apply(self: &Rc<Self>, action: Action) {
        match action {
            Action::Map => {
                self.place_ring();
                self.window.set_visible(true);
                set_mode(MODE);
                self.arm_watchdog();
                {
                    let st = self.state.borrow();
                    if let Some(r) = &st.row {
                        send(row::open(r, &st.names));
                    }
                }
                // Harness hook: the nested session has no keyboard to let go
                // of, so `SWAYPPLET_JUMP_RELEASE_MS=<ms>` releases Super that
                // long after the switcher maps, down the same path.
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
                let moved = {
                    let mut st = self.state.borrow_mut();
                    let moved = st.cursor != i;
                    st.cursor = i;
                    moved
                };
                self.show_caption(i);
                if moved {
                    let st = self.state.borrow();
                    if let Some(r) = &st.row {
                        send(row::layout(r, &st.names, i));
                    }
                }
                self.arm_watchdog();
            }
            Action::Unmap => {
                self.disarm_watchdog();
                self.window.set_visible(false);
                set_mode("default");
            }
            Action::Run(command) => {
                let (names, cursor, r) = {
                    let st = self.state.borrow();
                    (st.names.clone(), st.cursor, st.row)
                };
                let origin = names.first().cloned().unwrap_or_default();
                // Refuse to move if something already did (a click on the
                // bar, a script): committing on top of that would move you
                // twice. Put the row away instead.
                if focused_workspace().is_some_and(|now| now != origin) {
                    log::debug!("jump: cancelled, something else moved us from {origin:?}");
                    if let Some(r) = &r {
                        send(row::cancel(r, &names));
                    }
                    return;
                }
                // One connection, in order: the others fade where they are,
                // the switch, then the selected one grows to full size. sway
                // keeps a workspace that is already on screen where it is when
                // it is switched to, so nothing blinks between the two.
                let mut cmds = Vec::new();
                let mut after = None;
                if let Some(r) = &r {
                    let (before, grow) = row::commit(r, &names, cursor);
                    cmds.extend(before.iter().map(|(n, l)| l.command(n)));
                    after = grow.map(|(n, l)| l.command(&n));
                }
                cmds.push(command);
                cmds.extend(after);
                crate::sway_ipc::run_commands(cmds);
            }
        }
    }

    /// The ring round the middle workspace, and the caption under it, in the
    /// surface's coordinates.
    fn place_ring(&self) {
        let (r, (ox, oy)) = {
            let st = self.state.borrow();
            (st.row, st.origin_xy)
        };
        let Some(r) = r else {
            self.ring.set_visible(false);
            self.caption.set_visible(false);
            return;
        };
        let (x, y, w, h) = r.middle();
        let (x, y) = (x - ox, y - oy);
        // The ring sits just outside the workspace, so it frames the windows
        // and covers none of them.
        const OUT: f64 = 6.0;
        self.ring
            .set_size_request((w + 2.0 * OUT) as i32, (h + 2.0 * OUT) as i32);
        self.stage.move_(&self.ring, x - OUT, y - OUT);
        self.caption.set_size_request(w as i32, -1);
        self.stage.move_(&self.caption, x, y + h + 14.0);
        self.ring.set_visible(true);
        self.caption.set_visible(true);
    }

    fn show_caption(&self, i: usize) {
        let st = self.state.borrow();
        let Some(row) = st.rows.get(i) else { return };
        self.chord.set_label(row.chord.as_deref().unwrap_or(""));
        self.chord.set_visible(row.chord.is_some());
        self.label.set_label(rows::caption_label(row));
        self.detail.set_label(&row.detail);
        let pinned = st.names.get(i + 1).is_some_and(|n| pin::is_pinned(n));
        if pinned {
            self.ring.add_css_class("pinned");
        } else {
            self.ring.remove_css_class("pinned");
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
        let (name, cursor) = {
            let st = self.state.borrow();
            (st.names.get(st.cursor + 1).cloned(), st.cursor)
        };
        if let (Some(name), Some(pin)) = (name, &*self.pin.borrow()) {
            pin(name);
        }
        self.show_caption(cursor);
    }
}

/// Every look as a command, on one connection, in order.
fn send(looks: Vec<(String, row::Look)>) {
    if looks.is_empty() {
        return;
    }
    crate::sway_ipc::run_commands(looks.iter().map(|(n, l)| l.command(n)).collect());
}

/// Everything the row needs.
struct Session {
    places: Vec<place::Place>,
    bindings: Vec<crate::keybinds::Binding>,
    /// The focused output's name.
    output: String,
    /// That output's rect and its workspace rect, in layout coordinates.
    geometry: Option<(row::Rect, row::Rect)>,
    tree: swayipc::Node,
}

fn read_session() -> Option<Session> {
    let mut conn = crate::sway_ipc::connect().ok()?;
    let tree = conn.get_tree().ok()?;
    let config = conn.get_config().ok().map(|c| c.config).unwrap_or_default();
    let places = place::mru(&tree);
    let output = places.first().map(|p| p.output.clone()).unwrap_or_default();
    let rect = |r: swayipc::Rect| {
        (
            f64::from(r.x),
            f64::from(r.y),
            f64::from(r.width),
            f64::from(r.height),
        )
    };
    let out = conn
        .get_outputs()
        .ok()
        .and_then(|os| os.into_iter().find(|o| o.name == output))
        .map(|o| rect(o.rect));
    let area = conn
        .get_workspaces()
        .ok()
        .and_then(|ws| ws.into_iter().find(|w| w.output == output && w.visible))
        .map(|w| rect(w.rect));
    Some(Session {
        places,
        bindings: crate::keybinds::parse(&config),
        output,
        geometry: out.zip(area),
        tree,
    })
}

/// Synchronous, unlike the other commands here. Each asynchronous command
/// runs on its own thread, so a tap that maps and unmaps within a few
/// milliseconds could reach sway as `default` first and leave the keyboard
/// in the switcher's mode. One round trip on a unix socket is well under a
/// millisecond.
fn set_mode(mode: &str) {
    let done = crate::sway_ipc::connect()
        .map_err(|e| e.to_string())
        .and_then(|mut c| {
            c.run_command(format!("mode {mode}"))
                .map_err(|e| e.to_string())
        });
    if let Err(e) = done {
        log::debug!("jump: mode {mode}: {e}");
    }
}

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
