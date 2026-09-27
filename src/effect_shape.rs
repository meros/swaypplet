//! The exact outline of every glass card, told to the compositor through
//! `wlrfx_background_effect_shape_v1` (`protocols/`).
//!
//! # Why the compositor is told instead of left to find out
//!
//! The material is the compositor's, and without this it finds the card by
//! reading the buffer's alpha: a flood from the pixels above the mask
//! threshold, a bounds pass, and a box measured at that threshold. That
//! reconstructs a rounded rectangle this process already knows to the
//! sub-pixel, and every error in the reconstruction is visible in the glass
//! (ridges at the corners, a crease down the diagonal, a light line where the
//! measured box and the real one disagree). Here the card's rectangle and
//! radius go over the wire instead, in the same commit as the buffer that
//! draws it.
//!
//! # What counts as a glass card
//!
//! Whatever `ui::card::adopt` made glass ([`crate::ui::card::glass_radius`]
//! says which, and with what radius). `adopt` hands each one to [`follow`],
//! which finds its window when it maps, so every surface that shows a glass
//! card sends shapes without being wired up by hand: the layer surfaces, the
//! lock's session-lock surfaces and the greeter alike.
//!
//! # When
//!
//! After the `layout` phase of the window's frame clock: the allocations and
//! the tick-driven animations of the frame are final there, and GDK paints
//! and commits in the next phase of the same frame, so the shapes land with
//! the pixels they describe. [`crate::anim::SlideBin`] moves its child at
//! paint time only, so it asks for a layout phase itself when it moves
//! ([`moved`]) and its offset is applied here ([`crate::anim::SlideBin::drawn_rect`]).
//!
//! # What the states mean to the compositor
//!
//! No object on a surface: the compositor finds the effect area by its own
//! means (the alpha mask). An empty list: this surface has no background
//! effect this commit, which is what is sent when every card on a window is
//! hidden, rather than leaving the last card's shape behind. So an object is
//! made only once a card maps, and destroyed with the surface.
//!
//! Falls back silently: a compositor without the global leaves every card on
//! the alpha-mask path, exactly as before. Nothing here depends on which
//! compositor implements the protocol.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use gdk4_wayland::prelude::*;
use gtk4::prelude::*;
use gtk4::{gdk, glib, graphene};
use wayland_client::protocol::wl_registry;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, delegate_noop};

use protocol::wlrfx_background_effect_shape_manager_v1::WlrfxBackgroundEffectShapeManagerV1 as Manager;
use protocol::wlrfx_background_effect_shape_v1::WlrfxBackgroundEffectShapeV1 as ShapeObject;

mod protocol {
    #![allow(dead_code, non_camel_case_types, unused_unsafe, unused_variables)]
    #![allow(non_upper_case_globals, non_snake_case, unused_imports)]
    #![allow(missing_docs, clippy::all)]

    use wayland_client;
    use wayland_client::protocol::*;

    pub mod __interfaces {
        // The interface tables name `wayland_backend`, which wayland-client
        // re-exports as `backend`.
        use wayland_client::backend as wayland_backend;
        use wayland_client::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("protocols/wlrfx-background-effect-shape-v1.xml");
    }
    use self::__interfaces::*;

    wayland_scanner::generate_client_code!("protocols/wlrfx-background-effect-shape-v1.xml");
}

/// The protocol's limit; one more is a fatal `too_many_shapes`.
pub const MAX_SHAPES: usize = 16;

/// One card in surface-local logical coordinates. Radii run clockwise from
/// the top left, as on the wire.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shape {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub radii: [f64; 4],
}

impl Shape {
    fn overlaps(&self, o: &Shape) -> bool {
        self.x < o.x + o.w && o.x < self.x + self.w && self.y < o.y + o.h && o.y < self.y + self.h
    }
}

