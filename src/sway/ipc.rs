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

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fs;
use std::hash::{Hash, Hasher};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Instant, SystemTime};

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
    /// A window is fullscreen on the output holding the focused workspace,
    /// whether or not it has focus (quiet by context).
    pub fullscreen_on_focused_output: bool,
    /// Two outputs show the same part of the layout, or wl-mirror is up
    /// (quiet by context).
    pub mirrored: bool,
    /// Current binding mode ("default" at rest; "" only before the first
    /// snapshot). The hazard lane shows non-default modes (increment 7).
    pub binding_mode: String,
    /// How many times sway reloaded its config on this connection. A reload
    /// puts every layer_effects back to the config's, dropping what this
    /// process set at runtime (the glass tuning); a change here is the cue
    /// to set it again.
    pub reloads: u32,
}

// ── Connecting ──────────────────────────────────────────────────────────

/// Open an IPC connection, surviving a sway that restarted under us.
///
/// swayipc resolves the socket from `I3SOCK`/`SWAYSOCK`, and a process keeps
/// the environment it was started with. When systemd restarts swaypplet
/// during a session handover, that variable can still name the *previous*
/// sway's socket: the file is gone, every reconnect fails the same way, and
/// the bar sits there with an empty workspace strip (seen 2026-08-19).
///
/// So the environment is a hint rather than the answer. Try it first — it is
/// right in the normal case and costs one connect — then any
/// `sway-ipc.*.sock` actually lying in `$XDG_RUNTIME_DIR`, newest first. A
/// live socket accepts and a stale one refuses, which is the whole test.
pub fn connect() -> Result<Connection, swayipc::Error> {
    let mut failure = None;
    for path in socket_candidates() {
        match UnixStream::connect(&path) {
            Ok(stream) => return Ok(Connection::from(stream)),
            Err(e) => failure = Some(e),
        }
    }
    Err(failure.map_or(swayipc::Error::SocketNotFound, swayipc::Error::Io))
}

/// The environment's socket paths, then the runtime directory's.
fn socket_candidates() -> Vec<PathBuf> {
    let env = ["I3SOCK", "SWAYSOCK"]
        .iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .collect();
    order_candidates(env, runtime_dir_sockets())
}

/// Every `sway-ipc.<uid>.<pid>.sock` in `$XDG_RUNTIME_DIR`, with the mtime
/// that orders them. Unreadable directory or entry: skip it, the caller
/// still has the environment's path to try.
fn runtime_dir_sockets() -> Vec<(SystemTime, PathBuf)> {
    let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            name.starts_with("sway-ipc.") && name.ends_with(".sock")
        })
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect()
}

/// Order to try paths in: the environment's first, then the discovered ones
/// newest-mtime first — the newest socket belongs to the sway still running.
/// A discovered path equal to an environment one drops out, so the failing
/// path is not retried under a second name.
fn order_candidates(env: Vec<PathBuf>, mut found: Vec<(SystemTime, PathBuf)>) -> Vec<PathBuf> {
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let mut paths = env;
    for (_, path) in found {
        if !paths.contains(&path) {
            paths.push(path);
        }
    }
    paths
}

// ── GTK-side service ────────────────────────────────────────────────────

/// Sway model service; [`Observed`] documents the `Rc` lifetime story.
pub struct SwayService {
    state: Observed<SwayState>,
    /// [`windows_signature`] of the last snapshot.
    windows: Cell<u64>,
    on_windows: RefCell<Vec<Rc<dyn Fn()>>>,
}

impl SwayService {
    pub fn start() -> Rc<Self> {
        let service = Rc::new(Self {
            state: Observed::new(SwayState::default()),
            windows: Cell::new(0),
            on_windows: RefCell::default(),
        });

        let (tx, rx) = async_channel::unbounded::<(SwayState, u64)>();
        std::thread::Builder::new()
            .name("sway-ipc".into())
            .spawn(move || run(tx))
            .expect("spawn sway-ipc thread");

        let for_events = service.clone();
        glib::MainContext::default().spawn_local(async move {
            while let Ok((snapshot, windows)) = rx.recv().await {
                // Window events fire per keystroke in some terminals
                // (title changes) without altering this model; dropping
                // no-op snapshots keeps observers from re-rendering.
                let changed = for_events.state.set_if_changed(snapshot);
                let moved = for_events.windows.replace(windows) != windows;
                if changed || moved {
                    let callbacks: Vec<_> = for_events.on_windows.borrow().clone();
                    for cb in &callbacks {
                        cb();
                    }
                }
            }
        });

        service
    }

