//! Claude Code sessions as status items.
//!
//! The file contract is the one TaskStateService reads, written by the
//! hooks (~/.claude/hooks/claude-task.sh), one set of files per Claude PID:
//! pid-<PID> (the task label), status-<PID>, progress-<PID>, asks-<PID>
//! (the last turn ended with a question) and transcript-<PID>.
//!
//! What a session means for the owner, the same rule as the lid dot
//! (nixos users/modules/lid-dot.py):
//!
//!   Attention  blocked on a prompt, or waiting after a turn that asked
//!              the owner something
//!   Active     working
//!   Info       waiting, or a status the hooks did not write (stale)
//!
//! A stopped session is no item. Neither is a PID whose process is gone or
//! is no longer Claude.

use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use super::{Severity, StatusItem, StatusSource};
use crate::services::task_state::{
    Activity, claude_pids, first_line, is_claude_comm, proc_comm, state_dir,
};

pub struct ClaudeSource {
    dir: PathBuf,
}

impl ClaudeSource {
    pub fn new() -> Self {
        Self { dir: state_dir() }
    }
}

impl StatusSource for ClaudeSource {
    fn watch(&self) -> Vec<PathBuf> {
        vec![self.dir.clone()]
    }

    fn scan(&self) -> Vec<StatusItem> {
        scan_with(&self.dir, proc_comm)
    }
}

/// comm injected so tests can fake /proc.
fn scan_with(dir: &Path, comm: impl Fn(i32) -> Option<String>) -> Vec<StatusItem> {
    let mut items = Vec::new();
    for pid in claude_pids(dir) {
        let Some(title) = first_line(&dir.join(format!("pid-{pid}"))) else {
            continue;
        };
        // A recycled PID must not resurrect a dead session's files.
        if !comm(pid).is_some_and(|c| is_claude_comm(&c)) {
            continue;
        }
        let status = dir.join(format!("status-{pid}"));
        let activity = settle(
            first_line(&status).map_or(Activity::Stale, |s| Activity::parse(&s)),
            dir,
            pid,
        );
        let asks = dir.join(format!("asks-{pid}")).exists();
        let severity = match activity {
            Activity::Blocked => Severity::Attention,
            Activity::Waiting if asks => Severity::Attention,
            Activity::Working => Severity::Active,
            Activity::Waiting | Activity::Stale => Severity::Info,
            Activity::Stopped => continue,
        };
        // A progress line outlives the work it described; only a working
        // session's is current.
        let detail = (activity == Activity::Working)
            .then(|| first_line(&dir.join(format!("progress-{pid}"))))
            .flatten();
        items.push(StatusItem {
            source: "claude",
            id: pid.to_string(),
            severity,
            title,
            detail,
            since: fs::metadata(&status).and_then(|m| m.modified()).ok(),
        });
    }
    items
}

const INTERRUPT: &str = "[Request interrupted by user";

/// The activity the owner sees, with the one case the hooks cannot report:
/// no hook fires when a turn is interrupted (Esc), so a session stopped at
/// a permission prompt stays `blocked`, and one stopped mid-tool stays
/// `working`. The transcript records it: the interrupt is its last
/// message, until the next prompt adds a newer one. Such a session waits.
pub(crate) fn settle(activity: Activity, dir: &Path, pid: i32) -> Activity {
    if !matches!(activity, Activity::Blocked | Activity::Working) {
        return activity;
    }
    let interrupted = first_line(&dir.join(format!("transcript-{pid}")))
        .is_some_and(|t| ends_interrupted(Path::new(&t)));
    if interrupted {
        Activity::Waiting
    } else {
        activity
    }
}

/// Whether the last message in the transcript is the owner interrupting.
/// Reads the last 64 KiB only: a transcript grows to megabytes, and the
/// last message is near its end.
fn ends_interrupted(transcript: &Path) -> bool {
    const TAIL: u64 = 64 * 1024;
    let Ok(mut f) = fs::File::open(transcript) else {
        return false;
    };
    let Ok(len) = f.seek(SeekFrom::End(0)) else {
        return false;
    };
    if f.seek(SeekFrom::Start(len.saturating_sub(TAIL))).is_err() {
        return false;
    }
    let mut bytes = Vec::new();
    if f.read_to_end(&mut bytes).is_err() {
        return false;
    }
    last_message_interrupted(&String::from_utf8_lossy(&bytes))
}