/// The list the compositor gets, from the cards in the order found.
///
/// Everything here is something the protocol would otherwise refuse or
/// leave undefined: a card with no area (a zero-size allocation, a card
/// scaled to nothing) is a fatal `invalid_shapes`, so it is dropped; a radius
/// is clamped to half the shorter side, which is what CSS does to
/// `border-radius` and what the compositor would do anyway; a card that
/// overlaps one already in the list is dropped, because a pixel in both
/// would belong to either; and the list stops at [`MAX_SHAPES`].
pub fn build(cards: impl IntoIterator<Item = Shape>) -> Vec<Shape> {
    let mut out: Vec<Shape> = Vec::new();
    for mut s in cards {
        if !(s.w.is_finite() && s.h.is_finite() && s.x.is_finite() && s.y.is_finite()) {
            continue;
        }
        // Below one wl_fixed step the size packs to zero.
        if s.w < 1.0 / 256.0 || s.h < 1.0 / 256.0 {
            continue;
        }
        let half = s.w.min(s.h) / 2.0;
        for r in &mut s.radii {
            *r = if r.is_finite() {
                r.clamp(0.0, half)
            } else {
                0.0
            };
        }
        if out.iter().any(|o| o.overlaps(&s)) {
            once(&WARNED_OVERLAP, || {
                log::warn!("effect_shape: dropped a glass card that overlaps another: {s:?}")
            });
            continue;
        }
        if out.len() == MAX_SHAPES {
            // One more is the fatal `too_many_shapes`.
            once(&WARNED_CAP, || {
                log::warn!(
                    "effect_shape: more than {MAX_SHAPES} glass cards on one surface; the rest get none"
                )
            });
            break;
        }
        out.push(s);
    }
    out
}