    pub fn connect_change(&self, cb: impl Fn() + 'static) {
        self.state.connect_change(cb);
    }

    /// Call `cb` on every change [`Self::connect_change`] reports, and also
    /// when only the windows changed: one opened, closed or moved, resized,
    /// or mapped again under a new identifier. A title change is not one.
    pub fn connect_windows(&self, cb: impl Fn() + 'static) {
        self.on_windows.borrow_mut().push(Rc::new(cb));
    }

    /// Full state snapshot (cloned — the model is a handful of small rows).
    pub fn snapshot(&self) -> SwayState {
        self.state.with(Clone::clone)
    }

    /// The focused output's connector name, from the last snapshot: the
    /// output holding the focused workspace. `None` before the first
    /// snapshot has arrived.
    pub fn focused_output(&self) -> Option<String> {
        self.state.with(|s| {
            s.workspaces
                .iter()
                .find(|w| w.focused)
                .map(|w| w.output.clone())
        })
    }
}

/// The focused output, read from `service` when this process runs one and
/// it has reported, else by [`focused_output`]'s blocking round trip. The
/// panel starts a [`SwayService`] only when it hosts the bar, and the
/// service knows nothing until its first snapshot lands; both cases take the
/// round trip rather than go without an answer.
pub fn focused_output_from(service: Option<&SwayService>) -> Option<String> {
    service
        .and_then(SwayService::focused_output)
        .or_else(focused_output)
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
        move || run_command_blocking(&cmd),
        |result| {
            if let Err(msg) = result {
                log::warn!("{msg}");
            }
            then();
        },
    );
}

/// Run `cmds` in order on one connection, each on its own: a command sway
/// rejects does not stop the ones after it, where a `;`-separated list would
/// end at the first unknown command.
pub fn run_commands(cmds: Vec<String>) {
    crate::spawn::spawn_work(
        move || {
            let Ok(mut c) = connect() else { return };
            for cmd in cmds {
                match c.run_command(&cmd) {
                    Ok(outcomes) => {
                        if let Some(Err(e)) = outcomes.into_iter().find(Result::is_err) {
                            log::debug!("sway ipc: `{cmd}`: {e}");
                        }
                    }
                    Err(e) => log::warn!("sway ipc: `{cmd}` failed: {e}"),
                }
            }
        },
        |()| {},
    );
}

/// [`run_command_then`], telling `then` whether sway accepted the command:
/// for a caller that falls back to something else when it did not.
pub fn run_command_result(cmd: &str, then: impl FnOnce(bool) + 'static) {
    let cmd = cmd.to_string();
    crate::spawn::spawn_work(
        move || run_command_blocking(&cmd),
        |result| {
            if let Err(msg) = &result {
                log::info!("{msg}");
            }
            then(result.is_ok());
        },
    );
}

/// One command on a fresh connection, from a worker thread: sway's refusal
/// of the command is an error like a dead socket is.
pub fn run_command_blocking(cmd: &str) -> Result<(), String> {
    connect()
        .and_then(|mut c| c.run_command(cmd))
        .and_then(|outcomes| outcomes.into_iter().find(Result::is_err).unwrap_or(Ok(())))
        .map_err(|e| format!("sway ipc: command `{cmd}` failed: {e}"))
}

