//! The popup notification stack at the top-right.
//!
//! Each card is its own [`GlassSurface`], which is what makes the rest of
//! this file short. The compositor's frost and its alpha are per surface, so
//! a card that owns one fades as a material: the blur collapses with the
//! tint instead of hanging on as a frosted rectangle holding no colour. It
//! also unmaps when it goes, so it cannot leave anything of itself behind,
//! and it takes only the clicks that land on it.
//!
//! What is left here is the choreography: which card sits where, which one
//! gives way when the stack is full, and when each one expires. The
//! transitions themselves belong to `anim::Reveal`, and the surface to
//! `surface::GlassSurface`.
//!
//! One column per output, not one column. A card is pinned to the focused
//! output when it is created (`sway::ipc::focused_output_from`: the panel's
//! `SwayService` when it runs one, else a round trip), and both the
//! layout and the depth cap are per pinned output. They used to be global
//! while the surfaces were placed by the compositor: a notification arriving
//! on the screen you were using pushed the OTHER screen's cards down a slot,
//! and could evict one of them, on a display it was not even on. A card that
//! cannot be pinned (no sway, or an output GDK does not have) keeps the old
//! behaviour, sharing one column with every other unpinned card.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gtk4::prelude::*;
use gtk4_layer_shell::Edge;

use super::card::{age_label, populate_card, set_critical_class, wants_keyboard};
use super::timers::{Timer, cancel_timer, make_timer, pause_timers, resume_timers};
use crate::anim;
use crate::shell::layer::LayerShellConfig;
use crate::services::notifications::store::{self, NotificationStore};
use crate::services::notifications::{CloseReason, Notification};
use crate::settings::store::{Alerts, Corner};
use crate::surface::GlassSurface;
use crate::sway::ipc::SwayService;

// ── Stack geometry ──
pub(super) const CARD_WIDTH: i32 = 360;
// Offset of the card column from the screen's top/right corner
const EDGE_MARGIN: i32 = crate::tokens::space(4);
// Every card spans the whole column and is placed inside it, so the surface
// never has to move: layer-shell margins are protocol state and animating a
// slot by moving the surface would be a configure round trip per frame.
const WINDOW_WIDTH: i32 = CARD_WIDTH + 2 * EDGE_MARGIN;
const WINDOW_HEIGHT: i32 = 720;
// Cards shown at full size before older ones collapse behind the stack is
// the Alerts tab's `stack`; behind them this many more peek out before the
// oldest is evicted.
const COLLAPSED_TAIL: usize = 2;
// Vertical gap between fully visible cards
const GAP: f64 = crate::tokens::space(3) as f64;
// Collapsed cards peek out below the last full card by this much per level
const PEEK: f64 = 12.0;
const PEEK_SCALE_STEP: f64 = 0.05;
// How many cards one app may hold at once is the stack depth: one app may
// fill the cards shown at full size and no more, which still leaves the
// collapsed tail for everyone else. Past the cap its oldest card gives way
// to its newest and the overflow is counted on the survivor instead.

/// The Alerts tab, read once per card: a card keeps the corner and the
/// stack depth it was born with, so a change lands on the next card rather
/// than moving the ones on screen.
pub(super) fn alerts() -> Alerts {
    crate::settings::store::with(|s| s.alerts())
}

/// One column per corner. Anchors are protocol state on the surface, so a
/// corner is a config rather than a number.
const fn popup_config(anchors: &'static [(Edge, bool)]) -> LayerShellConfig {
    LayerShellConfig {
        namespace: crate::shell::Namespace::Notification,
        layer: gtk4_layer_shell::Layer::Overlay,
        exclusive: false,
        default_width: Some(WINDOW_WIDTH),
        default_height: Some(WINDOW_HEIGHT),
        anchors,
        margins: &[],
        keyboard_mode: gtk4_layer_shell::KeyboardMode::None,
    }
}

static POPUP_TOP_RIGHT: LayerShellConfig = popup_config(&[(Edge::Top, true), (Edge::Right, true)]);
static POPUP_TOP_LEFT: LayerShellConfig = popup_config(&[(Edge::Top, true), (Edge::Left, true)]);
static POPUP_BOTTOM_RIGHT: LayerShellConfig =
    popup_config(&[(Edge::Bottom, true), (Edge::Right, true)]);
static POPUP_BOTTOM_LEFT: LayerShellConfig =
    popup_config(&[(Edge::Bottom, true), (Edge::Left, true)]);

