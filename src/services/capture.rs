//! What is being captured: a screen cast, or the camera. Read from
//! PipeWire's node graph, through `pw-dump --monitor`.
//!
//! # Why the graph, and why `pw-dump`
//!
//! Neither condition has a signal of its own (history/shell-ideas-2026-08.md,
//! item 4): v4l2 has no in-use broadcast, and the ScreenCast portal can start
//! a cast but not list the live ones. Both are plain in PipeWire's graph. A
//! screen cast is a `Video/Source` node that is no device: the wlr portal
//! makes one per session (`xdpw-stream`) and removes it when the session
//! ends. The camera is a `Video/Source` node from a device API (`libcamera`,
//! `v4l2`) whose state is `running`.
//!
//! libpipewire itself cannot be linked here (`services::audio` explains the
//! bindgen collision), and the PulseAudio protocol that the audio service
//! uses carries no video. `pw-dump --monitor` is PipeWire's own client
//! printing the graph as JSON and then each change as it happens, so this is
//! still pushed rather than polled: one child process blocked in its own
//! loop, one thread here blocked on its pipe, and nothing wakes while the
//! graph is still.
//!
//! What it cannot see: a capture that never enters PipeWire, such as OBS
//! reading the screen directly over `wlr-screencopy`. The portal is the path
//! every browser and call app takes.
//!
//! `SWAYPPLET_PW_DUMP` names another program to run instead, for the
//! harness: a script that prints a portal stream and later its removal
//! drives the whole path without a real cast.

use std::collections::HashMap;
use std::io::BufReader;
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::time::Instant;

use serde_json::Value;

use crate::service::{Backoff, Observed};

/// The two things the graph says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CaptureState {
    /// A screen-cast stream exists.
    pub screen: bool,
    /// A camera is streaming.
    pub camera: bool,
}

pub struct CaptureService {
    state: Observed<CaptureState>,
}

impl CaptureService {
    pub fn start() -> Rc<Self> {
        let service = Rc::new(Self {
            state: Observed::new(CaptureState::default()),
        });
        let (tx, rx) = async_channel::unbounded::<CaptureState>();
        std::thread::Builder::new()
            .name("capture".into())
            .spawn(move || run(&tx))
            .map_err(|e| log::error!("capture: could not start thread: {e}"))
            .ok();
        let for_recv = service.clone();
        glib::spawn_future_local(async move {
            while let Ok(state) = rx.recv().await {
                if state != for_recv.snapshot() {
                    log::info!("capture: {state:?}");
                }
                for_recv.state.set_if_changed(state);
            }
        });
        service
    }

    pub fn connect_change(&self, cb: impl Fn() + 'static) {
        self.state.connect_change(cb);
    }

    pub fn snapshot(&self) -> CaptureState {
        self.state.with(|s| *s)
    }
}

// ── The thread ──────────────────────────────────────────────────────────

fn run(tx: &async_channel::Sender<CaptureState>) {
    let program = std::env::var("SWAYPPLET_PW_DUMP").unwrap_or_else(|_| "pw-dump".into());
    let mut backoff = Backoff::new();
    loop {
        let started = Instant::now();
        match session(&program, tx) {
            Ok(()) => return,
            Err(Fatal(e)) => {
                log::warn!("capture: {e}; screen sharing and the camera go unseen");
                return;
            }
            Err(Retry(e)) => {
                // Whatever was live went with the process that saw it.
                if tx.send_blocking(CaptureState::default()).is_err() {
                    return;
                }
                let delay = backoff.next_delay(started.elapsed());
                log::warn!("capture: {e}; restarting in {delay:?}");
                std::thread::sleep(delay);
            }
        }
    }
}

enum Failure {
    /// Not worth another try: the program is not there.
    Fatal(String),
    /// The child ended or said something unreadable: start it again.
    Retry(String),
}
use Failure::{Fatal, Retry};

/// One child's lifetime. `Ok(())` means the GTK side is gone.
fn session(program: &str, tx: &async_channel::Sender<CaptureState>) -> Result<(), Failure> {
    let mut child = Command::new(program)
        .args(["--monitor", "--no-colors"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => Fatal(format!("{program} not found")),
            _ => Retry(format!("{program}: {e}")),
        })?;
    let stdout = child.stdout.take().ok_or(Retry("no stdout".into()))?;
    let mut graph = Graph::default();
    let mut last = None;
    let batches =
        serde_json::Deserializer::from_reader(BufReader::new(stdout)).into_iter::<Vec<Value>>();
    let result = (|| {
        for batch in batches {
            let batch = batch.map_err(|e| Retry(format!("{program}: {e}")))?;
            graph.apply(&batch);
            let state = graph.state();
            if last != Some(state) {
                last = Some(state);
                if tx.send_blocking(state).is_err() {
                    return Ok(());
                }
            }
        }
        Err(Retry(format!("{program} exited")))
    })();
    let _ = child.kill();
    let _ = child.wait();
    result
}

