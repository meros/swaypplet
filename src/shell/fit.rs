//! A card centred on the output it opened on, a quarter of the way down:
//! the launcher's and the panel's placement, fitted to that output rather
//! than to whichever screen GDK happened to list first.
//!
//! [`install_monitor_fit`] wires a card and the spacer above it to the
//! window's output; [`CardSize`] is what the card asks for with room to
//! spare.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::glib;
use gtk4::prelude::*;

/// Breathing room kept between the card and the screen edges when the card's
/// preferred size does not fit the output it opened on.
const CARD_SIDE_MARGIN: i32 = 16;
const CARD_BOTTOM_MARGIN: i32 = 24;

/// Marks a card that had to give up vertical density to fit its output.
/// data/css/09-helm.css answers it by dropping the fixed minimum heights that would
/// otherwise hold the card taller than the screen it is on.
const COMPACT_CLASS: &str = "card-compact";

/// Offset used until the compositor tells us which output the surface landed
/// on. A quarter of 1080p, so the card is roughly in place on the first frame
/// of a display we have not measured yet.
const FALLBACK_TOP_OFFSET: i32 = 270;

/// The size a card asks for when the screen has room, and the sizes it is
/// willing to shrink to when the screen has not.
///
/// `height` is `None` for a card that sizes itself to its content, which is
/// then never given a height request at all. The panel is that case: its
/// sections decide how tall it is.
pub struct CardSize {
    pub width: i32,
    pub height: Option<i32>,
}

/// Everything the fit needs about one card: its preferred size, and how to
/// ask it to give up vertical density when the output is too short for it.
///
/// The density hook is a callback rather than a CSS class because the fit
/// measures the card immediately after asking it to thin out, and GTK
/// validates style lazily. A class added here does not reach `measure` until
/// a later frame, so the fit would go on placing a card of the height it used
/// to have. Widget properties change the measurement on the spot, so the
/// caller sets those and the fit stays the only thing that reads geometry.
struct Fit {
    size: CardSize,
    on_compact: Option<Rc<dyn Fn(bool)>>,
    /// The output size the card was last fitted to.
    ///
    /// Deciding whether the card has to thin out means asking it for full
    /// density and measuring, which queues a resize, which brings us straight
    /// back here through the `layout` hook. GDK cuts that off with "layout
    /// continuously requested, giving up after 4 tries" and the panel never
    /// appears. Repeating the fit for an output whose size has not changed
    /// cannot reach a different answer, so it is skipped.
    fitted_to: std::cell::Cell<Option<(i32, i32)>>,
}

impl Fit {
    fn set_compact(&self, compact: bool) {
        if let Some(on_compact) = &self.on_compact {
            on_compact(compact);
        }
    }
}

