//! nx, run as a transient user unit so it outlives this process: the one
//! runner for every pane that deploys. The System tab's Apply now and
//! Update start `nx apply` and `nx`; the Quality tab's Merge & apply starts
//! `nx` after the merge. One unit name ([`UNIT`]), so systemd refuses a
//! second run while one is going whichever pane asked, and one log, so each
//! pane sees the run the other started.
//!
//! It has to outlive the panel: a switch restarts every home-manager unit
//! whose store path moved, swaypplet included, and a child of the panel
//! would die with it halfway through `nx`'s last steps. So the job is
//! `systemd-run --user`, its output goes to a file in `$XDG_RUNTIME_DIR`,
//! and the panes read that file back, which also gives a restarted panel the
//! result of the run that restarted it. The script brackets nx's output
//! with two marker lines, the action and the exit status, so the file alone
//! says what ran and how it ended.
//!
//! `SWAYPPLET_DEPLOY` (a shell command line) runs in nx's place, for a test
//! of the panes that must not switch the host. Under
//! `SWAYPPLET_DRY_SESSION` nothing starts at all (`systemd::Transient`).

use std::path::PathBuf;

use crate::systemd::{Output, Transient, Unit};

/// The unit every nx run is.
pub const UNIT: &str = "swaypplet-nx";
/// The marker lines the script writes around nx's output.
pub const START: &str = "swaypplet-nx: start ";
pub const EXIT: &str = "swaypplet-nx: exit ";

/// How many lines of the log a [`Log`] keeps.
pub const TAIL: usize = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// `nx apply`: switch this host to origin/main.
    Apply,
    /// `nx`: commit, build every host, push, switch this host.
    Update,
}

impl Action {
    pub fn arg(self) -> &'static str {
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

/// `$XDG_RUNTIME_DIR/swaypplet/nx.log`.
pub fn log_path() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map_or_else(std::env::temp_dir, PathBuf::from)
        .join("swaypplet/nx.log")
}

/// The job as it stands. A file read and one `systemctl`; call it off the
/// main thread.
pub fn read() -> Job {
    let log = std::fs::read_to_string(log_path())
        .ok()
        .map(|t| parse_log(&t));
    job(log, Unit::user(UNIT).is_active())
}

/// The script the unit runs, with the action as `$1`: the markers around
/// nx, or around `deploy` when one is given.
fn script(deploy: Option<&str>) -> String {
    let run = deploy.unwrap_or(r#"nx "$1""#);
    format!(r#"echo "{START}$1"; {run}; echo "{EXIT}$?""#)
}

/// `path` with the places a NixOS user profile puts programs added, so nx
/// is found: the panel runs as a user unit, whose `PATH` need not include
/// the profile.
fn with_profile(path: Option<&str>, user: &str, home: &str) -> String {
    let mut dirs: Vec<String> = path
        .unwrap_or_default()
        .split(':')
        .filter(|d| !d.is_empty())
        .map(String::from)
        .collect();
    for dir in [
        format!("/etc/profiles/per-user/{user}/bin"),
        format!("{home}/.nix-profile/bin"),
        "/run/current-system/sw/bin".to_string(),
    ] {
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    dirs.join(":")
}

/// The unit for `action`, its output into `log`.
fn unit(action: Action, log: &std::path::Path) -> Transient {
    let deploy = std::env::var("SWAYPPLET_DEPLOY")
        .ok()
        .filter(|c| !c.trim().is_empty());
    let path = with_profile(
        std::env::var("PATH").ok().as_deref(),
        &swaypplet_base::user::current().unwrap_or_default(),
        &std::env::var("HOME").unwrap_or_default(),
    );
    Transient::new(
        UNIT,
        [
            "/bin/sh".to_string(),
            "-c".into(),
            script(deploy.as_deref()),
            "sh".into(),
            action.arg().into(),
        ],
    )
    .description(format!("swaypplet: {}", action.command()))
    .output(log, Output::Truncate)
    .env("PATH", path)
    // The rest of what nx needs from the session: the home, the SSH agent
    // for the push, the session for a lock check.
    .env_from_session(&[
        "HOME",
        "USER",
        "SSH_AUTH_SOCK",
        "NIX_PATH",
        "XDG_SESSION_ID",
    ])
}

/// Start `action` in [`UNIT`]. Returns once systemd has the unit; `Err`
/// when it refused, which is what it does while a run is going.
pub fn start(action: Action) -> Result<(), String> {
    let path = log_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    unit(action, &path).start()
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

    /// The script's own markers are what `parse_log` reads back.
    #[test]
    fn the_script_brackets_nx_or_its_stand_in() {
        assert_eq!(
            script(None),
            r#"echo "swaypplet-nx: start $1"; nx "$1"; echo "swaypplet-nx: exit $?""#
        );
        assert_eq!(
            script(Some("sleep 1")),
            r#"echo "swaypplet-nx: start $1"; sleep 1; echo "swaypplet-nx: exit $?""#
        );
    }

    #[test]
    fn the_profile_joins_the_path_once() {
        assert_eq!(
            with_profile(
                Some("/a/bin:/run/current-system/sw/bin"),
                "ada",
                "/home/ada"
            ),
            "/a/bin:/run/current-system/sw/bin:/etc/profiles/per-user/ada/bin:/home/ada/.nix-profile/bin"
        );
        assert_eq!(
            with_profile(None, "ada", "/home/ada"),
            "/etc/profiles/per-user/ada/bin:/home/ada/.nix-profile/bin:/run/current-system/sw/bin"
        );
    }

    /// Both panes' runs are this one unit, whatever the action.
    #[test]
    fn every_action_runs_in_the_one_unit() {
        let log = std::path::Path::new("/run/user/1/swaypplet/nx.log");
        for action in [Action::Apply, Action::Update] {
            let args = unit(action, log).args();
            assert!(args.contains(&"--unit=swaypplet-nx".to_string()));
            assert!(args.contains(
                &"--property=StandardOutput=truncate:/run/user/1/swaypplet/nx.log".to_string()
            ));
            assert_eq!(args.last().map(String::as_str), Some(action.arg()));
        }
    }
}