/// The focused output's connector name, or `None` when sway cannot say.
///
/// Blocking. The popup stack has to pin a card's surface to an output
/// *before* the surface exists, so the answer cannot arrive later on a worker
/// thread's callback; in the panel it reads the [`SwayService`] instead
/// ([`focused_output_from`]), and this round trip is the fallback for a
/// process without one. One `get_outputs` round trip on a local socket, with
/// the connection kept between calls so a burst of notifications is not a
/// burst of connects.
///
/// A dropped socket (sway restarted under us) costs one reconnect and then
/// gives up for this call. The caller's fallback is to let the compositor
/// place the surface, which is what it did before this existed.
pub fn focused_output() -> Option<String> {
    thread_local! {
        static CONN: RefCell<Option<Connection>> = const { RefCell::new(None) };
    }
    CONN.with(|slot| {
        let mut slot = slot.borrow_mut();
        for attempt in 0..2 {
            if slot.is_none() {
                match connect() {
                    Ok(c) => *slot = Some(c),
                    Err(e) => {
                        log::warn!("sway ipc: get_outputs cannot connect: {e}");
                        return None;
                    }
                }
            }
            match slot.as_mut()?.get_outputs() {
                Ok(outputs) => return outputs.into_iter().find(|o| o.focused).map(|o| o.name),
                Err(e) => {
                    // The kept connection is the likely casualty, so drop it
                    // and let the second pass reconnect.
                    *slot = None;
                    if attempt == 1 {
                        log::warn!("sway ipc: get_outputs failed: {e}");
                    }
                }
            }
        }
        None
    })
}

/// `get_inputs` as sway sends it, on a fresh connection.
///
/// Blocking, for a worker thread. Raw JSON rather than swayipc's typed
/// reply, which drops `accel_profile` and the keyboard's repeat settings,
/// the values the Input tab shows beside its rows.
pub fn inputs_json() -> Result<serde_json::Value, String> {
    let mut failure = "no sway socket".to_string();
    for path in socket_candidates() {
        match UnixStream::connect(&path) {
            Ok(stream) => {
                return raw_request(stream, GET_INPUTS)
                    .map_err(|e| format!("sway ipc: get_inputs failed: {e}"));
            }
            Err(e) => failure = e.to_string(),
        }
    }
    Err(format!("sway ipc: get_inputs cannot connect: {failure}"))
}

const GET_INPUTS: u32 = 100;

/// One i3-ipc message with an empty payload, and its reply parsed. The
/// header is the magic, the payload length and the type, in host order.
fn raw_request(mut stream: UnixStream, kind: u32) -> std::io::Result<serde_json::Value> {
    use std::io::{Read, Write};
    let mut message = b"i3-ipc".to_vec();
    message.extend_from_slice(&0u32.to_ne_bytes());
    message.extend_from_slice(&kind.to_ne_bytes());
    stream.write_all(&message)?;
    let mut header = [0u8; 14];
    stream.read_exact(&mut header)?;
    let len = u32::from_ne_bytes([header[6], header[7], header[8], header[9]]) as usize;
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body)?;
    Ok(serde_json::from_slice(&body)?)
}

/// The config sway actually loaded, as text.
///
/// Blocking, so it runs on a worker thread (`spawn::spawn_work`); the reply is
/// the whole config in one string and arrives in a few milliseconds. Only the
/// keybinding sheet wants it, and only when it is first opened.
pub fn config_text() -> Result<String, String> {
    connect()
        .and_then(|mut c| c.get_config())
        .map(|config| config.config)
        .map_err(|e| format!("sway ipc: get_config failed: {e}"))
}

// ── Worker thread ───────────────────────────────────────────────────────