thread_local! {
    static WARNED_OVERLAP: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static WARNED_CAP: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Run `f` the first time only: `build` runs every frame a card moves, and a
/// layout that trips a limit trips it on every one of those frames.
fn once(flag: &'static std::thread::LocalKey<std::cell::Cell<bool>>, f: impl FnOnce()) {
    if !flag.with(|w| w.replace(true)) {
        f();
    }
}

/// A logical coordinate as `wl_fixed_t`: 24.8 fixed point.
pub fn fixed(v: f64) -> i32 {
    (v * 256.0).round() as i32
}

/// The request's array: eight `wl_fixed_t` per card, in the host's byte
/// order (a `wl_array` is raw memory, not a network encoding).
pub fn pack(shapes: &[Shape]) -> Vec<u8> {
    let mut out = Vec::with_capacity(shapes.len() * 8 * 4);
    for s in shapes {
        for v in [
            s.x, s.y, s.w, s.h, s.radii[0], s.radii[1], s.radii[2], s.radii[3],
        ] {
            out.extend_from_slice(&fixed(v).to_ne_bytes());
        }
    }
    out
}

/// One surface's shape object, owned for as long as the caller keeps this.
///
/// Guarded exactly like [`crate::alpha::SurfaceAlpha`], for the same reason:
/// the `wl_surface` is GDK's, wayland-rs cannot tell when it is freed, and a
/// request on a freed proxy is a SIGSEGV. The window is the authority: an
/// `unrealize` hook and a comparison against the window's current surface.
struct SurfaceShape {
    object: ShapeObject,
    wl_surface: WlSurface,
    window: glib::WeakRef<gtk4::Window>,
    dead: Rc<std::cell::Cell<bool>>,
    hook: std::cell::Cell<Option<glib::SignalHandlerId>>,
    conn: Connection,
    /// What the compositor has now, packed; `None` until the first send, so
    /// that an empty first list still goes out.
    last: RefCell<Option<Vec<u8>>>,
}

/// The window's Wayland surface as GDK holds it *right now*.
fn current_wl_surface(window: &gtk4::Window) -> Option<WlSurface> {
    window
        .surface()?
        .downcast::<gdk4_wayland::WaylandSurface>()
        .ok()?
        .wl_surface()
}

impl SurfaceShape {
    fn attach(window: &gtk4::Window) -> Option<Self> {
        let wl_surface = current_wl_surface(window)?;
        let conn = Connection::from_backend(wl_surface.backend().upgrade()?);
        let manager = manager(&conn)?;

        // The object never sends events, so it needs no dispatch of its own.
        let queue = conn.new_event_queue::<Finder>();
        let qh = queue.handle();
        let object = manager.get_shape(&wl_surface, &qh, ());
        let _ = queue.flush();

        let dead = Rc::new(std::cell::Cell::new(false));
        let hook = window.connect_unrealize({
            let dead = dead.clone();
            move |_| dead.set(true)
        });

        Some(SurfaceShape {
            object,
            wl_surface,
            window: window.downgrade(),
            dead,
            hook: std::cell::Cell::new(Some(hook)),
            conn,
            last: RefCell::new(None),
        })
    }

    fn is_valid(&self) -> bool {
        !self.dead.get()
            && self
                .window
                .upgrade()
                .and_then(|w| current_wl_surface(&w))
                .is_some_and(|live| live.id() == self.wl_surface.id())
    }

    /// Queue `shapes` as pending surface state, without committing (the
    /// toolkit's next commit carries it). Whether anything was sent.
    fn set_pending(&self, shapes: &[Shape]) -> bool {
        if !self.is_valid() {
            return false;
        }
        let packed = pack(shapes);
        if self.last.borrow().as_ref() == Some(&packed) {
            return false;
        }
        self.object.set_shapes(packed.clone());
        let _ = self.conn.flush();
        *self.last.borrow_mut() = Some(packed);
        true
    }
}

impl Drop for SurfaceShape {
    fn drop(&mut self) {
        if let Some(window) = self.window.upgrade()
            && let Some(hook) = self.hook.take()
        {
            window.disconnect(hook);
        }
        // "The wl_surface was destroyed before this object" is fatal, and a
        // freed proxy is worse; see the type's docs.
        if !self.is_valid() {
            return;
        }
        self.object.destroy();
        let _ = self.conn.flush();
    }
}

/// The glass cards of one window, and the frame-clock hook that sends them.
struct Tracker {
    window: glib::WeakRef<gtk4::Window>,
    cards: RefCell<Vec<glib::WeakRef<gtk4::Widget>>>,
    bound: RefCell<Option<Bound>>,
}

/// What exists only while the window is realized: its surface's shape
/// object and the hook on its surface's frame clock.
struct Bound {
    shape: SurfaceShape,
    clock: gdk::FrameClock,
    handler: glib::SignalHandlerId,
}

thread_local! {
    /// Every window with a tracker. Weak both ways: the window's own signal
    /// handlers hold the tracker, so it goes with the window.
    static TRACKERS: RefCell<Vec<Weak<Tracker>>> = const { RefCell::new(Vec::new()) };
}

/// Send `card`'s shape whenever it is mapped in a window.
///
/// Called by `ui::card::adopt` for every glass card; nothing else needs to.
pub fn follow(card: &gtk4::Widget) {
    card.connect_map(|card| {
        let Some(window) = card.root().and_downcast::<gtk4::Window>() else {
            return;
        };
        let tracker = tracker_for(&window);
        {
            let mut cards = tracker.cards.borrow_mut();
            cards.retain(|c| c.upgrade().is_some());
            if !cards.iter().any(|c| c.upgrade().as_ref() == Some(card)) {
                cards.push(card.downgrade());
            }
        }
        tracker.bind(&window);
        if let Some(clock) = window.frame_clock() {
            clock.request_phase(gdk::FrameClockPhase::LAYOUT);
        }
    });
}

/// `widget` moved at paint time, where the layout phase does not see it
/// (a [`crate::anim::SlideBin`]): run that phase anyway, so the shapes follow.
pub fn moved(widget: &impl IsA<gtk4::Widget>) {
    if let Some(clock) = widget.frame_clock() {
        clock.request_phase(gdk::FrameClockPhase::LAYOUT);
    }
}

fn tracker_for(window: &gtk4::Window) -> Rc<Tracker> {
    let found = TRACKERS.with(|t| {
        let mut all = t.borrow_mut();
        all.retain(|w| w.upgrade().is_some_and(|t| t.window.upgrade().is_some()));
        all.iter()
            .filter_map(Weak::upgrade)
            .find(|t| t.window.upgrade().as_ref() == Some(window))
    });
    if let Some(tracker) = found {
        return tracker;
    }
    let tracker = Rc::new(Tracker {
        window: window.downgrade(),
        cards: RefCell::new(Vec::new()),
        bound: RefCell::new(None),
    });
    TRACKERS.with(|t| t.borrow_mut().push(Rc::downgrade(&tracker)));
    // A re-realized window has a new surface and a new frame clock.
    window.connect_realize({
        let tracker = tracker.clone();
        move |w| tracker.bind(w)
    });
    // RUN_LAST: this runs before GTK frees the surface, so the object is
    // destroyed while its surface still exists.
    window.connect_unrealize({
        let tracker = tracker.clone();
        move |_| tracker.unbind()
    });
    tracker
}

impl Tracker {
    fn bind(self: &Rc<Self>, window: &gtk4::Window) {
        if self
            .bound
            .borrow()
            .as_ref()
            .is_some_and(|b| b.shape.is_valid())
        {
            return;
        }
        self.unbind();
        let Some(clock) = window.frame_clock() else {
            return;
        };
        let Some(shape) = SurfaceShape::attach(window) else {
            return;
        };
        let weak = Rc::downgrade(self);
        let handler = clock.connect_local("layout", true, move |_| {
            if let Some(tracker) = weak.upgrade() {
                tracker.send();
            }
            None
        });
        *self.bound.borrow_mut() = Some(Bound {
            shape,
            clock,
            handler,
        });
    }

    fn unbind(&self) {
        let Some(bound) = self.bound.borrow_mut().take() else {
            return;
        };
        let Bound {
            shape,
            clock,
            handler,
        } = bound;
        clock.disconnect(handler);
        drop(shape);
    }

    /// Measure the cards and send the list if it changed.
    fn send(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let cards: Vec<gtk4::Widget> = {
            let mut cards = self.cards.borrow_mut();
            cards.retain(|c| c.upgrade().is_some());
            cards.iter().filter_map(|c| c.upgrade()).collect()
        };
        let (ox, oy) = window.surface_transform();
        let shapes = build(
            cards
                .iter()
                .filter_map(|card| measure(card, &window))
                .map(|s| Shape {
                    x: s.x + ox,
                    y: s.y + oy,
                    ..s
                }),
        );
        let sent = self
            .bound
            .borrow()
            .as_ref()
            .is_some_and(|b| b.shape.set_pending(&shapes));
        if sent {
            // Usually a paint is already due (whatever moved the card
            // queued one), but a commit must follow either way or the new
            // shapes wait for the next unrelated redraw.
            let drawn: Vec<_> = cards.iter().filter(|c| c.is_drawable()).collect();
            if drawn.is_empty() {
                window.queue_draw();
            } else {
                drawn.iter().for_each(|c| c.queue_draw());
            }
        }
    }
}

/// `card`'s shape in `window`'s coordinates, or `None` when it draws nothing.
fn measure(card: &gtk4::Widget, window: &gtk4::Window) -> Option<Shape> {
    if !card.is_drawable() {
        return None;
    }
    let radius = crate::ui::card::glass_radius(card)?;
    // Faded to nothing by a widget-side fade: no glass either.
    let mut opacity = 1.0;
    let mut w = Some(card.clone());
    while let Some(widget) = w {
        opacity *= widget.opacity();
        w = widget.parent();
    }
    if opacity < 1.0 / 255.0 {
        return None;
    }
    // The border box, through every allocation and CSS transform.
    let bounds = card.compute_bounds(window)?;
    // The scale those transforms apply, against the same box untransformed.
    // `width()` is the content box, which is short of the border box by the
    // padding and border: the lock card's 50px made this 1.16 at rest and
    // sent a 20.9px radius for its 18px corners.
    let own = card.compute_bounds(card)?;
    let scale = if own.width() > 0.0 {
        f64::from(bounds.width()) / f64::from(own.width())
    } else {
        1.0
    };
    let mut rect = bounds;
    let mut r = radius * scale;
    // Paint-time moves on the way up, innermost first.
    let mut w = card.parent();
    while let Some(widget) = w {
        if widget == *window.upcast_ref::<gtk4::Widget>() {
            break;
        }
        if let Some(bin) = widget.downcast_ref::<crate::anim::SlideBin>() {
            let origin = bin
                .compute_point(window, &graphene::Point::zero())
                .unwrap_or_else(graphene::Point::zero);
            let (moved, s) = bin.drawn_rect(&rect, &origin);
            rect = moved;
            r *= s;
        }
        w = widget.parent();
    }
    Some(Shape {
        x: f64::from(rect.x()),
        y: f64::from(rect.y()),
        w: f64::from(rect.width()),
        h: f64::from(rect.height()),
        radii: [r; 4],
    })
}

/// Bind the manager before any surface exists, and report whether the
/// compositor offers the protocol. For the lock, for the reason
/// [`crate::alpha::preload`] gives: the roundtrip must not land inside the
/// interval the compositor holds the live desktop on screen for.
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

/// Bind the manager once per process, on a queue of our own (see
/// `alpha::manager`).
fn manager(conn: &Connection) -> Option<Manager> {
    thread_local! {
        static MANAGER: std::cell::OnceCell<Option<Manager>> =
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
                    "wlrfx_background_effect_shape_v1 not offered; the compositor finds glass cards by their alpha"
                );
            }
            finder.manager
        })
        .clone()
    })
}

