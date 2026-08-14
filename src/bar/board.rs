//! The board — four-bay cross-task instrument, middle segment of the
//! bar's right track (docs/BAR_VISION.md, increment 3).
//!
//! Replaces the per-output task pill: one fixed bay per task 1-4,
//! identical on every output, so the whole cross-task picture is one
//! glance. Position encodes task, each bay draws its numeral, hue only
//! reinforces (vision P3) — every state stays distinct under grayscale:
//! socket = hollow ring, working = mid fill, waiting = bright hue fill,
//! stopped = near-off dot, stale = amber hollow OFF-flag. Motion is
//! onset-only (P2): one SlideBin nudge when a bay turns unacked-waiting,
//! then static rest; escalation at 10 min is a static luminance step.
//!
//! Hidden since 2026-08-12 (see bar/mod.rs): the bays were not earning
//! their width. The widget is still built, so nothing else in the bar had
//! to change; flip `SHOW_BOARD` back to display it. The accent ripple it
//! used to stamp is gone with the bar's task hue.
//!
//! Bay click opens the read-layer task popover (bar/popover.rs, vision
//! increment 8); the old direct task-find/task-rename click bindings live
//! there as footer actions.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use gtk4::prelude::*;

use super::popover::TaskPopover;
use crate::anim::{self, SlideBin};
use crate::sway_ipc::SwayService;
use crate::task_state::{Activity, SessionState, TaskState, TaskStateService, task_of_name};

const BAY_TASK_CLASSES: [&str; 4] = ["t1", "t2", "t3", "t4"];
const STATE_CLASSES: [&str; 8] = [
    "socket", "working", "waiting", "blocked", "unacked", "overdue", "stopped", "stale",
];

/// Age chip appears once a wait is no longer a blip.
const CHIP_AFTER: Duration = Duration::from_secs(2 * 60);
/// Unacked past this earns the static luminance tier step (owner
/// decision 1: never a loop).
const OVERDUE_AFTER: Duration = Duration::from_secs(10 * 60);

// ── View model ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum BayState {
    #[default]
    Socket,
    Working,
    /// Halted on a permission prompt: the top of the ladder, because it
    /// is the only state where the session cannot continue without the
    /// owner and the answer takes two seconds.
    Blocked,
    Waiting {
        acked: bool,
        overdue: bool,
    },
    Stopped,
    Stale,
}

/// Everything one bay shows, comparable so no-op refreshes (sway fires
/// per keystroke on title changes) skip the widget update — and so the
/// unacked *onset* is a detectable edge for the nudge.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct BayView {
    state: BayState,
    /// Age chip text; `None` below the 2 min threshold.
    chip: Option<String>,
    /// `N/M` fraction for the bottom fill; single-session tasks only.
    fill: Option<(u32, u32)>,
    /// This output's visible workspace belongs to this bay's task.
    local: bool,
    tooltip: String,
}

impl BayView {
    /// Drives the onset nudge: both states that want the owner qualify,
    /// and a blocked bay always does (a permission prompt has no acked
    /// tier — answering it is the acknowledgment).
    fn is_unacked(&self) -> bool {
        matches!(
            self.state,
            BayState::Blocked | BayState::Waiting { acked: false, .. }
        )
    }
}

fn state_classes(state: BayState) -> &'static [&'static str] {
    match state {
        BayState::Socket => &["socket"],
        BayState::Working => &["working"],
        // Rides the same solid-hue fill as an unacked wait; the marker
        // glyph is what separates them, per the shape rule below.
        BayState::Blocked => &["waiting", "unacked", "blocked"],
        BayState::Waiting { acked: true, .. } => &["waiting"],
        BayState::Waiting {
            acked: false,
            overdue: false,
        } => &["waiting", "unacked"],
        BayState::Waiting {
            acked: false,
            overdue: true,
        } => &["waiting", "unacked", "overdue"],
        BayState::Stopped => &["stopped"],
        BayState::Stale => &["stale"],
    }
}

/// Shape channel beside the numeral: states whose CSS box (border/fill)
/// alone would not survive grayscale get a glyph. Stale's hollow ring
/// marks "data invalid", never a live filled state (P9).
fn state_marker(state: BayState) -> &'static str {
    match state {
        BayState::Blocked => "!",
        BayState::Stopped => "·",
        BayState::Stale => "○",
        _ => "",
    }
}

