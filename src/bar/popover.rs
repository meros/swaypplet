//! The read layer — one popover chassis for every bar mark
//! (docs/BAR_VISION.md, increment 8).
//!
//! Ambient marks stay glyph-quiet (P5); prose lives here, opened by click
//! and only by click — nothing on the bar is hover-gated (P8). Board bays
//! open the task section: full description, raw `N/M ETA` text,
//! per-session rows with status_mtime-derived durations (ages straddling
//! a detected suspend read approximate, "~9h"), and row activation (click
//! or Enter) focusing the session's workspace over the same sway_ipc path
//! as the keybindings. The task-find/task-rename actions the bay click
//! used to fire directly live on as footer buttons; their sway
//! keybindings are independent and unaffected. A last-message row appears
//! per session once the nixos-side Stop hook writes last-<pid>; without
//! the hook the row is simply absent. Battery and media marks reuse the
//! chassis with their own sections (bar/battery.rs, bar/media.rs).
//!
//! Cadence: content renders at open and, for the task section, on
//! TaskStateService change while open — no timer of its own.

use std::rc::Rc;
use std::time::{Duration, SystemTime};

use gtk4::prelude::*;

use super::board::session_age;
use super::workspaces::switch_command;
use crate::spawn::spawn_work;
use crate::sway_ipc;
use crate::task_state::{Activity, SessionState, TaskState, first_line, state_dir};

/// The shared chassis: a top-anchored popover whose child is the glass
/// card. The card lives on the child, not the popover node: popup
/// surfaces sit outside swayfx's layer_effects list, so there is no
/// frost behind them and .bar-popover-body swaps the translucent fill
/// for the raised opaque one (keeping the glass-card radius + border).
pub fn chassis(parent: &impl IsA<gtk4::Widget>) -> (gtk4::Popover, gtk4::Box) {
    let body = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(8)
        .css_classes(["glass-card", "bar-popover-body"])
        .build();
    let popover = gtk4::Popover::builder()
        .position(gtk4::PositionType::Top)
        .has_arrow(false)
        .css_classes(["bar-popover"])
        .child(&body)
        .build();
    popover.set_parent(parent);
    // set_parent without a container: the popover must be unparented when
    // the parent dies or GTK warns about a finalized widget with children.
    let weak = popover.downgrade();
    parent.connect_destroy(move |_| {
        if let Some(popover) = weak.upgrade() {
            popover.unparent();
        }
    });
    (popover, body)
}

/// Left-aligned prose label — the popovers' basic row unit.
pub fn line(text: &str, class: &str) -> gtk4::Label {
    gtk4::Label::builder()
        .label(text)
        .xalign(0.0)
        .css_classes([class])
        .build()
}

// ── Task section (board bays) ───────────────────────────────────────────

/// Cheap handle, cloned into the bay's click handler.
#[derive(Clone)]
pub struct TaskPopover {
    inner: Rc<Inner>,
}

struct Inner {
    n: u8,
    popover: gtk4::Popover,
    body: gtk4::Box,
}

impl TaskPopover {
    pub fn new(parent: &impl IsA<gtk4::Widget>, n: u8) -> Self {
        let (popover, body) = chassis(parent);
        Self {
            inner: Rc::new(Inner { n, popover, body }),
        }
    }

    pub fn open(&self, task: &TaskState, skew: Option<SystemTime>) {
        self.render(task, skew);
        self.inner.popover.popup();
    }

    /// Live refresh while open (TaskStateService change, including its
    /// 1/min waiting-age tick); closed popovers cost nothing.
    pub fn refresh_if_open(&self, task: &TaskState, skew: Option<SystemTime>) {
        if self.inner.popover.is_visible() {
            self.render(task, skew);
        }
    }

