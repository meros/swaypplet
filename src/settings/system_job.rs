//! The System tab's two actions: `nx apply` and `nx`, run as a transient
//! user unit so they outlive this process.
//!
//! They have to: a switch restarts every home-manager unit whose store path
//! moved, swaypplet included, and a child of the panel would die with it
//! halfway through `nx`'s last steps. So the job is `systemd-run --user`,
//! its output goes to a file in `$XDG_RUNTIME_DIR`, and the tab reads that
//! file back, which also gives a restarted panel the result of the run that
//! restarted it. The script brackets nx's output with two marker lines, the
//! action and the exit status, so the file alone says what ran and how it
//! ended.
//!
//! Nothing here runs on its own: [`start`] is called from a button press.
//! Under `SWAYPPLET_SYSTEM_FIXTURE` the pane never calls it, and plays a
//! canned log instead ([`FIXTURE_LINES`]).

use std::path::PathBuf;
use std::process::{Command, Stdio};

const UNIT: &str = "swaypplet-nx";
const START: &str = "swaypplet-nx: start ";
const EXIT: &str = "swaypplet-nx: exit ";

/// How many lines of the log the tab shows.
pub const TAIL: usize = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// `nx apply`: switch this host to origin/main.
    Apply,
    /// `nx`: commit, build every host, push, switch this host.
    Update,
}

impl Action {
    fn arg(self) -> &'static str {
        match self {
            Action::Apply => "apply",
            Action::Update => "sync",
        }
    }

    fn from_arg(arg: &str) -> Option<Action> {
        match arg {
            "apply" => Some(Action::Apply),
            "sync" => Some(Action::Update),
            _ => None,
        }
    }

    pub fn command(self) -> &'static str {
        match self {
            Action::Apply => "nx apply",
            Action::Update => "nx",
        }
    }
}

/// The log file, read back.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Log {
    pub action: Option<Action>,
    /// The last nx step header (`── build`), without the rule.
    pub step: Option<String>,
    /// The last [`TAIL`] lines, markers and escapes removed.
    pub tail: Vec<String>,
    pub exit: Option<i32>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Job {
    /// No run on record since boot.
    Idle,
    Running(Log),
    Done(Log),
    /// The unit is gone and the log has no exit line: it was stopped or
    /// killed.
    Lost(Log),
}

impl Job {
    pub fn running(&self) -> bool {
        matches!(self, Job::Running(_))
    }
}

/// Terminal escapes out: nx bolds its step headers.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            continue;
        }
        if c != '\r' {
            out.push(c);
        }
    }
    out
}

pub fn parse_log(text: &str) -> Log {
    let mut log = Log::default();
    let mut body = Vec::new();
    for line in text.lines().map(strip_ansi) {
        if let Some(arg) = line.strip_prefix(START) {
            log.action = Action::from_arg(arg.trim());
        } else if let Some(code) = line.strip_prefix(EXIT) {
            log.exit = code.trim().parse().ok();
        } else if !line.trim().is_empty() {
            if let Some(step) = line.trim().strip_prefix("── ") {
                log.step = Some(step.to_string());
            }
            body.push(line);
        }
    }
    let skip = body.len().saturating_sub(TAIL);
    log.tail = body.split_off(skip);
    log
}

/// What the file and the unit say together.
pub fn job(log: Option<Log>, active: bool) -> Job {
    match log {
        None if active => Job::Running(Log::default()),
        None => Job::Idle,
        Some(log) if log.exit.is_some() => Job::Done(log),
        Some(log) if active => Job::Running(log),
        Some(log) => Job::Lost(log),
    }
}

fn log_path() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map_or_else(std::env::temp_dir, PathBuf::from)
        .join("swaypplet/nx.log")
}

