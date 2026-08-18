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
use std::time::{Duration, Instant, SystemTime};

use gio::prelude::*;

use crate::service::Observed;
use crate::sway_ipc::{SwayService, SwayState};

// ── Model ───────────────────────────────────────────────────────────────

/// What a session is doing. These mirror the states Claude Code itself
/// distinguishes rather than inventing a taxonomy beside them:
/// `UserPromptSubmit`/`PreToolUse`/`PostToolUse` mean working, the
/// `Notification` hook's `permission_prompt` family means blocked, `Stop`
/// and `idle_prompt` mean waiting, `SessionEnd` means stopped. Claude
/// Code's own agent view draws the same line between "needs input" and
/// "finished its turn", and every CI system separates "waiting for
/// approval" from "succeeded" for the same reason: one halts progress,
/// the other is progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activity {
    Working,
    /// Halted mid-turn on a permission or elicitation prompt. Nothing
    /// proceeds until the owner answers, so this outranks Waiting in
    /// every mux. Self-clearing: approving produces the next tool call,
    /// whose hook writes working.
    Blocked,
    /// Turn finished; the owner's move, at their pace.
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
            "blocked" => Self::Blocked,
            "waiting" => Self::Waiting,
            "stopped" => Self::Stopped,
            _ => Self::Stale,
        }
    }

    /// Both states that want the owner. They differ in urgency, not in
    /// kind, so callers that only ask "does this need me" use this.
    pub fn wants_owner(self) -> bool {
        matches!(self, Self::Blocked | Self::Waiting)
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
    /// Focus-as-acknowledgment (vision P10): true once this waiting
    /// episode's workspace (or task) has been focused. Meaningful only
    /// while `activity` is Waiting; ack drops luminance, the state stays
    /// "waiting" until the hook writes working/stopped.
    pub acked: bool,
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
    /// Acked waiting episodes, pid → status mtime at ack. Sticky for the
    /// episode: focus leaving does not un-acknowledge; a status rewrite
    /// (new mtime) starts a fresh, unacked episode.
    acked: RefCell<HashMap<i32, Option<SystemTime>>>,
    /// A dropped GFileMonitor stops watching.
    _monitor: RefCell<Option<gio::FileMonitor>>,
    /// Suspend detection for honest ages (the popover's ~ prefix): the
    /// wall and monotonic clocks last sampled together, plus the wall
    /// time of the most recent detected jump.
    skew: RefCell<SkewTracker>,
}

struct SkewTracker {
    wall: SystemTime,
    mono: Instant,
    boundary: Option<SystemTime>,
}

