//! The outputs over `zwlr_output_manager_v1`, on a Wayland connection of
//! their own.
//!
//! One thread dispatches, blocked on the connection: it costs nothing at
//! rest, and a hotplug is one batch of events ending in `done`. Each `done`
//! sends the whole head model to the GTK thread, with the serial a
//! configuration built from it must carry. A configuration is built and
//! sent from the GTK thread (proxies are `Send`; the model sits behind one
//! mutex, held for the build and no longer) and its answer comes back the
//! same way as the heads.
//!
//! Every configuration is sent as `test` first and applied only when the
//! test succeeds, so a layout the compositor cannot show never reaches the
//! screen. The protocol uses a configuration once, so the apply is built
//! again from the same request, against the same serial.

use std::sync::{Arc, Mutex};

use wayland_client::backend::ObjectId;
use wayland_client::protocol::{wl_output, wl_registry};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum};
use wayland_protocols_wlr::output_management::v1::client::{
    zwlr_output_configuration_head_v1::{self, ZwlrOutputConfigurationHeadV1},
    zwlr_output_configuration_v1::{self, ZwlrOutputConfigurationV1},
    zwlr_output_head_v1::{self, ZwlrOutputHeadV1},
    zwlr_output_manager_v1::{self, ZwlrOutputManagerV1},
    zwlr_output_mode_v1::{self, ZwlrOutputModeV1},
};

use super::matcher::{HeadPlan, HeadState, Mode, ModeChoice};
use crate::settings::store::OutputTransform;

/// What the dispatch thread reports.
#[derive(Debug)]
pub enum Event {
    /// The compositor finished describing the outputs.
    Heads { serial: u32, heads: Vec<HeadState> },
    /// The answer to the configuration sent with this id.
    Outcome { id: u64, outcome: Outcome },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Succeeded,
    /// The compositor refused it and kept the outputs as they were.
    Failed,
    /// The outputs changed while it was on its way; the serial was stale.
    Cancelled,
}

struct ModeEntry {
    proxy: ZwlrOutputModeV1,
    size: Option<(u32, u32)>,
    refresh: u32,
}

struct Head {
    proxy: ZwlrOutputHeadV1,
    state: HeadState,
    modes: Vec<ModeEntry>,
    current_mode: Option<ObjectId>,
}

impl Head {
    fn snapshot(&self) -> HeadState {
        let listed = |m: &ModeEntry| m.size.map(|(w, h)| (w, h, m.refresh));
        HeadState {
            modes: self.modes.iter().filter_map(listed).collect(),
            mode: self
                .state
                .enabled
                .then(|| {
                    self.current_mode
                        .as_ref()
                        .and_then(|id| self.modes.iter().find(|m| &m.proxy.id() == id))
                        .and_then(listed)
                })
                .flatten(),
            ..self.state.clone()
        }
    }
}

/// A configuration on its way: kept from the test to the apply.
struct Request {
    serial: u32,
    heads: Vec<HeadState>,
    plans: Vec<HeadPlan>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Test,
    Apply,
}

struct Inner {
    manager: Option<ZwlrOutputManagerV1>,
    heads: Vec<Head>,
    requests: std::collections::HashMap<u64, Request>,
}

struct Shared {
    conn: Connection,
    qh: QueueHandle<State>,
    inner: Mutex<Inner>,
    tx: async_channel::Sender<Event>,
}

/// The panel's hold on the output configuration.
pub struct Outputs {
    shared: Arc<Shared>,
}

