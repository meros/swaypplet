//! BackupStatusService — what the nightly restic jobs left behind.
//!
//! The units write one JSON file per job into `/var/lib/backup-status`
//! (nixos `modules/nixos/services/backup.nix`): the state, the result, when
//! the run started and ended, the snapshot's short id and size, and the two
//! numbers that need the server — how big the repository is and what is free
//! beside it. Reading those files rather than asking systemd and restic
//! ourselves means this process needs no repository password, no network and
//! no journal parsing, and it keeps saying something true while
//! meros-server is unreachable.
//!
//! A GFileMonitor on the directory covers every transition a run makes: the
//! `running` record at ExecStartPre, the result at ExecStopPost. Staleness is
//! the one condition no file event announces, so a ten-minute tick
//! re-evaluates the tier. The threshold is in days, so that tick polls
//! nothing — it is the clock catching up with a deadline.

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use gio::prelude::*;
use serde::Deserialize;

use crate::service::Observed;

const DIR: &str = "/var/lib/backup-status";
/// Read order, which is also display order: the big one first.
const JOBS: [&str; 2] = ["home", "locked"];
/// The jobs run nightly. One missed night is noise (a laptop that stayed
/// shut), two is a fact worth a glyph.
const STALE_AFTER_S: u64 = 48 * 3600;
/// Staleness has no file event. The threshold is in days, so this costs one
/// comparison per tick and never touches the disk.
const TICK_S: u32 = 600;

// ── Model ───────────────────────────────────────────────────────────────

/// One job's last word about itself. Every field is optional on the wire:
/// a file written by an older unit, or half-written, must degrade to
/// [`Tier::Unknown`] rather than panic the bar.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Job {
    #[serde(default)]
    pub job: String,
    /// `running` between ExecStartPre and ExecStopPost, `idle` otherwise.
    #[serde(default)]
    pub state: String,
    /// systemd's `$SERVICE_RESULT`: `success` or the reason it was not.
    #[serde(default)]
    pub result: String,
    #[serde(default)]
    pub started: u64,
    #[serde(default)]
    pub ended: u64,
    #[serde(default)]
    pub snapshot: String,
    #[serde(default)]
    pub snapshot_bytes: u64,
    #[serde(default)]
    pub repo_bytes: u64,
    #[serde(default)]
    pub target_avail_bytes: u64,
}

impl Job {
    pub fn running(&self) -> bool {
        self.state == "running"
    }

    fn succeeded(&self) -> bool {
        self.result == "success"
    }

    fn age_s(&self, now: u64) -> u64 {
        now.saturating_sub(self.ended)
    }
}

/// How loud the segment is. Ordered by precedence: a failure outranks
/// staleness, which outranks a run in progress. `Unknown` is its own tier
/// rather than a quiet `Ok`, for the reason the task pill states: a channel
/// that cannot say "I don't know" cannot be trusted when it says "fine".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Ok,
    Running,
    Stale,
    Failed,
    Unknown,
}

impl Tier {
    /// The CSS class the bar segment and the Helm header both key off.
    pub fn css(self) -> &'static str {
        match self {
            Tier::Ok => "backup-ok",
            Tier::Running => "backup-running",
            Tier::Stale | Tier::Failed => "backup-warn",
            Tier::Unknown => "backup-unknown",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Tier::Running => "󰁯",
            Tier::Failed => "󰅚",
            Tier::Stale => "󰀦",
            Tier::Ok => "󰄬",
            Tier::Unknown => "󰋗",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub jobs: Vec<Job>,
}

impl Snapshot {
    fn read() -> Self {
        let dir = PathBuf::from(DIR);
        let jobs = JOBS
            .iter()
            .filter_map(|name| read_job(&dir, name))
            .collect();
        Self { jobs }
    }

    pub fn tier(&self) -> Tier {
        if self.jobs.is_empty() {
            return Tier::Unknown;
        }
        let now = now_s();
        if self.jobs.iter().any(|j| !j.running() && !j.succeeded()) {
            return Tier::Failed;
        }
        if self
            .jobs
            .iter()
            .any(|j| !j.running() && j.age_s(now) > STALE_AFTER_S)
        {
            return Tier::Stale;
        }
        if self.jobs.iter().any(Job::running) {
            return Tier::Running;
        }
        Tier::Ok
    }

