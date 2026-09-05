//! One frosted card on its own layer-shell surface.
//!
//! Everything swaypplet puts on screen is the same object: a `.glass-card`
//! pane on a transparent apron, frosted by the compositor, that fades in and
//! out as a material. [`GlassSurface`] is that object, so building a new UI
//! is choosing a corner of the screen and handing over the content.
//!
//! It also settles a question the notification stack used to answer on its
//! own. A stack of cards can be drawn two ways: many cards on one surface, or
//! a surface per card. One surface is cheaper and was the original choice,
//! and it costs more than it saves. The compositor's frost and its alpha are
//! per surface, so cards sharing one cannot fade their own material and have
//! to fake it; the shared surface must hand-roll an input region, because it
//! covers ground no card occupies; and it stays mapped with nothing on it,
//! which is how it came to hold a ghost of the last card that left. A surface
//! per card has none of those problems, and the compositor is built for it.
//!
//! Placement is client-side on purpose. Layer-shell margins are protocol
//! state, so animating a slot by moving the surface is a configure round trip
//! per frame; each surface instead spans the column it lives in and the card
//! is translated inside it ([`anim::SlideBin`]), which keeps the motion on
//! the client's own frame clock where the rest of the animation already is.

use std::cell::Cell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4_layer_shell::LayerShell;

use crate::anim::{self, SlideBin};
use crate::layer_shell::{self, LayerShellConfig};

/// A card, its surface, and the transitions both need.
#[derive(Clone)]
pub struct GlassSurface {
    inner: Rc<Inner>,
}

struct Inner {
    window: gtk4::Window,
    /// The `.glass-card` pane. Its alpha is what the compositor stencils
    /// blur against, so [`anim::Reveal`] owns it outright.
    pane: gtk4::Box,
    /// Vertical placement within the column, plus the collapsed scale.
    place: SlideBin,
    /// The horizontal settle: the entrance, and the drag that dismisses.
    settle: SlideBin,
    reveal: anim::Reveal,
    /// Where the card currently sits, so the input region can follow it
    /// without measuring the widget tree again.
    offset: Cell<f64>,
    scale: Cell<f64>,
}

/// Take the surface down with the last handle to it.
///
/// The window is the application's from the moment it is built
/// ([`layer_shell::create_layer_window`] passes `.application(app)`), and a
/// `GtkApplication` holds that reference until the window is *destroyed*.
/// Hiding only unmaps: the window stays on the application's list, realized,
/// still owning the buffers the renderer allocated for it.
///
/// That is invisible for a surface built once and reused, and it is a leak
/// for the notification stack, which builds one per card. Every stranded
/// surface held four dmabufs and eight DRM syncobj fds open, so about
/// eighty-five notifications walked the process into its file-descriptor
/// limit; the panel then died wherever the next fd was asked for, usually
/// `vkCreateSwapchainKHR` or the Wayland dispatch.
///
/// Destroying here rather than at the notification's call site keeps the
/// module's contract in one piece: the surface exists while a handle to it
/// does, and `connect_hidden` stays the moment it is safe to drop the last
/// one.
impl Drop for Inner {
    fn drop(&mut self) {
        // Order is load-bearing: the compositor alpha handle is bound to this
        // window's `wl_surface` and has to be destroyed before it, so it goes
        // first and the window follows.
        self.reveal.release_alpha();
        self.window.destroy();
    }
}

impl GlassSurface {
    /// Build the surface. The pane starts empty; fill it through [`pane`]
    /// and point the transition at whatever you put there with
    /// [`set_content`].
    ///
    /// `settle_px` is the horizontal distance the card travels as it fades,
    /// giving the entrance a direction; 0 for a card that simply appears.
    ///
    /// [`pane`]: Self::pane
    /// [`set_content`]: Self::set_content
    pub fn new(app: &gtk4::Application, config: &LayerShellConfig, settle_px: f64) -> Self {
        let window = layer_shell::create_layer_window(app, config);
        window.set_resizable(false);
        window.set_decorated(false);

        let pane = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .build();
        pane.add_css_class("glass-card");

        // Two bins, one axis each: the outer one is the card's slot in its
        // column, the inner one is its entrance. Nested so neither
        // animation's tick can tread on the other's.
        let settle = SlideBin::horizontal();
        settle.set_child(&pane);
        let place = SlideBin::new();
        place.set_child(&settle);
        window.set_child(Some(&place));

        let reveal = anim::Reveal::new(&window, &pane).slide(&settle, settle_px);

        GlassSurface {
            inner: Rc::new(Inner {
                window,
                pane,
                place,
                settle,
                reveal,
                offset: Cell::new(0.0),
                scale: Cell::new(1.0),
            }),
        }
    }

