//! TaskStateService — the bar's shared Claude task-state store.
//!
//! One scan feeds every consumer (task pill today; board, decision slot
//! and ribbons per docs/BAR_VISION.md) instead of a /proc walk per
//! output. The file contract is unchanged: ~/.local/state/claude-tasks/
//! holds pid-<PID> (description), status-<PID> (working|waiting|stopped),
//! progress-<PID> and manual-t<N> rename overrides — the ~/.claude hooks
//! keep writing them. A GFileMonitor on that directory plus the
//! SwayService observer both funnel into one comparable snapshot, so
//! no-op events die here rather than in every widget. The /proc
//! parent-chain hop remains: the sway window belongs to Claude's terminal
//! ancestor, not to the Claude process that wrote the pid file.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use gio::prelude::*;

use crate::service::Observed;
use crate::sway_ipc::{SwayService, SwayState};

// ── Model ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activity {
    Working,
    Waiting,
    Stopped,
    /// Data invalid — unknown status value or missing status file. Rendered
    /// as the amber OFF-flag, never as Waiting: a channel that can't say
    /// "I don't know" can't be trusted when it says "act" (vision P9).
    Stale,
}

impl Activity {
    pub fn parse(s: &str) -> Self {
        match s {
            "working" => Self::Working,
            "waiting" => Self::Waiting,
            "stopped" => Self::Stopped,
            _ => Self::Stale,
        }
    }

    pub fn css_class(self) -> &'static str {
        match self {
            Self::Working => "working",
            Self::Waiting => "waiting",
            Self::Stopped => "stopped",
            Self::Stale => "stale",
        }
    }

    /// Stale draws hollow so "data invalid" survives grayscale and never
    /// reads as a live filled state.
    pub fn glyph(self) -> &'static str {
        match self {
            Self::Stale => "○",
            _ => "●",
        }
    }
}

/// One progress-<PID> line: raw text for prose surfaces, the leading
/// `N/M` parsed out for the board's fraction hairline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    pub raw: String,
    pub fraction: Option<(u32, u32)>,
}