/// Only messages count; the transcript also holds bookkeeping lines.
fn last_message_interrupted(tail: &str) -> bool {
    tail.lines()
        .rev()
        .find(|l| l.contains(r#""type":"user""#) || l.contains(r#""type":"assistant""#))
        .is_some_and(|l| l.contains(INTERRUPT))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Dir(PathBuf);

    impl Dir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "swaypplet-status-claude-{name}-{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn write(&self, name: &str, text: &str) {
            fs::write(self.0.join(name), text).unwrap();
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn claude(_: i32) -> Option<String> {
        Some(".claude-unwrapp".into())
    }

    const USER: &str = r#"{"type":"user","message":{"content":"fix it"}}"#;
    const ASSISTANT: &str = r#"{"type":"assistant","message":{"content":"done"}}"#;
    const INTERRUPTED: &str =
        r#"{"type":"user","message":{"content":"[Request interrupted by user for tool use]"}}"#;
    const BOOKKEEPING: &str = r#"{"type":"summary","summary":"x"}"#;

    fn session(dir: &Dir, pid: i32, status: &str) {
        dir.write(&format!("pid-{pid}"), "[lock] status\n");
        dir.write(&format!("status-{pid}"), &format!("{status}\n"));
    }

    fn severities(items: &[StatusItem]) -> Vec<(String, Severity)> {
        items.iter().map(|i| (i.id.clone(), i.severity)).collect()
    }

    #[test]
    fn each_status_maps_to_a_severity() {
        let dir = Dir::new("map");
        session(&dir, 1, "blocked");
        session(&dir, 2, "working");
        session(&dir, 3, "waiting");
        session(&dir, 4, "waiting");
        dir.write("asks-4", "");
        session(&dir, 5, "stopped");
        session(&dir, 6, "garbled");
        assert_eq!(
            severities(&scan_with(&dir.0, claude)),
            [
                ("1".into(), Severity::Attention),
                ("2".into(), Severity::Active),
                ("3".into(), Severity::Info),
                ("4".into(), Severity::Attention),
                ("6".into(), Severity::Info),
            ]
        );
    }

    #[test]
    fn a_dead_or_recycled_pid_is_no_item() {
        let dir = Dir::new("comm");
        session(&dir, 1, "blocked");
        session(&dir, 2, "blocked");
        let comm = |pid: i32| match pid {
            1 => Some("zsh".to_string()),
            _ => None,
        };
        assert!(scan_with(&dir.0, comm).is_empty());
    }

    #[test]
    fn a_working_session_carries_its_progress() {
        let dir = Dir::new("progress");
        session(&dir, 1, "working");
        dir.write("progress-1", "2/5 check + review ~15m\n");
        session(&dir, 2, "waiting");
        dir.write("progress-2", "5/5 done\n");
        let items = scan_with(&dir.0, claude);
        assert_eq!(items[0].title, "[lock] status");
        assert_eq!(items[0].detail.as_deref(), Some("2/5 check + review ~15m"));
        assert!(items[0].since.is_some());
        assert_eq!(items[1].detail, None);
    }

    #[test]
    fn an_interrupt_turns_blocked_and_working_into_waiting() {
        let dir = Dir::new("interrupt");
        let transcript = dir.0.join("t.jsonl");
        fs::write(&transcript, [USER, INTERRUPTED, BOOKKEEPING].join("\n")).unwrap();
        for (pid, status) in [(1, "blocked"), (2, "working")] {
            session(&dir, pid, status);
            dir.write(&format!("transcript-{pid}"), transcript.to_str().unwrap());
        }
        assert_eq!(
            severities(&scan_with(&dir.0, claude)),
            [("1".into(), Severity::Info), ("2".into(), Severity::Info)]
        );
    }

    #[test]
    fn a_newer_message_ends_the_interrupt() {
        assert!(last_message_interrupted(
            &[USER, INTERRUPTED, BOOKKEEPING].join("\n")
        ));
        assert!(!last_message_interrupted(
            &[INTERRUPTED, USER, ASSISTANT].join("\n")
        ));
        assert!(!last_message_interrupted(BOOKKEEPING));
    }

    #[test]
    fn only_blocked_and_working_consult_the_transcript() {
        let dir = Dir::new("settle");
        let transcript = dir.0.join("t.jsonl");
        fs::write(&transcript, INTERRUPTED).unwrap();
        dir.write("transcript-1", transcript.to_str().unwrap());
        assert_eq!(settle(Activity::Stopped, &dir.0, 1), Activity::Stopped);
        assert_eq!(settle(Activity::Blocked, &dir.0, 1), Activity::Waiting);
        // No transcript file: the hooks' word stands.
        assert_eq!(settle(Activity::Blocked, &dir.0, 2), Activity::Blocked);
    }
}
