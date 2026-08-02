use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk4::prelude::*;
use gtk4::{graphene, gsk};
use gtk4_layer_shell::Edge;

use crate::anim::{self, ease_out_cubic};
use crate::icons;
use crate::layer_shell::{self, LayerShellConfig};

use super::store::{self, NotificationStore};
use super::{CloseReason, Notification, Urgency};

// ── Stack geometry ──
const CARD_WIDTH: i32 = 360;
// Offset of the card column from the screen's top/right corner
const EDGE_MARGIN: i32 = 12;
// The window is a fixed-size transparent canvas; cards animate inside it.
// Layer-shell margins jump discretely per commit (double-buffered protocol
// state), so the surface itself never moves — only child transforms do.
const WINDOW_WIDTH: i32 = CARD_WIDTH + 2 * EDGE_MARGIN;
const WINDOW_HEIGHT: i32 = 720;
// Cards shown at full size before older ones collapse behind the stack
const FULL_VISIBLE: usize = 3;
// Vertical gap between fully visible cards
const GAP: f64 = 8.0;
// Collapsed cards peek out below the last full card by this much per level
const PEEK: f64 = 12.0;
const PEEK_SCALE_STEP: f64 = 0.05;
const PEEK_OPACITY_STEP: f64 = 0.15;
const MAX_POPUPS: usize = 5;

// ── Animation (durations and easing come from crate::anim) ──
// Entry and exit fade the card while it settles a short SLIDE_PX from/toward
// the right edge (motion on glass, anim.rs): the card itself is the glass
// pane and rides the fast `glass_channel` tint ramp, while its content
// (the inner hbox) fades over the full duration. The overshoot past the
// canvas's right margin is clipped by the overlay clipper.

const BASE_TIMEOUT_MS: u64 = 5000;
const PER_CHAR_MS: u64 = 40;

static POPUP_CONFIG: LayerShellConfig = LayerShellConfig {
    namespace: "swaypplet-notification",
    default_width: Some(WINDOW_WIDTH),
    default_height: Some(WINDOW_HEIGHT),
    anchors: &[(Edge::Top, true), (Edge::Right, true)],
    margins: &[],
    keyboard_mode: gtk4_layer_shell::KeyboardMode::None,
};

/// Animated properties of a card. `y` is the visual top edge, `x_off` a
/// horizontal slide offset (entry/exit), `scale` shrinks collapsed cards.
/// `opacity` is the card (glass pane) alpha — it also carries the collapsed
/// stack dimming — and `content` is the inner hbox alpha on top of it.
#[derive(Clone, Copy, PartialEq)]
struct Pose {
    y: f64,
    x_off: f64,
    scale: f64,
    opacity: f64,
    content: f64,
}

fn lerp_pose(a: Pose, b: Pose, t: f64) -> Pose {
    let l = |a: f64, b: f64| a + (b - a) * t;
    Pose {
        y: l(a.y, b.y),
        x_off: l(a.x_off, b.x_off),
        scale: l(a.scale, b.scale),
        opacity: l(a.opacity, b.opacity),
        content: l(a.content, b.content),
    }
}

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
    widget: gtk4::Box,
    timer: Timer,
    // Measured natural size (width can exceed CARD_WIDTH slightly for
    // wide content; cards are right-aligned so it stays invisible)
    width: f64,
    height: f64,
    cur: Pose,
    from: Pose,
    to: Pose,
    anim_start: i64, // glib::monotonic_time() µs
    anim_ms: f64,
    animating: bool,
    exiting: bool,
    newborn: bool,
}

impl Card {
    fn retarget(&mut self, to: Pose, ms: f64, now: i64) {
        if self.animating && self.to == to {
            return;
        }
        if !self.animating && self.cur == to {
            return;
        }
        self.from = self.cur;
        self.to = to;
        self.anim_start = now;
        self.anim_ms = ms;
        self.animating = true;
    }
}