impl Progress {
    fn parse(raw: String) -> Self {
        let fraction = leading_fraction(&raw);
        Self { raw, fraction }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionState {
    pub pid: i32,
    pub desc: String,
    pub activity: Activity,
    pub progress: Option<Progress>,
    /// Workspace owning the session's terminal window (focus-ack target).
    pub workspace: String,
    /// mtime of status-<PID> — waiting age; may be suspend-skewed.
    pub status_mtime: Option<SystemTime>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TaskState {
    /// Sorted by pid for stable render order.
    pub sessions: Vec<SessionState>,
    /// manual-t<N> rename override.
    pub manual: Option<String>,
}

/// Full comparable snapshot, tasks 1-4 at indices 0-3.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TaskSnapshot {
    pub tasks: [TaskState; 4],
}

impl TaskSnapshot {
    pub fn task(&self, task: u8) -> &TaskState {
        &self.tasks[task as usize - 1]
    }
}

// ── Service ─────────────────────────────────────────────────────────────

/// [`Observed`] documents the `Rc` lifetime story; here the strong ref
/// registered on SwayService (which lives for the process) is what keeps
/// the service alive.
pub struct TaskStateService {
    state: Observed<TaskSnapshot>,
    sway: Rc<SwayService>,
    dir: PathBuf,
    /// Armed only while some session is Waiting (vision P7); one-shot,
    /// aimed at the next whole-minute age boundary (clock.rs pattern).
    age_timer: RefCell<Option<glib::SourceId>>,
    /// A dropped GFileMonitor stops watching.
    _monitor: RefCell<Option<gio::FileMonitor>>,
}

impl TaskStateService {
    pub fn start(sway: &Rc<SwayService>) -> Rc<Self> {
        let service = Rc::new(Self {
            state: Observed::new(TaskSnapshot::default()),
            sway: sway.clone(),
            dir: state_dir(),
            age_timer: RefCell::new(None),
            _monitor: RefCell::new(None),
        });

        // The watcher needs the directory before the first session writes it.
        if let Err(e) = fs::create_dir_all(&service.dir) {
            log::warn!("task state: create {}: {e}", service.dir.display());
        }
        match gio::File::for_path(&service.dir)
            .monitor_directory(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
        {
            Ok(monitor) => {
                let weak = Rc::downgrade(&service);
                monitor.connect_changed(move |_, _, _, _| {
                    if let Some(service) = weak.upgrade() {
                        service.refresh(false);
                    }
                });
                *service._monitor.borrow_mut() = Some(monitor);
            }
            Err(e) => log::warn!("task state: watch {}: {e}", service.dir.display()),
        }

        let for_sway = service.clone();
        sway.connect_change(move || for_sway.refresh(false));

        service.refresh(false);
        service
    }

    pub fn connect_change(&self, cb: impl Fn() + 'static) {
        self.state.connect_change(cb);
    }

    /// Full snapshot (cloned — a handful of small rows).
    pub fn snapshot(&self) -> TaskSnapshot {
        self.state.with(Clone::clone)
    }

    /// Rescan into a snapshot. `notify_unchanged` forces the observer fire
    /// for age-dependent renders: the snapshot carries no clock, so an age
    /// tick compares equal.
    fn refresh(self: &Rc<Self>, notify_unchanged: bool) {
        let snapshot = scan(&self.dir, &self.sway.snapshot());
        if notify_unchanged {
            self.state.set(snapshot);
        } else {
            self.state.set_if_changed(snapshot);
        }
        self.reconcile_age_timer();
    }

    fn reconcile_age_timer(self: &Rc<Self>) {
        if let Some(id) = self.age_timer.borrow_mut().take() {
            id.remove();
        }
        let now = SystemTime::now();
        let Some(delay) = self.state.with(|s| next_age_delay(s, now)) else {
            return;
        };
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(delay, move || {
            let Some(service) = weak.upgrade() else {
                return;
            };
            // The fired source id is dead; drop it so reconcile can't
            // remove() a stale id.
            service.age_timer.borrow_mut().take();
            service.refresh(true);
        });
        *self.age_timer.borrow_mut() = Some(id);
    }
}

// ── State assembly ──────────────────────────────────────────────────────

fn state_dir() -> PathBuf {
    glib::home_dir().join(".local/state/claude-tasks")
}

fn scan(dir: &Path, sway: &SwayState) -> TaskSnapshot {
    scan_with(dir, sway, proc_comm, parent_pid)
}

/// comm/parent injected so tests can fake /proc.
fn scan_with(
    dir: &Path,
    sway: &SwayState,
    comm: impl Fn(i32) -> Option<String>,
    parent: impl Fn(i32) -> Option<i32> + Copy,
) -> TaskSnapshot {
    let mut snapshot = TaskSnapshot::default();
    for pid in claude_pids(dir) {
        let Some(desc) = first_line(&dir.join(format!("pid-{pid}"))) else {
            continue;
        };
        // comm gate: a recycled PID must not resurrect a dead session's
        // description file.
        if !comm(pid).is_some_and(|c| is_claude_comm(&c)) {
            continue;
        }
        let Some(workspace) = window_workspace(pid, &sway.pid_workspaces, parent) else {
            continue;
        };
        let Some(task) = task_of_name(&workspace) else {
            continue;
        };
        let status = dir.join(format!("status-{pid}"));
        snapshot.tasks[task as usize - 1]
            .sessions
            .push(SessionState {
                pid,
                desc,
                activity: first_line(&status).map_or(Activity::Stale, |s| Activity::parse(&s)),
                progress: first_line(&dir.join(format!("progress-{pid}"))).map(Progress::parse),
                workspace,
                status_mtime: fs::metadata(&status).and_then(|m| m.modified()).ok(),
            });
    }
    for (i, task) in snapshot.tasks.iter_mut().enumerate() {
        task.manual = first_line(&dir.join(format!("manual-t{}", i + 1)));
    }
    snapshot
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

// ── Pure helpers (unit-tested below) ────────────────────────────────────

/// The ":tN" infix is the one piece coupled to the workspace naming scheme
/// ("N:tXY", workspace-config.nix in the nixos repo) — the same coupling
/// the shell scripts documented.
pub fn task_of_name(name: &str) -> Option<u8> {
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

/// Leading `N/M` token of a progress line ("1/5 ETA ~15m"); a zero total
/// is not a fraction.
fn leading_fraction(s: &str) -> Option<(u32, u32)> {
    let (n, m) = s.split_whitespace().next()?.split_once('/')?;
    let (n, m) = (n.parse().ok()?, m.parse().ok()?);
    (m > 0).then_some((n, m))
}

/// Delay until the next waiting session crosses a whole-minute age
/// boundary; `None` when nothing waits, so the timer stands down. +50 ms
/// lands safely past the boundary (clock.rs). A missing or future mtime
/// (suspend skew) counts as age zero.
fn next_age_delay(snapshot: &TaskSnapshot, now: SystemTime) -> Option<Duration> {
    snapshot
        .tasks
        .iter()
        .flat_map(|t| &t.sessions)
        .filter(|s| s.activity == Activity::Waiting)
        .map(|s| {
            let elapsed = s
                .status_mtime
                .and_then(|m| now.duration_since(m).ok())
                .unwrap_or(Duration::ZERO);
            Duration::from_secs(60 - elapsed.as_secs() % 60)
        })
        .min()
        .map(|d| d + Duration::from_millis(50))
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

    #[test]
    fn fraction_parses_from_the_progress_prefix() {
        assert_eq!(leading_fraction("1/5 ETA ~15m"), Some((1, 5)));
        assert_eq!(leading_fraction("3/7"), Some((3, 7)));
        assert_eq!(leading_fraction("ETA ~15m"), None);
        assert_eq!(leading_fraction("almost/done"), None);
        // Zero total is not a fraction.
        assert_eq!(leading_fraction("0/0"), None);
    }

    fn waiting_at(mtime: Option<SystemTime>) -> SessionState {
        SessionState {
            pid: 1,
            desc: "d".into(),
            activity: Activity::Waiting,
            progress: None,
            workspace: "5:t2a".into(),
            status_mtime: mtime,
        }
    }

    fn snapshot_of(sessions: Vec<SessionState>) -> TaskSnapshot {
        let mut snap = TaskSnapshot::default();
        snap.tasks[0].sessions = sessions;
        snap
    }

    #[test]
    fn age_timer_aims_at_the_next_minute_boundary() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        // Waiting 90 s → next boundary in 30 s (+pad).
        let snap = snapshot_of(vec![waiting_at(Some(now - Duration::from_secs(90)))]);
        assert_eq!(
            next_age_delay(&snap, now),
            Some(Duration::from_millis(30_050))
        );
        // Several waiting: the soonest boundary wins.
        let snap = snapshot_of(vec![
            waiting_at(Some(now - Duration::from_secs(90))),
            waiting_at(Some(now - Duration::from_secs(55))),
        ]);
        assert_eq!(
            next_age_delay(&snap, now),
            Some(Duration::from_millis(5_050))
        );
        // Missing mtime → full minute.
        let snap = snapshot_of(vec![waiting_at(None)]);
        assert_eq!(
            next_age_delay(&snap, now),
            Some(Duration::from_millis(60_050))
        );
    }

    #[test]
    fn age_timer_stands_down_without_a_waiting_session() {
        let now = SystemTime::now();
        assert_eq!(next_age_delay(&TaskSnapshot::default(), now), None);
        let mut stopped = waiting_at(Some(now));
        stopped.activity = Activity::Stopped;
        assert_eq!(next_age_delay(&snapshot_of(vec![stopped]), now), None);
    }

    #[test]
    fn scan_assembles_sessions_per_task() {
        let dir = std::env::temp_dir().join(format!("swaypplet-task-state-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("pid-300"), "fix flaky auth retry\n").unwrap();
        fs::write(dir.join("status-300"), "waiting\n").unwrap();
        fs::write(dir.join("progress-300"), "1/5 ETA ~15m\n").unwrap();
        // comm gate rejects this one: recycled pid.
        fs::write(dir.join("pid-666"), "ghost\n").unwrap();
        fs::write(dir.join("manual-t3"), "manual name\n").unwrap();

        let sway = SwayState {
            workspaces: Vec::new(),
            pid_workspaces: HashMap::from([(100, "5:t2a".to_string())]),
        };
        let comm = |pid: i32| (pid == 300).then(|| "claude".to_string());
        let parent = |p: i32| match p {
            300 => Some(200),
            200 => Some(100),
            _ => None,
        };
        let snap = scan_with(&dir, &sway, comm, parent);
        fs::remove_dir_all(&dir).unwrap();

        let session = &snap.task(2).sessions[0];
        assert_eq!(session.pid, 300);
        assert_eq!(session.desc, "fix flaky auth retry");
        assert_eq!(session.activity, Activity::Waiting);
        assert_eq!(
            session.progress,
            Some(Progress {
                raw: "1/5 ETA ~15m".into(),
                fraction: Some((1, 5)),
            })
        );
        assert_eq!(session.workspace, "5:t2a");
        assert!(session.status_mtime.is_some());
        assert_eq!(snap.task(2).sessions.len(), 1);
        assert_eq!(snap.task(3).manual.as_deref(), Some("manual name"));
        assert!(snap.task(1).sessions.is_empty());
        assert!(snap.task(3).sessions.is_empty());
    }
}
