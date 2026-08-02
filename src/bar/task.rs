//! Claude task pill — middle segment of the bar's right instrument track.
//!
//! Ports waybar's custom/task indicator plus the task-accent emitters
//! (users/modules/task-scripts.nix + waybar.nix in the nixos repo). The
//! file contract is unchanged: ~/.local/state/claude-tasks/ holds
//! pid-<PID> (description), status-<PID> (working|waiting|stopped),
//! progress-<PID> and manual-t<N> rename overrides — the ~/.claude hooks
//! keep writing them. A GFileMonitor (inotify under GIO) replaces the
//! inotifywait loop, and the SwayService pid→workspace cache replaces the
//! per-event swaymsg+jq tree walk. The /proc parent-chain hop remains:
//! the sway window belongs to Claude's terminal ancestor, not to the
//! Claude process that wrote the pid file.
//!
//! Accent ripple: the pill stamps bar-task1..4 on its bar's root and
//! style.css colors the track band + focused workspace from there —
//! descendant selectors replace waybar's invisible emitter modules and
//! their general-sibling hack.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gio::prelude::*;
use gtk4::prelude::*;

use crate::spawn::spawn_work;
use crate::sway_ipc::SwayService;

const TASK_CLASSES: [&str; 4] = ["bar-task1", "bar-task2", "bar-task3", "bar-task4"];
/// Waybar left descriptions uncapped; a runaway description would push the
/// clock off the card here, so each one ellipsizes instead.
const MAX_DESC_CHARS: i32 = 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Activity {
    Working,
    Waiting,
    Stopped,
    /// Data invalid — unknown status value or missing status file. Rendered
    /// as the amber OFF-flag, never as Waiting: a channel that can't say
    /// "I don't know" can't be trusted when it says "act" (vision P9).
    Stale,
}

impl Activity {
    fn parse(s: &str) -> Self {
        match s {
            "working" => Self::Working,
            "waiting" => Self::Waiting,
            "stopped" => Self::Stopped,
            _ => Self::Stale,
        }
    }

    fn css_class(self) -> &'static str {
        match self {
            Self::Working => "working",
            Self::Waiting => "waiting",
            Self::Stopped => "stopped",
            Self::Stale => "stale",
        }
    }

    /// Stale draws hollow so "data invalid" survives grayscale and never
    /// reads as a live filled state.
    fn glyph(self) -> &'static str {
        match self {
            Self::Stale => "○",
            _ => "●",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Session {
    desc: String,
    activity: Activity,
    progress: Option<String>,
}

/// Everything the pill shows, comparable so no-op refreshes (sway fires
/// per keystroke on title changes) skip the widget rebuild.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct PillView {
    /// 1–4 when this output's visible workspace belongs to a task.
    task: Option<u8>,
    sessions: Vec<Session>,
    manual: Option<String>,
}

impl PillView {
    fn is_empty(&self) -> bool {
        self.sessions.is_empty() && self.manual.is_none()
    }
}

/// `output` is this bar's sway output name (gdk connector); `bar_root`
/// receives the bar-taskN accent class.
pub fn build(
    sway: &Rc<SwayService>,
    output: Option<String>,
    bar_root: &impl IsA<gtk4::Widget>,
) -> gtk4::Button {
    let content = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(5)
        .build();
    let btn = gtk4::Button::builder()
        .child(&content)
        .css_classes(["bar-task", "bar-seg", "none"])
        .build();

    let dir = state_dir();
    // The watcher needs the directory before the first session writes it.
    if let Err(e) = fs::create_dir_all(&dir) {
        log::warn!("bar task: create {}: {e}", dir.display());
    }

    let refresh: Rc<dyn Fn()> = {
        let weak = btn.downgrade();
        let content = content.clone();
        let root = bar_root.upcast_ref::<gtk4::Widget>().downgrade();
        let sway = sway.clone();
        let dir = dir.clone();
        let cache: RefCell<Option<PillView>> = RefCell::new(None);
        Rc::new(move || {
            // Output unplugged → the leftover observer no-ops (same story
            // as workspaces.rs).
            let Some(btn) = weak.upgrade() else {
                return;
            };
            let view = read_view(&dir, output.as_deref(), &sway);
            if cache.borrow().as_ref() == Some(&view) {
                return;
            }
            if let Some(root) = root.upgrade() {
                apply_accent(&root, view.task);
            }
            render(&btn, &content, &view);
            *cache.borrow_mut() = Some(view);
        })
    };

    refresh();
    {
        let refresh = refresh.clone();
        sway.connect_change(move || refresh());
    }

    match gio::File::for_path(&dir)
        .monitor_directory(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
    {
        Ok(monitor) => {
            let refresh = refresh.clone();
            monitor.connect_changed(move |_, _, _, _| refresh());
            // A dropped GFileMonitor stops watching; parking it in a
            // destroy handler ties its lifetime to the widget's.
            btn.connect_destroy(move |_| {
                let _ = &monitor;
            });
        }
        Err(e) => log::warn!("bar task: watch {}: {e}", dir.display()),
    }

    btn.connect_clicked(|_| run_task_command("task-find"));
    let right = gtk4::GestureClick::new();
    right.set_button(gtk4::gdk::BUTTON_SECONDARY);
    right.connect_pressed(|_, _, _, _| run_task_command("task-rename"));
    btn.add_controller(right);

    btn
}

fn state_dir() -> PathBuf {
    glib::home_dir().join(".local/state/claude-tasks")
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
                log::warn!("bar task: `{cmd}`: {e}");
            }
        },
    );
}

