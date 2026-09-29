//! One popup card on its own layer surface: a [`Surface`] plus what only the
//! notification stack needs, a slot in its column, the collapsed scale, the
//! drag that dismisses, and an input region that covers the card alone.
//!
//! A surface per card rather than one surface for the stack, because the
//! compositor's frost and its alpha are per surface: cards sharing one
//! cannot fade their own material and have to fake it, the shared surface
//! must hand-roll an input region over ground no card occupies, and it stays
//! mapped with nothing on it (which is how it came to hold a ghost of the
//! last card that left).
//!
//! Placement is client-side on purpose. Layer-shell margins are protocol
//! state, so animating a slot by moving the surface is a configure round
//! trip per frame; each surface instead spans the column it lives in and the
//! card is translated inside it ([`SlideBin`]), which keeps the motion on
//! the client's own frame clock where the rest of the animation already is.
//! Two bins, one axis each: the outer one is the card's slot in its column,
//! the inner one (the Surface's slide) its entrance and its drag, nested so
//! neither animation's tick can tread on the other's.

use std::cell::Cell;
use std::rc::Rc;

use gtk4::gdk;
use gtk4::prelude::*;
use gtk4_layer_shell::Edge;

use crate::anim::{self, SlideBin};
use crate::shell::{Namespace, Surface};

/// A card, its surface, and the transitions both need. Cloning shares it;
/// the surface is destroyed with the last handle, which is why the stack
/// drops a card only from `connect_hidden`.
#[derive(Clone)]
pub struct CardSurface {
    inner: Rc<Inner>,
}

struct Inner {
    surface: Surface,
    /// Vertical placement within the column, plus the collapsed scale.
    place: SlideBin,
    /// Where the card currently sits, so the input region can follow it
    /// without measuring the widget tree again.
    offset: Cell<f64>,
    scale: Cell<f64>,
}

impl CardSurface {
    /// Build the surface for a column anchored to `anchors`, `width` by
    /// `height`. The card starts empty; fill it through [`pane`](Self::pane)
    /// and point the transition at what you put there with
    /// [`set_content`](Self::set_content).
    ///
    /// `monitor` names the output; `None` leaves the choice to the
    /// compositor, which uses the output that has focus when the surface
    /// maps. Naming it is what lets the stack lay out one column per screen.
    pub fn new(
        app: &gtk4::Application,
        anchors: &[Edge],
        width: i32,
        height: i32,
        monitor: Option<&gdk::Monitor>,
    ) -> Self {
        let place = SlideBin::new();
        let surface = Surface::builder(app, Namespace::Notification)
            .monitor(monitor)
            .anchor(anchors)
            .width(width)
            .height(height)
            .card(crate::ui::Card::Floating)
            // The entrance: the card travels in from the side as it fades.
            .slide(gtk4::Orientation::Horizontal, anim::SLIDE_PX)
            .wrap({
                let place = place.clone();
                move |settle| {
                    place.set_child(settle);
                    // The whole column, so a slot can be anywhere in it.
                    place.set_hexpand(true);
                    place.set_vexpand(true);
                    place.upcast()
                }
            })
            .build();
        CardSurface {
            inner: Rc::new(Inner {
                surface,
                place,
                offset: Cell::new(0.0),
                scale: Cell::new(1.0),
            }),
        }
    }

    /// The card (`.ui-card`).
    pub fn pane(&self) -> &gtk4::Box {
        self.inner.surface.card()
    }

    pub fn is_shown(&self) -> bool {
        self.inner.surface.is_shown()
    }

    /// Point the transition at the content, after filling the pane.
    pub fn set_content(&self, content: &impl IsA<gtk4::Widget>) {
        self.inner.surface.set_content(content);
    }

    fn settle(&self) -> &SlideBin {
        self.inner
            .surface
            .slide()
            .expect("a notification card is built with a slide")
    }

    fn reveal(&self) -> &anim::Reveal {
        self.inner
            .surface
            .reveal()
            .expect("a notification card is built with a card")
    }

    /// Follow a drag: the card tracks the finger and fades with distance, so
    /// a release that dismisses carries on from where it was let go.
    pub fn drag_to(&self, dx: f64, alpha: f64) {
        self.settle().jump_to(dx);
        self.reveal().set_alpha(alpha);
    }

    /// Put a drag that did not reach the threshold back in its slot.
    pub fn settle_back(&self, ms: f64) {
        self.settle().slide_to(0.0, ms);
        self.reveal().set_alpha(1.0);
    }

    /// Map and fade in.
    pub fn show(&self) {
        self.inner.surface.show();
    }

    /// Fade out and unmap. The surface is gone for good once `connect_hidden`
    /// fires, which is the only safe moment to drop the card that owns it.
    pub fn hide(&self) {
        self.inner.surface.hide();
    }

    pub fn connect_hidden(&self, f: impl Fn() + 'static) {
        self.inner.surface.connect_hidden(f);
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
        let window = self.inner.surface.window();
        let Some(surface) = window.surface() else {
            return;
        };
        let (width, height) = self.natural_size();
        let scale = self.inner.scale.get();
        let w = (width as f64 * scale).ceil() as i32;
        let h = (height as f64 * scale).ceil() as i32;
        // A card hugs one side of its column; the region hugs the same one.
        let pane = self.pane();
        let x = if pane.halign() == gtk4::Align::Start {
            pane.margin_start()
        } else {
            window.width() - w
        };
        let region = cairo::Region::create_rectangle(&cairo::RectangleInt::new(
            x.max(0),
            self.inner.offset.get().floor() as i32,
            w,
            h,
        ));
        surface.set_input_region(Some(&region));
    }

    /// Whether this card takes the keyboard when it is clicked (`OnDemand`),
    /// or never. A surface that appears unbidden has no business holding the
    /// keyboard, and `OnDemand` only ever takes it on a click.
    pub fn set_wants_keyboard(&self, wants: bool) {
        self.inner.surface.set_wants_keyboard(wants);
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
    /// notifications/card.rs) so the pane's size request is what drives
    /// allocation. A wrapped, line-capped body answers that question with its
    /// full line cap whatever it actually holds: a one-line body measured
    /// 76 px where it renders 40, so the stack slotted every card as if its
    /// body ran the full three lines and left a hole under the short ones.
    /// Measure the width, then the height at that width — which is what GTK
    /// itself does when it allocates, and the only way a height-for-width
    /// widget answers truthfully.
    fn natural_size(&self) -> (i32, i32) {
        let pane = self.pane();
        let (_, width, _, _) = pane.measure(gtk4::Orientation::Horizontal, -1);
        let (_, height, _, _) = pane.measure(gtk4::Orientation::Vertical, width);
        (width, height)
    }
}
