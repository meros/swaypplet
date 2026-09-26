//! One swaypplet surface: a layer-shell window, its root, its card and the
//! transition that shows and hides them, built and torn down in one place.
//!
//! Every glass surface is the same object: a transparent root on a layer
//! surface (`ui::surface` on the window's child, never the window), a card on
//! it (`ui::card`) that the compositor frosts, and an [`anim::Reveal`] that
//! fades the two in and out as a material. The builder names the parts that
//! differ (where the surface sits, whether it takes the keyboard, which card,
//! whether the card slides as it fades) and the rest is decided here:
//!
//! ```ignore
//! let surface = Surface::builder(app, Namespace::Osd)
//!     .monitor(Some(&monitor))
//!     .anchor(&[Edge::Bottom])
//!     .margin(Edge::Bottom, 72)
//!     .card(ui::Card::Thin)
//!     .build();
//! surface.card().append(&content);
//! surface.set_content(&content);
//! surface.show();
//! ```
//!
//! Teardown is the other half, and the reason this is a type rather than a
//! helper. The window is the application's from the moment it is built
//! (`.application(app)`), and a `GtkApplication` holds that reference until
//! the window is *destroyed*: hiding only unmaps, and the window keeps its
//! buffers (the notification stack once walked the process into its fd limit
//! that way, four dmabufs and eight syncobjs per stranded card). Destroying
//! has an order, too: the compositor alpha handle is bound to the window's
//! `wl_surface` and must go first, or every request on it, its destructor
//! included, is a fatal protocol error (`crate::alpha`); and a window that
//! was never shown has to be realized before GTK will destroy it without a
//! NULL dereference ([`layer::destroy_window`]). Dropping the last handle to
//! a [`Surface`] does all three, once, in that order.

use std::rc::Rc;

use gtk4::gdk;
use gtk4::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

use super::layer::{self, LayerShellConfig};
use super::namespace::Namespace;
use crate::anim::{self, SlideBin};

/// A layer surface and what is on it. Cloning shares it; the surface is
/// destroyed with the last handle (see the module docs).
#[derive(Clone)]
pub struct Surface {
    inner: Rc<Inner>,
}

struct Inner {
    window: gtk4::Window,
    /// The window's child, carrying `ui::surface`.
    root: gtk4::Box,
    /// The glass card, when the surface has one. Its alpha is what the
    /// compositor stencils glass against, so the [`anim::Reveal`] owns it.
    card: Option<gtk4::Box>,
    /// The settle the card rides as it fades, when it has one.
    slide: Option<SlideBin>,
    /// Present exactly when `card` is.
    reveal: Option<anim::Reveal>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        // Order is load-bearing (module docs): the alpha handle and the
        // namespace's glass count go while the `wl_surface` still exists,
        // and the window follows.
        if let Some(reveal) = &self.reveal {
            reveal.release();
        }
        layer::destroy_window(&self.window);
    }
}

/// Describes a [`Surface`]; [`Surface::builder`] starts one.
#[must_use]
pub struct Builder<'a> {
    app: &'a gtk4::Application,
    namespace: Namespace,
    monitor: Option<gdk::Monitor>,
    layer: Layer,
    anchors: Vec<(Edge, bool)>,
    margins: Vec<(Edge, i32)>,
    keyboard: KeyboardMode,
    exclusive: bool,
    ignore_exclusive_zones: bool,
    width: Option<i32>,
    height: Option<i32>,
    opaque: bool,
    resizable: Option<bool>,
    /// `None` until the caller says: a card, or none.
    card: Option<Option<crate::ui::Card>>,
    slide: Option<(gtk4::Orientation, f64)>,
    wrap: Option<Box<dyn FnOnce(&gtk4::Widget) -> gtk4::Widget + 'a>>,
}

