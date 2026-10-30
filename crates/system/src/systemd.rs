//! systemd, as this shell drives it: the one place that runs `systemctl`
//! and `systemd-run`.
//!
//! Most of it is the user manager (`--user`): the panel's own unit, the
//! wallpaper video, the inhibitors, the jobs that must outlive the panel.
//! The backup units and the host's switch units are the system manager's,
//! which a [`Unit`] names with [`Manager::System`].
//!
//! # Test runs
//!
//! `dev/render.sh` and the other harnesses set `SWAYPPLET_DRY_SESSION`: the
//! binary runs as the real user, and `systemctl --user` reaches the real
//! user manager whatever display it is nested in. A harness run that picked
//! a wallpaper used to stop the video playing on the person's own desktop,
//! and one that typed "restart" could have restarted the machine. So every
//! call here that changes something ([`Unit::queue`], [`Unit::run`],
//! [`Transient::start`]) is logged and skipped under [`dry_session`]; the
//! reads ([`Unit::is_active`], [`Unit::show`]) still go out, since they
//! change nothing.
//!
//! Every call blocks on a subprocess; callers are on a worker thread or in
//! a process with no main loop.

use std::process::{Command, Stdio};

/// A test run: see the module docs.
pub fn dry_session() -> bool {
    std::env::var_os("SWAYPPLET_DRY_SESSION").is_some()
}

/// Which manager a unit belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Manager {
    /// The session's own (`systemctl --user`).
    User,
    /// The host's. Starting one needs a polkit rule for this user.
    System,
}

impl Manager {
    fn flag(self) -> Option<&'static str> {
        match self {
            Manager::User => Some("--user"),
            Manager::System => None,
        }
    }
}

/// What a [`Unit`] can be told to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verb {
    Start,
    Restart,
    Stop,
    /// systemd's cgroup freezer: the unit's processes stop being scheduled.
    Freeze,
    Thaw,
}

impl Verb {
    fn as_str(self) -> &'static str {
        match self {
            Verb::Start => "start",
            Verb::Restart => "restart",
            Verb::Stop => "stop",
            Verb::Freeze => "freeze",
            Verb::Thaw => "thaw",
        }
    }
}

/// A unit by name, under its manager.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unit {
    manager: Manager,
    name: String,
}

impl Unit {
    pub fn user(name: impl Into<String>) -> Self {
        Unit {
            manager: Manager::User,
            name: name.into(),
        }
    }

