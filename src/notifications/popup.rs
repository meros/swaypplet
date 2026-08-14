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

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk4::prelude::*;
use gtk4_layer_shell::Edge;

use crate::anim;
use crate::icons;
use crate::layer_shell::LayerShellConfig;
use crate::notifications::ImageSource;
use crate::surface::GlassSurface;

use super::store::{self, NotificationStore};
use super::{CloseReason, Notification, Urgency};

// ── Stack geometry ──
const CARD_WIDTH: i32 = 360;
// Offset of the card column from the screen's top/right corner
const EDGE_MARGIN: i32 = 12;
// Every card spans the whole column and is placed inside it, so the surface
// never has to move: layer-shell margins are protocol state and animating a
// slot by moving the surface would be a configure round trip per frame.
const WINDOW_WIDTH: i32 = CARD_WIDTH + 2 * EDGE_MARGIN;
const WINDOW_HEIGHT: i32 = 720;
// Cards shown at full size before older ones collapse behind the stack
const FULL_VISIBLE: usize = 3;
// Vertical gap between fully visible cards
const GAP: f64 = 8.0;
// Collapsed cards peek out below the last full card by this much per level
const PEEK: f64 = 12.0;
const PEEK_SCALE_STEP: f64 = 0.05;
const MAX_POPUPS: usize = 5;
// How many cards one app may hold at once. Without this a chatty app fills
// the stack and evicts everything you had not read yet; past the cap its
// oldest card gives way to its newest and the overflow is counted on the
// survivor instead.
//
// Set to the fully-visible band rather than lower: one app may fill the
// cards shown at full size and no more, which still leaves the collapsed
// tail for everyone else. Tighter than this and three ordinary notifications
// in a row from one app drop one immediately, which reads as a bug.
const MAX_PER_APP: usize = FULL_VISIBLE;

/// The action key a sender uses to ask for a reply field (KDE's convention,
/// which is what the `inline-reply` capability promises).
const INLINE_REPLY_KEY: &str = "inline-reply";

// Drag-to-dismiss: how far before the gesture takes the event sequence off
// the click handler, and how far before releasing means "gone".
const DRAG_CLAIM_PX: f64 = 8.0;
const DRAG_DISMISS_PX: f64 = 72.0;

const BASE_TIMEOUT_MS: u64 = 5000;
const PER_CHAR_MS: u64 = 40;

static POPUP_CONFIG: LayerShellConfig = LayerShellConfig {
    namespace: "swaypplet-notification",
    layer: gtk4_layer_shell::Layer::Overlay,
    exclusive: false,
    default_width: Some(WINDOW_WIDTH),
    default_height: Some(WINDOW_HEIGHT),
    anchors: &[(Edge::Top, true), (Edge::Right, true)],
    margins: &[],
    keyboard_mode: gtk4_layer_shell::KeyboardMode::None,
};

enum Timer {
    None,
    Running {
        source: glib::SourceId,
        deadline: Instant,
    },
    Paused {
        remaining: Duration,
    },
}

struct Card {
    id: u32,
    surface: GlassSurface,
    timer: Timer,
    /// Which app sent it, for the per-app cap.
    app: String,
    /// The card carries a field that has to be typed into, which is the only
    /// reason its surface ever accepts keyboard focus.
    wants_keyboard: bool,
    /// When the notification arrived, for the age in the header.
    stamp: std::time::SystemTime,
    /// The header's age label, so the minute tick can rewrite it without
    /// rebuilding the card under the pointer.
    age: Option<gtk4::Label>,
    /// On its way out: still on screen, but no longer part of the stack.
    exiting: bool,
}

struct State {
    app: gtk4::Application,
    // Oldest → newest.
    cards: Vec<Card>,
    store: Rc<RefCell<NotificationStore>>,
    hovered: bool,
    /// Cards an app has had pushed off the stack while it still holds one,
    /// shown as a count on its newest card. Cleared when the app's last card
    /// goes, so the number always means "since this run of chatter began".
    overflow: std::collections::HashMap<String, u32>,
    /// The once-a-minute age refresh. Boundary-aimed and alive only while
    /// cards are on screen (P7): a timer that outlives its reason is a
    /// wakeup for nothing.
    age_timer: Option<glib::SourceId>,
}

impl State {
    fn active(&self) -> impl Iterator<Item = &Card> {
        self.cards.iter().filter(|c| !c.exiting)
    }
}

/// Manages the popup notification stack at the top-right: newest on top,
/// up to FULL_VISIBLE cards fully expanded, older ones collapsed behind
/// the last full card with peeking edges.
pub struct PopupManager;