fn config_for(corner: Corner) -> &'static LayerShellConfig<'static> {
    match corner {
        Corner::TopRight => &POPUP_TOP_RIGHT,
        Corner::TopLeft => &POPUP_TOP_LEFT,
        Corner::BottomRight => &POPUP_BOTTOM_RIGHT,
        Corner::BottomLeft => &POPUP_BOTTOM_LEFT,
    }
}

pub(super) struct Card {
    pub(super) id: u32,
    pub(super) surface: GlassSurface,
    pub(super) timer: Timer,
    /// Which app sent it, for the per-app cap.
    pub(super) app: String,
    /// The card carries a field that has to be typed into, which is the only
    /// reason its surface ever accepts keyboard focus.
    pub(super) wants_keyboard: bool,
    /// When the notification arrived, for the age in the header.
    pub(super) stamp: std::time::SystemTime,
    /// The header's age label, so the minute tick can rewrite it without
    /// rebuilding the card under the pointer.
    pub(super) age: Option<gtk4::Label>,
    /// On its way out: still on screen, but no longer part of the stack.
    pub(super) exiting: bool,
    /// Which column the surface was anchored to, for the reflow.
    pub(super) corner: Corner,
    /// The output the surface is pinned to, by connector name, or `None`
    /// when sway could not say and the compositor placed it. Cards sharing a
    /// value share a column: a card stacks under the cards on ITS screen and
    /// is never moved by one that arrived on the other screen.
    pub(super) output: Option<String>,
}

pub(super) struct State {
    pub(super) app: gtk4::Application,
    // Oldest → newest.
    pub(super) cards: Vec<Card>,
    pub(super) store: Rc<RefCell<NotificationStore>>,
    pub(super) hovered: bool,
    /// Cards an app has had pushed off the stack while it still holds one,
    /// shown as a count on its newest card. Cleared when the app's last card
    /// goes, so the number always means "since this run of chatter began".
    pub(super) overflow: std::collections::HashMap<String, u32>,
    /// The once-a-minute age refresh. Boundary-aimed and alive only while
    /// cards are on screen (P7): a timer that outlives its reason is a
    /// wakeup for nothing.
    pub(super) age_timer: Option<glib::SourceId>,
    /// The process's sway model, when it runs one (the panel hosting the
    /// bar): where the focused output is read from, instead of a blocking
    /// round trip per notification.
    pub(super) sway: Option<Rc<SwayService>>,
}

impl State {
    pub(super) fn active(&self) -> impl Iterator<Item = &Card> {
        self.cards.iter().filter(|c| !c.exiting)
    }
}

/// Manages the popup notification stack at the top-right: newest on top,
/// up to the Alerts tab's `stack` cards fully expanded, older ones collapsed behind
/// the last full card with peeking edges.
pub struct PopupManager {
    state: Rc<RefCell<State>>,
}

impl PopupManager {
    /// Wire the stack to the store's callbacks. Nothing is on screen, and no
    /// surface exists, until a notification arrives.
    pub fn register(app: &gtk4::Application, store: Rc<RefCell<NotificationStore>>) -> Self {
        let state = Rc::new(RefCell::new(State {
            app: app.clone(),
            cards: Vec::new(),
            store: store.clone(),
            hovered: false,
            overflow: std::collections::HashMap::new(),
            age_timer: None,
            sway: None,
        }));

        {
            let st = state.clone();
            store
                .borrow_mut()
                .connect_notify(move |notif| show(&st, notif));
        }
        {
            let st = state.clone();
            store
                .borrow_mut()
                .connect_close(move |id, _reason| dismiss(&st, id));
        }
        Self { state }
    }

    /// Read the focused output from `sway` rather than asking sway per card.
    pub fn set_sway(&self, sway: Rc<SwayService>) {
        self.state.borrow_mut().sway = Some(sway);
    }
}