// ── State assembly ──────────────────────────────────────────────────────

/// Snapshot the pill's inputs (sway cache + state files + /proc) into a
/// comparable view.
fn read_view(dir: &Path, output: Option<&str>, sway: &SwayService) -> PillView {
    let state = sway.snapshot();
    let task = output
        .and_then(|out| {
            state
                .workspaces
                .iter()
                .find(|w| w.visible && w.output == out)
        })
        .and_then(|w| task_of_name(&w.name));
    let Some(task) = task else {
        return PillView::default();
    };

    let mut sessions = Vec::new();
    for pid in claude_pids(dir) {
        let Some(desc) = first_line(&dir.join(format!("pid-{pid}"))) else {
            continue;
        };
        // comm gate: a recycled PID must not resurrect a dead session's
        // description file.
        if !proc_comm(pid).is_some_and(|c| is_claude_comm(&c)) {
            continue;
        }
        let Some(ws) = window_workspace(pid, &state.pid_workspaces, parent_pid) else {
            continue;
        };
        if task_of_name(&ws) != Some(task) {
            continue;
        }
        sessions.push(Session {
            desc,
            activity: first_line(&dir.join(format!("status-{pid}")))
                .map_or(Activity::Stale, |s| Activity::parse(&s)),
            progress: first_line(&dir.join(format!("progress-{pid}"))),
        });
    }

    PillView {
        task: Some(task),
        sessions,
        manual: first_line(&dir.join(format!("manual-t{task}"))),
    }
}

/// PIDs with a pid-<N> description file, sorted for stable render order.
fn claude_pids(dir: &Path) -> Vec<i32> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut pids: Vec<i32> = entries
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.strip_prefix("pid-")?.parse().ok())
        .collect();
    pids.sort_unstable();
    pids
}

/// First line of `path`; `None` when the file is missing or blank.
fn first_line(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let line = text.lines().next()?.trim();
    (!line.is_empty()).then(|| line.to_string())
}

fn proc_comm(pid: i32) -> Option<String> {
    let comm = fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    Some(comm.trim_end().to_string())
}

fn parent_pid(pid: i32) -> Option<i32> {
    parent_pid_from_stat(&fs::read_to_string(format!("/proc/{pid}/stat")).ok()?)
}

/// Walk `pid` and its ancestors until one owns a sway view. Claude sits a
/// few levels below the terminal that owns the window (shell, wrappers).
fn window_workspace(
    pid: i32,
    pid_workspaces: &HashMap<i32, String>,
    parent: impl Fn(i32) -> Option<i32>,
) -> Option<String> {
    let mut p = pid;
    while p > 1 {
        if let Some(ws) = pid_workspaces.get(&p) {
            return Some(ws.clone());
        }
        p = parent(p)?;
    }
    None
}

// ── Rendering ───────────────────────────────────────────────────────────

fn apply_accent(root: &gtk4::Widget, task: Option<u8>) {
    for (i, class) in TASK_CLASSES.iter().enumerate() {
        if task == Some(i as u8 + 1) {
            root.add_css_class(class);
        } else {
            root.remove_css_class(class);
        }
    }
}

fn render(btn: &gtk4::Button, content: &gtk4::Box, view: &PillView) {
    while let Some(child) = content.first_child() {
        content.remove(&child);
    }
    for (i, session) in view.sessions.iter().enumerate() {
        if i > 0 {
            content.append(&dim_label("|"));
        }
        let dot = gtk4::Label::new(Some(session.activity.glyph()));
        dot.add_css_class("bar-task-dot");
        dot.add_css_class(session.activity.css_class());
        content.append(&dot);
        content.append(&desc_label(&session.desc));
        if let Some(progress) = &session.progress {
            content.append(&dim_label("·"));
            content.append(&desc_label(progress));
        }
    }
    if let Some(manual) = &view.manual {
        if !view.sessions.is_empty() {
            content.append(&dim_label("|"));
        }
        content.append(&desc_label(manual));
    }

    // No session and no manual name → no pill. An empty reserved slot read
    // as a hole in the track; battery|clock close ranks instead (the CSS
    // first/last-child radii still see the hidden widget, so the track's
    // rounded ends stay put).
    let empty = view.is_empty();
    btn.set_visible(!empty);
    if empty {
        btn.set_tooltip_text(None);
    } else if let Some(task) = view.task {
        btn.set_tooltip_text(Some(&format!("Task {task}")));
    }
}