/// Collapse a task's sessions into one bay: waiting outranks everything
/// (that is the state that needs the owner), then stale (a lying bay is
/// worse than a busy one, P9), then working, then stopped.
fn bay_view(n: u8, task: &TaskState, local: bool, now: SystemTime) -> BayView {
    // Blocked and waiting share the chip and the fill: both are "wants
    // you", and the age of a stuck permission prompt is as worth knowing
    // as the age of a finished turn.
    let waiting: Vec<&SessionState> = task
        .sessions
        .iter()
        .filter(|s| s.activity.wants_owner())
        .collect();
    let any = |a: Activity| task.sessions.iter().any(|s| s.activity == a);
    let state = if any(Activity::Blocked) {
        BayState::Blocked
    } else if !waiting.is_empty() {
        BayState::Waiting {
            acked: waiting.iter().all(|s| s.acked),
            overdue: waiting
                .iter()
                .filter(|s| !s.acked)
                .map(|s| session_age(s, now))
                .max()
                .is_some_and(|age| age >= OVERDUE_AFTER),
        }
    } else if any(Activity::Stale) {
        BayState::Stale
    } else if any(Activity::Working) {
        BayState::Working
    } else if !task.sessions.is_empty() {
        BayState::Stopped
    } else {
        BayState::Socket
    };
    BayView {
        state,
        // Oldest wait owns the chip; it stays after ack (stand-down table).
        chip: waiting
            .iter()
            .map(|s| session_age(s, now))
            .max()
            .and_then(chip_label),
        fill: match task.sessions.as_slice() {
            [s] => s.progress.as_ref().and_then(|p| p.fraction),
            _ => None,
        },
        local,
        tooltip: tooltip(n, task),
    }
}

/// A missing or future mtime (suspend skew) counts as age zero, matching
/// the service's age-timer rule. Shared with the decision slot.
pub(crate) fn session_age(s: &SessionState, now: SystemTime) -> Duration {
    s.status_mtime
        .and_then(|m| now.duration_since(m).ok())
        .unwrap_or(Duration::ZERO)
}

/// `None` below the 2 min threshold. Shared with the decision slot.
pub(crate) fn chip_label(age: Duration) -> Option<String> {
    let mins = age.as_secs() / 60;
    (age >= CHIP_AFTER).then(|| {
        if mins < 60 {
            format!("{mins}m")
        } else {
            format!("{}h", mins / 60)
        }
    })
}

fn tooltip(n: u8, task: &TaskState) -> String {
    let mut lines = vec![match &task.manual {
        Some(manual) => format!("Task {n} · {manual}"),
        None => format!("Task {n}"),
    }];
    for s in &task.sessions {
        lines.push(match &s.progress {
            Some(p) => format!("{} · {}", s.desc, p.raw),
            None => s.desc.clone(),
        });
    }
    lines.join("\n")
}

// ── Widgets ─────────────────────────────────────────────────────────────

/// `output` is this bar's sway output name (gdk connector); `bar_root`
/// receives the bar-taskN accent class.
pub fn build(
    sway: &Rc<SwayService>,
    tasks: &Rc<TaskStateService>,
    output: Option<String>,
    bar_root: &impl IsA<gtk4::Widget>,
) -> gtk4::Box {
    let board = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(2)
        .css_classes(["bar-board", "bar-seg"])
        .build();
    let bays: Vec<Bay> = (1..=4).map(Bay::new).collect();
    for (i, bay) in bays.iter().enumerate() {
        board.append(&bay.button);
        let n = i as u8 + 1;
        let tasks = tasks.clone();
        let popover = bay.popover.clone();
        bay.button.connect_clicked(move |_| {
            popover.open(tasks.snapshot().task(n), tasks.skew_boundary());
        });
    }

    let refresh: Rc<dyn Fn()> = {
        let weak = board.downgrade();
        let root = bar_root.upcast_ref::<gtk4::Widget>().downgrade();
        let sway = sway.clone();
        let tasks = tasks.clone();
        Rc::new(move || {
            // Output unplugged → the leftover observer no-ops (same story
            // as workspaces.rs).
            if weak.upgrade().is_none() {
                return;
            }
            let local = local_task(output.as_deref(), &sway);
            if let Some(root) = root.upgrade() {
                apply_accent(&root, local);
            }
            let snapshot = tasks.snapshot();
            let skew = tasks.skew_boundary();
            let now = SystemTime::now();
            for (i, bay) in bays.iter().enumerate() {
                let n = i as u8 + 1;
                bay.apply(bay_view(n, snapshot.task(n), local == Some(n), now));
                bay.popover.refresh_if_open(snapshot.task(n), skew);
            }
        })
    };

    refresh();
    {
        let refresh = refresh.clone();
        sway.connect_change(move || refresh());
    }
    {
        let refresh = refresh.clone();
        tasks.connect_change(move || refresh());
    }

    board
}

