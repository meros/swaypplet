//! The gamma tables over `zwlr_gamma_control_v1`, on a Wayland connection
//! of the night light's own.
//!
//! One thread dispatches: outputs arriving and leaving, each control's
//! table size, and `failed` (another client holds the output's tables, or
//! the output went away). Tables are sent from whichever thread calls
//! [`Gamma::set`]: proxies are `Send`, and the dispatch thread is the one
//! that reads what follows. Both sides go through one mutex, held for a
//! table write and no longer.
//!
//! At the day the controls are destroyed rather than set to the identity.
//! The protocol restores the output's own tables on destroy, and a client
//! holding no control costs the compositor nothing per frame and blocks no
//! other tool from taking the tables.

use std::collections::HashMap;
use std::io::Write;
use std::os::fd::{AsFd, FromRawFd, OwnedFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use wayland_client::protocol::{wl_output, wl_registry};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, delegate_noop};
use wayland_protocols_wlr::gamma_control::v1::client::{
    zwlr_gamma_control_manager_v1::ZwlrGammaControlManagerV1,
    zwlr_gamma_control_v1::{self, ZwlrGammaControlV1},
};

use super::color;

struct Output {
    output: wl_output::WlOutput,
    control: Option<ZwlrGammaControlV1>,
    /// Entries per channel, once the control has said.
    size: Option<u32>,
    /// The compositor refused this output's tables: a second client
    /// (gammastep) holds them, or the output has none (a headless one). Not
    /// retried until the day releases every control or the output comes
    /// back; asking again every step would be a fight, not a retry.
    failed: bool,
}

struct Inner {
    manager: Option<ZwlrGammaControlManagerV1>,
    /// By registry name.
    outputs: HashMap<u32, Output>,
    white: [f64; 3],
    day: bool,
}

struct Shared {
    conn: Connection,
    qh: QueueHandle<State>,
    inner: Mutex<Inner>,
}

/// The night light's hold on the compositor's gamma tables.
pub struct Gamma {
    shared: Arc<Shared>,
}

/// Whether the compositor offers gamma control and at least one output took
/// it. Read by the tile's state reader, which runs off the GTK thread.
static AVAILABLE: AtomicBool = AtomicBool::new(false);

pub fn available() -> bool {
    AVAILABLE.load(Ordering::Relaxed)
}

impl Gamma {
    /// Connect and start dispatching. `None` off Wayland, or on a
    /// compositor without `zwlr_gamma_control_manager_v1`.
    pub fn start() -> Option<Gamma> {
        let conn = Connection::connect_to_env()
            .map_err(|e| log::info!("night light: no Wayland connection: {e}"))
            .ok()?;
        let mut queue = conn.new_event_queue::<State>();
        let qh = queue.handle();
        let shared = Arc::new(Shared {
            conn: conn.clone(),
            qh: qh.clone(),
            inner: Mutex::new(Inner {
                manager: None,
                outputs: HashMap::new(),
                white: [1.0; 3],
                day: true,
            }),
        });
        // The registry lives as long as the connection: outputs come and go.
        let _registry = conn.display().get_registry(&qh, ());
        let mut state = State {
            shared: shared.clone(),
        };
        queue.roundtrip(&mut state).ok()?;
        if shared.lock().manager.is_none() {
            log::info!("night light: zwlr_gamma_control_manager_v1 not offered");
            return None;
        }
        AVAILABLE.store(true, Ordering::Relaxed);
        log::info!(
            "night light: gamma control offered, {} output(s)",
            shared.lock().outputs.len()
        );
        std::thread::Builder::new()
            .name("swaypplet-gamma".into())
            .spawn(move || {
                loop {
                    if let Err(e) = queue.blocking_dispatch(&mut state) {
                        log::warn!("night light: dispatch stopped: {e}");
                        AVAILABLE.store(false, Ordering::Relaxed);
                        return;
                    }
                }
            })
            .map_err(|e| log::warn!("night light: thread: {e}"))
            .ok()?;
        Some(Gamma { shared })
    }

    /// Show `kelvin` on every output.
    pub fn set(&self, kelvin: f64) {
        let mut inner = self.shared.lock();
        inner.day = color::is_day(kelvin);
        inner.white = color::white_point(kelvin);
        self.shared.sync(&mut inner);
        drop(inner);
        let _ = self.shared.conn.flush();
    }
}