impl Surface {
    /// Start describing a surface in `namespace`. It defaults to the overlay
    /// layer, unanchored (centred on its output), and without the keyboard.
    pub fn builder(app: &gtk4::Application, namespace: Namespace) -> Builder<'_> {
        Builder {
            app,
            namespace,
            monitor: None,
            layer: Layer::Overlay,
            anchors: Vec::new(),
            margins: Vec::new(),
            keyboard: KeyboardMode::None,
            exclusive: false,
            ignore_exclusive_zones: false,
            width: None,
            height: None,
            opaque: false,
            resizable: None,
            card: None,
            slide: None,
            wrap: None,
        }
    }

    pub fn window(&self) -> &gtk4::Window {
        &self.inner.window
    }

    /// The window's child: `ui::surface`, transparent, holding the card (or,
    /// for a surface without one, whatever the caller puts in it).
    pub fn root(&self) -> &gtk4::Box {
        &self.inner.root
    }

    /// The glass card. Panics on a surface built with [`Builder::no_card`],
    /// which is a bug at the call site rather than a state to handle.
    pub fn card(&self) -> &gtk4::Box {
        self.inner
            .card
            .as_ref()
            .expect("Surface::card on a surface built with no_card()")
    }

    /// The bin the card slides in, for a surface built with
    /// [`Builder::slide`].
    pub fn slide(&self) -> Option<&SlideBin> {
        self.inner.slide.as_ref()
    }

    /// The card's transition, for a surface that has a card.
    pub fn reveal(&self) -> Option<&anim::Reveal> {
        self.inner.reveal.as_ref()
    }

    /// Point the transition at what is drawn on the card, which fades over
    /// the whole enter and exit while the card's tint arrives with the glass
    /// (motion on glass, `anim.rs`). Without it card and content fade as one.
    pub fn set_content(&self, content: &impl IsA<gtk4::Widget>) {
        if let Some(reveal) = &self.inner.reveal {
            reveal.set_content(content);
        }
    }

    /// What the surface is transitioning toward.
    pub fn is_shown(&self) -> bool {
        match &self.inner.reveal {
            Some(reveal) => reveal.is_shown(),
            None => self.inner.window.is_visible(),
        }
    }

    /// Map, and fade in when there is a card to fade.
    pub fn show(&self) {
        match &self.inner.reveal {
            Some(reveal) => reveal.show(),
            None => self.inner.window.set_visible(true),
        }
    }

    /// Fade out and unmap (or just unmap, without a card).
    pub fn hide(&self) {
        match &self.inner.reveal {
            Some(reveal) => reveal.hide(),
            None => self.inner.window.set_visible(false),
        }
    }

    /// Run `f` once a hide has finished and the surface is unmapped: the
    /// moment it is safe to drop the last handle. A surface without a card
    /// unmaps at once, so there is nothing to wait for and nothing is run.
    pub fn connect_hidden(&self, f: impl Fn() + 'static) {
        if let Some(reveal) = &self.inner.reveal {
            reveal.connect_hidden(f);
        }
    }

    /// Whether the surface may take the keyboard, on a click
    /// (`OnDemand`), or never.
    pub fn set_wants_keyboard(&self, wants: bool) {
        self.inner.window.set_keyboard_mode(if wants {
            KeyboardMode::OnDemand
        } else {
            KeyboardMode::None
        });
    }

    /// Run `f` on a click that lands on the root outside the card: the
    /// dismiss gesture of a surface that spans its output. A click on the
    /// card, including its padding, never reaches `f`.
    pub fn connect_backdrop_click(&self, f: impl Fn() + 'static) {
        let gesture = gtk4::GestureClick::new();
        let root = self.inner.root.downgrade();
        let card = self.inner.card.as_ref().map(|c| c.downgrade());
        gesture.connect_released(move |_, _, x, y| {
            let Some(root) = root.upgrade() else { return };
            let on_card = card.as_ref().and_then(|c| c.upgrade()).is_some_and(|card| {
                root.pick(x, y, gtk4::PickFlags::DEFAULT)
                    .is_some_and(|hit| hit == card || hit.is_ancestor(&card))
            });
            if !on_card {
                f();
            }
        });
        self.inner.root.add_controller(gesture);
    }
}

impl<'a> Builder<'a> {
    /// Put the surface on `monitor`. `None` leaves the output to the
    /// compositor, which picks the focused one when the surface maps.
    pub fn monitor(mut self, monitor: Option<&gdk::Monitor>) -> Self {
        self.monitor = monitor.cloned();
        self
    }

    pub fn layer(mut self, layer: Layer) -> Self {
        self.layer = layer;
        self
    }

    /// Anchor to each of `edges`.
    pub fn anchor(mut self, edges: &[Edge]) -> Self {
        self.anchors.extend(edges.iter().map(|&e| (e, true)));
        self
    }

    /// Anchor to all four edges: the surface is its output.
    pub fn fill(self) -> Self {
        self.anchor(&[Edge::Top, Edge::Bottom, Edge::Left, Edge::Right])
    }

    pub fn margin(mut self, edge: Edge, px: i32) -> Self {
        self.margins.push((edge, px));
        self
    }

    pub fn keyboard(mut self, mode: KeyboardMode) -> Self {
        self.keyboard = mode;
        self
    }

    /// Reserve the surface's own size from its anchored edge (a bar).
    pub fn exclusive(mut self) -> Self {
        self.exclusive = true;
        self
    }

    /// Lie over everyone else's exclusive zone rather than inside what they
    /// leave: a stage that must cover the whole output, bar included.
    pub fn over_exclusive_zones(mut self) -> Self {
        self.ignore_exclusive_zones = true;
        self
    }

    /// The size asked for on an axis nothing anchors. Zero on such an axis
    /// asks the compositor to choose, and it chooses the whole output.
    pub fn width(mut self, px: i32) -> Self {
        self.width = Some(px);
        self
    }