    /// The whole status in the tooltip, because the glyph provokes exactly
    /// one question and answering it should not need a panel.
    pub fn tooltip(&self) -> String {
        if self.jobs.is_empty() {
            return "Backup: no run recorded yet".to_string();
        }
        let now = now_s();
        let mut lines = vec!["Backup".to_string()];
        for job in &self.jobs {
            lines.push(format!("{}  {}", job.job, describe(job, now)));
        }
        if let Some(target) = self
            .jobs
            .iter()
            .max_by_key(|j| j.ended)
            .filter(|j| j.repo_bytes > 0)
        {
            lines.push(format!(
                "meros-server  {} used, {} free",
                bytes(target.repo_bytes),
                bytes(target.target_avail_bytes)
            ));
        }
        lines.join("\n")
    }
}

/// One job as a line: what happened, when, and how much of it.
pub fn describe(job: &Job, now: u64) -> String {
    if job.running() {
        return format!("running, started {}", ago(now.saturating_sub(job.started)));
    }
    if !job.succeeded() {
        return format!("failed {}", ago(job.age_s(now)));
    }
    let size = if job.snapshot_bytes > 0 {
        format!(", {}", bytes(job.snapshot_bytes))
    } else {
        String::new()
    };
    let id = if job.snapshot.is_empty() {
        String::new()
    } else {
        format!(" ({})", job.snapshot)
    };
    format!("{}{}{}", ago(job.age_s(now)), size, id)
}

fn read_job(dir: &Path, name: &str) -> Option<Job> {
    let text = fs::read_to_string(dir.join(format!("{name}.json"))).ok()?;
    let mut job: Job = serde_json::from_str(&text).ok()?;
    if job.job.is_empty() {
        job.job = name.to_string();
    }
    Some(job)
}

fn now_s() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Coarse on purpose: nobody acts on the difference between 61 and 62
/// minutes, and a bar that rewrites itself every second is a moving target.
pub fn ago(secs: u64) -> String {
    match secs {
        0..=89 => "just now".to_string(),
        90..=5399 => format!("{} min ago", (secs + 30) / 60),
        5400..=169_199 => format!("{} h ago", (secs + 1800) / 3600),
        _ => format!("{} days ago", (secs + 43200) / 86400),
    }
}

pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

// ── Service ─────────────────────────────────────────────────────────────

pub struct BackupStatusService {
    state: Observed<Snapshot>,
    /// A dropped GFileMonitor stops watching.
    _monitor: RefCell<Option<gio::FileMonitor>>,
}

impl BackupStatusService {
    pub fn start() -> Rc<Self> {
        let service = Rc::new(Self {
            state: Observed::new(Snapshot::read()),
            _monitor: RefCell::new(None),
        });

        // The directory appears when the first run does. Watching a missing
        // path is not an error worth logging on a machine with no backups
        // configured, so a failure here just leaves the segment Unknown.
        match gio::File::for_path(DIR)
            .monitor_directory(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
        {
            Ok(monitor) => {
                let weak = Rc::downgrade(&service);
                monitor.connect_changed(move |_, _, _, _| {
                    if let Some(service) = weak.upgrade() {
                        service.refresh();
                    }
                });
                *service._monitor.borrow_mut() = Some(monitor);
            }
            Err(e) => log::debug!("backup status: watch {DIR}: {e}"),
        }

        let weak = Rc::downgrade(&service);
        glib::timeout_add_seconds_local(TICK_S, move || match weak.upgrade() {
            Some(service) => {
                service.refresh();
                glib::ControlFlow::Continue
            }
            None => glib::ControlFlow::Break,
        });

        service
    }

    pub fn connect_change(&self, cb: impl Fn() + 'static) {
        self.state.connect_change(cb);
    }

    pub fn snapshot(&self) -> Snapshot {
        self.state.with(Clone::clone)
    }

    fn refresh(&self) {
        self.state.set(Snapshot::read());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(state: &str, result: &str, ended: u64) -> Job {
        Job {
            job: "home".into(),
            state: state.into(),
            result: result.into(),
            ended,
            ..Job::default()
        }
    }

    #[test]
    fn no_files_is_unknown_not_ok() {
        assert_eq!(Snapshot::default().tier(), Tier::Unknown);
    }

    #[test]
    fn failure_outranks_a_running_sibling() {
        let snap = Snapshot {
            jobs: vec![job("idle", "exit-code", now_s()), job("running", "", 0)],
        };
        assert_eq!(snap.tier(), Tier::Failed);
    }

    #[test]
    fn two_missed_nights_are_stale() {
        let snap = Snapshot {
            jobs: vec![job("idle", "success", now_s() - STALE_AFTER_S - 60)],
        };
        assert_eq!(snap.tier(), Tier::Stale);
    }

    #[test]
    fn a_fresh_success_is_quiet() {
        let snap = Snapshot {
            jobs: vec![job("idle", "success", now_s() - 300)],
        };
        assert_eq!(snap.tier(), Tier::Ok);
    }

    /// The contract with `modules/nixos/services/backup.nix`, verbatim from
    /// a real run. A field renamed on either side breaks here rather than in
    /// a bar that silently reads Unknown forever.
    #[test]
    fn the_units_own_output_parses() {
        let written = r#"{
          "job": "home",
          "state": "idle",
          "result": "success",
          "started": 1789586889,
          "ended": 1789586909,
          "snapshot": "796a8304",
          "snapshot_bytes": 67819475074,
          "repo_bytes": 32679960054,
          "target_avail_bytes": 1301167185920
        }"#;
        let job: Job = serde_json::from_str(written).expect("the unit's own JSON");
        assert_eq!(job.job, "home");
        assert!(!job.running());
        assert!(job.succeeded());
        assert_eq!(job.snapshot, "796a8304");
        assert_eq!(bytes(job.snapshot_bytes), "63.2 GiB");
        let snap = Snapshot { jobs: vec![job] };
        assert!(snap.tooltip().contains("meros-server"));
    }

    #[test]
    fn ago_rounds_to_something_a_human_would_say() {
        assert_eq!(ago(30), "just now");
        assert_eq!(ago(720), "12 min ago");
        assert_eq!(ago(7200), "2 h ago");
        assert_eq!(ago(3 * 86400), "3 days ago");
    }
}