struct Finder {
    manager: Option<Manager>,
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
            && interface == Manager::interface().name
        {
            state.manager = Some(registry.bind(name, version.min(1), qh, ()));
        }
    }
}

delegate_noop!(Finder: ignore Manager);
delegate_noop!(Finder: ignore ShapeObject);

#[cfg(test)]
mod tests {
    use super::*;

    fn card(x: f64, y: f64, w: f64, h: f64, r: f64) -> Shape {
        Shape {
            x,
            y,
            w,
            h,
            radii: [r; 4],
        }
    }

    #[test]
    fn fixed_is_24_8() {
        assert_eq!(fixed(0.0), 0);
        assert_eq!(fixed(1.0), 256);
        assert_eq!(fixed(-1.5), -384);
        assert_eq!(fixed(18.0 / 1.5), 3072);
        // Rounded to the nearest step, not truncated.
        assert_eq!(fixed(0.999), 256);
    }

    #[test]
    fn pack_is_eight_fixed_values_per_card_in_wire_order() {
        let s = Shape {
            x: 1.0,
            y: 2.0,
            w: 3.0,
            h: 4.0,
            radii: [0.5, 0.25, 0.0, 1.0],
        };
        let bytes = pack(&[s, s]);
        assert_eq!(bytes.len(), 2 * 8 * 4);
        let words: Vec<i32> = bytes
            .chunks_exact(4)
            .map(|c| i32::from_ne_bytes(c.try_into().unwrap()))
            .collect();
        assert_eq!(&words[..8], &[256, 512, 768, 1024, 128, 64, 0, 256]);
        assert_eq!(words[..8], words[8..]);
        assert!(pack(&[]).is_empty());
    }