    fn render(&self, task: &TaskState, skew: Option<SystemTime>) {
        let body = &self.inner.body;
        while let Some(child) = body.first_child() {
            body.remove(&child);
        }

        let title = match &task.manual {
            Some(manual) => format!("TASK {} · {manual}", self.inner.n),
            None => format!("TASK {}", self.inner.n),
        };
        body.append(&line(&title, "bar-popover-title"));

        if task.sessions.is_empty() {
            body.append(&line("No session", "bar-popover-empty"));
        } else {
            let now = SystemTime::now();
            let list = gtk4::ListBox::builder()
                .selection_mode(gtk4::SelectionMode::None)
                .css_classes(["bar-popover-list"])
                .build();
            for session in &task.sessions {
                list.append(&session_row(session, now, skew));
            }
            // Row activation (click or Enter) focuses the session's
            // workspace — the same act that acks the wait (P10).
            let targets: Vec<String> = task.sessions.iter().map(|s| s.workspace.clone()).collect();
            let popover = self.inner.popover.clone();
            list.connect_row_activated(move |_, row| {
                if let Some(workspace) = targets.get(row.index() as usize) {
                    sway_ipc::run_command(&focus_command(workspace));
                    popover.popdown();
                }
            });
            body.append(&list);
        }

        let actions = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(6)
            .css_classes(["bar-popover-actions"])
            .build();
        for (label, cmd) in [("Find", "task-find"), ("Rename", "task-rename")] {
            let btn = gtk4::Button::builder()
                .label(label)
                .css_classes(["bar-popover-action"])
                .build();
            let popover = self.inner.popover.clone();
            btn.connect_clicked(move |_| {
                popover.popdown();
                run_task_command(cmd);
            });
            actions.append(&btn);
        }
        body.append(&actions);
    }
}

fn session_row(s: &SessionState, now: SystemTime, skew: Option<SystemTime>) -> gtk4::ListBoxRow {
    let col = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(2)
        .css_classes(["bar-popover-row"])
        .build();
    let desc = line(&s.desc, "bar-popover-desc");
    desc.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    desc.set_max_width_chars(44);
    col.append(&desc);
    col.append(&line(&meta_line(s, now, skew), "bar-popover-meta"));
    if let Some(msg) = last_message(s.pid) {
        let last = line(&msg, "bar-popover-last");
        last.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        last.set_max_width_chars(44);
        col.append(&last);
    }
    gtk4::ListBoxRow::builder().child(&col).build()
}

/// First line of the last assistant message, written by the nixos-side
/// Stop hook (vision increment 9, other repo). Absent file = absent row.
///
/// Nothing sweeps `last-<PID>`, so the directory holds one per session the
/// machine has ever run, and PIDs come back around. A file written before
/// its process started belongs to whoever held the number last, and
/// showing it would attribute a dead session's words to a live one — the
/// same failure the comm gate closes for descriptions (task_state.rs).
/// An unreadable start time leaves the file trusted, which is where this
/// stood before.
fn last_message(pid: i32) -> Option<String> {
    let path = state_dir().join(format!("last-{pid}"));
    let written = std::fs::metadata(&path).and_then(|m| m.modified()).ok()?;
    if crate::task_state::proc_start_time(pid).is_some_and(|start| written < start) {
        return None;
    }
    first_line(&path)
}

/// task-find / task-rename come from the nixos config's PATH; a machine
/// without them just logs. `status()` runs on a worker so the picker the
/// script opens can't block the bar, and the child gets reaped.
fn run_task_command(cmd: &'static str) {
    spawn_work(
        move || {
            std::process::Command::new(cmd)
                .status()
                .map_err(|e| e.to_string())
        },
        move |result| {
            if let Err(e) = result {
                log::warn!("bar popover: `{cmd}`: {e}");
            }
        },
    );
}

// ── Pure helpers (unit-tested below) ────────────────────────────────────

/// Same target resolution as the workspace buttons: numbered workspaces
/// switch by number, so the row and the keybinding land identically.
fn focus_command(name: &str) -> String {
    let num = name
        .split(':')
        .next()
        .and_then(|s| s.parse::<i32>().ok())
        .unwrap_or(-1);
    switch_command(num, name)
}