/// Fit `card` to the monitor the window is actually on: clamp its size
/// request to what the output can show, then size `top_spacer` so the card
/// sits at the optical foveal sweet spot, a quarter of the way down.
///
/// Every part of this fixes the same class of bug, a fixed number that
/// silently assumes one screen's geometry. The offset came from
/// `monitors().item(0)`, whatever GDK happened to enumerate first, so a
/// 2560x1440 primary handed a 360 px spacer to a card opening on a 1440x900
/// secondary and pushed its lower half past the bottom edge. The sizes came
/// from literals in the builders and from CSS minimums taller than the screen
/// the card had to fit, with no way for anyone to scroll or drag the card
/// back into view.
///
/// Everything is recomputed on map, whenever the surface enters another
/// output, whenever the compositor sends a new size, and whenever the
/// output's geometry changes.
///
/// The card is not re-fitted while it stays mapped and its content grows.
/// Doing that would mean queueing a resize from inside layout, and both cards
/// settle their content before they are shown, so map time is late enough.
pub fn install_monitor_fit(
    window: &gtk4::Window,
    top_spacer: &gtk4::Box,
    card: &impl IsA<gtk4::Widget>,
    size: CardSize,
    on_compact: Option<Rc<dyn Fn(bool)>>,
) {
    let card = card.as_ref().clone();
    let fit = Rc::new(Fit {
        size,
        on_compact,
        fitted_to: std::cell::Cell::new(None),
    });

    // The preferred size and offset stand in until an output is known, so the
    // card looks exactly as designed on a screen big enough to hold it and
    // never flashes at some placeholder size on the way there.
    card.set_width_request(fit.size.width);
    if let Some(height) = fit.size.height {
        card.set_height_request(height);
    }
    top_spacer.set_height_request(FALLBACK_TOP_OFFSET);

    // The geometry watch has to move with the surface: a mode change on an
    // output we already left must not resize this card.
    let tracked: Rc<RefCell<Option<(gtk4::gdk::Monitor, glib::SignalHandlerId)>>> =
        Rc::new(RefCell::new(None));

    // Wayland delivers wl_surface.enter after the surface is mapped, so the
    // map pass usually has no output to measure yet and the enter below is
    // what places the card. Both paths are wired because a remap onto the
    // same output emits no enter.
    window.connect_map({
        let top_spacer = top_spacer.clone();
        let card = card.clone();
        let fit = fit.clone();
        let tracked = tracked.clone();
        move |window| refresh_monitor_fit(window, &top_spacer, &card, &fit, &tracked)
    });

    window.connect_realize({
        let top_spacer = top_spacer.clone();
        let card = card.clone();
        let fit = fit.clone();
        let tracked = tracked.clone();
        move |window| {
            let Some(surface) = window.surface() else {
                return;
            };
            surface.connect_enter_monitor({
                let window = window.downgrade();
                let top_spacer = top_spacer.clone();
                let card = card.clone();
                let fit = fit.clone();
                let tracked = tracked.clone();
                move |_, _| {
                    if let Some(window) = window.upgrade() {
                        refresh_monitor_fit(&window, &top_spacer, &card, &fit, &tracked);
                    }
                }
            });

            // The compositor's configure is the only place the surface's real
            // height comes from, and it arrives after both the map and the
            // enter that precede it. Without this the fit runs against a
            // height that is either uninitialised or left over from the
            // output the surface just left. `Fit::fitted_to` is what keeps
            // this from becoming a resize loop.
            surface.connect_layout({
                let window = window.downgrade();
                let top_spacer = top_spacer.clone();
                let card = card.clone();
                let fit = fit.clone();
                let tracked = tracked.clone();
                move |_, _, _| {
                    if let Some(window) = window.upgrade() {
                        refresh_monitor_fit(&window, &top_spacer, &card, &fit, &tracked);
                    }
                }
            });
        }
    });
}

/// Resolve the current output, re-hang the geometry watch if it changed, then
/// re-fit. Does nothing while the surface has no output yet, leaving whatever
/// size and offset were set last.
fn refresh_monitor_fit(
    window: &gtk4::Window,
    top_spacer: &gtk4::Box,
    card: &gtk4::Widget,
    fit: &Rc<Fit>,
    tracked: &Rc<RefCell<Option<(gtk4::gdk::Monitor, glib::SignalHandlerId)>>>,
) {
    let Some(monitor) = surface_monitor(window) else {
        return;
    };

    let unchanged = tracked
        .borrow()
        .as_ref()
        .is_some_and(|(watched, _)| watched == &monitor);
    if !unchanged {
        if let Some((previous, handler)) = tracked.borrow_mut().take() {
            previous.disconnect(handler);
        }
        // Weak on the widgets, because the monitor outlives the window and a
        // strong capture would keep a closed launcher alive for the session.
        let handler = monitor.connect_geometry_notify({
            let window = window.downgrade();
            let top_spacer = top_spacer.downgrade();
            let card = card.downgrade();
            let fit = fit.clone();
            let tracked = Rc::downgrade(tracked);
            move |_| {
                if let (Some(window), Some(top_spacer), Some(card), Some(tracked)) = (
                    window.upgrade(),
                    top_spacer.upgrade(),
                    card.upgrade(),
                    tracked.upgrade(),
                ) {
                    refresh_monitor_fit(&window, &top_spacer, &card, &fit, &tracked);
                }
            }
        });
        *tracked.borrow_mut() = Some((monitor.clone(), handler));
    }

    fit_card(window, &monitor, top_spacer, card, fit);
}