fn desc_label(text: &str) -> gtk4::Label {
    gtk4::Label::builder()
        .label(text)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .max_width_chars(MAX_DESC_CHARS)
        .build()
}

fn dim_label(text: &str) -> gtk4::Label {
    let label = gtk4::Label::new(Some(text));
    label.add_css_class("bar-task-dim");
    label
}

// ── Pure helpers (unit-tested below) ────────────────────────────────────

/// The ":tN" infix is the one piece coupled to the workspace naming scheme
/// ("N:tXY", workspace-config.nix in the nixos repo) — the same coupling
/// the shell scripts documented.
fn task_of_name(name: &str) -> Option<u8> {
    let rest = &name[name.find(":t")? + 2..];
    let digit = rest.chars().next()?.to_digit(10)?;
    (1..=4).contains(&digit).then_some(digit as u8)
}

/// The nix wrapper renames the binary to .claude-unwrapped (truncated to
/// 15 bytes in comm); a bare `claude` covers non-wrapped installs.
fn is_claude_comm(comm: &str) -> bool {
    comm == "claude" || comm.starts_with(".claude-unwrapp")
}

/// comm in /proc/<pid>/stat may itself contain ')' — the numeric fields
/// start after the LAST one, and ppid is the second of them.
fn parent_pid_from_stat(stat: &str) -> Option<i32> {
    stat.rsplit_once(')')?
        .1
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_parses_from_the_workspace_infix() {
        assert_eq!(task_of_name("1:t1a"), Some(1));
        assert_eq!(task_of_name("5:t2a"), Some(2));
        assert_eq!(task_of_name("16:t4d"), Some(4));
    }

    #[test]
    fn non_task_workspaces_have_no_task() {
        assert_eq!(task_of_name("17:wb"), None);
        assert_eq!(task_of_name("mail"), None);
        // Digit outside the 4-task scheme.
        assert_eq!(task_of_name("40:t9a"), None);
        assert_eq!(task_of_name("trailing:t"), None);
    }

    #[test]
    fn ppid_survives_parens_in_comm() {
        assert_eq!(parent_pid_from_stat("1234 (kitty) S 42 1234 1"), Some(42));
        // comm containing the delimiter itself.
        assert_eq!(parent_pid_from_stat("999 (a) b) R 7 999 1"), Some(7));
        assert_eq!(parent_pid_from_stat("garbage"), None);
    }

    #[test]
    fn comm_gate_accepts_only_claude() {
        assert!(is_claude_comm("claude"));
        assert!(is_claude_comm(".claude-unwrapp")); // 15-byte comm truncation
        assert!(!is_claude_comm("kitty"));
        assert!(!is_claude_comm("zsh"));
    }

    #[test]
    fn unknown_activity_is_stale_not_waiting() {
        assert_eq!(Activity::parse("working"), Activity::Working);
        assert_eq!(Activity::parse("stopped"), Activity::Stopped);
        assert_eq!(Activity::parse("waiting"), Activity::Waiting);
        // Unknown must never impersonate Waiting (the cry-wolf trainer).
        assert_eq!(Activity::parse("banana"), Activity::Stale);
        assert_eq!(Activity::parse(""), Activity::Stale);
    }

    #[test]
    fn stale_renders_hollow() {
        assert_eq!(Activity::Stale.glyph(), "○");
        assert_eq!(Activity::Waiting.glyph(), "●");
        assert_eq!(Activity::Stale.css_class(), "stale");
    }

    #[test]
    fn workspace_walk_climbs_the_parent_chain() {
        let map = HashMap::from([(100, "5:t2a".to_string())]);
        let parent = |p: i32| match p {
            300 => Some(200),
            200 => Some(100),
            _ => None,
        };
        // Claude (300) → shell (200) → terminal (100) owns the view.
        assert_eq!(window_workspace(300, &map, parent), Some("5:t2a".into()));
        // Direct hit needs no walk.
        assert_eq!(window_workspace(100, &map, |_| None), Some("5:t2a".into()));
        // Chain ends without a window.
        assert_eq!(window_workspace(400, &map, |_| Some(1)), None);
        assert_eq!(window_workspace(400, &map, |_| None), None);
    }
}