impl Outputs {
    /// Connect and start dispatching; events arrive on the returned channel.
    /// `None` off Wayland, or on a compositor without output management.
    pub fn start() -> Option<(Outputs, async_channel::Receiver<Event>)> {
        let conn = Connection::connect_to_env()
            .map_err(|e| log::info!("displays: no Wayland connection: {e}"))
            .ok()?;
        let mut queue = conn.new_event_queue::<State>();
        let qh = queue.handle();
        let (tx, rx) = async_channel::unbounded();
        let shared = Arc::new(Shared {
            conn: conn.clone(),
            qh: qh.clone(),
            inner: Mutex::new(Inner {
                manager: None,
                heads: Vec::new(),
                requests: std::collections::HashMap::new(),
            }),
            tx,
        });
        let _registry = conn.display().get_registry(&qh, ());
        let mut state = State {
            shared: shared.clone(),
        };
        queue.roundtrip(&mut state).ok()?;
        if shared.lock().manager.is_none() {
            log::info!("displays: zwlr_output_manager_v1 not offered");
            return None;
        }
        std::thread::Builder::new()
            .name("swaypplet-outputs".into())
            .spawn(move || {
                loop {
                    if let Err(e) = queue.blocking_dispatch(&mut state) {
                        log::warn!("displays: dispatch stopped: {e}");
                        return;
                    }
                }
            })
            .map_err(|e| log::warn!("displays: thread: {e}"))
            .ok()?;
        Some((Outputs { shared }, rx))
    }

    /// Send `plans` (one per head of `heads`, by name) as one configuration
    /// against `serial`: tested, then applied if the test succeeds. The
    /// answer arrives as [`Event::Outcome`] with `id`.
    pub fn apply(&self, id: u64, serial: u32, heads: &[HeadState], plans: &[HeadPlan]) {
        let mut inner = self.shared.lock();
        inner.requests.insert(
            id,
            Request {
                serial,
                heads: heads.to_vec(),
                plans: plans.to_vec(),
            },
        );
        send(&inner, &self.shared.qh, id, Phase::Test);
        drop(inner);
        let _ = self.shared.conn.flush();
    }
}

/// Build request `id` as one configuration and send it as `phase`. A head
/// the compositor knows that the request did not have is enabled as it is:
/// every head must be named, and one that arrived since is what the stale
/// serial will cancel anyway.
fn send(inner: &Inner, qh: &QueueHandle<State>, id: u64, phase: Phase) {
    let (Some(manager), Some(req)) = (&inner.manager, inner.requests.get(&id)) else {
        return;
    };
    let config = manager.create_configuration(req.serial, qh, (id, phase));
    let version = manager.version();
    for head in &inner.heads {
        let plan = req
            .heads
            .iter()
            .position(|h| h.name == head.state.name)
            .and_then(|i| req.plans.get(i));
        match plan {
            Some(HeadPlan::Disable) => config.disable_head(&head.proxy),
            Some(HeadPlan::Enable {
                mode,
                position,
                scale,
                transform,
                adaptive_sync,
            }) => {
                let c = config.enable_head(&head.proxy, qh, ());
                match mode {
                    Some(ModeChoice::Listed(m)) => match head.mode_proxy(*m) {
                        Some(proxy) => c.set_mode(proxy),
                        None => set_custom(&c, *m),
                    },
                    Some(ModeChoice::Custom(m)) => set_custom(&c, *m),
                    None => {}
                }
                if let Some((x, y)) = position {
                    c.set_position(*x, *y);
                }
                if let Some(s) = scale {
                    c.set_scale(*s);
                }
                if let Some(t) = transform {
                    c.set_transform(to_wl(*t));
                }
                if let Some(a) = adaptive_sync
                    && version >= 4
                {
                    c.set_adaptive_sync(if *a {
                        zwlr_output_head_v1::AdaptiveSyncState::Enabled
                    } else {
                        zwlr_output_head_v1::AdaptiveSyncState::Disabled
                    });
                }
            }
            None => {
                if head.state.enabled {
                    config.enable_head(&head.proxy, qh, ());
                } else {
                    config.disable_head(&head.proxy);
                }
            }
        }
    }
    match phase {
        Phase::Test => config.test(),
        Phase::Apply => config.apply(),
    }
}

impl Head {
    fn mode_proxy(&self, (w, h, r): Mode) -> Option<&ZwlrOutputModeV1> {
        self.modes
            .iter()
            .find(|m| m.size == Some((w, h)) && m.refresh == r)
            .map(|m| &m.proxy)
    }
}

fn set_custom(c: &ZwlrOutputConfigurationHeadV1, (w, h, r): Mode) {
    c.set_custom_mode(w as i32, h as i32, r as i32);
}