/// One row's state line: activity + status_mtime age, raw progress text
/// appended verbatim (the popover is the prose surface for `N/M ETA`).
fn meta_line(s: &SessionState, now: SystemTime, skew: Option<SystemTime>) -> String {
    let state = match s.activity {
        Activity::Working => format!("working {}", age_text(s, now, skew)),
        // Named for what it wants, since the popover is the one surface
        // with room to say it.
        Activity::Blocked => format!("needs permission {}", age_text(s, now, skew)),
        Activity::Waiting => format!("waiting {}", age_text(s, now, skew)),
        Activity::Stopped => format!("stopped {}", age_text(s, now, skew)),
        // No age for an invalid write — dating it would lend it
        // credibility (P9).
        Activity::Stale => "status unknown".to_string(),
    };
    match &s.progress {
        Some(p) => format!("{state} · {}", p.raw),
        None => state,
    }
}

/// "~" marks ages that straddle a detected suspend: the mtime is
/// wall-clock, so slept hours count as waited hours and only an
/// approximate reading is honest.
fn age_text(s: &SessionState, now: SystemTime, skew: Option<SystemTime>) -> String {
    let approx = matches!((s.status_mtime, skew), (Some(m), Some(b)) if m < b);
    format!(
        "{}{}",
        if approx { "~" } else { "" },
        fmt_age(session_age(s, now))
    )
}

/// Minute-floor prose age ("<1m", "12m", "1h 5m") — the bay chip keeps
/// its terser form (board.rs).
fn fmt_age(age: Duration) -> String {
    let mins = age.as_secs() / 60;
    match mins {
        0 => "<1m".into(),
        m if m < 60 => format!("{m}m"),
        m if m % 60 == 0 => format!("{}h", m / 60),
        m => format!("{}h {}m", m / 60, m % 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task_state::Progress;

    fn session(activity: Activity, age: Option<Duration>, now: SystemTime) -> SessionState {
        SessionState {
            pid: 7,
            desc: "fix flaky auth retry".into(),
            activity,
            progress: None,
            workspace: "5:t2a".into(),
            status_mtime: age.map(|a| now - a),
            acked: false,
        }
    }

    #[test]
    fn ages_format_as_prose() {
        assert_eq!(fmt_age(Duration::ZERO), "<1m");
        assert_eq!(fmt_age(Duration::from_secs(59)), "<1m");
        assert_eq!(fmt_age(Duration::from_secs(12 * 60)), "12m");
        assert_eq!(fmt_age(Duration::from_secs(65 * 60)), "1h 5m");
        assert_eq!(fmt_age(Duration::from_secs(9 * 3600)), "9h");
    }

    #[test]
    fn meta_line_carries_state_age_and_raw_progress() {
        let now = SystemTime::now();
        let mut s = session(Activity::Working, Some(Duration::from_secs(12 * 60)), now);
        s.progress = Some(Progress {
            raw: "1/5 ETA ~15m".into(),
            fraction: Some((1, 5)),
        });
        assert_eq!(meta_line(&s, now, None), "working 12m · 1/5 ETA ~15m");

        let s = session(Activity::Waiting, Some(Duration::from_secs(30)), now);
        assert_eq!(meta_line(&s, now, None), "waiting <1m");

        // Stale carries no age (P9): dating an invalid write dresses it up.
        let s = session(Activity::Stale, Some(Duration::from_secs(600)), now);
        assert_eq!(meta_line(&s, now, None), "status unknown");
    }

    #[test]
    fn suspend_skewed_ages_read_approximate() {
        let now = SystemTime::now();
        // Status written before the detected suspend: the age straddles it.
        let s = session(Activity::Waiting, Some(Duration::from_secs(9 * 3600)), now);
        let boundary = Some(now - Duration::from_secs(3600));
        assert_eq!(meta_line(&s, now, boundary), "waiting ~9h");
        // Written after the last suspend: the age is clean.
        let s = session(Activity::Waiting, Some(Duration::from_secs(60)), now);
        assert_eq!(meta_line(&s, now, boundary), "waiting 1m");
        // No suspend ever detected: never approximate.
        let s = session(Activity::Waiting, Some(Duration::from_secs(9 * 3600)), now);
        assert_eq!(meta_line(&s, now, None), "waiting 9h");
    }

    #[test]
    fn focus_command_matches_the_keybinding_path() {
        assert_eq!(focus_command("5:t2a"), "workspace number 5");
        assert_eq!(focus_command("19:wb"), "workspace number 19");
        assert_eq!(focus_command("mail"), "workspace \"mail\"");
    }
}
