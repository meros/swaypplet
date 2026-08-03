//! Persistent sway IPC service — the bar's source for workspace and
//! window state.
//!
//! One background thread owns two IPC sockets: an event subscription
//! (workspace, window, output, tick) and a query connection. On every
//! event it rebuilds the full model from `get_workspaces` + `get_tree` and
//! ships it to the GTK thread over an async channel — snapshots, not
//! deltas, so a missed event can never leave the cache stale. If the
//! socket drops (sway restart) the thread reconnects with exponential
//! backoff.
//!
//! GTK side is an [`Observed`] snapshot behind an `Rc` service (shared
//! skeleton in `crate::service`).

use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

use swayipc::{Connection, EventType, Node, NodeType};

use crate::service::{Backoff, Observed};

// ── Model ───────────────────────────────────────────────────────────────

/// One sway workspace, reduced to what the bar needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceInfo {
    /// Leading number, or -1 for purely named workspaces.
    pub num: i32,
    pub name: String,
    pub output: String,
    pub focused: bool,
    pub urgent: bool,
    /// Showing on some output right now (multi-monitor: several can be
    /// visible while only one is focused).
    pub visible: bool,
}

/// One connected output and where it sits in the layout, so consumers can
/// order per-output UI the way the screens actually stand on the desk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputInfo {
    pub name: String,
    pub x: i32,
    pub y: i32,
}

/// Full cached model, replaced wholesale on every sway event.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SwayState {
    pub workspaces: Vec<WorkspaceInfo>,
    /// Connected outputs in tree order; [`OutputInfo`] carries placement.
    pub outputs: Vec<OutputInfo>,
    /// pid → workspace name for every view in the tree (task-pill lookup).
    pub pid_workspaces: HashMap<i32, String>,
    /// The focused view is fullscreen — OSD interjections route to the
    /// center card instead of the bar (docs/BAR_VISION.md, increment 5).
    pub focused_fullscreen: bool,
    /// Current binding mode ("default" at rest; "" only before the first
    /// snapshot). The hazard lane shows non-default modes (increment 7).
    pub binding_mode: String,
}

// ── GTK-side service ────────────────────────────────────────────────────

/// Sway model service; [`Observed`] documents the `Rc` lifetime story.
pub struct SwayService {
    state: Observed<SwayState>,
}

impl SwayService {
    pub fn start() -> Rc<Self> {
        let service = Rc::new(Self {
            state: Observed::new(SwayState::default()),
        });

        let (tx, rx) = async_channel::unbounded::<SwayState>();
        std::thread::Builder::new()
            .name("sway-ipc".into())
            .spawn(move || run(tx))
            .expect("spawn sway-ipc thread");

        let for_events = service.clone();
        glib::MainContext::default().spawn_local(async move {
            while let Ok(snapshot) = rx.recv().await {
                // Window events fire per keystroke in some terminals
                // (title changes) without altering this model; dropping
                // no-op snapshots keeps observers from re-rendering.
                for_events.state.set_if_changed(snapshot);
            }
        });

        service
    }

    pub fn connect_change(&self, cb: impl Fn() + 'static) {
        self.state.connect_change(cb);
    }

    /// Full state snapshot (cloned — the model is a handful of small rows).
    pub fn snapshot(&self) -> SwayState {
        self.state.with(Clone::clone)
    }
}

/// Fire a sway command (workspace switch on bar click, etc.) without
/// blocking the GTK thread. Each call opens a throwaway connection —
/// commands arrive at click rate, and a fresh socket can't be left wedged
/// by a sway restart the way a cached one could.
pub fn run_command(cmd: &str) {
    run_command_then(cmd, || {});
}

/// [`run_command`] plus a completion hook that runs on the GTK thread once
/// sway has replied — the ordering guarantee callers need when a command has
/// to land *before* the next thing they do (see `anim::set_layer_blur`).
/// `then` runs on failure too, so a wedged socket can never strand a caller.
pub fn run_command_then(cmd: &str, then: impl FnOnce() + 'static) {
    let cmd = cmd.to_string();
    crate::spawn::spawn_work(
        move || {
            Connection::new()
                .and_then(|mut c| c.run_command(&cmd))
                .and_then(|outcomes| outcomes.into_iter().find(Result::is_err).unwrap_or(Ok(())))
                .map_err(|e| format!("sway ipc: command `{cmd}` failed: {e}"))
        },
        |result| {
            if let Err(msg) = result {
                log::warn!("{msg}");
            }
            then();
        },
    );
}