/// The settings spell a transform as sway's `output … transform` does,
/// which is what `swaymsg -t get_outputs` shows: `90` turns the picture
/// clockwise. `wl_output`'s `90` turns it the other way, and sway inverts
/// its own spelling before it reaches the output (`wlr_output_transform_
/// invert`: the unflipped quarter turns swap), so this does too, both ways.
fn to_wl(t: OutputTransform) -> wl_output::Transform {
    use wl_output::Transform as T;
    match t {
        OutputTransform::Normal => T::Normal,
        OutputTransform::R90 => T::_270,
        OutputTransform::R180 => T::_180,
        OutputTransform::R270 => T::_90,
        OutputTransform::Flipped => T::Flipped,
        OutputTransform::Flipped90 => T::Flipped90,
        OutputTransform::Flipped180 => T::Flipped180,
        OutputTransform::Flipped270 => T::Flipped270,
    }
}

fn from_wl(t: WEnum<wl_output::Transform>) -> OutputTransform {
    use wl_output::Transform as T;
    match t {
        WEnum::Value(T::_90) => OutputTransform::R270,
        WEnum::Value(T::_180) => OutputTransform::R180,
        WEnum::Value(T::_270) => OutputTransform::R90,
        WEnum::Value(T::Flipped) => OutputTransform::Flipped,
        WEnum::Value(T::Flipped90) => OutputTransform::Flipped90,
        WEnum::Value(T::Flipped180) => OutputTransform::Flipped180,
        WEnum::Value(T::Flipped270) => OutputTransform::Flipped270,
        _ => OutputTransform::Normal,
    }
}

impl Shared {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn head_mut<'a>(inner: &'a mut Inner, proxy: &ZwlrOutputHeadV1) -> Option<&'a mut Head> {
        inner.heads.iter_mut().find(|h| &h.proxy == proxy)
    }
}

// ── Dispatch ─────────────────────────────────────────────────────────────

struct State {
    shared: Arc<Shared>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
            && interface == ZwlrOutputManagerV1::interface().name
        {
            state.shared.lock().manager = Some(registry.bind(name, version.min(4), qh, ()));
        }
    }
}

impl Dispatch<ZwlrOutputManagerV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwlrOutputManagerV1,
        event: zwlr_output_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let shared = &state.shared;
        match event {
            zwlr_output_manager_v1::Event::Head { head } => {
                shared.lock().heads.push(Head {
                    proxy: head,
                    state: HeadState {
                        name: String::new(),
                        make: String::new(),
                        model: String::new(),
                        serial: String::new(),
                        enabled: false,
                        mode: None,
                        modes: Vec::new(),
                        position: (0, 0),
                        scale: 1.0,
                        transform: OutputTransform::Normal,
                        adaptive_sync: None,
                        physical_mm: (0, 0),
                    },
                    modes: Vec::new(),
                    current_mode: None,
                });
            }
            zwlr_output_manager_v1::Event::Done { serial } => {
                let heads = shared.lock().heads.iter().map(Head::snapshot).collect();
                let _ = shared.tx.try_send(Event::Heads { serial, heads });
            }
            zwlr_output_manager_v1::Event::Finished => {
                log::warn!("displays: the compositor stopped output management");
                shared.lock().manager = None;
            }
            _ => {}
        }
    }

    wayland_client::event_created_child!(State, ZwlrOutputManagerV1, [
        zwlr_output_manager_v1::EVT_HEAD_OPCODE => (ZwlrOutputHeadV1, ()),
    ]);
}