struct State {
    window: gtk4::Window,
    canvas: gtk4::Fixed,
    // Oldest → newest; matches child paint order, so the newest card
    // draws on top where collapsed cards overlap.
    cards: Vec<Card>,
    store: Rc<RefCell<NotificationStore>>,
    ticking: bool,
    hovered: bool,
}

/// Manages the popup notification stack at the top-right: newest on top,
/// up to FULL_VISIBLE cards fully expanded, older ones collapsed behind
/// the last full card with peeking edges.
pub struct PopupManager;

impl PopupManager {
    /// Create the stack window and wire it to the store's callbacks.
    pub fn register(app: &gtk4::Application, store: Rc<RefCell<NotificationStore>>) {
        let window = layer_shell::create_layer_window(app, &POPUP_CONFIG);
        window.add_css_class("notification-popup");

        // The canvas must not drive the surface size: GtkFixed reports its
        // children's transformed extents as its minimum size, so the entry
        // slide (x_off past the right margin) would grow the surface — and a
        // top+right-anchored surface grows leftward, dragging every visible
        // card with it. Hosting the canvas as a non-measured, clipped overlay
        // child keeps the surface at its default size; the slide overshoot is
        // clipped at the screen edge, where it was never visible anyway.
        let canvas = gtk4::Fixed::new();
        let clipper = gtk4::Overlay::new();
        clipper.add_overlay(&canvas);
        clipper.set_measure_overlay(&canvas, false);
        clipper.set_clip_overlay(&canvas, true);
        window.set_child(Some(&clipper));

        let state = Rc::new(RefCell::new(State {
            window: window.clone(),
            canvas: canvas.clone(),
            cards: Vec::new(),
            store: store.clone(),
            ticking: false,
            hovered: false,
        }));

        // Hovering the stack pauses auto-dismiss timers
        let motion = gtk4::EventControllerMotion::new();
        {
            let st = state.clone();
            motion.connect_enter(move |_, _, _| pause_timers(&st));
        }
        {
            let st = state.clone();
            motion.connect_leave(move |_| resume_timers(&st));
        }
        canvas.add_controller(motion);

        // The input region only exists once the surface is mapped.
        // try_borrow: map can fire synchronously from present() while the
        // caller still holds the state borrow; the region is recomputed by
        // the reflow that always follows, so skipping here is safe.
        {
            let st = state.clone();
            window.connect_map(move |_| {
                if let Ok(s) = st.try_borrow() {
                    update_input_region(&s);
                }
            });
        }

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

    // Replacing an existing popup: rebuild its content in place.
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
                card.widget.clone()
            })
    };
    if let Some(widget) = existing {
        populate_card(&widget, notif, &store);
        set_critical_class(&widget, notif);
        reflow(st);
        return;
    }

    // Evict the oldest popup when full (popup only — the notification
    // stays open in the store/history)
    let evict = {
        let s = st.borrow();
        if s.cards.iter().filter(|c| !c.exiting).count() >= MAX_POPUPS {
            s.cards.iter().find(|c| !c.exiting).map(|c| c.id)
        } else {
            None
        }
    };
    if let Some(old_id) = evict {
        start_exit(st, old_id);
    }

    let card = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .build();
    card.add_css_class("glass-card");
    card.add_css_class("notification-popup-content");
    card.set_size_request(CARD_WIDTH, -1);
    set_critical_class(&card, notif);
    populate_card(&card, notif, &store);

    // present() maps the window and the map signal fires synchronously —
    // its handler borrows the state, so present outside any borrow.
    let window = {
        let s = st.borrow();
        (!s.window.is_visible()).then(|| s.window.clone())
    };
    if let Some(window) = window {
        window.present();
    }

    // Parenting the card can synthesize a pointer enter whose handler
    // borrows the state — put it on the canvas before taking the borrow.
    let (canvas, hovered) = {
        let s = st.borrow();
        (s.canvas.clone(), s.hovered)
    };
    canvas.put(&card, 0.0, 0.0);

    {
        let mut s = st.borrow_mut();
        let timer = make_timer(&store, notif, hovered);
        s.cards.push(Card {
            id,
            widget: card,
            timer,
            width: CARD_WIDTH as f64,
            height: 0.0,
            // Placeholder pose — reflow() measures the card and re-poses the
            // newborn before the first frame.
            cur: Pose {
                y: 0.0,
                x_off: anim::SLIDE_PX,
                scale: 1.0,
                opacity: 0.0,
                content: 0.0,
            },
            from: Pose {
                y: 0.0,
                x_off: anim::SLIDE_PX,
                scale: 1.0,
                opacity: 0.0,
                content: 0.0,
            },
            to: Pose {
                y: 0.0,
                x_off: 0.0,
                scale: 1.0,
                opacity: 1.0,
                content: 1.0,
            },
            anim_start: 0,
            anim_ms: anim::ENTER_MS,
            animating: false,
            exiting: false,
            newborn: true,
        });
    }

    reflow(st);
}

