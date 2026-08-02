//! Persistent sway IPC service — the bar's source for workspace, window and
//! binding-mode state.
//!
//! One background thread owns two IPC sockets: an event subscription
//! (workspace, window, mode, output, tick) and a query connection. On every
//! event it rebuilds the full model from `get_workspaces` + `get_tree` and
//! ships it to the GTK thread over an async channel — snapshots, not deltas,
//! so a missed event can never leave the cache stale. If the socket drops
//! (sway restart) the thread reconnects with exponential backoff.
//!
//! GTK side mirrors `notifications::store`: `connect_change` observers plus
//! state getters on an `Rc` service living on the main thread.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use swayipc::{Connection, Event, EventType, Node, NodeType};

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

/// Full cached model, replaced wholesale on every sway event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwayState {
    pub workspaces: Vec<WorkspaceInfo>,
    /// Title of the focused view; `None` when focus sits on an empty
    /// workspace or a bare container.
    pub focused_title: Option<String>,
    /// Active binding mode ("default" when none is engaged).
    pub mode: String,
    /// pid → workspace name for every view in the tree (task-pill lookup).
    pub pid_workspaces: HashMap<i32, String>,
}

impl Default for SwayState {
    fn default() -> Self {
        Self {
            workspaces: Vec::new(),
            focused_title: None,
            mode: "default".into(),
            pid_workspaces: HashMap::new(),
        }
    }
}

// ── GTK-side service ────────────────────────────────────────────────────

/// Lives on the GTK main thread behind `Rc`; the event loop spawned by
/// [`SwayService::start`] holds a strong ref, so the service persists for
/// the process (same lifetime story as `bar::BarManager`).
pub struct SwayService {
    state: RefCell<SwayState>,
    on_change: RefCell<Vec<Rc<dyn Fn()>>>,
}

impl SwayService {
    pub fn start() -> Rc<Self> {
        let service = Rc::new(Self {
            state: RefCell::new(SwayState::default()),
            on_change: RefCell::new(Vec::new()),
        });

        let (tx, rx) = async_channel::unbounded::<SwayState>();
        std::thread::Builder::new()
            .name("sway-ipc".into())
            .spawn(move || run(tx))
            .expect("spawn sway-ipc thread");

        let for_events = service.clone();
        glib::MainContext::default().spawn_local(async move {
            while let Ok(snapshot) = rx.recv().await {
                // Title events fire per keystroke in some terminals; skip
                // no-op snapshots so observers don't re-render for nothing.
                if *for_events.state.borrow() == snapshot {
                    continue;
                }
                *for_events.state.borrow_mut() = snapshot;
                // Clone the list and fire outside the borrow — a callback
                // may re-enter a getter or connect another observer.
                let callbacks: Vec<_> = for_events.on_change.borrow().clone();
                for cb in &callbacks {
                    cb();
                }
            }
        });

        service
    }

    pub fn connect_change(&self, cb: impl Fn() + 'static) {
        self.on_change.borrow_mut().push(Rc::new(cb));
    }

    /// Full state snapshot (cloned — the model is a handful of small rows).
    pub fn snapshot(&self) -> SwayState {
        self.state.borrow().clone()
    }

    // Getters below are consumed by the bar widgets as they land.

    pub fn workspaces(&self) -> Vec<WorkspaceInfo> {
        self.state.borrow().workspaces.clone()
    }

    #[allow(dead_code)]
    pub fn focused_title(&self) -> Option<String> {
        self.state.borrow().focused_title.clone()
    }

    #[allow(dead_code)]
    pub fn binding_mode(&self) -> String {
        self.state.borrow().mode.clone()
    }
}

/// Fire a sway command (workspace switch on bar click, etc.) without
/// blocking the GTK thread. Each call opens a throwaway connection —
/// commands arrive at click rate, and a fresh socket can't be left wedged
/// by a sway restart the way a cached one could.
pub fn run_command(cmd: &str) {
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
        },
    );
}

// ── Worker thread ───────────────────────────────────────────────────────

