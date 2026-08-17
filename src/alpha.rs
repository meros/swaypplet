//! Surface-wide alpha that the compositor applies, via `wp_alpha_modifier_v1`.
//!
//! # Why a protocol instead of `set_opacity`
//!
//! Fading a glass surface by fading its widgets is fading the wrong thing.
//! The frost behind the surface is the compositor's, and it has no idea the
//! client is fading: swayfx blurs behind every pixel above alpha zero at full
//! strength, so a pane on its way out is a fully frosted rectangle holding
//! less and less of its own colour. That is Apple's material rule stated in
//! the negative — never alpha-fade the effect view — and client-side opacity
//! can only ever break it.
//!
//! `wp_alpha_modifier_v1` hands the compositor a single number instead: the
//! opacity the whole surface should be composited at. sway already applies it
//! to the surface's buffer, and a container's blur already rides the same
//! number down (`sway/desktop/output.c`), collapsing the blur's radius and
//! passes rather than cross-fading it. With the layer-shell path taught the
//! same trick, handing over one float per frame fades the material and
//! everything drawn on it together, which is the effect that cannot be had
//! from this side of the socket at all.
//!
//! Falls back silently: a compositor without the global leaves
//! [`SurfaceAlpha::attach`] returning `None`, and callers keep their
//! widget-side fade.

use std::cell::Cell;
use std::rc::Rc;

use gdk4_wayland::prelude::*;
use gtk4::glib;
use gtk4::prelude::*;
use wayland_client::protocol::wl_registry;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, delegate_noop};
use wayland_protocols::wp::alpha_modifier::v1::client::{
    wp_alpha_modifier_surface_v1::WpAlphaModifierSurfaceV1, wp_alpha_modifier_v1::WpAlphaModifierV1,
};

/// The multiplier is a fixed-point fraction of `u32::MAX`, not a float.
const OPAQUE: u32 = u32::MAX;

/// One surface's alpha, owned for as long as the caller keeps this.
///
/// # Why this tracks the window and not just the surface
///
/// `wl_surface` here is GDK's object, borrowed by pointer. wayland-rs did not
/// create it, so it has no liveness cell for it
/// (`wayland-backend`'s `ObjectId::from_ptr` stores `alive: None` for a proxy
/// it does not manage, and `is_alive()` answers `true` for those
/// unconditionally). Once GDK unrealizes the window, libwayland frees the
/// proxy and every handle here still reports it healthy — so a `commit()`
/// walks freed memory. That is not theoretical: it read a recycled
/// `wl_proxy`, took `interface->methods` as NULL and faulted at 0x98 inside
/// `wl_proxy_marshal_array_flags`, taking the panel down with SIGSEGV.
///
/// So the window is the authority on whether this handle may still speak.
/// [`SurfaceAlpha::is_valid`] asks it twice over: an `unrealize` hook fires
/// the moment GDK takes the surface down, and the id is compared against the
/// window's current one, which catches a surface swapped out while nobody was
/// listening.
pub struct SurfaceAlpha {
    surface: WpAlphaModifierSurfaceV1,
    wl_surface: WlSurface,
    /// Weak: the window owns the `Reveal` that owns this.
    window: glib::WeakRef<gtk4::Window>,
    /// Set by the `unrealize` hook below, and never cleared — a re-realized
    /// window needs a fresh handle bound to its fresh surface, not this one.
    dead: Rc<Cell<bool>>,
    hook: Cell<Option<glib::SignalHandlerId>>,
    conn: Connection,
    last: Cell<u32>,
}

/// The window's Wayland surface as GDK holds it *right now*.
fn current_wl_surface(window: &gtk4::Window) -> Option<WlSurface> {
    window
        .surface()?
        .downcast::<gdk4_wayland::WaylandSurface>()
        .ok()?
        .wl_surface()
}

impl SurfaceAlpha {
    /// Bind the window's Wayland surface, or `None` when this is not Wayland,
    /// the surface is not realized yet, or the compositor does not offer the
    /// protocol.
    pub fn attach(window: &gtk4::Window) -> Option<Self> {
        let wl_surface = current_wl_surface(window)?;
        let conn = Connection::from_backend(wl_surface.backend().upgrade()?);
        let manager = manager(&conn)?;

        // The manager object lives on its own queue; the per-surface object
        // never sends events, so it needs no dispatch of its own.
        let queue = conn.new_event_queue::<Finder>();
        let qh = queue.handle();
        let alpha = manager.get_surface(&wl_surface, &qh, ());
        let _ = queue.flush();

        // GDK frees the surface during unrealize, so this has to land before
        // it happens rather than be noticed after.
        let dead = Rc::new(Cell::new(false));
        let hook = window.connect_unrealize({
            let dead = dead.clone();
            move |_| dead.set(true)
        });

        Some(SurfaceAlpha {
            surface: alpha,
            wl_surface,
            window: window.downgrade(),
            dead,
            hook: Cell::new(Some(hook)),
            conn,
            last: Cell::new(OPAQUE),
        })
    }

    /// Whether the surface this handle was bound to is still the window's
    /// own. `false` means every request on it — the destructor included —
    /// would touch a freed proxy, so the handle stays silent for good.
    pub fn is_valid(&self) -> bool {
        !self.dead.get()
            && self
                .window
                .upgrade()
                .and_then(|w| current_wl_surface(&w))
                .is_some_and(|live| live.id() == self.wl_surface.id())
    }