    pub fn system(name: impl Into<String>) -> Self {
        Unit {
            manager: Manager::System,
            name: name.into(),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Ask for `verb` and return once systemd has queued the job
    /// (`--no-block`): for a start or a stop nothing waits on, and for a
    /// `Type=oneshot` unit, whose start would otherwise wait minutes.
    pub fn queue(&self, verb: Verb) -> Result<(), String> {
        start_together(std::slice::from_ref(self), verb)
    }

    /// `verb`, waiting for the job to finish: for a caller that reads the
    /// unit back next, or a freeze that must have landed.
    pub fn run(&self, verb: Verb) -> Result<(), String> {
        systemctl(self.manager, &self.verb_args(verb, false))
    }

    fn verb_args(&self, verb: Verb, no_block: bool) -> Vec<String> {
        let mut args = vec![verb.as_str().to_string()];
        if no_block {
            args.push("--no-block".into());
        }
        args.push(self.name.clone());
        args
    }

    /// Whether the unit is active now. `false` when `systemctl` could not
    /// be asked.
    pub fn is_active(&self) -> bool {
        command(self.manager, &["is-active", "--quiet", &self.name])
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }

    /// The unit's `props` as `systemctl show` gives them. A unit that does
    /// not exist still answers (`LoadState=not-found`); `Err` is only for a
    /// `systemctl` that could not be run.
    pub fn show(&self, props: &[&str]) -> Result<Properties, String> {
        let mut args = vec!["show".to_string()];
        for p in props {
            args.push(format!("--property={p}"));
        }
        args.push(self.name.clone());
        let out = command(self.manager, &args)
            .stderr(Stdio::null())
            .output()
            .map_err(|e| format!("systemctl show {}: {e}", self.name))?;
        Ok(Properties(
            String::from_utf8_lossy(&out.stdout).into_owned(),
        ))
    }
}

/// `verb` on several units in one `--no-block` call, so systemd queues
/// them together and orders them by what the units declare.
pub fn start_together(units: &[Unit], verb: Verb) -> Result<(), String> {
    let Some(first) = units.first() else {
        return Ok(());
    };
    debug_assert!(units.iter().all(|u| u.manager == first.manager));
    let mut args = first.verb_args(verb, true);
    args.extend(units[1..].iter().map(|u| u.name.clone()));
    systemctl(first.manager, &args)
}

/// What `systemctl show` printed: `Key=value` lines.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Properties(String);

impl Properties {
    /// The value of `key`, empty when it is missing.
    pub fn get(&self, key: &str) -> &str {
        self.0
            .lines()
            .find_map(|l| l.strip_prefix(key)?.strip_prefix('='))
            .unwrap_or_default()
    }
}

/// `systemctl [--user] <args>`, with stdin closed.
fn command<S: AsRef<std::ffi::OsStr>>(manager: Manager, args: &[S]) -> Command {
    let mut cmd = Command::new("systemctl");
    cmd.args(manager.flag()).args(args).stdin(Stdio::null());
    cmd
}

/// Run a `systemctl` that changes something, unless this is a test run.
/// `Err` carries the exit status and what systemctl said.
fn systemctl(manager: Manager, args: &[String]) -> Result<(), String> {
    let shown = shown(
        "systemctl",
        manager
            .flag()
            .into_iter()
            .chain(args.iter().map(String::as_str)),
    );
    if dry_session() {
        log::info!("{shown} (dry run)");
        return Ok(());
    }
    finish(&shown, command(manager, args).output())
}

fn finish(shown: &str, out: std::io::Result<std::process::Output>) -> Result<(), String> {
    let out = out.map_err(|e| format!("{shown}: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let said = String::from_utf8_lossy(&out.stderr).trim().to_string();
    Err(if said.is_empty() {
        format!("{shown}: {}", out.status)
    } else {
        format!("{shown}: {}: {said}", out.status)
    })
}

fn shown<'a>(program: &'a str, args: impl Iterator<Item = &'a str>) -> String {
    std::iter::once(program)
        .chain(args)
        .collect::<Vec<_>>()
        .join(" ")
}

// ── Transient units ─────────────────────────────────────────────────────

/// How a [`Transient`]'s output file is opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    /// Emptied when the unit starts.
    Truncate,
    /// Added to.
    Append,
}

/// A command run as its own user unit (`systemd-run --user`), so it
/// outlives this process: a switch restarts the panel, and a job started
/// from it must not die with it. `--collect`, so a failed run does not
/// leave a unit name the next start cannot reuse.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transient {
    unit: String,
    description: Option<String>,
    properties: Vec<String>,
    env: Vec<(String, String)>,
    argv: Vec<String>,
}

impl Transient {
    pub fn new(unit: impl Into<String>, argv: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Transient {
            unit: unit.into(),
            description: None,
            properties: Vec::new(),
            env: Vec::new(),
            argv: argv.into_iter().map(Into::into).collect(),
        }
    }

    pub fn description(mut self, text: impl Into<String>) -> Self {
        self.description = Some(text.into());
        self
    }

    /// The unit's standard output into `path`. Standard error follows it
    /// (systemd's `StandardError=inherit` default).
    pub fn output(mut self, path: &std::path::Path, how: Output) -> Self {
        let mode = match how {
            Output::Truncate => "truncate",
            Output::Append => "append",
        };
        self.properties
            .push(format!("StandardOutput={mode}:{}", path.display()));
        self
    }

    /// `name=value` in the unit's environment.
    pub fn env(mut self, name: &str, value: impl Into<String>) -> Self {
        self.env.push((name.to_string(), value.into()));
        self
    }