fn dismiss(st: &Rc<RefCell<State>>, id: u32) {
    if start_exit(st, id) {
        reflow(st);
    }
}

/// Begin the exit animation for a card. Returns false if no such card.
fn start_exit(st: &Rc<RefCell<State>>, id: u32) -> bool {
    let started = {
        let mut s = st.borrow_mut();
        let now = glib::monotonic_time();
        match s.cards.iter_mut().find(|c| c.id == id && !c.exiting) {
            Some(card) => {
                cancel_timer(&mut card.timer);
                card.exiting = true;
                // Fade out while drifting a short settle toward the edge
                // (motion on glass, anim.rs).
                let to = Pose {
                    x_off: anim::SLIDE_PX,
                    opacity: 0.0,
                    content: 0.0,
                    ..card.cur
                };
                card.retarget(to, anim_ms(anim::EXIT_MS), now);
                true
            }
            None => false,
        }
    };
    if started {
        ensure_tick(st);
    }
    started
}

/// Recompute layout targets for all active cards and retarget their
/// animations. Newest card sits at the top; cards beyond FULL_VISIBLE
/// collapse behind the last full card with peeking bottom edges.
fn reflow(st: &Rc<RefCell<State>>) {
    {
        let mut s = st.borrow_mut();
        let now = glib::monotonic_time();
        let canvas = s.canvas.clone();

        // Cards are fixed-width: GtkFixed clamps a child's allocation to the
        // canvas width, so anchoring on a wider natural measure would place
        // the card for a box that never gets allocated (clipped at the left
        // screen edge, right edge short of the margin).
        for card in s.cards.iter_mut().filter(|c| !c.exiting) {
            card.width = CARD_WIDTH as f64;
            let (_, nat_h, _, _) = card.widget.measure(gtk4::Orientation::Vertical, CARD_WIDTH);
            card.height = nat_h as f64;
        }

        // Walk newest → oldest assigning stack positions
        let mut y = EDGE_MARGIN as f64;
        let mut full_top = EDGE_MARGIN as f64;
        let mut full_bottom = EDGE_MARGIN as f64;
        let active: Vec<usize> = (0..s.cards.len())
            .rev()
            .filter(|&i| !s.cards[i].exiting)
            .collect();
        for (rank, &i) in active.iter().enumerate() {
            let card = &mut s.cards[i];
            let to = if rank < FULL_VISIBLE {
                let pose = Pose {
                    y,
                    x_off: 0.0,
                    scale: 1.0,
                    opacity: 1.0,
                    content: 1.0,
                };
                full_top = y;
                full_bottom = y + card.height;
                y += card.height + GAP;
                pose
            } else {
                let k = (rank - FULL_VISIBLE + 1) as f64;
                let scale = 1.0 - PEEK_SCALE_STEP * k;
                let visual_h = card.height * scale;
                // Bottom edge peeks PEEK px per level below the last full
                // card; clamp so a tall collapsed card can't poke out above
                let ty = (full_bottom + PEEK * k - visual_h).max(full_top + 2.0 * k);
                Pose {
                    y: ty,
                    x_off: 0.0,
                    scale,
                    opacity: 1.0 - PEEK_OPACITY_STEP * k,
                    content: 1.0,
                }
            };

            if card.newborn {
                card.newborn = false;
                // Enter at the final slot, fading in while settling a short
                // slide from the right (motion on glass, anim.rs)
                card.cur = Pose {
                    y: to.y,
                    x_off: anim::SLIDE_PX,
                    scale: to.scale,
                    opacity: 0.0,
                    content: 0.0,
                };
                card.from = card.cur;
                card.to = to;
                card.anim_start = now;
                card.anim_ms = anim_ms(anim::ENTER_MS);
                card.animating = true;
                apply_pose(&canvas, card);
            } else {
                card.retarget(to, anim_ms(anim::MOVE_MS), now);
            }
        }

        update_input_region(&s);
    }
    ensure_tick(st);
}