fn show(st: &Rc<RefCell<State>>, notif: &Notification) {
    let store = st.borrow().store.clone();
    if !store.borrow().should_popup(notif) {
        return;
    }

    let id = notif.id;

    // Replacing an existing popup: rebuild its content in place, keeping the
    // surface so the card does not blink out and back.
    // populate_card unparents the old children, which can synthesize pointer
    // crossing events whose handlers borrow the state — run it unborrowed.
    let existing = {
        let mut s = st.borrow_mut();
        let hovered = s.hovered;
        s.cards
            .iter_mut()
            .find(|c| c.id == id && !c.exiting)
            .map(|card| {
                cancel_timer(&mut card.timer);
                card.timer = make_timer(&store, notif, hovered);
                card.surface.clone()
            })
    };
    if let Some(surface) = existing {
        let overflow = overflow_for(st, &notif.app_name);
        let age = populate_card(surface.pane(), notif, &store, st, overflow);
        set_critical_class(surface.pane(), notif);
        if let Some(content) = surface.pane().first_child() {
            surface.set_content(&content);
        }
        {
            let mut s = st.borrow_mut();
            if let Some(card) = s.cards.iter_mut().find(|c| c.id == id && !c.exiting) {
                card.stamp = notif.timestamp;
                card.age = age;
                card.wants_keyboard = wants_keyboard(notif);
            }
        }
        sync_keyboard_mode(st);
        reflow(st);
        return;
    }

    // Which screen this card belongs to, decided before anything counts the
    // stack: the depth cap below is per column, and a column is one output.
    //
    // Left unpinned the compositor still puts the surface on the focused
    // output — but nothing then knows WHICH output that was, so the stack
    // laid every card out in one column regardless of screen and a
    // notification arriving on one display pushed the other display's cards
    // down. Ask sway, pin the surface, and keep the name for `reflow`.
    let sway = st.borrow().sway.clone();
    let output = crate::sway::ipc::focused_output_from(sway.as_deref());
    let monitor = output
        .as_deref()
        .and_then(crate::shell::layer::monitor_by_connector);
    if output.is_some() && monitor.is_none() {
        // Sway named an output GDK does not have. Letting the compositor
        // place the surface is still correct, so this is a note rather than
        // a failure.
        log::warn!(
            "notifications: no monitor for output {output:?}; leaving the card to the compositor"
        );
    }
    // The name is kept only when the surface really was pinned to it, so the
    // grouping below can never disagree with where a card is.
    let output = monitor.as_ref().and(output);

    // The app's own oldest card gives way before anyone else's: a burst from
    // one sender should cost that sender its slots, not the stack. Counted
    // across every screen, because this cap is about the sender rather than
    // about the column.
    let crowded = {
        let s = st.borrow();
        let mine: Vec<u32> = s
            .active()
            .filter(|c| c.app == notif.app_name)
            .map(|c| c.id)
            .collect();
        (mine.len() >= usize::from(alerts().stack)).then(|| mine[0])
    };
    if let Some(old_id) = crowded {
        start_exit(st, old_id);
        *st.borrow_mut()
            .overflow
            .entry(notif.app_name.clone())
            .or_insert(0) += 1;
    }

    // Evict the oldest popup when this screen's column is full (popup only —
    // the notification stays open in the store/history). Per column, for the
    // same reason the layout is: a busy second screen must not cost the
    // screen you are looking at the card you were reading.
    let evict = {
        let s = st.borrow();
        let column: Vec<u32> = s
            .active()
            .filter(|c| c.output.as_deref() == output.as_deref())
            .map(|c| c.id)
            .collect();
        (column.len() >= usize::from(alerts().stack) + COLLAPSED_TAIL).then(|| column[0])
    };
    if let Some(old_id) = evict {
        start_exit(st, old_id);
    }

    let (app, hovered) = {
        let s = st.borrow();
        (s.app.clone(), s.hovered)
    };
    let corner = alerts().corner;
    let surface = GlassSurface::new(&app, config_for(corner), anim::SLIDE_PX, monitor.as_ref());
    // The surface's pane is already the design system's card: one key, one
    // radius, one hairline, and the tints `set_critical_class` and `reflow`
    // lay over the key.
    surface.pane().add_css_class("notification-popup-content");
    surface.pane().set_size_request(CARD_WIDTH, -1);
    // The surface spans the whole column so the card can be placed inside it
    // without the surface moving, which means the card itself must hug its
    // content. A pane left to fill would put the card over the entire
    // column, and the compositor would frost every bit of it.
    surface.pane().set_valign(gtk4::Align::Start);
    if corner.is_left() {
        surface.pane().set_halign(gtk4::Align::Start);
        surface.pane().set_margin_start(EDGE_MARGIN);
    } else {
        surface.pane().set_halign(gtk4::Align::End);
        surface.pane().set_margin_end(EDGE_MARGIN);
    }
    crate::ui::surface::adopt(surface.pane());
    set_critical_class(surface.pane(), notif);

    let overflow = overflow_for(st, &notif.app_name);
    let age = populate_card(surface.pane(), notif, &store, st, overflow);
    if let Some(content) = surface.pane().first_child() {
        surface.set_content(&content);
    }

    // Hovering any card pauses every auto-dismiss timer, so reading one card
    // does not cost you the ones under it.
    let motion = gtk4::EventControllerMotion::new();
    {
        let st = st.clone();
        motion.connect_enter(move |_, _, _| pause_timers(&st));
    }
    {
        let st = st.clone();
        motion.connect_leave(move |_| resume_timers(&st));
    }
    surface.pane().add_controller(motion);

    {
        let mut s = st.borrow_mut();
        let timer = make_timer(&store, notif, hovered);
        s.cards.push(Card {
            id,
            surface,
            timer,
            app: notif.app_name.clone(),
            wants_keyboard: wants_keyboard(notif),
            stamp: notif.timestamp,
            age,
            exiting: false,
            corner,
            output,
        });
    }

    ensure_age_timer(st);
    sync_keyboard_mode(st);
    // Place before showing, so the card fades in where it belongs rather
    // than sliding into its slot from wherever the last one left off.
    reflow(st);
    // The surface is cloned out of the borrow before it is shown: showing it
    // can move keyboard focus into a reply field, whose focus handler pauses
    // the timers and so borrows the state mutably.
    let newest = st.borrow().cards.last().map(|c| c.surface.clone());
    if let Some(surface) = newest {
        surface.show();
    }
}

