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

use gdk4_wayland::prelude::*;
use gtk4::prelude::*;
use wayland_client::protocol::wl_registry;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, delegate_noop};
use wayland_protocols::wp::alpha_modifier::v1::client::{
    wp_alpha_modifier_surface_v1::WpAlphaModifierSurfaceV1, wp_alpha_modifier_v1::WpAlphaModifierV1,
};

/// The multiplier is a fixed-point fraction of `u32::MAX`, not a float.
const OPAQUE: u32 = u32::MAX;

/// One surface's alpha, owned for as long as the caller keeps this.
pub struct SurfaceAlpha {
    surface: WpAlphaModifierSurfaceV1,
    wl_surface: wayland_client::protocol::wl_surface::WlSurface,
    conn: Connection,
    last: Cell<u32>,
}

impl SurfaceAlpha {
    /// Bind the window's Wayland surface, or `None` when this is not Wayland,
    /// the surface is not realized yet, or the compositor does not offer the
    /// protocol.
    pub fn attach(window: &gtk4::Window) -> Option<Self> {
        let surface = window.surface()?;
        let wl_surface = surface
            .downcast::<gdk4_wayland::WaylandSurface>()
            .ok()?
            .wl_surface()?;
        let conn = Connection::from_backend(wl_surface.backend().upgrade()?);
        let manager = manager(&conn)?;

        // The manager object lives on its own queue; the per-surface object
        // never sends events, so it needs no dispatch of its own.
        let queue = conn.new_event_queue::<Finder>();
        let qh = queue.handle();
        let alpha = manager.get_surface(&wl_surface, &qh, ());
        let _ = queue.flush();
        Some(SurfaceAlpha {
            surface: alpha,
            wl_surface,
            conn,
            last: Cell::new(OPAQUE),
        })
    }

    /// The multiplier currently in effect.
    pub fn get(&self) -> f64 {
        f64::from(self.last.get()) / f64::from(OPAQUE)
    }

    /// Set the surface's opacity. Double-buffered like everything else on a
    /// surface, so it lands with the next frame the toolkit commits, which is
    /// why callers redraw alongside it.
    pub fn set(&self, alpha: f64) {
        let v = (alpha.clamp(0.0, 1.0) * f64::from(OPAQUE)).round() as u32;
        if v == self.last.get() {
            return;
        }
        self.last.set(v);
        self.surface.set_multiplier(v);
        // The multiplier is double-buffered surface state, so it lands on
        // the surface's next commit — and a toolkit with nothing new to
        // draw does not produce one. GTK redraws the pane alongside every
        // set (anim.rs), but a surface whose widgets never change during
        // the fade (the compositor is doing the fading) gives GTK nothing
        // to damage, and the whole ramp can sit pending while the surface
        // stays at whatever alpha its first commit carried. Committing here
        // is the only thing that makes the number take effect on the frame
        // it was set for; there is no buffer attached, so it applies the
        // pending state and nothing else.
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
        if !self.wl_surface.is_alive() {
            return;
        }
        // destroy() is "equivalent to set_multiplier with a value of
        // UINT32_MAX", so resetting the multiplier first sends a second
        // request to say the same thing.
        self.surface.destroy();
        let _ = self.conn.flush();
    }
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