/// Reduced motion: collapse any animation to a single frame so state flow
/// (exit removal, unmap) still runs through the tick path.
fn anim_ms(ms: f64) -> f64 {
    if anim::animations_enabled() { ms } else { 1.0 }
}

/// Apply a card's current pose as a Fixed child transform. The card widget
/// is the glass pane; its first child (the hbox) is the content fading on
/// top of it.
fn apply_pose(canvas: &gtk4::Fixed, card: &Card) {
    let right_x = (WINDOW_WIDTH - EDGE_MARGIN) as f64;
    // Right-align the visual box; collapsed cards shrink toward the
    // column's horizontal center
    let tx = right_x - card.width * (1.0 + card.cur.scale) / 2.0 + card.cur.x_off;
    let transform = gsk::Transform::new()
        .translate(&graphene::Point::new(tx as f32, card.cur.y as f32))
        .scale(card.cur.scale as f32, card.cur.scale as f32);
    canvas.set_child_transform(&card.widget, Some(&transform));
    card.widget.set_opacity(card.cur.opacity);
    if let Some(content) = card.widget.first_child() {
        content.set_opacity(card.cur.content);
    }
}

fn ensure_tick(st: &Rc<RefCell<State>>) {
    let canvas = {
        let mut s = st.borrow_mut();
        if s.ticking || !s.cards.iter().any(|c| c.animating) {
            return;
        }
        s.ticking = true;
        s.canvas.clone()
    };

    let st = st.clone();
    canvas.add_tick_callback(move |_, _| {
        let mut s = st.borrow_mut();
        let now = glib::monotonic_time();
        let canvas = s.canvas.clone();
        let window = s.window.clone();
        let mut any_running = false;
        let mut finished_exits = Vec::new();

        for (i, card) in s.cards.iter_mut().enumerate() {
            if !card.animating {
                continue;
            }
            let t = (((now - card.anim_start) as f64 / 1000.0) / card.anim_ms).clamp(0.0, 1.0);
            card.cur = lerp_pose(card.from, card.to, ease_out_cubic(t));
            // The pane channel overrides the eased lerp: tint arrives (and
            // leaves) inside GLASS_MS so it lands with the compositor frost
            // (motion on glass, anim.rs). Driven off linear t.
            card.cur.opacity =
                anim::glass_channel(card.from.opacity, card.to.opacity, t, card.anim_ms);
            apply_pose(&canvas, card);
            if t >= 1.0 {
                card.animating = false;
                if card.exiting {
                    finished_exits.push(i);
                }
            } else {
                any_running = true;
            }
        }

        let removed: Vec<_> = finished_exits
            .iter()
            .rev()
            .map(|&i| s.cards.remove(i).widget)
            .collect();
        let hide = !any_running && s.cards.is_empty();
        if !any_running {
            s.ticking = false;
        }
        // Unparenting and unmapping synthesize pointer crossing events whose
        // handlers (pause/resume_timers) borrow the state — release it first,
        // or a hover during the last exit aborts on a nested borrow.
        drop(s);
        for widget in &removed {
            canvas.remove(widget);
        }
        if hide {
            window.set_visible(false);
        }

        if any_running {
            glib::ControlFlow::Continue
        } else {
            glib::ControlFlow::Break
        }
    });
}