fn overflow_for(st: &Rc<RefCell<State>>, app: &str) -> u32 {
    st.borrow().overflow.get(app).copied().unwrap_or(0)
}

/// Keep the header ages honest while cards are on screen.
///
/// One timer for the whole stack rather than one per card, aimed at the
/// minute so every card turns over together, and dropped as soon as the
/// stack empties.
fn ensure_age_timer(st: &Rc<RefCell<State>>) {
    if st.borrow().age_timer.is_some() {
        return;
    }
    let st_c = st.clone();
    let source = glib::timeout_add_local(Duration::from_secs(60), move || {
        let mut s = st_c.borrow_mut();
        if s.cards.is_empty() {
            s.age_timer = None;
            return glib::ControlFlow::Break;
        }
        let now = std::time::SystemTime::now();
        for card in &s.cards {
            if let Some(label) = &card.age {
                label.set_label(&age_label(card.stamp, now).unwrap_or_default());
            }
        }
        glib::ControlFlow::Continue
    });
    st.borrow_mut().age_timer = Some(source);
}

fn dismiss(st: &Rc<RefCell<State>>, id: u32) {
    if start_exit(st, id) {
        reflow(st);
    }
}

/// Begin the exit for a card. Returns false if no such card.
///
/// The card stays in the list, marked `exiting`, until its surface says it is
/// actually gone: it is still on screen until then, and dropping it early
/// would take the surface out mid-fade.
fn start_exit(st: &Rc<RefCell<State>>, id: u32) -> bool {
    let surface = {
        let mut s = st.borrow_mut();
        match s.cards.iter_mut().find(|c| c.id == id && !c.exiting) {
            Some(card) => {
                cancel_timer(&mut card.timer);
                card.exiting = true;
                Some(card.surface.clone())
            }
            None => None,
        }
    };
    let Some(surface) = surface else {
        return false;
    };
    let st_c = st.clone();
    surface.connect_hidden(move || retire(&st_c, id));
    surface.hide();
    true
}

/// Drop a card whose surface has finished leaving.
fn retire(st: &Rc<RefCell<State>>, id: u32) {
    {
        let mut s = st.borrow_mut();
        let Some(i) = s.cards.iter().position(|c| c.id == id && c.exiting) else {
            return;
        };
        s.cards.remove(i);
        // An app with no cards left starts its next burst from zero. The set
        // is collected first because `retain` holds the map borrowed while it
        // runs, and the predicate has to read `cards` on the same struct.
        let live: std::collections::HashSet<String> = s.active().map(|c| c.app.clone()).collect();
        s.overflow.retain(|app, _| live.contains(app));
    }
    sync_keyboard_mode(st);
    reflow(st);
}

/// Each card's rank in its OWN output's column, given the cards in the order
/// `reflow` walks them (newest first, the exiting ones already dropped).
///
/// One column per screen is the whole point: a card is stacked under the
/// cards on its own output, and a card that arrives on another screen must
/// not move it. Cards with no output share the one column the compositor
/// placed them in, which is what the stack did for every card before it knew
/// about outputs.
fn ranks_per_output(outputs: &[Option<&str>]) -> Vec<usize> {
    let mut counts: std::collections::HashMap<Option<&str>, usize> =
        std::collections::HashMap::new();
    outputs
        .iter()
        .map(|output| {
            let rank = counts.entry(*output).or_insert(0);
            let this = *rank;
            *rank += 1;
            this
        })
        .collect()
}