impl Shared {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Bring every output in line with `inner.white`: controls released at
    /// the day, taken and written otherwise. Called with the lock held, from
    /// either thread.
    fn sync(&self, inner: &mut Inner) {
        let Inner {
            manager,
            outputs,
            white,
            day,
        } = inner;
        for (name, out) in outputs.iter_mut() {
            if *day {
                if let Some(c) = out.control.take() {
                    c.destroy();
                }
                out.size = None;
                out.failed = false;
                continue;
            }
            if out.failed {
                continue;
            }
            match (&out.control, out.size) {
                (None, _) => {
                    if let Some(m) = manager {
                        out.control = Some(m.get_gamma_control(&out.output, &self.qh, *name));
                    }
                }
                (Some(control), Some(size)) => write(control, size, *white),
                // Waiting for gamma_size, which writes when it lands.
                (Some(_), None) => {}
            }
        }
    }
}

/// Send one table. A write that fails leaves the output as it was; the next
/// step tries again.
fn write(control: &ZwlrGammaControlV1, size: u32, white: [f64; 3]) {
    let table = color::ramp(size as usize, white);
    let Some(file) = memfd() else { return };
    let mut file = std::fs::File::from(file);
    let bytes: Vec<u8> = table.iter().flat_map(|v| v.to_ne_bytes()).collect();
    if let Err(e) = file.write_all(&bytes) {
        log::warn!("night light: table write: {e}");
        return;
    }
    control.set_gamma(file.as_fd());
}

fn memfd() -> Option<OwnedFd> {
    let fd = unsafe { libc::memfd_create(c"swaypplet-gamma".as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        log::warn!(
            "night light: memfd_create: {}",
            std::io::Error::last_os_error()
        );
        return None;
    }
    Some(unsafe { OwnedFd::from_raw_fd(fd) })
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
        let shared = &state.shared;
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => {
                let mut inner = shared.lock();
                if interface == ZwlrGammaControlManagerV1::interface().name {
                    inner.manager = Some(registry.bind(name, version.min(1), qh, ()));
                } else if interface == wl_output::WlOutput::interface().name {
                    let output = registry.bind(name, version.min(4), qh, ());
                    inner.outputs.insert(
                        name,
                        Output {
                            output,
                            control: None,
                            size: None,
                            failed: false,
                        },
                    );
                } else {
                    return;
                }
                // A new output takes the current temperature at once.
                shared.sync(&mut inner);
            }
            wl_registry::Event::GlobalRemove { name } => {
                let mut inner = shared.lock();
                if let Some(out) = inner.outputs.remove(&name) {
                    if let Some(c) = out.control {
                        c.destroy();
                    }
                    if out.output.version() >= 3 {
                        out.output.release();
                    }
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwlrGammaControlV1, u32> for State {
    fn event(
        state: &mut Self,
        control: &ZwlrGammaControlV1,
        event: zwlr_gamma_control_v1::Event,
        name: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let shared = &state.shared;
        let mut inner = shared.lock();
        let white = inner.white;
        let Some(out) = inner.outputs.get_mut(name) else {
            return;
        };
        // A control destroyed since this event was sent is not ours now.
        if out.control.as_ref() != Some(control) {
            return;
        }
        match event {
            zwlr_gamma_control_v1::Event::GammaSize { size } => {
                log::info!("night light: output {name} takes {size}-entry tables");
                out.size = Some(size);
                write(control, size, white);
                AVAILABLE.store(true, Ordering::Relaxed);
                let _ = shared.conn.flush();
            }
            zwlr_gamma_control_v1::Event::Failed => {
                log::warn!(
                    "night light: output {name} refused gamma control: another client \
                     (gammastep) holds its tables, or it has none"
                );
                out.failed = true;
                out.size = None;
                if let Some(c) = out.control.take() {
                    c.destroy();
                }
                let any = inner.outputs.values().any(|o| !o.failed);
                AVAILABLE.store(any, Ordering::Relaxed);
            }
            _ => {}
        }
    }
}

delegate_noop!(State: ignore ZwlrGammaControlManagerV1);
delegate_noop!(State: ignore wl_output::WlOutput);