impl TaskStateService {
    pub fn start(sway: &Rc<SwayService>) -> Rc<Self> {
        let service = Rc::new(Self {
            state: Observed::new(TaskSnapshot::default()),
            sway: sway.clone(),
            dir: state_dir(),
            age_timer: RefCell::new(None),
            acked: RefCell::new(HashMap::new()),
            _monitor: RefCell::new(None),
            skew: RefCell::new(SkewTracker {
                wall: SystemTime::now(),
                mono: Instant::now(),
                boundary: None,
            }),
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

    /// Wall time of the most recent detected suspend; ages of status
    /// writes older than this straddle slept hours and are only
    /// approximate. CLOCK_MONOTONIC stops during suspend, so a wall delta
    /// that outruns the monotonic delta between two samples marks "a
    /// suspend ended in this window". Sampled on every refresh and on
    /// popover open — no timer of its own (cadence budget).
    pub fn skew_boundary(&self) -> Option<SystemTime> {
        let mut t = self.skew.borrow_mut();
        let (now_wall, now_mono) = (SystemTime::now(), Instant::now());
        let wall_delta = now_wall.duration_since(t.wall).unwrap_or(Duration::ZERO);
        if clock_jumped(wall_delta, now_mono - t.mono) {
            t.boundary = Some(now_wall);
        }
        t.wall = now_wall;
        t.mono = now_mono;
        t.boundary
    }

    /// Rescan into a snapshot. `notify_unchanged` forces the observer fire
    /// for age-dependent renders: the snapshot carries no clock, so an age
    /// tick compares equal.
    fn refresh(self: &Rc<Self>, notify_unchanged: bool) {
        // Sample the clocks while we're here: the denser the sampling,
        // the tighter the suspend boundary the popover flags against.
        self.skew_boundary();
        let sway = self.sway.snapshot();
        let mut snapshot = scan(&self.dir, &sway);
        let focused: Vec<&str> = sway
            .workspaces
            .iter()
            .filter(|w| w.focused)
            .map(|w| w.name.as_str())
            .collect();
        reconcile_acks(&mut snapshot, &mut self.acked.borrow_mut(), &focused);
        if notify_unchanged {
            self.state.set(snapshot);
        } else {
            self.state.set_if_changed(snapshot);
        }
        self.reconcile_age_timer();
    }

    fn reconcile_age_timer(self: &Rc<Self>) {
        if let Some(id) = self.age_timer.borrow_mut().take() {
            crate::spawn::remove_source(id);
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

/// Also the popover's root for last-<pid> rows (bar/popover.rs).
pub(crate) fn state_dir() -> PathBuf {
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
                acked: false,
            });
    }
    for (i, task) in snapshot.tasks.iter_mut().enumerate() {
        task.manual = first_line(&dir.join(format!("manual-t{}", i + 1)));
    }
    snapshot
}

/// Focus is acknowledgment (vision P10, the input the owner already
/// produces hundreds of times a day): a waiting session whose workspace —
/// or any workspace of its task, the stated fallback — is focused becomes
/// acked, and stays acked for that waiting episode (keyed by status
/// mtime) after focus moves on. Any output's focus acks everywhere: the
/// episode map is service-global, so every board renders the same drop.
fn reconcile_acks(
    snapshot: &mut TaskSnapshot,
    acked: &mut HashMap<i32, Option<SystemTime>>,
    focused: &[&str],
) {
    let focused_tasks: Vec<u8> = focused.iter().filter_map(|f| task_of_name(f)).collect();
    for (i, task) in snapshot.tasks.iter_mut().enumerate() {
        let n = i as u8 + 1;
        for s in &mut task.sessions {
            if s.activity != Activity::Waiting {
                acked.remove(&s.pid);
                continue;
            }
            let same_episode = acked.get(&s.pid) == Some(&s.status_mtime);
            let focused_now = focused.contains(&s.workspace.as_str()) || focused_tasks.contains(&n);
            s.acked = same_episode || focused_now;
            if s.acked {
                acked.insert(s.pid, s.status_mtime);
            } else {
                // A rewritten status (new mtime) is a fresh episode.
                acked.remove(&s.pid);
            }
        }
    }
    let live: std::collections::HashSet<i32> = snapshot
        .tasks
        .iter()
        .flat_map(|t| &t.sessions)
        .map(|s| s.pid)
        .collect();
    acked.retain(|pid, _| live.contains(pid));
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
pub(crate) fn first_line(path: &Path) -> Option<String> {
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

/// pid → owning workspace through the live /proc parent chain — the
/// entry point for callers outside the scan (stop-notification
/// attribution, vision O2).
pub fn workspace_of_pid(pid: i32, pid_workspaces: &HashMap<i32, String>) -> Option<String> {
    window_workspace(pid, pid_workspaces, parent_pid)
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

/// Wall-clock start of `pid`, or `None` when it cannot be determined.
///
/// A PID is a name the kernel hands back out; the start time is what makes
/// it an identity. The session scan already gates on comm, so a recycled
/// PID cannot resurrect a dead session — but the per-PID files a session
/// leaves behind (`last-<PID>`, and nothing sweeps them) outlive it, and a
/// new session that inherits the number would serve them as its own until
/// it wrote its first. Comparing a file's mtime against this tells the two
/// apart.
pub(crate) fn proc_start_time(pid: i32) -> Option<SystemTime> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    start_time_from(start_ticks_from_stat(&stat)?, boot_time()?, clock_ticks()?)
}

/// Field 22 of /proc/<pid>/stat: process start, in clock ticks since boot.
/// Same last-')' rule as ppid above; field 22 is the twentieth after it.
fn start_ticks_from_stat(stat: &str) -> Option<u64> {
    stat.rsplit_once(')')?
        .1
        .split_whitespace()
        .nth(19)?
        .parse()
        .ok()
}

/// Boot time from /proc/stat's `btime` line, in seconds since the epoch.
/// Read once: the kernel's answer does not change, and a suspend does not
/// move it (unlike every wall-clock reading derived from uptime).
fn boot_time() -> Option<SystemTime> {
    static BOOT: std::sync::OnceLock<Option<SystemTime>> = std::sync::OnceLock::new();
    *BOOT.get_or_init(|| {
        let stat = fs::read_to_string("/proc/stat").ok()?;
        let secs: u64 = stat
            .lines()
            .find_map(|l| l.strip_prefix("btime "))?
            .trim()
            .parse()
            .ok()?;
        Some(SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
    })
}

/// `sysconf(_SC_CLK_TCK)` — the unit field 22 is counted in. 100 on every
/// mainstream kernel, asked for rather than assumed.
fn clock_ticks() -> Option<u64> {
    let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    (hz > 0).then(|| hz as u64)
}

/// Split out from [`proc_start_time`] so the arithmetic is testable
/// without a process to point it at.
fn start_time_from(ticks: u64, boot: SystemTime, hz: u64) -> Option<SystemTime> {
    boot.checked_add(Duration::from_nanos(ticks.checked_mul(1_000_000_000)? / hz))
}

/// A minute of tolerance: timer latency and scheduling never add up to
/// that between two samples; a real suspend does.
fn clock_jumped(wall_delta: Duration, mono_delta: Duration) -> bool {
    wall_delta > mono_delta + Duration::from_secs(60)
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
        assert_eq!(task_of_name("19:wb"), None);
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
    fn start_ticks_reads_field_22() {
        // 1234 (comm) then fields 3..: S ppid pgrp sid tty tpgid flags
        // minflt cminflt majflt cmajflt utime stime cutime cstime prio
        // nice threads itrealvalue starttime
        let stat = "1234 (claude) S 42 1234 1234 0 -1 4194304 100 0 0 0                     5 6 0 0 20 0 12 0 648773 rest ignored";
        assert_eq!(start_ticks_from_stat(stat), Some(648_773));
        // comm containing the delimiter, same rule as ppid.
        let odd = "9 (a) b) S 7 9 9 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 999 x";
        assert_eq!(start_ticks_from_stat(odd), Some(999));
        assert_eq!(start_ticks_from_stat("garbage"), None);
    }

    /// The field index is the fragile part, and a fake /proc cannot catch
    /// it drifting. Our own process is the one whose start time is known
    /// to be recent, so read it back and check it lands in a window no
    /// wrong field could.
    #[test]
    fn start_time_of_this_process_is_recent() {
        let start = proc_start_time(std::process::id() as i32).expect("own /proc/<pid>/stat");
        let age = SystemTime::now()
            .duration_since(start)
            .expect("this process started in the past");
        assert!(age < Duration::from_secs(3600), "start time {age:?} old");
    }

    #[test]
    fn start_time_is_boot_plus_ticks() {
        let boot = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        // 250 ticks at 100 Hz is 2.5 s after boot.
        assert_eq!(
            start_time_from(250, boot, 100),
            Some(boot + Duration::from_millis(2_500))
        );
        // Sub-tick resolution survives: 1 tick at 100 Hz is 10 ms, not 0 s.
        assert_eq!(
            start_time_from(1, boot, 100),
            Some(boot + Duration::from_millis(10))
        );
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
    fn clock_jump_needs_a_minute_of_drift() {
        let s = Duration::from_secs;
        assert!(!clock_jumped(s(30), s(30)));
        // Half a minute of drift is scheduling noise, not a suspend.
        assert!(!clock_jumped(s(90), s(60)));
        // An hour asleep between samples.
        assert!(clock_jumped(s(3700), s(30)));
        // Wall behind monotonic (NTP step back) is not a suspend.
        assert!(!clock_jumped(s(10), s(60)));
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
            acked: false,
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
            pid_workspaces: HashMap::from([(100, "5:t2a".to_string())]),
            ..SwayState::default()
        };
        let comm = |pid: i32| (pid == 300).then(|| "claude".to_string());
        let parent = |p: i32| match p {
            300 => Some(200),
            200 => Some(100),
            _ => None,
        };
        let snap = scan_with(&dir, &sway, comm, parent);
        fs::remove_dir_all(&dir).unwrap();
        assert!(!snap.task(2).sessions[0].acked);

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

    #[test]
    fn focusing_the_workspace_acks_and_the_ack_sticks() {
        let mtime = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1000));
        let mut acked = HashMap::new();

        let mut snap = snapshot_of(vec![waiting_at(mtime)]);
        reconcile_acks(&mut snap, &mut acked, &["19:wb"]);
        assert!(!snap.task(1).sessions[0].acked);

        // Session lives on 5:t2a (task 2, but stored under task 1 by the
        // fixture — reconcile keys focus on the session's own fields).
        reconcile_acks(&mut snap, &mut acked, &["5:t2a"]);
        assert!(snap.task(1).sessions[0].acked);

        // Focus moves away: the episode stays acked.
        reconcile_acks(&mut snap, &mut acked, &["19:wb"]);
        assert!(snap.task(1).sessions[0].acked);

        // Status rewritten (new mtime) → fresh episode, unacked again.
        let mut snap = snapshot_of(vec![waiting_at(Some(
            SystemTime::UNIX_EPOCH + Duration::from_secs(2000),
        ))]);
        reconcile_acks(&mut snap, &mut acked, &["19:wb"]);
        assert!(!snap.task(1).sessions[0].acked);
    }

    #[test]
    fn focusing_any_workspace_of_the_task_acks_as_fallback() {
        // waiting_at sits on 5:t2a; snapshot_of stores it under task 1, so
        // build a task-2 snapshot to exercise the task-level fallback.
        let mut snap = TaskSnapshot::default();
        snap.tasks[1].sessions = vec![waiting_at(None)];
        let mut acked = HashMap::new();
        // A different task-2 workspace than the session's own.
        reconcile_acks(&mut snap, &mut acked, &["6:t2b"]);
        assert!(snap.task(2).sessions[0].acked);
    }

    #[test]
    fn leaving_waiting_forgets_the_episode() {
        let mtime = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1000));
        let mut acked = HashMap::new();
        let mut snap = snapshot_of(vec![waiting_at(mtime)]);
        reconcile_acks(&mut snap, &mut acked, &["5:t2a"]);
        assert!(snap.task(1).sessions[0].acked);

        // Back to working: the map entry dies with the wait...
        let mut working = waiting_at(mtime);
        working.activity = Activity::Working;
        let mut snap = snapshot_of(vec![working]);
        reconcile_acks(&mut snap, &mut acked, &[]);
        assert!(acked.is_empty());

        // ...so waiting again (even with the same mtime) starts unacked.
        let mut snap = snapshot_of(vec![waiting_at(mtime)]);
        reconcile_acks(&mut snap, &mut acked, &[]);
        assert!(!snap.task(1).sessions[0].acked);

        // A vanished session is pruned from the map entirely.
        let mut snap = snapshot_of(vec![waiting_at(mtime)]);
        reconcile_acks(&mut snap, &mut acked, &["5:t2a"]);
        assert!(!acked.is_empty());
        reconcile_acks(&mut TaskSnapshot::default(), &mut acked, &[]);
        assert!(acked.is_empty());
    }
}