fn run(tx: async_channel::Sender<SwayState>) {
    const INITIAL: Duration = Duration::from_millis(250);
    const MAX: Duration = Duration::from_secs(8);
    let mut backoff = INITIAL;
    loop {
        let started = Instant::now();
        match session(&tx) {
            Ok(()) => return, // receiver gone — process is shutting down
            Err(e) => log::warn!("sway ipc: {e}; reconnecting in {backoff:?}"),
        }
        std::thread::sleep(backoff);
        // A session that lived a while was a real connection that died
        // (sway restart), not a refused socket — restart the ladder.
        backoff = if started.elapsed() > Duration::from_secs(10) {
            INITIAL
        } else {
            (backoff * 2).min(MAX)
        };
    }
}

/// One connection lifetime. `Ok(())` means the GTK side hung up; `Err`
/// means the socket dropped and the caller should reconnect.
fn session(tx: &async_channel::Sender<SwayState>) -> Result<(), swayipc::Error> {
    let mut query = Connection::new()?;
    // Binding mode arrives only as an event, so it is carried across
    // snapshots instead of queried. A reconnect resets it to "default",
    // which matches sway: a restart leaves no mode engaged.
    let mut mode = String::from("default");
    if tx.send_blocking(snapshot(&mut query, &mode)?).is_err() {
        return Ok(());
    }

    let events = Connection::new()?.subscribe([
        EventType::Workspace,
        EventType::Window,
        EventType::Mode,
        EventType::Output,
        EventType::Tick,
    ])?;
    for event in events {
        if let Event::Mode(m) = event? {
            mode = m.change;
        }
        if tx.send_blocking(snapshot(&mut query, &mode)?).is_err() {
            return Ok(());
        }
    }
    Ok(()) // unreachable: the stream only ends by erroring
}

fn snapshot(query: &mut Connection, mode: &str) -> Result<SwayState, swayipc::Error> {
    let workspaces = workspace_infos(query.get_workspaces()?);
    let (focused_title, pid_workspaces) = index_tree(&query.get_tree()?);
    Ok(SwayState {
        workspaces,
        focused_title,
        mode: mode.to_string(),
        pid_workspaces,
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

/// Walk the layout tree once, collecting the focused view's title and a
/// pid → workspace map.
fn index_tree(root: &Node) -> (Option<String>, HashMap<i32, String>) {
    let mut title = None;
    let mut pids = HashMap::new();
    walk(root, None, &mut title, &mut pids);
    (title, pids)
}

fn walk(
    node: &Node,
    workspace: Option<&str>,
    title: &mut Option<String>,
    pids: &mut HashMap<i32, String>,
) {
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
    // Only views carry a pid; a focused workspace or split container is
    // not a window and must not produce a title.
    if node.focused && node.pid.is_some() {
        *title = node.name.clone();
    }
    for child in node.nodes.iter().chain(&node.floating_nodes) {
        walk(child, workspace, title, pids);
    }
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

    /// Root → one output → two workspaces; "1" holds a focused kitty and a
    /// floating pavucontrol, "2" holds an unfocused firefox.
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
    fn focused_view_title_is_found() {
        let (title, _) = index_tree(&contract());
        assert_eq!(title.as_deref(), Some("kitty"));
    }

    #[test]
    fn pid_map_covers_tiled_and_floating_views() {
        let (_, pids) = index_tree(&contract());
        assert_eq!(pids.get(&100).map(String::as_str), Some("1"));
        assert_eq!(pids.get(&101).map(String::as_str), Some("1"));
        assert_eq!(pids.get(&102).map(String::as_str), Some("2"));
        assert_eq!(pids.len(), 3);
    }

    #[test]
    fn focused_empty_workspace_yields_no_title() {
        let root = tree(node(json!({
            "type": "root",
            "nodes": [node(json!({
                "type": "output",
                "name": "eDP-1",
                "nodes": [node(json!({
                    "type": "workspace", "name": "3", "focused": true,
                }))],
            }))],
        })));
        let (title, pids) = index_tree(&root);
        assert_eq!(title, None);
        assert!(pids.is_empty());
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
        let (title, pids) = index_tree(&root);
        assert_eq!(title, None);
        assert!(pids.is_empty());
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

    #[test]
    fn default_state_starts_in_default_mode() {
        assert_eq!(SwayState::default().mode, "default");
    }
}