/// 1–4 when this output's visible workspace belongs to a task.
fn local_task(output: Option<&str>, sway: &SwayService) -> Option<u8> {
    output
        .and_then(|out| {
            sway.snapshot()
                .workspaces
                .into_iter()
                .find(|w| w.visible && w.output == out)
        })
        .and_then(|w| task_of_name(&w.name))
}

/// Was the accent ripple: stamped bar-task1..4 on the bar root so the right
/// track's band lane took the local task's hue. The hue was removed from the
/// bar on 2026-08-12 and style.css no longer defines those selectors, so this
/// is a no-op kept only to leave the call sites and the local-task plumbing
/// intact while the board is hidden.
fn apply_accent(_root: &gtk4::Widget, _task: Option<u8>) {}

struct Bay {
    button: gtk4::Button,
    popover: TaskPopover,
    slide: SlideBin,
    marker: gtk4::Label,
    chip_reveal: gtk4::Revealer,
    chip: gtk4::Label,
    fill: gtk4::DrawingArea,
    /// Fraction the draw func paints; eased toward the model's value.
    fill_shown: Rc<Cell<f64>>,
    fill_tick: Rc<RefCell<Option<gtk4::TickCallbackId>>>,
    /// `None` until the first apply, so startup styling never reads as an
    /// onset (no nudge for a wait that predates the bar).
    cache: RefCell<Option<BayView>>,
}

impl Bay {
    fn new(n: u8) -> Self {
        let marker = gtk4::Label::builder()
            .css_classes(["bar-bay-marker"])
            .visible(false)
            .build();
        let numeral = gtk4::Label::new(Some(&n.to_string()));
        let chip = gtk4::Label::builder().css_classes(["bar-bay-chip"]).build();
        let chip_reveal = gtk4::Revealer::builder()
            .transition_type(gtk4::RevealerTransitionType::Crossfade)
            .transition_duration(200)
            .child(&chip)
            .build();
        let content = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .halign(gtk4::Align::Center)
            .build();
        content.append(&marker);
        content.append(&numeral);
        content.append(&chip_reveal);

        let fill_shown: Rc<Cell<f64>> = Rc::new(Cell::new(0.0));
        let fill = gtk4::DrawingArea::builder()
            .content_height(2)
            .halign(gtk4::Align::Fill)
            .valign(gtk4::Align::End)
            .visible(false)
            .build();
        {
            let shown = fill_shown.clone();
            fill.set_draw_func(move |area, cr, w, h| {
                let f = shown.get().clamp(0.0, 1.0);
                if f <= 0.0 {
                    return;
                }
                let c = area.color();
                cr.set_source_rgba(
                    c.red().into(),
                    c.green().into(),
                    c.blue().into(),
                    c.alpha().into(),
                );
                cr.rectangle(0.0, 0.0, f64::from(w) * f, f64::from(h));
                let _ = cr.fill();
            });
        }

        let overlay = gtk4::Overlay::builder().child(&content).build();
        overlay.add_overlay(&fill);
        let slide = SlideBin::new();
        slide.set_child(&overlay);

        let button = gtk4::Button::builder()
            .child(&slide)
            .css_classes(["bar-bay", BAY_TASK_CLASSES[n as usize - 1]])
            .build();
        let popover = TaskPopover::new(&button, n);

        Self {
            button,
            popover,
            slide,
            marker,
            chip_reveal,
            chip,
            fill,
            fill_shown,
            fill_tick: Rc::new(RefCell::new(None)),
            cache: RefCell::new(None),
        }
    }