fn active() -> bool {
    Command::new("systemctl")
        .args(["--user", "is-active", "--quiet", UNIT])
        .stdin(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// The job as it stands. A file read and one `systemctl`; call it off the
/// main thread.
pub fn read() -> Job {
    let log = std::fs::read_to_string(log_path())
        .ok()
        .map(|t| parse_log(&t));
    job(log, active())
}

/// Start `action` in its own unit. Returns once systemd has the unit.
pub fn start(action: Action) -> Result<(), String> {
    let path = log_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let mut cmd = Command::new("systemd-run");
    cmd.args(["--user", "--quiet", "--collect"])
        .arg(format!("--unit={UNIT}"))
        .arg(format!("--description=swaypplet: {}", action.command()))
        .arg(format!(
            "--property=StandardOutput=truncate:{}",
            path.display()
        ));
    // The user manager's environment lacks the session's: nx is on the
    // home-manager PATH, the push needs the agent, and a lock check needs
    // the session.
    for var in ["PATH", "SSH_AUTH_SOCK", "XDG_SESSION_ID"] {
        if let Ok(value) = std::env::var(var) {
            cmd.arg(format!("--setenv={var}={value}"));
        }
    }
    cmd.args([
        "/bin/sh",
        "-c",
        r#"echo "swaypplet-nx: start $1"; nx "$1"; echo "swaypplet-nx: exit $?""#,
        "sh",
        action.arg(),
    ]);
    let out = cmd
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("systemd-run: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// The canned run the fixture mode plays, one line per tick.
pub const FIXTURE_LINES: &[&str] = &[
    "── apply",
    "building the system configuration...",
    "activating the configuration...",
    "setting up /etc...",
    "reloading user units for meros...",
    "restarting the following units: home-manager-meros.service",
    "<<< /nix/var/nix/profiles/system-1149-link",
    ">>> /nix/var/nix/profiles/system-1150-link",
    "Version changes:",
    "[U*]  #1  swaypplet  0.1.0 (3a815c4) -> 0.1.0 (7374675)",
];

/// The fixture's log after `ticks` ticks of `action`; it ends with exit 0.
pub fn fixture_log(action: Action, ticks: usize) -> String {
    let mut text = format!("{START}{}\n", action.arg());
    for line in FIXTURE_LINES.iter().take(ticks) {
        text.push_str(line);
        text.push('\n');
    }
    if ticks > FIXTURE_LINES.len() {
        text.push_str(EXIT);
        text.push_str("0\n");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_and_carriage_returns_go() {
        assert_eq!(strip_ansi("\u{1b}[1m── build\u{1b}[0m"), "── build");
        assert_eq!(strip_ansi("50%\r"), "50%");
        assert_eq!(strip_ansi("plain"), "plain");
    }

    #[test]
    fn the_markers_say_what_ran_and_how_it_ended() {
        let text = "swaypplet-nx: start sync\n\n\u{1b}[1m── build\u{1b}[0m\nnx: building meros-laptop\nswaypplet-nx: exit 1\n";
        let log = parse_log(text);
        assert_eq!(log.action, Some(Action::Update));
        assert_eq!(log.step.as_deref(), Some("build"));
        assert_eq!(log.exit, Some(1));
        assert_eq!(log.tail, vec!["── build", "nx: building meros-laptop"]);
    }

    #[test]
    fn the_tail_keeps_the_last_lines() {
        let text: String = (0..25).map(|i| format!("line {i}\n")).collect();
        let log = parse_log(&text);
        assert_eq!(log.tail.len(), TAIL);
        assert_eq!(log.tail.last().map(String::as_str), Some("line 24"));
        assert_eq!(log.action, None);
        assert_eq!(log.exit, None);
    }

    #[test]
    fn the_file_and_the_unit_make_the_state() {
        assert_eq!(job(None, false), Job::Idle);
        assert!(job(None, true).running());
        let running = parse_log("swaypplet-nx: start apply\n── apply\n");
        assert!(job(Some(running.clone()), true).running());
        assert_eq!(job(Some(running.clone()), false), Job::Lost(running));
        let done = parse_log("swaypplet-nx: start apply\nswaypplet-nx: exit 0\n");
        assert_eq!(job(Some(done.clone()), true), Job::Done(done));
    }

    #[test]
    fn the_fixture_plays_to_a_success() {
        let mid = parse_log(&fixture_log(Action::Apply, 3));
        assert_eq!(mid.exit, None);
        assert_eq!(mid.tail.len(), 3);
        let end = parse_log(&fixture_log(Action::Apply, FIXTURE_LINES.len() + 1));
        assert_eq!(end.exit, Some(0));
        assert_eq!(end.action, Some(Action::Apply));
    }
}