    /// Each of `names` that this process has, passed on: the user
    /// manager's environment lacks the session's (the profile's `PATH`,
    /// the SSH agent, the session id).
    pub fn env_from_session(mut self, names: &[&str]) -> Self {
        for name in names {
            if let Ok(value) = std::env::var(name) {
                self.env.push((name.to_string(), value));
            }
        }
        self
    }

    pub fn unit(&self) -> &str {
        &self.unit
    }

    /// The arguments after `systemd-run`.
    pub fn args(&self) -> Vec<String> {
        let mut args: Vec<String> = ["--user", "--quiet", "--collect"]
            .into_iter()
            .map(String::from)
            .collect();
        args.push(format!("--unit={}", self.unit));
        if let Some(d) = &self.description {
            args.push(format!("--description={d}"));
        }
        args.extend(self.properties.iter().map(|p| format!("--property={p}")));
        args.extend(self.env.iter().map(|(k, v)| format!("--setenv={k}={v}")));
        args.push("--".into());
        args.extend(self.argv.iter().cloned());
        args
    }

    /// Start the unit; returns once systemd has it. `Err` when it was
    /// refused, which is what a unit of that name already running gets.
    pub fn start(&self) -> Result<(), String> {
        let args = self.args();
        let shown = format!("systemd-run --unit={}", self.unit);
        if dry_session() {
            log::info!("{shown} -- {} (dry run)", self.argv.join(" "));
            return Ok(());
        }
        finish(
            &shown,
            Command::new("systemd-run")
                .args(&args)
                .stdin(Stdio::null())
                .output(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_verb_names_the_unit_and_queues_only_when_asked() {
        let u = Unit::user("wallpaper-video.service");
        assert_eq!(
            u.verb_args(Verb::Restart, true),
            ["restart", "--no-block", "wallpaper-video.service"]
        );
        assert_eq!(
            u.verb_args(Verb::Freeze, false),
            ["freeze", "wallpaper-video.service"]
        );
        assert_eq!(Manager::User.flag(), Some("--user"));
        assert_eq!(Manager::System.flag(), None);
    }

    #[test]
    fn properties_read_by_key() {
        let p = Properties(
            "LoadState=loaded\nActiveState=active\nDescription=a = b until 14:30\n".into(),
        );
        assert_eq!(p.get("LoadState"), "loaded");
        assert_eq!(p.get("ActiveState"), "active");
        assert_eq!(p.get("Description"), "a = b until 14:30");
        assert_eq!(p.get("Load"), "");
        assert_eq!(p.get("Missing"), "");
    }

    #[test]
    fn a_transient_unit_carries_its_log_env_and_argv() {
        let t = Transient::new(
            "swaypplet-nx",
            ["/bin/sh", "-c", "nx \"$1\"", "sh", "apply"],
        )
        .description("swaypplet: nx apply")
        .output(
            std::path::Path::new("/run/user/1/swaypplet/nx.log"),
            Output::Truncate,
        )
        .env("PATH", "/bin");
        assert_eq!(
            t.args(),
            [
                "--user",
                "--quiet",
                "--collect",
                "--unit=swaypplet-nx",
                "--description=swaypplet: nx apply",
                "--property=StandardOutput=truncate:/run/user/1/swaypplet/nx.log",
                "--setenv=PATH=/bin",
                "--",
                "/bin/sh",
                "-c",
                "nx \"$1\"",
                "sh",
                "apply",
            ]
        );
        assert_eq!(t.unit(), "swaypplet-nx");
    }

    #[test]
    fn a_failure_says_what_ran_and_what_systemd_said() {
        use std::os::unix::process::ExitStatusExt;
        let out = |code: i32, err: &str| {
            Ok(std::process::Output {
                status: std::process::ExitStatus::from_raw(code << 8),
                stdout: Vec::new(),
                stderr: err.as_bytes().to_vec(),
            })
        };
        assert_eq!(finish("systemctl stop x", out(0, "")), Ok(()));
        assert_eq!(
            finish("systemctl stop x", out(5, "Unit x not loaded.\n")),
            Err("systemctl stop x: exit status: 5: Unit x not loaded.".into())
        );
        assert_eq!(
            finish("systemctl stop x", out(1, "")),
            Err("systemctl stop x: exit status: 1".into())
        );
    }
}