// ── The graph (unit-tested below) ───────────────────────────────────────

/// What one video node is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Screen,
    Camera,
}

#[derive(Debug, Clone)]
struct Node {
    kind: Kind,
    running: bool,
}

/// The video nodes, by PipeWire object id.
#[derive(Debug, Default)]
struct Graph {
    nodes: HashMap<u64, Node>,
}

impl Graph {
    /// One batch of `pw-dump` output: the whole graph at first, then each
    /// change. A removed object is `{"id": N, "info": null}`. An update may
    /// carry only what changed, so a node that already has a kind keeps it
    /// when its props are absent.
    fn apply(&mut self, batch: &[Value]) {
        for object in batch {
            let Some(id) = object.get("id").and_then(Value::as_u64) else {
                continue;
            };
            let info = object.get("info");
            if info.is_none_or(Value::is_null) {
                self.nodes.remove(&id);
                continue;
            }
            if object.get("type").and_then(Value::as_str) != Some("PipeWire:Interface:Node") {
                continue;
            }
            let info = info.unwrap_or(&Value::Null);
            let state = info.get("state").and_then(Value::as_str);
            let kind = info.get("props").map(kind_of);
            match kind {
                Some(None) => {
                    self.nodes.remove(&id);
                }
                Some(Some(kind)) => {
                    let running = match state {
                        Some(state) => state == "running",
                        None => self.nodes.get(&id).is_some_and(|n| n.running),
                    };
                    self.nodes.insert(id, Node { kind, running });
                }
                None => {
                    if let (Some(node), Some(state)) = (self.nodes.get_mut(&id), state) {
                        node.running = state == "running";
                    }
                }
            }
        }
    }

    fn state(&self) -> CaptureState {
        CaptureState {
            screen: self.nodes.values().any(|n| n.kind == Kind::Screen),
            camera: self
                .nodes
                .values()
                .any(|n| n.kind == Kind::Camera && n.running),
        }
    }
}

/// A node's kind from its props, or `None` for anything that is not video.
fn kind_of(props: &Value) -> Option<Kind> {
    let get = |k: &str| props.get(k).and_then(Value::as_str);
    if get("media.class") != Some("Video/Source") {
        return None;
    }
    let device = get("device.api").is_some() || get("api.v4l2.path").is_some();
    if device || get("media.role") == Some("Camera") {
        Some(Kind::Camera)
    } else {
        Some(Kind::Screen)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn node(id: u64, state: &str, props: Value) -> Value {
        json!({ "id": id, "type": "PipeWire:Interface:Node",
                "info": { "state": state, "props": props } })
    }

    fn camera(id: u64, state: &str) -> Value {
        node(
            id,
            state,
            json!({ "media.class": "Video/Source", "device.api": "libcamera",
                    "media.role": "Camera", "node.name": "libcamera_input.LNK0" }),
        )
    }

    fn portal(id: u64, state: &str) -> Value {
        node(
            id,
            state,
            json!({ "media.class": "Video/Source", "node.name": "xdpw-stream" }),
        )
    }

    #[test]
    fn an_idle_camera_is_not_capture() {
        let mut g = Graph::default();
        g.apply(&[
            camera(53, "suspended"),
            node(60, "running", json!({ "media.class": "Audio/Sink" })),
        ]);
        assert_eq!(g.state(), CaptureState::default());
    }

    #[test]
    fn a_running_camera_is_and_stops_being() {
        let mut g = Graph::default();
        g.apply(&[camera(53, "suspended")]);
        // An update that carries only the state keeps the node's kind.
        g.apply(&[json!({ "id": 53, "type": "PipeWire:Interface:Node",
                          "info": { "state": "running" } })]);
        assert!(g.state().camera);
        g.apply(&[json!({ "id": 53, "type": "PipeWire:Interface:Node",
                          "info": { "state": "idle" } })]);
        assert!(!g.state().camera);
    }

    #[test]
    fn a_portal_stream_is_a_shared_screen_until_it_is_removed() {
        let mut g = Graph::default();
        g.apply(&[camera(53, "suspended"), portal(90, "paused")]);
        assert_eq!(
            g.state(),
            CaptureState {
                screen: true,
                camera: false
            }
        );
        g.apply(&[json!({ "id": 90, "info": null })]);
        assert!(!g.state().screen);
    }

    #[test]
    fn a_node_whose_props_stop_being_video_drops_out() {
        let mut g = Graph::default();
        g.apply(&[portal(90, "running")]);
        g.apply(&[node(
            90,
            "running",
            json!({ "media.class": "Stream/Output/Audio" }),
        )]);
        assert!(!g.state().screen);
    }
}