    pub fn height(mut self, px: i32) -> Self {
        self.height = Some(px);
        self
    }

    /// Fully opaque. Every other surface keeps the near-unity opacity that
    /// forces compositor blending ([`layer::make_layer_window`]); a surface
    /// that paints the whole output itself must not let the live screen
    /// through it.
    pub fn opaque(mut self) -> Self {
        self.opaque = true;
        self
    }

    /// Override whether GTK may size the window away from its natural size
    /// (see `build`). Only for a surface whose size is its default size and
    /// not its content's: the locker's one-pixel warm-up window, which has no
    /// content, and must not ask the compositor for a size of zero.
    pub fn resizable(mut self, resizable: bool) -> Self {
        self.resizable = Some(resizable);
        self
    }

    /// A glass card of `kind` on the root, with the transition that owns it.
    pub fn card(mut self, kind: crate::ui::Card) -> Self {
        self.card = Some(Some(kind));
        self
    }

    /// No card and no transition: a full-screen stage, or a surface that
    /// draws something other than a card and maps and unmaps outright.
    pub fn no_card(mut self) -> Self {
        self.card = Some(None);
        self
    }

    /// Slide the card `px` along `axis` as it fades: in from there on show,
    /// back out there on hide, on the fade's own clock (`anim::Reveal::slide`).
    pub fn slide(mut self, axis: gtk4::Orientation, px: f64) -> Self {
        self.slide = Some((axis, px));
        self
    }

    /// Put something between the root and the card (or its slide bin): `f`
    /// is given that widget and returns what the root holds instead. For a
    /// card placed inside its surface by a parent of its own (a notification
    /// in its column).
    pub fn wrap(mut self, f: impl FnOnce(&gtk4::Widget) -> gtk4::Widget + 'a) -> Self {
        self.wrap = Some(Box::new(f));
        self
    }

    pub fn build(self) -> Surface {
        let card_kind = self
            .card
            .expect("Surface::builder: say .card(kind) or .no_card()");
        let config = LayerShellConfig {
            namespace: self.namespace,
            layer: self.layer,
            default_width: self.width,
            default_height: self.height,
            anchors: &self.anchors,
            margins: &self.margins,
            keyboard_mode: self.keyboard,
            exclusive: self.exclusive,
        };
        let window = layer::create_layer_window_on(self.app, &config, self.monitor.as_ref());
        window.set_decorated(false);
        // A surface stretched between two opposite edges takes its size on
        // that axis from the compositor's configure. Non-resizable, GTK would
        // pin it to its natural size instead (a bar that ends after its
        // clock); everything else sizes to its content and must not grow.
        let anchored = |e: Edge| self.anchors.iter().any(|&(a, on)| a == e && on);
        let stretched = (anchored(Edge::Left) && anchored(Edge::Right))
            || (anchored(Edge::Top) && anchored(Edge::Bottom));
        window.set_resizable(self.resizable.unwrap_or(stretched));
        if self.ignore_exclusive_zones {
            window.set_exclusive_zone(-1);
        }
        if self.opaque {
            window.set_opacity(1.0);
        }

        let root = crate::ui::vbox(0);
        // Base type and colour on the window's child, not the window: the
        // GTK theme's `window.background` outranks a class on the window.
        crate::ui::surface::adopt(&root);

        let card = card_kind.map(|kind| {
            let card = crate::ui::vbox(0);
            crate::ui::card::adopt(&card, kind);
            card
        });
        let slide = self.slide.and_then(|(axis, px)| {
            card.as_ref()?;
            let bin = match axis {
                gtk4::Orientation::Horizontal => SlideBin::horizontal(),
                _ => SlideBin::new(),
            };
            // The hidden pose, so a surface mapped before its first show
            // (the panel is presented once at startup) never draws a frame
            // of the card at rest.
            bin.jump_to(px);
            Some((bin, px))
        });

        let held: Option<gtk4::Widget> = match (&card, &slide) {
            (Some(card), Some((bin, _))) => {
                bin.set_child(card);
                Some(bin.clone().upcast())
            }
            (Some(card), None) => Some(card.clone().upcast()),
            _ => None,
        };
        if let Some(held) = held {
            let held = match self.wrap {
                Some(wrap) => wrap(&held),
                None => held,
            };
            root.append(&held);
        }
        window.set_child(Some(&root));

        let reveal = card.as_ref().map(|card| {
            let reveal = anim::Reveal::new(&window, card);
            match &slide {
                Some((bin, px)) => reveal.slide(bin, *px),
                None => reveal,
            }
        });

        Surface {
            inner: Rc::new(Inner {
                window,
                root,
                card,
                slide: slide.map(|(bin, _)| bin),
                reveal,
            }),
        }
    }
}