    fn apply(&self, view: BayView) {
        let mut cache = self.cache.borrow_mut();
        if cache.as_ref() == Some(&view) {
            return;
        }
        let onset = matches!(&*cache, Some(prev) if view.is_unacked() && !prev.is_unacked());

        for class in STATE_CLASSES {
            self.button.remove_css_class(class);
        }
        for class in state_classes(view.state) {
            self.button.add_css_class(class);
        }
        if view.local {
            self.button.add_css_class("local");
        } else {
            self.button.remove_css_class("local");
        }

        let marker = state_marker(view.state);
        self.marker.set_text(marker);
        self.marker.set_visible(!marker.is_empty());

        if let Some(text) = &view.chip {
            self.chip.set_text(text);
        }
        self.chip_reveal.set_reveal_child(view.chip.is_some());

        self.fill.set_visible(view.fill.is_some());
        let target = view
            .fill
            .map_or(0.0, |(n, m)| f64::from(n) / f64::from(m).max(1.0));
        self.animate_fill(target);

        self.button.set_tooltip_text(Some(&view.tooltip));
        if onset {
            anim::nudge(&self.slide);
        }
        *cache = Some(view);
    }

    /// Ease the drawn fraction toward `target` over MOVE_MS; reduced
    /// motion jumps (all states are statically distinct).
    fn animate_fill(&self, target: f64) {
        if let Some(id) = self.fill_tick.borrow_mut().take() {
            id.remove();
        }
        let from = self.fill_shown.get();
        if !anim::animations_enabled() || from == target {
            self.fill_shown.set(target);
            self.fill.queue_draw();
            return;
        }
        let start = glib::monotonic_time();
        let shown = self.fill_shown.clone();
        let tick = self.fill_tick.clone();
        let id = self.fill.add_tick_callback(move |area, _| {
            let t = (((glib::monotonic_time() - start) as f64 / 1000.0)
                / anim::duration(anim::MOVE_MS))
            .clamp(0.0, 1.0);
            shown.set(from + (target - from) * anim::ease_out_cubic(t));
            area.queue_draw();
            if t >= 1.0 {
                tick.borrow_mut().take();
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
        *self.fill_tick.borrow_mut() = Some(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(
        activity: Activity,
        acked: bool,
        age: Option<Duration>,
        now: SystemTime,
    ) -> SessionState {
        SessionState {
            pid: 1,
            desc: "fix flaky auth retry".into(),
            activity,
            progress: None,
            workspace: "5:t2a".into(),
            status_mtime: age.map(|a| now - a),
            acked,
        }
    }

    fn task_of(sessions: Vec<SessionState>) -> TaskState {
        TaskState {
            sessions,
            manual: None,
        }
    }

    #[test]
    fn empty_bay_is_a_socket() {
        let now = SystemTime::now();
        let view = bay_view(1, &TaskState::default(), false, now);
        assert_eq!(view.state, BayState::Socket);
        assert_eq!(view.chip, None);
        assert_eq!(view.fill, None);
    }

    #[test]
    fn blocked_outranks_every_other_state_and_carries_the_marker() {
        let now = SystemTime::now();
        let task = task_of(vec![
            session(Activity::Working, false, None, now),
            session(Activity::Waiting, false, Some(Duration::from_secs(60)), now),
            session(Activity::Blocked, false, Some(Duration::ZERO), now),
        ]);
        let view = bay_view(1, &task, false, now);
        assert_eq!(view.state, BayState::Blocked);
        // Rides the unacked fill, separated by shape, and nudges on onset.
        assert_eq!(state_classes(view.state), ["waiting", "unacked", "blocked"]);
        assert_eq!(state_marker(view.state), "!");
        assert!(view.is_unacked());
    }

    #[test]
    fn a_blocked_session_still_earns_an_age_chip() {
        let now = SystemTime::now();
        let task = task_of(vec![session(
            Activity::Blocked,
            false,
            Some(Duration::from_secs(5 * 60)),
            now,
        )]);
        assert_eq!(bay_view(1, &task, false, now).chip, Some("5m".into()));
    }

    #[test]
    fn waiting_outranks_working_and_stale_outranks_stopped() {
        let now = SystemTime::now();
        let task = task_of(vec![
            session(Activity::Working, false, None, now),
            session(Activity::Waiting, false, Some(Duration::ZERO), now),
        ]);
        assert!(matches!(
            bay_view(1, &task, false, now).state,
            BayState::Waiting { acked: false, .. }
        ));

        let task = task_of(vec![
            session(Activity::Stopped, false, None, now),
            session(Activity::Stale, false, None, now),
        ]);
        assert_eq!(bay_view(1, &task, false, now).state, BayState::Stale);
        let task = task_of(vec![session(Activity::Stopped, false, None, now)]);
        assert_eq!(bay_view(1, &task, false, now).state, BayState::Stopped);
    }

    #[test]
    fn ack_needs_every_waiting_session() {
        let now = SystemTime::now();
        let task = task_of(vec![
            session(Activity::Waiting, true, Some(Duration::ZERO), now),
            session(Activity::Waiting, false, Some(Duration::ZERO), now),
        ]);
        assert!(bay_view(1, &task, false, now).is_unacked());
        let task = task_of(vec![session(
            Activity::Waiting,
            true,
            Some(Duration::ZERO),
            now,
        )]);
        assert!(!bay_view(1, &task, false, now).is_unacked());
    }

    #[test]
    fn overdue_steps_at_ten_minutes_unacked_only() {
        let now = SystemTime::now();
        let old = Some(Duration::from_secs(11 * 60));
        let task = task_of(vec![session(Activity::Waiting, false, old, now)]);
        assert_eq!(
            bay_view(1, &task, false, now).state,
            BayState::Waiting {
                acked: false,
                overdue: true
            }
        );
        // Ack drops the escalation with the luminance.
        let task = task_of(vec![session(Activity::Waiting, true, old, now)]);
        assert_eq!(
            bay_view(1, &task, false, now).state,
            BayState::Waiting {
                acked: true,
                overdue: false
            }
        );
    }

    #[test]
    fn chip_appears_at_two_minutes_and_survives_ack() {
        assert_eq!(chip_label(Duration::from_secs(90)), None);
        assert_eq!(chip_label(Duration::from_secs(12 * 60)), Some("12m".into()));
        assert_eq!(chip_label(Duration::from_secs(61 * 60)), Some("1h".into()));

        let now = SystemTime::now();
        let age = Some(Duration::from_secs(12 * 60));
        let task = task_of(vec![session(Activity::Waiting, true, age, now)]);
        assert_eq!(bay_view(1, &task, false, now).chip, Some("12m".into()));
    }

    #[test]
    fn fill_draws_only_for_single_session_tasks() {
        let now = SystemTime::now();
        let with_progress = |acked| {
            let mut s = session(Activity::Working, acked, None, now);
            s.progress = Some(crate::task_state::Progress {
                raw: "1/5 ETA ~15m".into(),
                fraction: Some((1, 5)),
            });
            s
        };
        let task = task_of(vec![with_progress(false)]);
        assert_eq!(bay_view(1, &task, false, now).fill, Some((1, 5)));
        let task = task_of(vec![with_progress(false), with_progress(false)]);
        assert_eq!(bay_view(1, &task, false, now).fill, None);
    }

    #[test]
    fn every_state_keeps_a_grayscale_shape() {
        // Stopped and stale have no fill/border luminance of their own, so
        // they carry the shape glyph; stale is hollow, never filled (P9).
        assert_eq!(state_marker(BayState::Stopped), "·");
        assert_eq!(state_marker(BayState::Stale), "○");
        assert_eq!(state_marker(BayState::Socket), "");
        assert_eq!(
            state_classes(BayState::Waiting {
                acked: false,
                overdue: true
            }),
            &["waiting", "unacked", "overdue"]
        );
        assert_eq!(
            state_classes(BayState::Waiting {
                acked: true,
                overdue: false
            }),
            &["waiting"]
        );
    }
}