// ── Worker thread ───────────────────────────────────────────────────────

fn run(tx: async_channel::Sender<SwayState>) {
    let mut backoff = Backoff::new();
    loop {
        let started = Instant::now();
        match session(&tx) {
            Ok(()) => return, // receiver gone — process is shutting down
            Err(e) => {
                let delay = backoff.next_delay(started.elapsed());
                log::warn!("sway ipc: {e}; reconnecting in {delay:?}");
                std::thread::sleep(delay);
            }
        }
    }
}

/// One connection lifetime. `Ok(())` means the GTK side hung up; `Err`
/// means the socket dropped and the caller should reconnect.
fn session(tx: &async_channel::Sender<SwayState>) -> Result<(), swayipc::Error> {
    let mut query = Connection::new()?;
    if tx.send_blocking(snapshot(&mut query)?).is_err() {
        return Ok(());
    }

    // Mode rides the existing subscription — a few events per day, no
    // poll (vision cadence budget).
    let events = Connection::new()?.subscribe([
        EventType::Workspace,
        EventType::Window,
        EventType::Output,
        EventType::Tick,
        EventType::Mode,
    ])?;
    for event in events {
        event?;
        if tx.send_blocking(snapshot(&mut query)?).is_err() {
            return Ok(());
        }
    }
    Ok(()) // unreachable: the stream only ends by erroring
}

fn snapshot(query: &mut Connection) -> Result<SwayState, swayipc::Error> {
    let tree = query.get_tree()?;
    Ok(SwayState {
        workspaces: workspace_infos(query.get_workspaces()?),
        // From the tree we already fetched — get_outputs would be a third
        // round-trip per event for the same two numbers.
        outputs: output_infos(&tree),
        pid_workspaces: index_tree(&tree),
        focused_fullscreen: focused_fullscreen(&tree),
        binding_mode: query.get_binding_state()?,
    })
}

// ── Pure helpers (unit-tested below) ────────────────────────────────────

fn workspace_infos(reply: Vec<swayipc::Workspace>) -> Vec<WorkspaceInfo> {
    reply
        .into_iter()
        .map(|w| WorkspaceInfo {
            num: w.num,
            name: w.name,
            output: w.output,
            focused: w.focused,
            urgent: w.urgent,
            visible: w.visible,
        })
        .collect()
}

/// The tree's output children, minus sway's `__i3` pseudo-output (which
/// hosts the scratchpad and has no place on any desk).
fn output_infos(root: &Node) -> Vec<OutputInfo> {
    root.nodes
        .iter()
        .filter(|n| n.node_type == NodeType::Output)
        .filter_map(|n| {
            let name = n.name.clone().filter(|s| !s.starts_with("__"))?;
            Some(OutputInfo {
                name,
                x: n.rect.x,
                y: n.rect.y,
            })
        })
        .collect()
}

/// Walk the layout tree once, collecting a pid → workspace map.
fn index_tree(root: &Node) -> HashMap<i32, String> {
    let mut pids = HashMap::new();
    walk(root, None, &mut pids);
    pids
}

fn walk(node: &Node, workspace: Option<&str>, pids: &mut HashMap<i32, String>) {
    let workspace = if node.node_type == NodeType::Workspace {
        // The scratchpad is a pseudo-workspace named __i3_scratch; its
        // views are off screen and must stay out of the task map.
        match node.name.as_deref() {
            Some("__i3_scratch") | None => None,
            name => name,
        }
    } else {
        workspace
    };
    if let (Some(pid), Some(ws)) = (node.pid, workspace) {
        pids.insert(pid, ws.to_string());
    }
    for child in node.nodes.iter().chain(&node.floating_nodes) {
        walk(child, workspace, pids);
    }
}

