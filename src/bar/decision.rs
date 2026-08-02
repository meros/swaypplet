//! Decision slot — the bar's center occupant (docs/BAR_VISION.md,
//! increment 4).
//!
//! Empty at nominal, and empty means "nobody needs you". One occupant at
//! a time behind a GtkRevealer, priority-muxed: battery critical (red,
//! stand-down on charger connect) over the oldest unacknowledged waiting
//! session (task-hue dot + full 40-char settask description + age chip,
//! `+N` when more wait; stand-down on focus-ack, which TaskStateService
//! already computes) over nothing. Handover between occupants: the
//! outgoing collapses over the 200 ms structural scale, then the incoming
//! reveals — never a hard swap mid-air.
//!
//! Cadence: renders only on TaskStateService change (file/sway
//! transitions plus its 1/min waiting-age tick) and on the battery
//! segment's existing 30 s poll, which feeds this slot through
//! `set_battery` — no poll of its own. Charger connect therefore clears
//! within one battery poll; sysfs has no edge to subscribe to here.
//!
//! The one transient tenant (increment 5): volume/brightness OSD
//! interjections. `interject` overlays the standing occupant on a
//! crossfaded Stack page — never destroying it — with icon + eased
//! center-anchored hairline + value; repeated keypresses retarget the
//! ease so they read as one continuous sweep, and 1.5 s after the last
//! press the slot yields back to whatever the mux holds.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use gtk4::prelude::*;

use crate::anim;
use crate::bar::battery::CRITICAL_PCT;
use crate::bar::board::{chip_label, session_age};
use crate::task_state::{Activity, SessionState, TaskSnapshot, TaskStateService};
use crate::widgets::power::{self, BatteryState};

/// Structural show/hide scale (anim.rs module header).
const SWAP_MS: u64 = 200;
/// settask caps descriptions at 40 chars; the slot shows all of them
/// (vision P5's one sanctioned exception) but never more.
const DESC_MAX_CHARS: i32 = 40;

/// OSD interjection: decay after the last keypress (matches the center
/// card's `OSD_TIMEOUT_MS`) and the per-press hairline ease.
const OSD_DECAY_MS: u64 = 1500;
const OSD_EASE_MS: f64 = 150.0;
const HAIRLINE_WIDTH: i32 = 64;

const DOT_CLASSES: [&str; 4] = ["t1", "t2", "t3", "t4"];

// ── View model ──────────────────────────────────────────────────────────

/// Battery reduced to what the mux needs, so the priority logic tests
/// without constructing a sysfs-backed `BatteryState`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct BatteryView {
    critical: bool,
    /// "13% · 1h 5m" — % + time-to-empty per the vision's occupant 1.
    text: String,
}

impl BatteryView {
    fn read(bat: &BatteryState) -> Self {
        Self {
            critical: bat.capacity <= CRITICAL_PCT && !bat.charging,
            text: match power::time_to_empty_text(bat) {
                Some(t) => format!("{}% · {t}", bat.capacity),
                None => format!("{}%", bat.capacity),
            },
        }
    }
}