    /// The multiplier currently in effect.
    pub fn get(&self) -> f64 {
        f64::from(self.last.get()) / f64::from(OPAQUE)
    }

    /// Queue the multiplier as pending surface state, without committing.
    ///
    /// This is the only safe setter for an `ext_session_lock_surface_v1`. A
    /// commit with no buffer attached is `null_buffer`, and a commit whose
    /// size disagrees with the last acked configure is `dimensions_mismatch`
    /// (wlroots `types/wlr_session_lock_v1.c`); both are fatal, and both are
    /// reachable from a timer-driven ramp, because GDK acks configures
    /// synchronously in the event handler and attaches the matching buffer a
    /// frame later. The multiplier is double-buffered surface state, so
    /// leaving it pending means the toolkit's own buffered commit promotes
    /// it, atomically and at the right size. Callers ask for that commit with
    /// `queue_draw`.
    pub fn set_pending(&self, alpha: f64) {
        // The one gate. `set` reaches the socket only through here, so a
        // stale handle cannot marshal on either object.
        if !self.is_valid() {
            return;
        }
        let v = (alpha.clamp(0.0, 1.0) * f64::from(OPAQUE)).round() as u32;
        if v == self.last.get() {
            return;
        }
        self.last.set(v);
        self.surface.set_multiplier(v);
        let _ = self.conn.flush();
    }

    /// Set the surface's opacity and commit it.
    ///
    /// For surfaces with no role restrictions (layer shell, via `anim::Reveal`),
    /// where committing here is the only thing that makes the number take
    /// effect on the frame it was set for: the multiplier is double-buffered
    /// state, so it lands on the surface's next commit, and a toolkit with
    /// nothing new to draw does not produce one. There is no buffer attached,
    /// so the commit applies the pending state and nothing else.
    pub fn set(&self, alpha: f64) {
        let before = self.last.get();
        self.set_pending(alpha);
        if self.last.get() == before {
            return;
        }
        self.wl_surface.commit();
        let _ = self.conn.flush();
    }
}

impl Drop for SurfaceAlpha {
    fn drop(&mut self) {
        // "This object has to be destroyed before the associated wl_surface.
        // Once the wl_surface is destroyed, all requests on this object will
        // raise the no_surface error." A protocol error takes the whole
        // client down, so a window torn down ahead of its alpha handle must
        // not be spoken to at all — including the destructor.
        //
        // Owners are expected to drop this first (GlassSurface does); the
        // check is what keeps a path that forgets from killing the process.
        //
        // `wl_surface.is_alive()` used to be that check and never fired: the
        // surface is GDK's, so wayland-rs has no liveness cell for it and
        // answers `true` forever. See the type's docs.
        if let Some(window) = self.window.upgrade()
            && let Some(hook) = self.hook.take()
        {
            window.disconnect(hook);
        }
        if !self.is_valid() {
            return;
        }
        // destroy() is "equivalent to set_multiplier with a value of
        // UINT32_MAX", so resetting the multiplier first sends a second
        // request to say the same thing.
        self.surface.destroy();
        let _ = self.conn.flush();
    }
}

/// Bind the manager before any surface exists, and report whether the
/// compositor offers the protocol at all.
///
/// [`manager`] does a roundtrip on first use, and for the lock screen that
/// first use would otherwise land inside `gtk_window_realize`, inside
/// `gtk_window_present`, inside the monitor callback, inside
/// `ext_session_lock_v1.lock` — that is, inside the interval the compositor
/// is holding the live desktop on screen for.
pub fn preload() -> bool {
    let Some(display) = gdk4::Display::default() else {
        return false;
    };
    let Ok(display) = display.downcast::<gdk4_wayland::WaylandDisplay>() else {
        return false;
    };
    let Some(wl_display) = display.wl_display() else {
        return false;
    };
    let Some(backend) = wl_display.backend().upgrade() else {
        return false;
    };
    manager(&Connection::from_backend(backend)).is_some()
}

/// Bind the manager once per process. The roundtrip runs on a queue of our
/// own, so GDK's queue keeps its own events; the shared backend routes each
/// event to whichever queue its object belongs to.
fn manager(conn: &Connection) -> Option<WpAlphaModifierV1> {
    thread_local! {
        static MANAGER: std::cell::OnceCell<Option<WpAlphaModifierV1>> =
            const { std::cell::OnceCell::new() };
    }
    MANAGER.with(|cell| {
        cell.get_or_init(|| {
            let mut queue = conn.new_event_queue::<Finder>();
            let qh = queue.handle();
            let _registry = conn.display().get_registry(&qh, ());
            let mut finder = Finder { manager: None };
            if queue.roundtrip(&mut finder).is_err() {
                return None;
            }
            if finder.manager.is_none() {
                log::info!(
                    "wp_alpha_modifier_v1 not offered; glass surfaces fade client-side instead"
                );
            }
            finder.manager
        })
        .clone()
    })
}

struct Finder {
    manager: Option<WpAlphaModifierV1>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for Finder {
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
            && interface == WpAlphaModifierV1::interface().name
        {
            state.manager = Some(registry.bind(name, version.min(1), qh, ()));
        }
    }
}

delegate_noop!(Finder: ignore WpAlphaModifierV1);
delegate_noop!(Finder: ignore WpAlphaModifierSurfaceV1);