/// Restrict pointer input to the area the cards occupy (current and
/// target extents), so the rest of the transparent canvas clicks through.
fn update_input_region(s: &State) {
    let Some(surface) = s.window.surface() else {
        return;
    };
    let region = cairo::Region::create();
    let right_x = (WINDOW_WIDTH - EDGE_MARGIN) as f64;
    for card in s.cards.iter().filter(|c| !c.exiting) {
        for pose in [card.cur, card.to] {
            let x0 = right_x - card.width * (1.0 + pose.scale) / 2.0 + pose.x_off;
            let w = card.width * pose.scale;
            let h = card.height * pose.scale;
            let rect = cairo::RectangleInt::new(
                x0.floor() as i32,
                pose.y.floor() as i32,
                w.ceil() as i32,
                h.ceil() as i32,
            );
            let _ = region.union_rectangle(&rect);
        }
    }
    surface.set_input_region(Some(&region));
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

// ── Card content ──

fn set_critical_class(card: &gtk4::Box, notif: &Notification) {
    if notif.urgency == Urgency::Critical {
        card.add_css_class("critical");
    } else {
        card.remove_css_class("critical");
    }
}

/// (Re)build a card's content. The card box itself persists across
/// `replaces_id` updates (keeping its z-order and transform); everything
/// inside — including gesture handlers — is rebuilt with fresh store refs.
fn populate_card(card: &gtk4::Box, notif: &Notification, store: &Rc<RefCell<NotificationStore>>) {
    while let Some(child) = card.first_child() {
        card.remove(&child);
    }

    let hbox = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(10)
        .build();

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
    if !notif.app_name.is_empty() {
        let app_label = gtk4::Label::builder()
            .label(notif.app_name.to_uppercase())
            .halign(gtk4::Align::Fill)
            .xalign(0.0)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .max_width_chars(1)
            .build();
        app_label.add_css_class("notification-app-name");
        vbox.append(&app_label);
    }

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
            let btn = gtk4::Button::builder().label(label).build();
            btn.add_css_class("flat");
            btn.add_css_class("notification-action-btn");

            let id = notif.id;
            let store_c = store.clone();
            let key_c = key.clone();
            btn.connect_clicked(move |_| {
                log::info!("Action invoked: notification {id}, action {key_c}");
                store::store_action_invoked(&store_c, id, &key_c);
                store::store_close(&store_c, id, CloseReason::Dismissed);
            });
            actions_box.append(&btn);
        }

        vbox.append(&actions_box);
    }

    hbox.append(&vbox);

    // Close button
    let close_btn = gtk4::Button::builder()
        .label(icons::CLOSE)
        .valign(gtk4::Align::Start)
        .build();
    close_btn.add_css_class("flat");
    close_btn.add_css_class("notification-close-btn");

    let id = notif.id;
    let store_c = store.clone();
    close_btn.connect_clicked(move |_| {
        store::store_close(&store_c, id, CloseReason::Dismissed);
    });
    hbox.append(&close_btn);

    // Click on body = focus the app's window, then dismiss
    let gesture = gtk4::GestureClick::new();
    let id = notif.id;
    let app_name = notif.app_name.clone();
    let store_c = store.clone();
    gesture.connect_released(move |_, _, _, _| {
        focus_app_window(&app_name);
        store::store_close(&store_c, id, CloseReason::Dismissed);
    });
    hbox.add_controller(gesture);

    card.append(&hbox);
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