/// The height the card actually has to play with.
///
/// The monitor is the wrong number on an output carrying a bar. The panel's
/// layer surface is anchored to all four edges, so the compositor hands it
/// the output minus the bar's exclusive zone, measured here at 61 logical px,
/// and a card fitted to the full monitor puts its bottom row behind the bar.
///
/// Only the surface knows the real figure, and it knows it late: before its
/// first configure it reports a height of 1, and just after it moves output
/// it still reports the size the previous output gave it.
///
/// So it is believed only when it is at least half the output, which no bar
/// eats into, and capped at the output, which no configure can exceed. A
/// height of 1 taken at face value would fit the card to nothing and squeeze
/// its rows below their minimum for a frame, which GTK complains about by the
/// screenful. Whatever this returns early is corrected by the `layout` hook
/// in [`install_monitor_fit`] as soon as the real size arrives.
fn usable_height(window: &gtk4::Window, monitor: &gtk4::gdk::Monitor) -> i32 {
    let screen = monitor.geometry().height();
    window
        .surface()
        .map(|surface| surface.height())
        .filter(|height| height * 2 >= screen)
        .map_or(screen, |height| height.min(screen))
}

/// The output this window is displayed on.
///
/// `monitor_at_surface` answers from the output the compositor sent
/// wl_surface.enter for, which is also the right answer when a layer surface
/// was pinned to an output explicitly. It is `None` until that enter arrives.
fn surface_monitor(window: &gtk4::Window) -> Option<gtk4::gdk::Monitor> {
    let surface = window.surface()?;
    gtk4::gdk::Display::default()?.monitor_at_surface(&surface)
}

/// Clamp the card's size request to the output, thin it out if it still does
/// not fit, then place it.
///
/// The order matters. Width is settled first because the card's minimum
/// height is a function of its width: rows wrap and grow taller as the card
/// narrows, and an offset computed against the wide measurement would put the
/// card back off the bottom of the screen.
fn fit_card(
    window: &gtk4::Window,
    monitor: &gtk4::gdk::Monitor,
    top_spacer: &gtk4::Box,
    card: &gtk4::Widget,
    fit: &Fit,
) {
    // Monitor geometry is in logical pixels, the same space size requests and
    // the spacer's height are in, so a scaled output needs no conversion.
    let screen_width = monitor.geometry().width();
    let screen_height = usable_height(window, monitor);

    if fit.fitted_to.get() == Some((screen_width, screen_height)) {
        return;
    }
    fit.fitted_to.set(Some((screen_width, screen_height)));

    // The clamp lowers the request, which is a floor, so it cannot pull a
    // card below the minimum width its own content demands: GTK allocates the
    // larger of the two. Rows that wrap are what let the content minimum fall
    // far enough for this to bite.
    let usable_width = (screen_width - 2 * CARD_SIDE_MARGIN).max(0);
    card.set_width_request(fit.size.width.min(usable_width));

    // Ask the card what it will really be given rather than assuming it got
    // the request: for a card whose content is wider than the clamp, the two
    // differ, and measuring a height for a width the card has already refused
    // is both an over-estimate and a GTK warning.
    let (width, _, _, _) = card.measure(gtk4::Orientation::Horizontal, -1);

    let room = (screen_height - CARD_BOTTOM_MARGIN).max(0);
    if let Some(height) = fit.size.height {
        // A fixed-height card flush against the top is the tallest it can
        // ever be here, so that is the ceiling. The offset below then decides
        // where in the remaining room it actually sits.
        card.set_height_request(height.min(room));
    }

    // A short output needs the card thinned before it is placed, because the
    // lists inside it hold a floor tall enough on its own to outgrow the whole
    // screen, and no offset can rescue a card that does not fit at any offset.
    //
    // Full density is asked for first every time, so a card that visits a
    // small output once does not stay thin for the rest of the session.
    fit.set_compact(false);
    card.remove_css_class(COMPACT_CLASS);
    let mut card_min = card.measure(gtk4::Orientation::Vertical, width).0;
    if card_min > room {
        fit.set_compact(true);
        // The class only carries padding and margin trims, which the callback
        // above cannot express. It lands a frame late, and that is harmless
        // here: everything it changes makes the card shorter, so measuring
        // without it errs towards leaving the card more room than it needs.
        card.add_css_class(COMPACT_CLASS);
        card_min = card.measure(gtk4::Orientation::Vertical, width).0;
    }

    // A card taller than three quarters of the screen has to start above the
    // sweet spot, or its bottom rows, the launcher's last results among them,
    // sit off-screen where nothing can reach them.
    let limit = (room - card_min).max(0);
    top_spacer.set_height_request((screen_height / 4).min(limit));
}