    pub fn window(&self) -> &gtk4::Window {
        &self.inner.window
    }

    pub fn pane(&self) -> &gtk4::Box {
        &self.inner.pane
    }

    pub fn is_shown(&self) -> bool {
        self.inner.reveal.is_shown()
    }

    /// Point the transition at the content, after filling the pane.
    pub fn set_content(&self, content: &impl IsA<gtk4::Widget>) {
        self.inner.reveal.set_content(content);
    }

    /// Follow a drag: the card tracks the finger and fades with distance, so
    /// a release that dismisses carries on from where it was let go.
    pub fn drag_to(&self, dx: f64, alpha: f64) {
        self.inner.settle.jump_to(dx);
        self.inner.reveal.set_alpha(alpha);
    }

    /// Put a drag that did not reach the threshold back in its slot.
    pub fn settle_back(&self, ms: f64) {
        self.inner.settle.slide_to(0.0, ms);
        self.inner.reveal.set_alpha(1.0);
    }

    /// Map and fade in.
    pub fn show(&self) {
        self.inner.reveal.show();
    }

    /// Fade out and unmap. The surface is gone for good once `connect_hidden`
    /// fires, which is the only safe moment to drop the card that owns it.
    pub fn hide(&self) {
        self.inner.reveal.hide();
    }

    pub fn connect_hidden(&self, f: impl Fn() + 'static) {
        self.inner.reveal.connect_hidden(f);
    }

    /// Move the card to `y` within its column over `ms`, or put it there
    /// outright when `ms` is zero (a card that has not been shown yet).
    pub fn place_at(&self, y: f64, ms: f64) {
        self.inner.offset.set(y);
        if ms <= 0.0 {
            self.inner.place.jump_to(y);
        } else {
            self.inner.place.slide_to(y, ms);
        }
    }

    /// Shrink the card in place, for the collapsed tail of a stack. Render
    /// only, so it cannot disturb layout or the surface's geometry.
    pub fn set_scale(&self, scale: f64) {
        self.inner.scale.set(scale);
        self.inner.place.set_scale(scale);
    }

    /// Only the card takes clicks; the rest of the column is transparent and
    /// must fall through to whatever is behind it.
    ///
    /// A surface accepts input across its whole area by default, which for a
    /// column-sized surface holding one card is mostly a hole in the desktop.
    pub fn clip_input_to_card(&self) {
        let Some(surface) = self.inner.window.surface() else {
            return;
        };
        let (width, height) = self.natural_size();
        let scale = self.inner.scale.get();
        let w = (width as f64 * scale).ceil() as i32;
        let h = (height as f64 * scale).ceil() as i32;
        // A card hugs one side of its column; the region hugs the same one.
        let x = if self.inner.pane.halign() == gtk4::Align::Start {
            self.inner.pane.margin_start()
        } else {
            self.inner.window.width() - w
        };
        let region = cairo::Region::create_rectangle(&cairo::RectangleInt::new(
            x.max(0),
            self.inner.offset.get().floor() as i32,
            w,
            h,
        ));
        surface.set_input_region(Some(&region));
    }

    /// Whether this card can be typed into. Off by default and for almost
    /// every card: a surface that appears unbidden has no business holding
    /// the keyboard, and `OnDemand` only ever takes focus on a click.
    pub fn set_wants_keyboard(&self, wants: bool) {
        use gtk4_layer_shell::KeyboardMode;
        self.inner.window.set_keyboard_mode(if wants {
            KeyboardMode::OnDemand
        } else {
            KeyboardMode::None
        });
    }

    /// The card's natural height, for stacking the ones below it.
    pub fn height(&self) -> f64 {
        f64::from(self.natural_size().1)
    }

    /// The card's natural size, width first and the height *for that width*.
    ///
    /// The order is the whole point. Asking for the height at `-1` asks every
    /// label inside for its height at its own natural width, and the cards
    /// deliberately collapse those to one character (`max_width_chars(1)` in
    /// notifications/popup.rs) so the pane's size request is what drives
    /// allocation. A wrapped, line-capped body answers that question with its
    /// full line cap whatever it actually holds: a one-line body measured
    /// 76 px where it renders 40, so the stack slotted every card as if its
    /// body ran the full three lines and left a hole under the short ones.
    /// Measure the width, then the height at that width — which is what GTK
    /// itself does when it allocates, and the only way a height-for-width
    /// widget answers truthfully.
    fn natural_size(&self) -> (i32, i32) {
        let (_, width, _, _) = self.inner.pane.measure(gtk4::Orientation::Horizontal, -1);
        let (_, height, _, _) = self.inner.pane.measure(gtk4::Orientation::Vertical, width);
        (width, height)
    }
}