    #[test]
    fn radii_clamp_to_half_the_shorter_side() {
        let out = build([card(0.0, 0.0, 200.0, 30.0, 999.0)]);
        assert_eq!(out[0].radii, [15.0; 4]);
        let out = build([Shape {
            radii: [-3.0, f64::NAN, 4.0, 1e9],
            ..card(0.0, 0.0, 10.0, 40.0, 0.0)
        }]);
        assert_eq!(out[0].radii, [0.0, 0.0, 4.0, 5.0]);
    }

    #[test]
    fn empty_cards_are_dropped() {
        let out = build([
            card(0.0, 0.0, 0.0, 30.0, 4.0),
            card(0.0, 0.0, 30.0, 0.001, 4.0),
            card(0.0, 0.0, f64::NAN, 30.0, 4.0),
            card(5.0, 5.0, 10.0, 10.0, 4.0),
        ]);
        assert_eq!(out, vec![card(5.0, 5.0, 10.0, 10.0, 4.0)]);
    }

    #[test]
    fn an_overlapping_card_is_dropped_and_touching_ones_are_kept() {
        let out = build([
            card(0.0, 0.0, 100.0, 50.0, 18.0),
            card(50.0, 25.0, 100.0, 50.0, 18.0),
            card(100.0, 0.0, 20.0, 20.0, 6.0),
        ]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[1].x, 100.0);
    }

    #[test]
    fn the_list_stops_at_the_protocol_limit() {
        let many = (0..20).map(|i| card(f64::from(i) * 20.0, 0.0, 10.0, 10.0, 2.0));
        assert_eq!(build(many).len(), MAX_SHAPES);
    }

    #[test]
    fn a_change_is_a_change_of_the_packed_list() {
        // Change detection compares packed bytes: movement below a wl_fixed
        // step is not a change, anything larger is.
        let a = pack(&build([card(10.0, 10.0, 100.0, 40.0, 14.0)]));
        let b = pack(&build([card(10.0 + 1.0 / 1024.0, 10.0, 100.0, 40.0, 14.0)]));
        let c = pack(&build([card(10.5, 10.0, 100.0, 40.0, 14.0)]));
        assert_eq!(a, b);
        assert_ne!(a, c);
    }
}