/// Give every card its slot: newest nearest the anchored edge, cards past
/// the stack depth collapsed behind the last full one with their far edges
/// peeking out. One column per output ([`ranks_per_output`]).
///
/// Computed with the anchored edge at y = 0 and mirrored for a card whose
/// column hangs from the bottom, so the two layouts are one piece of
/// arithmetic rather than two.
pub(super) fn reflow(st: &Rc<RefCell<State>>) {
    let full = usize::from(alerts().stack);
    let plan: Vec<(GlassSurface, f64, f64, bool)> = {
        let s = st.borrow();
        let showing: Vec<&Card> = s.cards.iter().rev().filter(|c| !c.exiting).collect();
        let outputs: Vec<Option<&str>> = showing.iter().map(|c| c.output.as_deref()).collect();
        let ranks = ranks_per_output(&outputs);
        // The running geometry is per column, so a card on one screen never
        // reads a `y` another screen's card left behind.
        let mut columns: std::collections::HashMap<Option<&str>, (f64, f64, f64)> =
            std::collections::HashMap::new();
        let mut plan = Vec::new();
        let margin = f64::from(EDGE_MARGIN);
        for (card, rank) in showing.iter().zip(ranks) {
            let column = columns
                .entry(card.output.as_deref())
                .or_insert((margin, margin, margin));
            let (y, full_top, full_bottom) = column;
            let height = card.surface.height();
            let (slot, scale) = if rank < full {
                let slot = *y;
                *full_top = *y;
                *full_bottom = *y + height;
                *y += height + GAP;
                (slot, 1.0)
            } else {
                let k = (rank - full + 1) as f64;
                let scale = 1.0 - PEEK_SCALE_STEP * k;
                // Far edge peeks PEEK px per level beyond the last full
                // card; clamp so a tall collapsed card can't poke out past
                // the anchored edge.
                let slot = (*full_bottom + PEEK * k - height * scale).max(*full_top + 2.0 * k);
                (slot, scale)
            };
            let slot = if card.corner.is_bottom() {
                f64::from(WINDOW_HEIGHT) - slot - height
            } else {
                slot
            };
            plan.push((card.surface.clone(), slot, scale, rank >= full));
        }
        plan
    };

    for (surface, slot, scale, collapsed) in plan {
        // A card that has not been shown yet is placed outright: it should
        // fade in where it belongs, not travel there.
        let ms = if surface.is_shown() {
            anim::duration(anim::MOVE_MS)
        } else {
            0.0
        };
        surface.place_at(slot, ms);
        surface.set_scale(scale);
        // Cards past the fully-visible band sit behind the stack, dimmer. A
        // tint over the key rather than widget opacity: each card owns its
        // surface, and that surface's alpha is the material fade, which this
        // must not fight. The frost stays: a collapsed card is still glass,
        // just further back.
        crate::ui::set_card_tint(surface.pane(), crate::ui::CardTint::Recessed, collapsed);
        surface.clip_input_to_card();
    }
}

/// Match each surface's keyboard mode to whether its own card can be typed
/// into. Per card, because the keyboard belongs to the card with the field
/// and to nothing else on screen.
fn sync_keyboard_mode(st: &Rc<RefCell<State>>) {
    let s = st.borrow();
    for card in &s.cards {
        card.surface
            .set_wants_keyboard(card.wants_keyboard && !card.exiting);
    }
}

/// Close every card currently on screen.
pub(super) fn dismiss_all(st: &Rc<RefCell<State>>) {
    let (ids, store) = {
        let s = st.borrow();
        (
            s.cards
                .iter()
                .filter(|c| !c.exiting)
                .map(|c| c.id)
                .collect::<Vec<_>>(),
            s.store.clone(),
        )
    };
    for id in ids {
        store::store_close(&store, id, CloseReason::Dismissed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bug this replaced: one rank counter for every card meant a card
    /// arriving on the second screen took rank 0 and pushed the first
    /// screen's cards down a slot, on a display the new card was not even on.
    #[test]
    fn each_output_ranks_its_own_column() {
        let a = Some("DP-1");
        let b = Some("eDP-1");
        // Newest first, as reflow walks them: a, b, a, a, b.
        assert_eq!(ranks_per_output(&[a, b, a, a, b]), vec![0, 0, 1, 2, 1]);
        // One screen alone is unchanged: 0, 1, 2 …
        assert_eq!(ranks_per_output(&[a, a, a]), vec![0, 1, 2]);
        // A card that could not be pinned falls back to one shared column,
        // which is what every card did before outputs were known.
        assert_eq!(ranks_per_output(&[None, a, None]), vec![0, 0, 1]);
        assert_eq!(ranks_per_output(&[]), Vec::<usize>::new());
    }
}