/// True when the focused node is fullscreen (`fullscreen_mode` 1 =
/// workspace, 2 = global; sway reports 0/absent otherwise).
fn focused_fullscreen(node: &Node) -> bool {
    if node.focused && node.fullscreen_mode.unwrap_or(0) != 0 {
        return true;
    }
    node.nodes
        .iter()
        .chain(&node.floating_nodes)
        .any(focused_fullscreen)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    /// Minimal valid node JSON with `extra` merged over it — swayipc's
    /// `Node` is `#[non_exhaustive]`, so fixtures go through serde like the
    /// real replies do.
    fn node(extra: Value) -> Value {
        let rect = json!({"x": 0, "y": 0, "width": 0, "height": 0});
        let mut base = json!({
            "id": 1,
            "type": "con",
            "border": "none",
            "current_border_width": 0,
            "layout": "splith",
            "orientation": "none",
            "rect": rect,
            "window_rect": rect,
            "deco_rect": rect,
            "geometry": rect,
            "urgent": false,
            "focused": false,
            "focus": [],
            "floating_nodes": [],
            "sticky": false,
        });
        let Value::Object(extra) = extra else {
            panic!("extra must be an object")
        };
        base.as_object_mut().unwrap().extend(extra);
        base
    }

    fn tree(v: Value) -> Node {
        serde_json::from_value(v).expect("valid node fixture")
    }

    fn workspace_reply(v: Value) -> Vec<swayipc::Workspace> {
        serde_json::from_value(v).expect("valid workspace fixture")
    }

    /// Root → one output → two workspaces; "1" holds a tiled kitty and a
    /// floating pavucontrol, "2" holds a firefox.
    fn contract() -> Node {
        tree(node(json!({
            "type": "root",
            "nodes": [node(json!({
                "type": "output",
                "name": "eDP-1",
                "nodes": [
                    node(json!({
                        "type": "workspace",
                        "name": "1",
                        "num": 1,
                        "nodes": [node(json!({
                            "id": 10, "pid": 100, "name": "kitty", "focused": true,
                        }))],
                        "floating_nodes": [node(json!({
                            "id": 11, "type": "floating_con", "pid": 101,
                            "name": "pavucontrol",
                        }))],
                    })),
                    node(json!({
                        "type": "workspace",
                        "name": "2",
                        "num": 2,
                        "nodes": [node(json!({
                            "id": 12, "pid": 102, "name": "firefox",
                        }))],
                    })),
                ],
            }))],
        })))
    }

    #[test]
    fn pid_map_covers_tiled_and_floating_views() {
        let pids = index_tree(&contract());
        assert_eq!(pids.get(&100).map(String::as_str), Some("1"));
        assert_eq!(pids.get(&101).map(String::as_str), Some("1"));
        assert_eq!(pids.get(&102).map(String::as_str), Some("2"));
        assert_eq!(pids.len(), 3);
    }

    #[test]
    fn scratchpad_views_stay_out_of_the_pid_map() {
        let root = tree(node(json!({
            "type": "root",
            "nodes": [node(json!({
                "type": "output",
                "name": "__i3",
                "nodes": [node(json!({
                    "type": "workspace",
                    "name": "__i3_scratch",
                    "floating_nodes": [node(json!({"pid": 200, "name": "stash"}))],
                }))],
            }))],
        })));
        assert!(index_tree(&root).is_empty());
    }

    #[test]
    fn fullscreen_flag_follows_the_focused_node_only() {
        // The contract tree's focused kitty is not fullscreen.
        assert!(!focused_fullscreen(&contract()));

        let fs = |focused: bool, mode: u8| {
            tree(node(json!({
                "type": "root",
                "nodes": [node(json!({
                    "type": "workspace",
                    "name": "1",
                    "nodes": [node(json!({
                        "id": 10, "pid": 100, "name": "mpv",
                        "focused": focused, "fullscreen_mode": mode,
                    }))],
                }))],
            })))
        };
        assert!(focused_fullscreen(&fs(true, 1)));
        // Global fullscreen counts too.
        assert!(focused_fullscreen(&fs(true, 2)));
        // Fullscreen but not focused: the OSD may still use the bar.
        assert!(!focused_fullscreen(&fs(false, 1)));
        assert!(!focused_fullscreen(&fs(true, 0)));
    }

    #[test]
    fn workspace_reply_maps_fields_through() {
        let rect = json!({"x": 0, "y": 0, "width": 0, "height": 0});
        let infos = workspace_infos(workspace_reply(json!([
            {
                "id": 1, "num": 1, "name": "1", "visible": true, "focused": true,
                "urgent": false, "rect": rect, "output": "eDP-1",
            },
            {
                // Named workspace: sway reports num -1.
                "id": 2, "num": -1, "name": "mail", "visible": false,
                "focused": false, "urgent": true, "rect": rect, "output": "DP-3",
            },
        ])));
        assert_eq!(
            infos,
            vec![
                WorkspaceInfo {
                    num: 1,
                    name: "1".into(),
                    output: "eDP-1".into(),
                    focused: true,
                    urgent: false,
                    visible: true,
                },
                WorkspaceInfo {
                    num: -1,
                    name: "mail".into(),
                    output: "DP-3".into(),
                    focused: false,
                    urgent: true,
                    visible: false,
                },
            ]
        );
    }
}