/// Who holds the slot. Comparable so a no-op refresh never re-animates,
/// and keyed so an in-place update (age tick) is distinguishable from a
/// handover (different occupant).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Occ {
    Battery {
        text: String,
    },
    Waiting {
        pid: i32,
        task: u8,
        desc: String,
        /// Age chip and/or "+N more wait" suffix.
        meta: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OccKey {
    Battery,
    Waiting(i32),
}

impl Occ {
    fn key(&self) -> OccKey {
        match self {
            Occ::Battery { .. } => OccKey::Battery,
            Occ::Waiting { pid, .. } => OccKey::Waiting(*pid),
        }
    }
}

/// The priority mux: battery critical, then the oldest unacknowledged
/// waiting session, then empty. Acked and non-waiting sessions never
/// occupy the slot — the board already shows them.
fn occupant(battery: Option<&BatteryView>, snap: &TaskSnapshot, now: SystemTime) -> Option<Occ> {
    if let Some(b) = battery.filter(|b| b.critical) {
        return Some(Occ::Battery {
            text: b.text.clone(),
        });
    }
    let waiting: Vec<(u8, &SessionState)> = snap
        .tasks
        .iter()
        .enumerate()
        .flat_map(|(i, t)| t.sessions.iter().map(move |s| (i as u8 + 1, s)))
        .filter(|(_, s)| s.activity == Activity::Waiting && !s.acked)
        .collect();
    // Oldest = earliest status mtime; a missing or future mtime (suspend
    // skew) counts as age zero, matching the service's age rule. Pid
    // tie-breaks for a stable pick.
    let (task, oldest) = waiting
        .iter()
        .copied()
        .min_by_key(|(_, s)| (s.status_mtime.map_or(now, |m| m.min(now)), s.pid))?;
    let chip = chip_label(session_age(oldest, now));
    let more = (waiting.len() > 1).then(|| format!("+{}", waiting.len() - 1));
    Some(Occ::Waiting {
        pid: oldest.pid,
        task,
        desc: oldest.desc.clone(),
        meta: match (chip, more) {
            (Some(c), Some(m)) => Some(format!("{c} {m}")),
            (c, m) => c.or(m),
        },
    })
}

// ── Widget ──────────────────────────────────────────────────────────────

struct Inner {
    revealer: gtk4::Revealer,
    /// Occupant page under the interjection page; crossfade between them.
    stack: gtk4::Stack,
    root: gtk4::Box,
    dot: gtk4::Label,
    text: gtk4::Label,
    meta: gtk4::Label,
    tasks: Rc<TaskStateService>,
    battery: RefCell<Option<BatteryView>>,
    /// Occupant currently rendered in the slot (`None` = collapsed).
    shown: RefCell<Option<OccKey>>,
    /// Latest computed occupant; what the slot converges toward.
    desired: RefCell<Option<Occ>>,
    /// A collapse is in flight; its timeout applies `desired` when done.
    swapping: Cell<bool>,
    // ── OSD interjection (transient tenant) ─────────────────────────────
    osd_page: gtk4::Box,
    osd_icon: gtk4::Label,
    osd_text: gtk4::Label,
    hairline: gtk4::DrawingArea,
    /// Fraction the draw func paints; eased toward the latest keypress.
    hair_shown: Rc<Cell<f64>>,
    hair_tick: Rc<RefCell<Option<gtk4::TickCallbackId>>>,
    interjecting: Cell<bool>,
    decay: RefCell<Option<glib::SourceId>>,
}

/// Cheap handle; the TaskStateService observer keeps the widgets updated
/// for the window's life (same leak-tolerant story as board.rs — an
/// unplugged output's leftover observer just paints unmapped widgets).
#[derive(Clone)]
pub struct DecisionSlot {
    inner: Rc<Inner>,
}

impl DecisionSlot {
    pub fn build(tasks: &Rc<TaskStateService>) -> Self {
        let dot = gtk4::Label::builder()
            .css_classes(["bar-decision-dot"])
            .build();
        let text = gtk4::Label::builder()
            .css_classes(["bar-decision-text"])
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .max_width_chars(DESC_MAX_CHARS)
            .build();
        let meta = gtk4::Label::builder()
            .css_classes(["bar-decision-meta"])
            .visible(false)
            .build();
        let root = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(6)
            .css_classes(["bar-decision"])
            .build();
        root.append(&dot);
        root.append(&text);
        root.append(&meta);

        // OSD interjection page: icon + center-anchored hairline + value.
        // Shares the .bar-decision pill so the crossfade only swaps
        // content, not the card.
        let osd_icon = gtk4::Label::builder()
            .css_classes(["bar-decision-osd-icon"])
            .build();
        let osd_text = gtk4::Label::builder()
            .css_classes(["bar-decision-osd-text"])
            .build();
        let hair_shown: Rc<Cell<f64>> = Rc::new(Cell::new(0.0));
        let hairline = gtk4::DrawingArea::builder()
            .content_width(HAIRLINE_WIDTH)
            .content_height(2)
            .valign(gtk4::Align::Center)
            .css_classes(["bar-decision-hairline"])
            .build();
        {
            let shown = hair_shown.clone();
            hairline.set_draw_func(move |area, cr, w, h| {
                let c = area.color();
                let (w, h) = (f64::from(w), f64::from(h));
                // Dim full-width track under the fill.
                cr.set_source_rgba(
                    c.red().into(),
                    c.green().into(),
                    c.blue().into(),
                    f64::from(c.alpha()) * 0.25,
                );
                cr.rectangle(0.0, 0.0, w, h);
                let _ = cr.fill();
                let f = shown.get().clamp(0.0, 1.0);
                if f <= 0.0 {
                    return;
                }
                // Center-anchored: the fill grows outward from the middle.
                cr.set_source_rgba(
                    c.red().into(),
                    c.green().into(),
                    c.blue().into(),
                    c.alpha().into(),
                );
                cr.rectangle((w - w * f) / 2.0, 0.0, w * f, h);
                let _ = cr.fill();
            });
        }
        let osd_page = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(8)
            .css_classes(["bar-decision", "bar-decision-osd"])
            .build();
        osd_page.append(&osd_icon);
        osd_page.append(&hairline);
        osd_page.append(&osd_text);

        // The interjection overlays the occupant without destroying it: a
        // Stack keeps both pages alive and crossfades at the 150 ms
        // sub-threshold scale.
        let stack = gtk4::Stack::builder()
            .transition_type(gtk4::StackTransitionType::Crossfade)
            .transition_duration(150)
            .hhomogeneous(false)
            .build();
        stack.add_child(&root);
        stack.add_child(&osd_page);

        // Horizontal slide so the collapse frees width smoothly — the
        // CenterBox recenters instead of snapping neighbors.
        let revealer = gtk4::Revealer::builder()
            .transition_type(gtk4::RevealerTransitionType::SlideRight)
            .transition_duration(SWAP_MS as u32)
            .reveal_child(false)
            .child(&stack)
            .build();

        let slot = Self {
            inner: Rc::new(Inner {
                revealer,
                stack,
                root,
                dot,
                text,
                meta,
                tasks: tasks.clone(),
                battery: RefCell::new(None),
                shown: RefCell::new(None),
                desired: RefCell::new(None),
                swapping: Cell::new(false),
                osd_page,
                osd_icon,
                osd_text,
                hairline,
                hair_shown,
                hair_tick: Rc::new(RefCell::new(None)),
                interjecting: Cell::new(false),
                decay: RefCell::new(None),
            }),
        };

        slot.refresh();
        {
            let slot = slot.clone();
            tasks.connect_change(move || slot.refresh());
        }
        slot
    }

    pub fn widget(&self) -> &gtk4::Revealer {
        &self.inner.revealer
    }

    /// Battery input, fed from the battery segment's existing 30 s poll
    /// (bar/battery.rs) — the slot adds no poll of its own.
    pub fn set_battery(&self, bat: &BatteryState) {
        *self.inner.battery.borrow_mut() = Some(BatteryView::read(bat));
        self.refresh();
    }

    fn refresh(&self) {
        let snap = self.inner.tasks.snapshot();
        let occ = occupant(
            self.inner.battery.borrow().as_ref(),
            &snap,
            SystemTime::now(),
        );
        self.apply(occ);
    }

    fn apply(&self, occ: Option<Occ>) {
        let inner = &self.inner;
        let same = occ.as_ref().map(Occ::key) == *inner.shown.borrow();
        if same {
            // Same occupant: update in place (age tick, battery %) — no
            // handover motion for a sub-threshold change.
            if let Some(o) = &occ {
                self.render(o);
            }
            *inner.desired.borrow_mut() = occ;
            return;
        }
        *inner.desired.borrow_mut() = occ;
        if inner.interjecting.get() {
            // The transient tenant holds the slot; the handover runs at
            // yield time instead.
            return;
        }
        self.begin_swap();
    }

    /// OSD interjection (occupant 3, the only transient tenant): show
    /// icon + hairline + value over the standing occupant, retargeting
    /// the eased fraction on every keypress, and arm the 1.5 s decay.
    pub fn interject(&self, icon: &str, fraction: f64, text: &str) {
        let inner = &self.inner;
        inner.osd_icon.set_text(icon);
        inner.osd_text.set_text(text);
        self.animate_hairline(fraction);
        if !inner.interjecting.get() {
            inner.interjecting.set(true);
            inner.stack.set_visible_child(&inner.osd_page);
        }
        inner.revealer.set_reveal_child(true);
        if let Some(id) = inner.decay.borrow_mut().take() {
            id.remove();
        }
        let slot = self.clone();
        let id = glib::timeout_add_local_once(Duration::from_millis(OSD_DECAY_MS), move || {
            slot.inner.decay.borrow_mut().take();
            slot.yield_back();
        });
        *inner.decay.borrow_mut() = Some(id);
    }

    /// Decay: hand the slot back to whatever the mux holds. The standing
    /// occupant was never destroyed, so yielding is a crossfade back (or
    /// a collapse when nobody needs the owner).
    fn yield_back(&self) {
        let inner = &self.inner;
        inner.interjecting.set(false);
        inner.stack.set_visible_child(&inner.root);
        let desired_key = inner.desired.borrow().as_ref().map(Occ::key);
        if desired_key == *inner.shown.borrow() {
            if desired_key.is_none() && !inner.swapping.get() {
                inner.revealer.set_reveal_child(false);
            }
        } else {
            // An occupant change arrived mid-interjection; run the
            // deferred handover now.
            self.begin_swap();
        }
    }

    /// Ease the hairline toward `target` over 150 ms so repeated
    /// keypresses read as one continuous sweep; reduced motion jumps.
    fn animate_hairline(&self, target: f64) {
        let inner = &self.inner;
        if let Some(id) = inner.hair_tick.borrow_mut().take() {
            id.remove();
        }
        let from = inner.hair_shown.get();
        if !anim::animations_enabled() || from == target {
            inner.hair_shown.set(target);
            inner.hairline.queue_draw();
            return;
        }
        let start = glib::monotonic_time();
        let shown = inner.hair_shown.clone();
        let tick = inner.hair_tick.clone();
        let id = inner.hairline.add_tick_callback(move |area, _| {
            let t =
                (((glib::monotonic_time() - start) as f64 / 1000.0) / OSD_EASE_MS).clamp(0.0, 1.0);
            shown.set(from + (target - from) * anim::ease_out_cubic(t));
            area.queue_draw();
            if t >= 1.0 {
                tick.borrow_mut().take();
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
        *inner.hair_tick.borrow_mut() = Some(id);
    }

    /// Handover: collapse the outgoing occupant over the structural
    /// 200 ms, then reveal whatever `desired` holds by the time the
    /// collapse ends (refreshes during the swap just retarget it).
    fn begin_swap(&self) {
        let inner = &self.inner;
        if inner.swapping.get() {
            return;
        }
        if inner.shown.borrow().is_none() {
            // Nothing on screen to collapse — reveal directly.
            self.finish_swap();
            return;
        }
        inner.swapping.set(true);
        inner.revealer.set_reveal_child(false);
        let delay = if anim::animations_enabled() {
            SWAP_MS
        } else {
            0
        };
        let slot = self.clone();
        glib::timeout_add_local_once(Duration::from_millis(delay), move || {
            slot.inner.swapping.set(false);
            slot.finish_swap();
        });
    }

    fn finish_swap(&self) {
        let inner = &self.inner;
        let desired = inner.desired.borrow().clone();
        *inner.shown.borrow_mut() = desired.as_ref().map(Occ::key);
        if let Some(o) = &desired {
            self.render(o);
            inner.revealer.set_reveal_child(true);
        }
        // None: the slot stays collapsed — empty means "nobody needs you".
    }

    fn render(&self, occ: &Occ) {
        let inner = &self.inner;
        for class in DOT_CLASSES {
            inner.dot.remove_css_class(class);
        }
        match occ {
            Occ::Battery { text } => {
                inner.root.add_css_class("critical");
                inner.dot.set_text("󰂃");
                inner.text.set_text(text);
                inner.meta.set_visible(false);
            }
            Occ::Waiting {
                task, desc, meta, ..
            } => {
                inner.root.remove_css_class("critical");
                inner.dot.set_text("●");
                inner.dot.add_css_class(DOT_CLASSES[*task as usize - 1]);
                inner.text.set_text(desc);
                match meta {
                    Some(m) => {
                        inner.meta.set_text(m);
                        inner.meta.set_visible(true);
                    }
                    None => inner.meta.set_visible(false),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn waiting(pid: i32, acked: bool, age: Option<Duration>, now: SystemTime) -> SessionState {
        SessionState {
            pid,
            desc: format!("task for {pid}"),
            activity: Activity::Waiting,
            progress: None,
            workspace: "5:t2a".into(),
            status_mtime: age.map(|a| now - a),
            acked,
        }
    }

    fn battery(critical: bool) -> BatteryView {
        BatteryView {
            critical,
            text: "13% · 1h 5m".into(),
        }
    }

    #[test]
    fn empty_when_nothing_needs_the_owner() {
        let now = SystemTime::now();
        assert_eq!(occupant(None, &TaskSnapshot::default(), now), None);
        // A healthy battery is not an occupant.
        assert_eq!(
            occupant(Some(&battery(false)), &TaskSnapshot::default(), now),
            None
        );
    }

    #[test]
    fn battery_critical_outranks_waiting() {
        let now = SystemTime::now();
        let mut snap = TaskSnapshot::default();
        snap.tasks[0].sessions = vec![waiting(1, false, Some(Duration::ZERO), now)];
        assert_eq!(
            occupant(Some(&battery(true)), &snap, now),
            Some(Occ::Battery {
                text: "13% · 1h 5m".into()
            })
        );
        // Charger connect (no longer critical) stands the battery down and
        // the waiting session takes the slot.
        assert!(matches!(
            occupant(Some(&battery(false)), &snap, now),
            Some(Occ::Waiting { pid: 1, .. })
        ));
    }

    #[test]
    fn oldest_unacked_wait_holds_the_slot_with_a_more_suffix() {
        let now = SystemTime::now();
        let mut snap = TaskSnapshot::default();
        snap.tasks[0].sessions = vec![waiting(1, false, Some(Duration::from_secs(60)), now)];
        snap.tasks[2].sessions = vec![
            waiting(2, false, Some(Duration::from_secs(12 * 60)), now),
            waiting(3, false, None, now), // no mtime = age zero, never oldest
        ];
        let occ = occupant(None, &snap, now).unwrap();
        assert_eq!(
            occ,
            Occ::Waiting {
                pid: 2,
                task: 3,
                desc: "task for 2".into(),
                meta: Some("12m +2".into()),
            }
        );
    }

    #[test]
    fn meta_is_chip_or_suffix_alone_when_the_other_is_absent() {
        let now = SystemTime::now();
        // Young wait (< 2 min chip threshold), alone: no meta at all.
        let mut snap = TaskSnapshot::default();
        snap.tasks[0].sessions = vec![waiting(1, false, Some(Duration::from_secs(30)), now)];
        assert!(matches!(
            occupant(None, &snap, now),
            Some(Occ::Waiting { meta: None, .. })
        ));
        // Young wait with company: suffix only.
        snap.tasks[1].sessions = vec![waiting(2, false, Some(Duration::from_secs(10)), now)];
        assert!(matches!(
            occupant(None, &snap, now),
            Some(Occ::Waiting { meta: Some(m), .. }) if m == "+1"
        ));
    }

    #[test]
    fn acked_sessions_never_occupy_the_slot() {
        let now = SystemTime::now();
        let mut snap = TaskSnapshot::default();
        snap.tasks[0].sessions = vec![waiting(1, true, Some(Duration::from_secs(12 * 60)), now)];
        assert_eq!(occupant(None, &snap, now), None);

        // Two waiting, oldest acked: the younger unacked one owns the
        // slot with no suffix (the suffix counts unacked waits only).
        snap.tasks[1].sessions = vec![waiting(2, false, Some(Duration::from_secs(60)), now)];
        assert!(matches!(
            occupant(None, &snap, now),
            Some(Occ::Waiting {
                pid: 2,
                meta: None,
                ..
            })
        ));
    }

    #[test]
    fn future_mtime_counts_as_age_zero_for_oldest_pick() {
        let now = SystemTime::now();
        let mut snap = TaskSnapshot::default();
        let mut skewed = waiting(1, false, None, now);
        skewed.status_mtime = Some(now + Duration::from_secs(3600)); // suspend skew
        snap.tasks[0].sessions = vec![
            skewed,
            waiting(2, false, Some(Duration::from_secs(60)), now),
        ];
        assert!(matches!(
            occupant(None, &snap, now),
            Some(Occ::Waiting { pid: 2, .. })
        ));
    }
}