impl Dispatch<ZwlrOutputHeadV1, ()> for State {
    fn event(
        state: &mut Self,
        proxy: &ZwlrOutputHeadV1,
        event: zwlr_output_head_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwlr_output_head_v1::Event as E;
        let mut inner = state.shared.lock();
        if let E::Finished = event {
            if let Some(i) = inner.heads.iter().position(|h| &h.proxy == proxy) {
                let head = inner.heads.remove(i);
                for m in head.modes {
                    if m.proxy.version() >= 3 {
                        m.proxy.release();
                    }
                }
                if head.proxy.version() >= 3 {
                    head.proxy.release();
                }
            }
            return;
        }
        let Some(head) = Shared::head_mut(&mut inner, proxy) else {
            return;
        };
        let s = &mut head.state;
        match event {
            E::Name { name } => s.name = name,
            E::Make { make } => s.make = make,
            E::Model { model } => s.model = model,
            E::SerialNumber { serial_number } => s.serial = serial_number,
            E::Enabled { enabled } => s.enabled = enabled != 0,
            E::Position { x, y } => s.position = (x, y),
            E::Scale { scale } => s.scale = scale,
            E::Transform { transform } => s.transform = from_wl(transform),
            E::AdaptiveSync { state } => {
                s.adaptive_sync =
                    Some(state == WEnum::Value(zwlr_output_head_v1::AdaptiveSyncState::Enabled))
            }
            E::PhysicalSize { width, height } => s.physical_mm = (width, height),
            E::Mode { mode } => head.modes.push(ModeEntry {
                proxy: mode,
                size: None,
                refresh: 0,
            }),
            E::CurrentMode { mode } => head.current_mode = Some(mode.id()),
            _ => {}
        }
    }

    wayland_client::event_created_child!(State, ZwlrOutputHeadV1, [
        zwlr_output_head_v1::EVT_MODE_OPCODE => (ZwlrOutputModeV1, ()),
    ]);
}

impl Dispatch<ZwlrOutputModeV1, ()> for State {
    fn event(
        state: &mut Self,
        proxy: &ZwlrOutputModeV1,
        event: zwlr_output_mode_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwlr_output_mode_v1::Event as E;
        let mut inner = state.shared.lock();
        for head in &mut inner.heads {
            let Some(i) = head.modes.iter().position(|m| &m.proxy == proxy) else {
                continue;
            };
            match event {
                E::Size { width, height } => {
                    head.modes[i].size = Some((width.max(0) as u32, height.max(0) as u32))
                }
                E::Refresh { refresh } => head.modes[i].refresh = refresh.max(0) as u32,
                E::Finished => {
                    let m = head.modes.remove(i);
                    if m.proxy.version() >= 3 {
                        m.proxy.release();
                    }
                }
                _ => {}
            }
            return;
        }
    }
}

impl Dispatch<ZwlrOutputConfigurationV1, (u64, Phase)> for State {
    fn event(
        state: &mut Self,
        config: &ZwlrOutputConfigurationV1,
        event: zwlr_output_configuration_v1::Event,
        &(id, phase): &(u64, Phase),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        use zwlr_output_configuration_v1::Event as E;
        let outcome = match event {
            E::Succeeded => Outcome::Succeeded,
            E::Failed => Outcome::Failed,
            E::Cancelled => Outcome::Cancelled,
            _ => return,
        };
        config.destroy();
        let mut inner = state.shared.lock();
        if phase == Phase::Test && outcome == Outcome::Succeeded {
            send(&inner, qh, id, Phase::Apply);
            drop(inner);
            let _ = state.shared.conn.flush();
            return;
        }
        if phase == Phase::Test && outcome == Outcome::Failed {
            log::info!("displays: configuration {id} failed its test; not applied");
        }
        inner.requests.remove(&id);
        drop(inner);
        let _ = state.shared.tx.try_send(Event::Outcome { id, outcome });
    }
}

impl Dispatch<ZwlrOutputConfigurationHeadV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZwlrOutputConfigurationHeadV1,
        _: zwlr_output_configuration_head_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transforms_are_in_sways_spelling_and_round_trip() {
        use OutputTransform as O;
        for t in [
            O::Normal,
            O::R90,
            O::R180,
            O::R270,
            O::Flipped,
            O::Flipped90,
            O::Flipped180,
            O::Flipped270,
        ] {
            assert_eq!(from_wl(WEnum::Value(to_wl(t))), t);
        }
        // sway's clockwise 90 is wl_output's 270.
        assert_eq!(to_wl(O::R90), wl_output::Transform::_270);
    }
}