fn run(tx: async_channel::Sender<(SwayState, u64)>) {
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
fn session(tx: &async_channel::Sender<(SwayState, u64)>) -> Result<(), swayipc::Error> {
    let mut query = connect()?;
    if tx.send_blocking(snapshot(&mut query)?).is_err() {
        return Ok(());
    }

    // Mode rides the existing subscription — a few events per day, no
    // poll (vision cadence budget).
    let events = connect()?.subscribe([
        EventType::Workspace,
        EventType::Window,
        EventType::Output,
        EventType::Tick,
        EventType::Mode,
    ])?;
    let mut reloads = 0;
    for event in events {
        if let swayipc::Event::Workspace(ws) = event?
            && ws.change == swayipc::WorkspaceChange::Reload
        {
            reloads += 1;
        }
        let (mut state, windows) = snapshot(&mut query)?;
        state.reloads = reloads;
        if tx.send_blocking((state, windows)).is_err() {
            return Ok(());
        }
    }
    Ok(()) // unreachable: the stream only ends by erroring
}

fn snapshot(query: &mut Connection) -> Result<(SwayState, u64), swayipc::Error> {
    let tree = query.get_tree()?;
    let windows = windows_signature(&tree);
    let workspaces = workspace_infos(query.get_workspaces()?);
    let state = SwayState {
        fullscreen_on_focused_output: fullscreen_on_focused_output(&tree, &workspaces),
        mirrored: mirrored(&tree),
        workspaces,
        // From the tree we already fetched — get_outputs would be a third
        // round-trip per event for the same two numbers.
        outputs: output_infos(&tree),
        pid_workspaces: index_tree(&tree),
        focused_fullscreen: focused_fullscreen(&tree),
        binding_mode: query.get_binding_state()?,
        reloads: 0,
    };
    Ok((state, windows))
}

/// A hash of every window's workspace, identity and place: what a live
/// picture of a workspace (`jump::pin`) is built from, and titles left out.
fn windows_signature(root: &Node) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    super::tree::for_each(root, |node, workspace| {
        if let Some(ident) = &node.foreign_toplevel_identifier {
            let r = &node.rect;
            let w = &node.window_rect;
            (
                workspace,
                node.id,
                ident,
                node.visible,
                node.fullscreen_mode,
            )
                .hash(&mut hasher);
            (r.x, r.y, r.width, r.height, w.x, w.y, w.width, w.height).hash(&mut hasher);
        }
    });
    hasher.finish()
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
    super::tree::for_each(root, |node, workspace| {
        // The scratchpad is a pseudo-workspace named __i3_scratch; its
        // views are off screen and must stay out of the task map.
        let workspace = workspace.filter(|name| *name != "__i3_scratch");
        if let (Some(pid), Some(ws)) = (node.pid, workspace) {
            pids.insert(pid, ws.to_string());
        }
    });
    pids
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

/// A view is fullscreen on the output that holds the focused workspace: on
/// a workspace visible there, or anywhere as a global fullscreen. The view
/// need not have focus. The panel, a layer surface, takes the keyboard
/// without entering the tree, and a floating window can sit on top of the
/// fullscreen one; the screen is still showing it.
fn fullscreen_on_focused_output(root: &Node, workspaces: &[WorkspaceInfo]) -> bool {
    let Some(output) = workspaces.iter().find(|w| w.focused).map(|w| &w.output) else {
        return false;
    };
    let visible: Vec<&str> = workspaces
        .iter()
        .filter(|w| w.visible && &w.output == output)
        .map(|w| w.name.as_str())
        .collect();
    super::tree::walk(root, &mut |node, workspace| {
        let view = matches!(node.node_type, NodeType::Con | NodeType::FloatingCon);
        let here = workspace.is_some_and(|w| visible.contains(&w));
        match node.fullscreen_mode.unwrap_or(0) {
            2 if view => std::ops::ControlFlow::Break(()),
            1 if view && here => std::ops::ControlFlow::Break(()),
            _ => std::ops::ControlFlow::Continue(()),
        }
    })
    .is_some()
}

/// wl-mirror's app id: the usual way to mirror an output on sway, which has
/// no mirroring of its own.
const WL_MIRROR: &str = "at.yrlf.wl_mirror";

/// Two outputs overlap in the layout, so both show the same pixels (the
/// way to duplicate a display with sway's own configuration), or a
/// wl-mirror window is open.
fn mirrored(root: &Node) -> bool {
    let rects: Vec<&swayipc::Rect> = root
        .nodes
        .iter()
        .filter(|n| n.node_type == NodeType::Output)
        .filter(|n| n.name.as_deref().is_some_and(|s| !s.starts_with("__")))
        .map(|n| &n.rect)
        .collect();
    let overlap = |a: &swayipc::Rect, b: &swayipc::Rect| {
        a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height
    };
    let overlapping = rects
        .iter()
        .enumerate()
        .any(|(i, a)| rects[i + 1..].iter().any(|b| overlap(a, b)));
    overlapping
        || super::tree::walk(root, &mut |node, _| {
            if node.app_id.as_deref() == Some(WL_MIRROR) {
                std::ops::ControlFlow::Break(())
            } else {
                std::ops::ControlFlow::Continue(())
            }
        })
        .is_some()
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

    fn ws(name: &str, output: &str, focused: bool, visible: bool) -> WorkspaceInfo {
        WorkspaceInfo {
            num: name.parse().unwrap_or(-1),
            name: name.into(),
            output: output.into(),
            focused,
            urgent: false,
            visible,
        }
    }

    /// Two outputs; workspace 1 on eDP-1, 2 and 3 on DP-1 (2 visible). The
    /// fullscreen view sits on `on` with `mode`.
    fn two_outputs(on: &str, mode: u8) -> Node {
        let view = |ws: &str, id: i64| {
            let mode = if ws == on { mode } else { 0 };
            node(json!({ "id": id, "pid": id, "name": "v", "fullscreen_mode": mode }))
        };
        let workspace = |name: &str, id: i64| {
            node(json!({ "type": "workspace", "name": name, "nodes": [view(name, id)] }))
        };
        tree(node(json!({
            "type": "root",
            "nodes": [
                node(json!({ "type": "output", "name": "eDP-1",
                             "rect": {"x": 0, "y": 0, "width": 1920, "height": 1200},
                             "nodes": [workspace("1", 10)] })),
                node(json!({ "type": "output", "name": "DP-1",
                             "rect": {"x": 1920, "y": 0, "width": 2560, "height": 1440},
                             "nodes": [workspace("2", 20), workspace("3", 30)] })),
            ],
        })))
    }

    #[test]
    fn fullscreen_counts_on_the_focused_output_only_and_only_where_visible() {
        let focused_on_edp = [
            ws("1", "eDP-1", true, true),
            ws("2", "DP-1", false, true),
            ws("3", "DP-1", false, false),
        ];
        assert!(fullscreen_on_focused_output(
            &two_outputs("1", 1),
            &focused_on_edp
        ));
        // A video fullscreen on the other screen does not quiet this one.
        assert!(!fullscreen_on_focused_output(
            &two_outputs("2", 1),
            &focused_on_edp
        ));
        // Global fullscreen covers every output.
        assert!(fullscreen_on_focused_output(
            &two_outputs("2", 2),
            &focused_on_edp
        ));

        let focused_on_dp = [
            ws("1", "eDP-1", false, true),
            ws("2", "DP-1", true, true),
            ws("3", "DP-1", false, false),
        ];
        assert!(fullscreen_on_focused_output(
            &two_outputs("2", 1),
            &focused_on_dp
        ));
        // Workspace 3 is on the focused output but not showing.
        assert!(!fullscreen_on_focused_output(
            &two_outputs("3", 1),
            &focused_on_dp
        ));
        assert!(!fullscreen_on_focused_output(
            &two_outputs("2", 0),
            &focused_on_dp
        ));
    }

    #[test]
    fn overlapping_outputs_or_wl_mirror_count_as_mirrored() {
        assert!(!mirrored(&two_outputs("1", 0)));
        let same_place = tree(node(json!({
            "type": "root",
            "nodes": [
                node(json!({ "type": "output", "name": "eDP-1",
                             "rect": {"x": 0, "y": 0, "width": 1920, "height": 1200} })),
                node(json!({ "type": "output", "name": "HDMI-A-1",
                             "rect": {"x": 0, "y": 0, "width": 1920, "height": 1080} })),
            ],
        })));
        assert!(mirrored(&same_place));
        let wl_mirror = tree(node(json!({
            "type": "root",
            "nodes": [node(json!({ "type": "output", "name": "HDMI-A-1",
                "nodes": [node(json!({ "type": "workspace", "name": "9",
                    "nodes": [node(json!({ "id": 5, "app_id": WL_MIRROR }))] }))] }))],
        })));
        assert!(mirrored(&wl_mirror));
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
    fn candidates_try_the_environment_then_the_newest_socket() {
        let t = |secs| SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(secs);
        let env = vec![PathBuf::from("/run/user/1000/sway-ipc.1000.100.sock")];
        let found = vec![
            (
                t(10),
                PathBuf::from("/run/user/1000/sway-ipc.1000.100.sock"),
            ),
            (
                t(20),
                PathBuf::from("/run/user/1000/sway-ipc.1000.300.sock"),
            ),
            (
                t(15),
                PathBuf::from("/run/user/1000/sway-ipc.1000.200.sock"),
            ),
        ];
        assert_eq!(
            order_candidates(env, found),
            vec![
                // The stale environment path goes first and only once...
                PathBuf::from("/run/user/1000/sway-ipc.1000.100.sock"),
                // ...then the live sockets, newest sway first.
                PathBuf::from("/run/user/1000/sway-ipc.1000.300.sock"),
                PathBuf::from("/run/user/1000/sway-ipc.1000.200.sock"),
            ]
        );
    }
}