impl PopupManager {
    /// Wire the stack to the store's callbacks. Nothing is on screen, and no
    /// surface exists, until a notification arrives.
    pub fn register(app: &gtk4::Application, store: Rc<RefCell<NotificationStore>>) {
        let state = Rc::new(RefCell::new(State {
            app: app.clone(),
            cards: Vec::new(),
            store: store.clone(),
            hovered: false,
            overflow: std::collections::HashMap::new(),
            age_timer: None,
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

    // The app's own oldest card gives way before anyone else's: a burst from
    // one sender should cost that sender its slots, not the stack.
    let crowded = {
        let s = st.borrow();
        let mine: Vec<u32> = s
            .active()
            .filter(|c| c.app == notif.app_name)
            .map(|c| c.id)
            .collect();
        (mine.len() >= MAX_PER_APP).then(|| mine[0])
    };
    if let Some(old_id) = crowded {
        start_exit(st, old_id);
        *st.borrow_mut()
            .overflow
            .entry(notif.app_name.clone())
            .or_insert(0) += 1;
    }

    // Evict the oldest popup when full (popup only — the notification
    // stays open in the store/history)
    let evict = {
        let s = st.borrow();
        (s.active().count() >= MAX_POPUPS).then(|| s.active().map(|c| c.id).next())
    };
    if let Some(Some(old_id)) = evict {
        start_exit(st, old_id);
    }

    let (app, hovered) = {
        let s = st.borrow();
        (s.app.clone(), s.hovered)
    };
    let surface = GlassSurface::new(&app, &POPUP_CONFIG, anim::SLIDE_PX);
    surface.pane().add_css_class("notification-popup-content");
    surface.pane().set_size_request(CARD_WIDTH, -1);
    // The surface spans the whole column so the card can be placed inside it
    // without the surface moving, which means the card itself must hug its
    // content. A pane left to fill would put `.glass-card` over the entire
    // column, and the compositor would frost every bit of it.
    surface.pane().set_valign(gtk4::Align::Start);
    surface.pane().set_halign(gtk4::Align::End);
    surface.pane().set_margin_end(EDGE_MARGIN);
    surface.window().add_css_class("notification-popup");
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
        });
    }

    ensure_age_timer(st);
    sync_keyboard_mode(st);
    // Place before showing, so the card fades in where it belongs rather
    // than sliding into its slot from wherever the last one left off.
    reflow(st);
    if let Some(card) = st.borrow().cards.last() {
        card.surface.show();
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

/// Give every card its slot: newest at the top, cards past FULL_VISIBLE
/// collapsed behind the last full one with their bottom edges peeking out.
fn reflow(st: &Rc<RefCell<State>>) {
    let plan: Vec<(GlassSurface, f64, f64, bool)> = {
        let s = st.borrow();
        let mut y = f64::from(EDGE_MARGIN);
        let mut full_top = f64::from(EDGE_MARGIN);
        let mut full_bottom = f64::from(EDGE_MARGIN);
        let mut plan = Vec::new();
        for (rank, card) in s.cards.iter().rev().filter(|c| !c.exiting).enumerate() {
            let height = card.surface.height();
            let (slot, scale) = if rank < FULL_VISIBLE {
                let slot = y;
                full_top = y;
                full_bottom = y + height;
                y += height + GAP;
                (slot, 1.0)
            } else {
                let k = (rank - FULL_VISIBLE + 1) as f64;
                let scale = 1.0 - PEEK_SCALE_STEP * k;
                // Bottom edge peeks PEEK px per level below the last full
                // card; clamp so a tall collapsed card can't poke out above.
                let slot = (full_bottom + PEEK * k - height * scale).max(full_top + 2.0 * k);
                (slot, scale)
            };
            plan.push((card.surface.clone(), slot, scale, rank >= FULL_VISIBLE));
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
        if collapsed {
            surface.pane().add_css_class("collapsed");
        } else {
            surface.pane().remove_css_class("collapsed");
        }
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

// ── Auto-dismiss timers ──

fn cancel_timer(timer: &mut Timer) {
    if let Timer::Running { source, .. } = std::mem::replace(timer, Timer::None) {
        source.remove();
    }
}

fn timeout_for(notif: &Notification) -> Option<u64> {
    // Critical notifications with no explicit timeout are persistent
    if notif.urgency == Urgency::Critical && notif.expire_timeout <= 0 {
        return None;
    }
    // Timeout 0 means persistent (spec: server decides; we honor 0 as "never")
    if notif.expire_timeout == 0 {
        return None;
    }
    if notif.expire_timeout > 0 {
        Some(notif.expire_timeout as u64)
    } else {
        // -1 means server decides: scale with content length
        let char_count = notif.summary.len() + notif.body.len();
        Some(BASE_TIMEOUT_MS + (char_count as u64) * PER_CHAR_MS)
    }
}

fn make_timer(
    store: &Rc<RefCell<NotificationStore>>,
    notif: &Notification,
    hovered: bool,
) -> Timer {
    let Some(ms) = timeout_for(notif) else {
        return Timer::None;
    };
    let remaining = Duration::from_millis(ms);
    if hovered {
        return Timer::Paused { remaining };
    }
    schedule_close(store, notif.id, remaining)
}

fn schedule_close(store: &Rc<RefCell<NotificationStore>>, id: u32, after: Duration) -> Timer {
    let store = store.clone();
    let source = glib::timeout_add_local_once(after, move || {
        store::store_close(&store, id, CloseReason::Expired);
    });
    Timer::Running {
        source,
        deadline: Instant::now() + after,
    }
}

fn pause_timers(st: &Rc<RefCell<State>>) {
    let mut s = st.borrow_mut();
    s.hovered = true;
    let now = Instant::now();
    for card in &mut s.cards {
        if matches!(card.timer, Timer::Running { .. }) {
            if let Timer::Running { source, deadline } =
                std::mem::replace(&mut card.timer, Timer::None)
            {
                source.remove();
                card.timer = Timer::Paused {
                    remaining: deadline.saturating_duration_since(now),
                };
            }
        }
    }
}

fn resume_timers(st: &Rc<RefCell<State>>) {
    let mut s = st.borrow_mut();
    s.hovered = false;
    let store = s.store.clone();
    for card in &mut s.cards {
        let Timer::Paused { remaining } = card.timer else {
            continue;
        };
        // Give a short grace period so a timer that expired mid-hover
        // doesn't vanish the instant the pointer leaves
        let after = remaining.max(Duration::from_millis(500));
        card.timer = schedule_close(&store, card.id, after);
    }
}

// ── Pictures ──

/// Icon size in the card's leading slot.
const ICON_PX: i32 = 24;
/// Thumbnail size when the notification carries its own picture.
const THUMB_PX: i32 = 40;
/// A picture at least this wide, and at least this much wider than it is
/// tall, is a screenshot or a banner rather than an avatar, so it goes under
/// the text at full card width instead of into the leading slot.
const WIDE_MIN_PX: i32 = 192;
const WIDE_MIN_ASPECT: f64 = 1.6;

/// Build a paintable widget for an image source, or `None` when it cannot be
/// resolved — a missing file or an icon name no theme has. Callers fall back
/// rather than showing a broken-image placeholder, because a card with no
/// picture reads better than a card with a question mark in it.
fn image_widget(src: &ImageSource, px: i32) -> Option<gtk4::Widget> {
    use gtk4::prelude::*;
    let image = match src {
        ImageSource::Named(name) => {
            let display = gtk4::gdk::Display::default()?;
            let theme = gtk4::IconTheme::for_display(&display);
            if !theme.has_icon(name) {
                return None;
            }
            gtk4::Image::from_icon_name(name)
        }
        ImageSource::Path(path) => {
            if !path.is_file() {
                return None;
            }
            gtk4::Image::from_file(path)
        }
        ImageSource::Data(d) => {
            let bytes = gtk4::glib::Bytes::from(&d.data);
            let pixbuf = gtk4::gdk_pixbuf::Pixbuf::from_bytes(
                &bytes,
                gtk4::gdk_pixbuf::Colorspace::Rgb,
                d.has_alpha,
                d.bits_per_sample,
                d.width,
                d.height,
                d.rowstride,
            );
            let texture = gtk4::gdk::Texture::for_pixbuf(&pixbuf);
            gtk4::Image::from_paintable(Some(&texture))
        }
    };
    image.set_pixel_size(px);
    Some(image.upcast())
}

/// Whether a picture should be laid out as a wide banner. Only the geometry
/// decides, and for files it is read from the header rather than by decoding
/// the whole image.
fn is_wide_picture(src: &ImageSource) -> bool {
    let (w, h) = match src {
        ImageSource::Data(d) => (d.width, d.height),
        ImageSource::Path(path) => match gtk4::gdk_pixbuf::Pixbuf::file_info(path) {
            Some((_, w, h)) => (w, h),
            None => return false,
        },
        // A themed name is an icon by definition.
        ImageSource::Named(_) => return false,
    };
    if h <= 0 {
        return false;
    }
    w >= WIDE_MIN_PX && (w as f64 / h as f64) >= WIDE_MIN_ASPECT
}

// ── Age ──

/// How old a notification reads on the card. Short forms, because this sits
/// in a header beside the app name and competes with it for width.
///
/// Anything under a minute is "now": a card that just arrived does not need
/// its age, and a per-second countdown would be a loop the bar's cadence
/// budget (P7) does not pay for.
fn age_label(ts: std::time::SystemTime, now: std::time::SystemTime) -> Option<String> {
    let secs = now.duration_since(ts).ok()?.as_secs();
    Some(match secs {
        0..=59 => return None,
        60..=3599 => format!("{}m", secs / 60),
        3600..=86_399 => format!("{}h", secs / 3600),
        _ => format!("{}d", secs / 86_400),
    })
}

/// A body long enough that the 3-line cap will hide part of it, so the card
/// earns an expander. Counted on the text rather than measured after layout:
/// the answer is needed while building the card, before it has an allocation.
fn body_is_truncated(body: &str) -> bool {
    body.lines().count() > 3 || body.chars().count() > 140
}

// ── Card content ──

fn set_critical_class(card: &gtk4::Box, notif: &Notification) {
    if notif.urgency == Urgency::Critical {
        card.add_css_class("critical");
    } else {
        card.remove_css_class("critical");
    }
}

/// Categories whose notifications are status reports rather than messages:
/// nothing to read, nothing to act on, and a full card around three words
/// reads as an empty box. Prefixes, because the spec's categories are
/// dotted and senders extend them (`device.added`, `device.removed`).
const COMPACT_CATEGORIES: &[&str] = &["device", "x-swaypplet.osd"];

/// Whether a notification wants the one-line template.
///
/// Two ways in. A transient toast with nothing to read or press is one by
/// construction — volume, brightness and this machine's own "nothing to jump
/// to" all land here. A category can also say so outright, which catches the
/// senders that describe what they are without setting `transient`.
/// Whether a notification puts a text field on its card.
fn wants_keyboard(notif: &Notification) -> bool {
    notif.actions.iter().any(|(key, _)| key == INLINE_REPLY_KEY)
}

fn is_compact(notif: &Notification) -> bool {
    if !notif.actions.is_empty() || !notif.body.is_empty() {
        return false;
    }
    if notif.transient {
        return true;
    }
    notif.category.as_deref().is_some_and(|c| {
        COMPACT_CATEGORIES
            .iter()
            .any(|p| c == *p || c.starts_with(&format!("{p}.")))
    })
}

/// (Re)build a card's content. The card box itself persists across
/// `replaces_id` updates (keeping its z-order and transform); everything
/// inside — including gesture handlers — is rebuilt with fresh store refs.
///
/// Returns the age label, if the card has one, so the stack can retitle it on
/// the minute tick without rebuilding the card underneath the pointer.
fn populate_card(
    card: &gtk4::Box,
    notif: &Notification,
    store: &Rc<RefCell<NotificationStore>>,
    st: &Rc<RefCell<State>>,
    overflow: u32,
) -> Option<gtk4::Label> {
    while let Some(child) = card.first_child() {
        card.remove(&child);
    }

    let compact = is_compact(notif);
    if compact {
        card.add_css_class("compact");
    } else {
        card.remove_css_class("compact");
    }

    let hbox = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(10)
        .build();

    // Leading rail. Identity as position and shape, with hue only
    // reinforcing it (P3), so the card still says whose it is in grayscale.
    let rail = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .build();
    rail.add_css_class("notification-rail");
    if let Some(task) = notif.task {
        rail.add_css_class(&format!("t{task}"));
    }
    if notif.urgency == Urgency::Critical {
        rail.add_css_class("critical");
    }
    hbox.append(&rail);

    // A picture wide enough to be a screenshot or a banner goes under the
    // text at full width; anything else is a thumbnail in the leading slot,
    // which is what a chat avatar or an app icon wants to be.
    let wide = notif.image.as_ref().filter(|i| is_wide_picture(i));
    let leading = if wide.is_some() {
        notif.icon.as_ref()
    } else {
        notif.image.as_ref().or(notif.icon.as_ref())
    };
    let leading_px = if wide.is_none() && notif.image.is_some() {
        THUMB_PX
    } else {
        ICON_PX
    };
    if let Some(widget) = leading.and_then(|src| image_widget(src, leading_px)) {
        widget.add_css_class("notification-icon");
        widget.set_valign(gtk4::Align::Start);
        if notif.image.is_some() && wide.is_none() {
            widget.add_css_class("thumb");
        }
        hbox.append(&widget);
    }

    // Text content
    let vbox = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .valign(gtk4::Align::Center)
        .build();

    // max_width_chars(1) collapses each label's natural width so the card's
    // CARD_WIDTH size request is what drives allocation — a larger cap
    // becomes the natural width and can push the card past the window (see
    // reflow). Fill + xalign(0) makes the label span that allocation and
    // ellipsize/wrap there instead of shrinking to the collapsed natural.
    // Header row: task attribution (vision O2 — hue dot + "T<N>" says
    // whose background session this is) ahead of the app name, with the
    // card's age closing it out on the right.
    let header = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(4)
        .build();
    if let Some(task) = notif.task {
        let dot = gtk4::Label::new(Some("●"));
        dot.add_css_class("notification-task-dot");
        dot.add_css_class(&format!("t{task}"));
        header.append(&dot);
        let num = gtk4::Label::new(Some(&format!("T{task}")));
        num.add_css_class("notification-task-num");
        header.append(&num);
    }
    if !notif.app_name.is_empty() {
        let app_label = gtk4::Label::builder()
            .label(notif.app_name.to_uppercase())
            .halign(gtk4::Align::Fill)
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .max_width_chars(1)
            .build();
        app_label.add_css_class("notification-app-name");
        header.append(&app_label);
    }

    // How many of this app's cards the stack has had to drop to make room
    // for this one. Says "there is more of this" without spending a slot.
    if overflow > 0 {
        let badge = gtk4::Label::new(Some(&format!("+{overflow}")));
        badge.add_css_class("notification-overflow");
        badge.set_tooltip_text(Some(&format!(
            "{overflow} more from {} — open the centre to read them",
            notif.app_name
        )));
        header.append(&badge);
    }

    // Age. Built for every card that has a header, empty until the card is a
    // minute old, so the minute tick has something to write into.
    let age = gtk4::Label::builder()
        .label(age_label(notif.timestamp, std::time::SystemTime::now()).unwrap_or_default())
        .halign(gtk4::Align::End)
        .build();
    age.add_css_class("notification-age");
    let age_label_handle = if header.first_child().is_some() {
        header.append(&age);
        vbox.append(&header);
        Some(age)
    } else {
        None
    };

    let summary = gtk4::Label::builder()
        .label(&notif.summary)
        .halign(gtk4::Align::Fill)
        .xalign(0.0)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .max_width_chars(1)
        .build();
    summary.add_css_class("notification-summary");
    vbox.append(&summary);

    if !notif.body.is_empty() {
        let markup = super::markup::sanitize(&notif.body);
        let body = gtk4::Label::builder()
            .label(&markup)
            .use_markup(true)
            .halign(gtk4::Align::Fill)
            .xalign(0.0)
            .wrap(true)
            .wrap_mode(gtk4::pango::WrapMode::WordChar)
            .max_width_chars(1)
            .lines(3)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .build();
        body.add_css_class("notification-body");
        vbox.append(&body);

        // The rest of a long body is one click away rather than only in the
        // centre. A button, not a hover reveal (P8).
        if body_is_truncated(&notif.body) {
            let more = gtk4::Button::builder().label("more").build();
            more.add_css_class("flat");
            more.add_css_class("notification-more-btn");
            more.set_halign(gtk4::Align::Start);
            let body_c = body.clone();
            let st_c = st.clone();
            more.connect_clicked(move |btn| {
                let expanded = body_c.lines() < 0;
                body_c.set_lines(if expanded { 3 } else { -1 });
                btn.set_label(if expanded { "more" } else { "less" });
                // The card's height changed, so the stack has to re-measure
                // and re-stack around it.
                reflow(&st_c);
            });
            vbox.append(&more);
        }
    }

    if let Some(widget) = wide.and_then(|src| image_widget(src, CARD_WIDTH)) {
        widget.add_css_class("notification-picture");
        widget.set_halign(gtk4::Align::Fill);
        vbox.append(&widget);
    }

    // Progress bar
    if let Some(progress) = notif.progress {
        let bar = gtk4::ProgressBar::builder()
            .fraction(progress as f64 / 100.0)
            .hexpand(true)
            .build();
        bar.add_css_class("notification-progress");
        vbox.append(&bar);
    }

    // Inline reply. The sender offers an `inline-reply` action whose label is
    // the placeholder; the text comes back on `NotificationReplied` rather
    // than as an action, because it is content, not a button press.
    if let Some((_, placeholder)) = notif
        .actions
        .iter()
        .find(|(key, _)| key == INLINE_REPLY_KEY)
    {
        let entry = gtk4::Entry::builder()
            .placeholder_text(placeholder)
            .hexpand(true)
            .build();
        entry.add_css_class("notification-reply");
        let id = notif.id;
        let store_c = store.clone();
        let resident = notif.resident;
        entry.connect_activate(move |e| {
            let text = e.text();
            if text.is_empty() {
                return;
            }
            store::store_reply(&store_c, id, &text);
            e.set_text("");
            if !resident {
                store::store_close(&store_c, id, CloseReason::Dismissed);
            }
        });
        // The surface is already on-demand by the time a card with this field
        // is on screen (see sync_keyboard_mode); the click only has to move
        // GTK's own focus onto the entry.
        let click = gtk4::GestureClick::new();
        let entry_c = entry.clone();
        click.connect_pressed(move |_, _, _, _| {
            entry_c.grab_focus();
        });
        entry.add_controller(click);

        // Escape hands the keyboard straight back, so the surface can never
        // be left holding focus with no obvious way out.
        let keys = gtk4::EventControllerKey::new();
        let entry_c = entry.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gtk4::gdk::Key::Escape {
                // Give the keyboard back by dropping the field's focus. The
                // surface stays on-demand until the card itself goes, which
                // costs nothing: on-demand never takes focus unasked.
                entry_c.set_text("");
                if let Some(root) = entry_c.root() {
                    root.set_focus(None::<&gtk4::Widget>);
                }
                return gtk4::glib::Propagation::Stop;
            }
            gtk4::glib::Propagation::Proceed
        });
        entry.add_controller(keys);

        // Typing must not race the auto-dismiss timer out from under the
        // sentence being written.
        let st_c = st.clone();
        let focus = gtk4::EventControllerFocus::new();
        focus.connect_enter(move |_| pause_timers(&st_c));
        let st_c = st.clone();
        focus.connect_leave(move |_| resume_timers(&st_c));
        entry.add_controller(focus);
        vbox.append(&entry);
    }

    // Action buttons
    if !notif.actions.is_empty() {
        let actions_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(4)
            .build();
        actions_box.add_css_class("notification-actions");

        for (key, label) in &notif.actions {
            if key == "default" {
                continue; // default action is handled by clicking the popup body
            }
            if key == INLINE_REPLY_KEY {
                continue; // rendered as the text field above
            }
            let btn = gtk4::Button::builder().build();
            // With `action-icons` the key names an icon rather than being an
            // opaque token, and the label is the tooltip instead.
            if notif.action_icons
                && let Some(display) = gtk4::gdk::Display::default()
                && gtk4::IconTheme::for_display(&display).has_icon(key)
            {
                btn.set_child(Some(&gtk4::Image::from_icon_name(key)));
                btn.set_tooltip_text(Some(label));
            } else {
                btn.set_label(label);
            }
            btn.add_css_class("flat");
            btn.add_css_class("notification-action-btn");

            let id = notif.id;
            let store_c = store.clone();
            let key_c = key.clone();
            // `resident` is the spec's way of saying an action does not
            // dismiss. Without honouring it every button is terminal, so a
            // "snooze" or a "mark read" could never be offered by a sender.
            let resident = notif.resident;
            btn.connect_clicked(move |_| {
                log::info!("Action invoked: notification {id}, action {key_c}");
                store::store_action_invoked(&store_c, id, &key_c);
                if !resident {
                    store::store_close(&store_c, id, CloseReason::Dismissed);
                }
            });
            actions_box.append(&btn);
        }

        vbox.append(&actions_box);
    }

    hbox.append(&vbox);

    // Trailing controls, stacked so the card keeps one gutter rather than
    // sprouting buttons along its edge.
    let controls = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(2)
        .valign(gtk4::Align::Start)
        .build();

    let close_btn = gtk4::Button::builder().label(icons::CLOSE).build();
    close_btn.add_css_class("flat");
    close_btn.add_css_class("notification-close-btn");
    let id = notif.id;
    let store_c = store.clone();
    close_btn.connect_clicked(move |_| {
        store::store_close(&store_c, id, CloseReason::Dismissed);
    });
    controls.append(&close_btn);

    // Snooze and mute live behind one visible button rather than only behind
    // a right-click, so they exist for someone who never dwells (P8).
    if !compact {
        let more_btn = gtk4::Button::builder().label("⋯").build();
        more_btn.add_css_class("flat");
        more_btn.add_css_class("notification-menu-btn");
        let menu = card_menu(notif, store, st);
        menu.set_parent(&more_btn);
        // GTK4 hands a popover's parent no ownership, so one that outlives
        // its button is a leak the toolkit complains about by name
        // ("Finalizing GtkButton, but it still has children left"). Cards are
        // rebuilt on every replaces_id update, so this is per notification.
        let menu_c = menu.clone();
        more_btn.connect_destroy(move |_| menu_c.unparent());
        more_btn.connect_clicked(move |_| menu.popup());
        controls.append(&more_btn);
    }

    hbox.append(&controls);

    // Drag the card aside to dismiss it. Once the pointer has clearly
    // committed, the drag claims the event sequence so the click gesture
    // below never fires — otherwise flicking a card away would also focus
    // the app it came from.
    let drag = gtk4::GestureDrag::new();
    {
        let st_c = st.clone();
        let id = notif.id;
        drag.connect_drag_update(move |g, dx, _| {
            if dx.abs() > DRAG_CLAIM_PX {
                g.set_state(gtk4::EventSequenceState::Claimed);
            }
            // Only outward: dragging a right-anchored card leftward would
            // pull it across the screen rather than off it.
            let dx = dx.max(0.0);
            // Fade with distance, so the card reads as leaving rather than
            // sliding. The surface carries both, so the frost goes with it.
            let alpha = (1.0 - dx / (DRAG_DISMISS_PX * 1.6)).clamp(0.0, 1.0);
            if let Some(card) = st_c
                .borrow()
                .cards
                .iter()
                .find(|c| c.id == id && !c.exiting)
            {
                card.surface.drag_to(dx, alpha);
            }
        });
    }
    {
        let st_c = st.clone();
        let store_c = store.clone();
        let id = notif.id;
        drag.connect_drag_end(move |_, dx, _| {
            if dx >= DRAG_DISMISS_PX {
                store::store_close(&store_c, id, CloseReason::Dismissed);
            } else if let Some(card) = st_c
                .borrow()
                .cards
                .iter()
                .find(|c| c.id == id && !c.exiting)
            {
                card.surface.settle_back(anim::duration(anim::MOVE_MS));
            }
        });
    }
    hbox.add_controller(drag);

    // Click on body = focus the app's window, then dismiss. Middle-click
    // dismisses without firing the default action, which is the difference
    // between "I dealt with this" and "I read this".
    let gesture = gtk4::GestureClick::new();
    gesture.set_button(0);
    let id = notif.id;
    let app_name = notif.app_name.clone();
    let store_c = store.clone();
    gesture.connect_released(move |g, _, _, _| match g.current_button() {
        gtk4::gdk::BUTTON_MIDDLE => store::store_close(&store_c, id, CloseReason::Dismissed),
        gtk4::gdk::BUTTON_PRIMARY => {
            focus_app_window(&app_name);
            store::store_close(&store_c, id, CloseReason::Dismissed);
        }
        _ => {}
    });
    hbox.add_controller(gesture);

    card.append(&hbox);
    age_label_handle
}

/// The per-card menu: postpone it, silence the app, or go read the rest in
/// the centre.
fn card_menu(
    notif: &Notification,
    store: &Rc<RefCell<NotificationStore>>,
    st: &Rc<RefCell<State>>,
) -> gtk4::Popover {
    let list = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(2)
        .build();
    let popover = gtk4::Popover::builder().child(&list).build();
    popover.add_css_class("notification-menu");
    popover.set_has_arrow(false);

    let item = |label: &str| {
        let b = gtk4::Button::builder().label(label).build();
        b.add_css_class("flat");
        b.set_halign(gtk4::Align::Fill);
        b
    };

    for (label, mins) in [("Snooze 5 min", 5u64), ("Snooze 30 min", 30)] {
        let b = item(label);
        let notif_c = notif.clone();
        let store_c = store.clone();
        let pop = popover.clone();
        b.connect_clicked(move |_| {
            pop.popdown();
            snooze(&store_c, &notif_c, Duration::from_secs(mins * 60));
        });
        list.append(&b);
    }

    if !notif.app_name.is_empty() {
        // A muted app can still be on screen: Critical overrides the mute, so
        // the one card you do see from it is the place to lift the silence.
        let muted = store.borrow().is_muted(&notif.app_name);
        let b = item(&if muted {
            format!("Unmute {}", notif.app_name)
        } else {
            format!("Mute {} for 1 h", notif.app_name)
        });
        let app = notif.app_name.clone();
        let id = notif.id;
        let store_c = store.clone();
        let pop = popover.clone();
        b.connect_clicked(move |_| {
            pop.popdown();
            if muted {
                store_c.borrow_mut().unmute_app(&app);
            } else {
                store_c
                    .borrow_mut()
                    .mute_app(&app, Duration::from_secs(3600));
                store::store_close(&store_c, id, CloseReason::Dismissed);
            }
        });
        list.append(&b);
    }

    let b = item("Dismiss all");
    let st_c = st.clone();
    let pop = popover.clone();
    b.connect_clicked(move |_| {
        pop.popdown();
        dismiss_all(&st_c);
    });
    list.append(&b);

    popover
}

/// Take a notification off screen and bring it back later, unchanged.
///
/// The re-presented copy carries no `replaces_id`: the original was closed,
/// so replacing it would target an id the store no longer holds open.
fn snooze(store: &Rc<RefCell<NotificationStore>>, notif: &Notification, after: Duration) {
    store::store_close(store, notif.id, CloseReason::Dismissed);
    let store_c = store.clone();
    let mut again = notif.clone();
    again.id = 0;
    again.replaces_id = 0;
    // Transient snoozed notifications would come back and immediately refuse
    // to be stored; the point of snoozing is that it comes back at all.
    again.transient = false;
    glib::timeout_add_local_once(after, move || {
        store::store_add(&store_c, again.clone());
    });
}

/// Close every card currently on screen.
fn dismiss_all(st: &Rc<RefCell<State>>) {
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

/// Try to focus a Sway window matching the notification's app name.
/// Uses `swaymsg -t get_tree` to find the window, then `[con_id=N] focus`.
fn focus_app_window(app_name: &str) {
    if app_name.is_empty() {
        return;
    }

    let output = match std::process::Command::new("swaymsg")
        .args(["-t", "get_tree", "--raw"])
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            log::warn!("swaymsg get_tree failed: {}", e);
            return;
        }
    };

    let text = String::from_utf8_lossy(&output.stdout);
    let app_lower = app_name.to_lowercase();

    // Parse the JSON tree to find a matching window con_id and its workspace.
    // We look for "app_id" or "class" matching the app_name (case-insensitive).
    if let Some((con_id, workspace)) = find_con_id_in_tree(&text, &app_lower) {
        // Switch to the workspace first, then focus the container.
        // Just `[con_id=N] focus` alone only highlights the workspace without switching.
        // The name is quoted so a renamed workspace can't inject extra sway
        // commands (`;` splits, quotes group). Names containing quote chars
        // themselves fall back to the bare focus rather than trusting sway's
        // escape handling.
        let cmd = match workspace {
            Some(ws) if !ws.contains(['"', '\\']) => {
                format!("workspace \"{}\"; [con_id={}] focus", ws, con_id)
            }
            _ => format!("[con_id={}] focus", con_id),
        };
        log::debug!("Focusing app '{}': swaymsg {}", app_name, cmd);
        let _ = std::process::Command::new("swaymsg")
            .arg(&cmd)
            .spawn()
            .map_err(|e| log::warn!("swaymsg focus failed: {}", e));
    }
}

/// Find a container in the swaymsg JSON tree whose `app_id` or
/// `window_properties.class` matches `app_lower` (case-insensitive substring
/// match in either direction). A `focused` match beats the first match found.
/// Returns `(con_id, Option<workspace_name>)` where the workspace is the
/// nearest enclosing workspace node.
fn find_con_id_in_tree(json: &str, app_lower: &str) -> Option<(u64, Option<String>)> {
    let tree: serde_json::Value = serde_json::from_str(json).ok()?;
    let mut best: Option<(u64, Option<String>, bool)> = None;
    walk_tree(&tree, app_lower, None, &mut best);
    best.map(|(id, ws, _)| (id, ws))
}

/// Recursive walk over `nodes`/`floating_nodes`, carrying the name of the
/// nearest enclosing workspace. `best` holds `(con_id, workspace, focused)`.
fn walk_tree<'a>(
    node: &'a serde_json::Value,
    app_lower: &str,
    workspace: Option<&'a str>,
    best: &mut Option<(u64, Option<String>, bool)>,
) {
    let workspace = if node["type"].as_str() == Some("workspace") {
        node["name"].as_str().or(workspace)
    } else {
        workspace
    };

    let matched = [
        node["app_id"].as_str(),
        node["window_properties"]["class"].as_str(),
    ]
    .into_iter()
    .flatten()
    .any(|value| {
        let value_lower = value.to_lowercase();
        value_lower.contains(app_lower) || app_lower.contains(&value_lower)
    });

    if matched && let Some(id) = node["id"].as_u64() {
        let focused = node["focused"].as_bool().unwrap_or(false);
        if best.is_none() || (focused && !best.as_ref().is_some_and(|b| b.2)) {
            *best = Some((id, workspace.map(str::to_string), focused));
        }
    }

    for key in ["nodes", "floating_nodes"] {
        if let Some(children) = node[key].as_array() {
            for child in children {
                walk_tree(child, app_lower, workspace, best);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notifications::Notification;

    #[test]
    fn age_reads_as_metadata_not_a_countdown() {
        let base = std::time::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let at = |secs| age_label(base, base + Duration::from_secs(secs));
        // Under a minute has nothing worth saying, and saying it every second
        // would be a loop the cadence budget does not pay for.
        assert_eq!(at(0), None);
        assert_eq!(at(59), None);
        assert_eq!(at(60).as_deref(), Some("1m"));
        assert_eq!(at(59 * 60).as_deref(), Some("59m"));
        assert_eq!(at(3600).as_deref(), Some("1h"));
        assert_eq!(at(86_400).as_deref(), Some("1d"));
        // A clock that went backwards is not an age.
        assert_eq!(age_label(base, base - Duration::from_secs(10)), None);
    }

    #[test]
    fn only_a_long_body_earns_an_expander() {
        assert!(!body_is_truncated("one line"));
        assert!(!body_is_truncated("a\nb\nc"));
        assert!(body_is_truncated("a\nb\nc\nd"));
        assert!(body_is_truncated(&"x".repeat(141)));
        assert!(!body_is_truncated(&"x".repeat(140)));
    }

    #[test]
    fn the_compact_template_is_for_toasts_with_nothing_to_act_on() {
        let toast = Notification {
            transient: true,
            summary: "Volume 40%".into(),
            ..Default::default()
        };
        assert!(is_compact(&toast));

        // A body or an action means there is something to read or press, and
        // the one-line template has room for neither.
        assert!(!is_compact(&Notification {
            body: "something".into(),
            ..toast.clone()
        }));
        assert!(!is_compact(&Notification {
            actions: vec![("ok".into(), "OK".into())],
            ..toast.clone()
        }));
        // Anything kept in history is not a toast...
        assert!(!is_compact(&Notification {
            transient: false,
            ..toast.clone()
        }));
        // ...unless it says what it is.
        assert!(is_compact(&Notification {
            transient: false,
            category: Some("device.added".into()),
            ..toast.clone()
        }));
        assert!(is_compact(&Notification {
            transient: false,
            category: Some("device".into()),
            ..toast.clone()
        }));
        // A prefix match must not swallow an unrelated category that merely
        // starts with the same letters.
        assert!(!is_compact(&Notification {
            transient: false,
            category: Some("devicemanager.thing".into()),
            ..toast
        }));
    }
}
