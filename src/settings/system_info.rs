//! What the System tab shows: which nixos-config commit this host runs
//! against origin/main, whether a newer generation is staged, which
//! swaypplet each of them carries, and the machine's plain facts.
//!
//! The same facts `nx status` prints (`users/modules/nx/nx` in the nixos
//! repo), read directly rather than parsed from its text:
//!
//! - the commit a generation was built from is the `configurationRevision`
//!   baked into that generation's `sw/bin/nixos-version` script, which is
//!   all `nixos-version --configuration-revision` does;
//! - a generation is staged when `/nix/var/nix/profiles/system` and
//!   `/run/current-system` resolve to different store paths (`nx pending`);
//! - origin/main is `git ls-remote`, as `nx status` asks, and the local
//!   `origin/main` ref as of the last fetch when the network does not answer;
//! - the swaypplet a nixos-config commit ships is that commit's `flake.lock`
//!   entry, so no binary is ever run to ask it.
//!
//! Every read here blocks (files, `git`, one network round trip) and runs on
//! a worker thread (`system_pane.rs`). The parsing is pure and tested.
//! `SWAYPPLET_SYSTEM_FIXTURE=behind|staged|in-sync|applying` replaces the
//! whole read with a canned [`Snapshot`], so the tab renders without this
//! host's state and without anything being run.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The swaypplet commit this binary was built from: the flake sets it
/// (`SWAYPPLET_REV`, `flake.nix`); a `cargo build` has none.
pub const BUILD_REV: Option<&str> = option_env!("SWAYPPLET_REV");

/// How many commits "what's new" lists per repository.
const NEW_COMMITS: usize = 6;

/// How long `git ls-remote` may take before the local ref stands in.
const REMOTE_TIMEOUT: Duration = Duration::from_secs(6);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Commit {
    pub hash: String,
    pub subject: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Disk {
    pub mount: &'static str,
    pub free: u64,
    pub total: u64,
}

/// Where the origin/main commit came from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OriginSource {
    /// `git ls-remote` answered.
    Remote,
    /// The local `origin/main` ref, as of the last fetch.
    LastFetch,
    #[default]
    Unknown,
}

/// The swaypplet commit in each place it can be.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shells {
    /// This process, as built.
    pub running: Option<String>,
    /// The staged generation's `flake.lock`, when one is staged.
    pub staged: Option<String>,
    /// origin/main's `flake.lock`.
    pub origin: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub host: String,
    pub nixos_version: Option<String>,
    pub kernel: Option<String>,
    pub uptime_s: Option<u64>,
    pub disks: Vec<Disk>,
    /// Available and total memory, in bytes.
    pub memory: Option<(u64, u64)>,
    /// The nixos-config commit the running generation was built from.
    pub running: Option<String>,
    /// `Some` when a newer generation is staged; inside, its commit.
    pub staged: Option<Option<String>>,
    pub origin: Option<String>,
    pub origin_source: OriginSource,
    /// Commits in origin/main that the running one lacks, when both are here.
    pub behind: Option<usize>,
    pub new_commits: Vec<Commit>,
    pub shells: Shells,
    pub shell_commits: Vec<Commit>,
    /// A system unit switching this host right now (`nx-apply`,
    /// `nx-converge`), started from here or anywhere else.
    pub switching: Option<&'static str>,
    /// The session's `LockedHint`, which `nx try` refuses to switch under.
    pub locked: bool,
}

/// The one line the tab leads with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Switching(&'static str),
    Staged,
    Behind(Option<usize>),
    /// The running commit is not on origin/main: a local build that has not
    /// been pushed, or a history that moved.
    Diverged,
    InSync,
    Unknown,
}

pub fn verdict(s: &Snapshot) -> Verdict {
    if let Some(unit) = s.switching {
        return Verdict::Switching(unit);
    }
    if s.staged.is_some() {
        return Verdict::Staged;
    }
    match (&s.running, &s.origin) {
        (Some(r), Some(o)) if r == o => Verdict::InSync,
        (Some(_), Some(_)) => match s.behind {
            Some(0) => Verdict::Diverged,
            n => Verdict::Behind(n),
        },
        _ => Verdict::Unknown,
    }
}

/// What the two buttons may do, and why not when they may not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Actions {
    pub apply: bool,
    pub update: bool,
    pub reason: Option<&'static str>,
}

pub const LOCKED_REASON: &str = "The screen is locked. A switch restarts swaypplet-idle, whose locker dies without unlocking; nx waits for the unlock.";

pub fn actions(v: Verdict, locked: bool, job_running: bool) -> Actions {
    let none = |reason| Actions {
        apply: false,
        update: false,
        reason: Some(reason),
    };
    if job_running || matches!(v, Verdict::Switching(_)) {
        return none("A switch is running.");
    }
    if locked {
        return none(LOCKED_REASON);
    }
    match v {
        Verdict::InSync => Actions {
            apply: false,
            update: true,
            reason: Some(
                "This host runs origin/main. Update still commits and pushes what is in ~/nixos.",
            ),
        },
        _ => Actions {
            apply: true,
            update: true,
            reason: None,
        },
    }
}

// ── Parsing ─────────────────────────────────────────────────────────────

/// A 40-character lowercase hex commit, or nothing.
fn full_rev(s: &str) -> Option<String> {
    (s.len() == 40
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
    .then(|| s.to_string())
}

/// The JSON line of a `nixos-version` script, whose `--json` branch is a
/// here-document of it. `None` when the script has none.
fn version_json(script: &str) -> Option<serde_json::Value> {
    script
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with('{') && l.contains("\"nixosVersion\""))
        .find_map(|l| serde_json::from_str(l).ok())
}

/// The `configurationRevision` a `nixos-version` script carries. A build
/// from a dirty tree has none, and neither does anything that is not a
/// 40-character commit.
pub fn configuration_revision(script: &str) -> Option<String> {
    version_json(script)?
        .get("configurationRevision")?
        .as_str()
        .and_then(full_rev)
}

pub fn nixos_version(script: &str) -> Option<String> {
    version_json(script)?
        .get("nixosVersion")?
        .as_str()
        .map(str::to_string)
}

/// The commit `flake.lock` pins `input` to.
pub fn locked_rev(lock: &str, input: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(lock).ok()?;
    v.get("nodes")?
        .get(input)?
        .get("locked")?
        .get("rev")?
        .as_str()
        .and_then(full_rev)
}

/// `git ls-remote origin refs/heads/main`: `<rev>\trefs/heads/main`.
pub fn ls_remote_rev(out: &str) -> Option<String> {
    out.lines().find_map(|l| {
        let (rev, name) = l.split_once('\t')?;
        (name.trim() == "refs/heads/main").then(|| full_rev(rev.trim()))?
    })
}

/// `git log --format=%h%x1f%s`, one commit per line.
pub fn commits(out: &str) -> Vec<Commit> {
    out.lines()
        .filter_map(|l| {
            let (hash, subject) = l.split_once('\u{1f}')?;
            Some(Commit {
                hash: hash.to_string(),
                subject: subject.to_string(),
            })
        })
        .collect()
}

/// `MemAvailable` and `MemTotal` from `/proc/meminfo`, in bytes.
pub fn meminfo(text: &str) -> Option<(u64, u64)> {
    let field = |name: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(name)?.strip_prefix(':'))
            .and_then(|rest| rest.split_whitespace().next()?.parse::<u64>().ok())
            .map(|kib| kib * 1024)
    };
    Some((field("MemAvailable")?, field("MemTotal")?))
}

/// The first field of `/proc/uptime`, whole seconds.
pub fn uptime(text: &str) -> Option<u64> {
    let secs: f64 = text.split_whitespace().next()?.parse().ok()?;
    (secs.is_finite() && secs >= 0.0).then_some(secs as u64)
}

// ── Formatting ──────────────────────────────────────────────────────────

pub fn short(rev: &str) -> &str {
    rev.get(..7).unwrap_or(rev)
}

pub fn uptime_label(secs: u64) -> String {
    let (d, h, m) = (secs / 86_400, secs % 86_400 / 3600, secs % 3600 / 60);
    match (d, h) {
        (0, 0) => format!("{m} min"),
        (0, h) => format!("{h} h {m} min"),
        (d, h) => format!("{d} d {h} h"),
    }
}

/// Bytes in the largest binary unit that keeps a whole part, one decimal.
pub fn bytes_label(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = n as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", UNITS[unit])
    }
}

// ── Reading ─────────────────────────────────────────────────────────────

fn home() -> PathBuf {
    std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from)
}

/// The nixos-config clone nx works in (`NX_FLAKE`).
pub fn flake_dir() -> PathBuf {
    home().join("nixos")
}

/// The swaypplet clone (`NX_PERSONAL/swaypplet`).
fn shell_dir() -> PathBuf {
    home().join("git/personal/swaypplet")
}

const CURRENT: &str = "/run/current-system";
const PROFILE: &str = "/nix/var/nix/profiles/system";

fn read(path: impl AsRef<Path>) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

fn version_script(generation: &str) -> Option<String> {
    read(Path::new(generation).join("sw/bin/nixos-version"))
}

/// `git -C dir args`, stdout on success.
fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Run `cmd`, and give up on it after `limit`.
fn output_within(mut cmd: Command, limit: Duration) -> Option<String> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => {
                let mut out = String::new();
                std::io::Read::read_to_string(child.stdout.as_mut()?, &mut out).ok()?;
                return Some(out);
            }
            Ok(Some(_)) | Err(_) => return None,
            Ok(None) if start.elapsed() > limit => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}

fn has_commit(dir: &Path, rev: &str) -> bool {
    git(dir, &["cat-file", "-e", &format!("{rev}^{{commit}}")]).is_some()
}

fn log_between(dir: &Path, from: &str, to: &str) -> Vec<Commit> {
    if !has_commit(dir, from) || !has_commit(dir, to) {
        return Vec::new();
    }
    let range = format!("{from}..{to}");
    let n = format!("-n{NEW_COMMITS}");
    git(dir, &["log", "--format=%h%x1f%s", &n, &range])
        .map(|o| commits(&o))
        .unwrap_or_default()
}

fn shell_at(flake: &Path, rev: &str) -> Option<String> {
    locked_rev(
        &git(flake, &["show", &format!("{rev}:flake.lock")])?,
        "swaypplet",
    )
}

fn origin(flake: &Path) -> (Option<String>, OriginSource) {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(flake)
        .args(["ls-remote", "origin", "refs/heads/main"])
        .env("GIT_TERMINAL_PROMPT", "0")
        .env(
            "GIT_SSH_COMMAND",
            "ssh -o BatchMode=yes -o ConnectTimeout=4",
        );
    if let Some(rev) = output_within(cmd, REMOTE_TIMEOUT)
        .as_deref()
        .and_then(ls_remote_rev)
    {
        return (Some(rev), OriginSource::Remote);
    }
    match git(flake, &["rev-parse", "refs/remotes/origin/main"]).and_then(|o| full_rev(o.trim())) {
        Some(rev) => (Some(rev), OriginSource::LastFetch),
        None => (None, OriginSource::Unknown),
    }
}

fn disk(mount: &'static str) -> Option<Disk> {
    let path = std::ffi::CString::new(mount).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `path` is NUL-terminated and `st` is a valid out-pointer.
    if unsafe { libc::statvfs(path.as_ptr(), &mut st) } != 0 {
        return None;
    }
    let frsize = st.f_frsize as u64;
    Some(Disk {
        mount,
        free: st.f_bavail as u64 * frsize,
        total: st.f_blocks as u64 * frsize,
    })
}

/// A system unit that is switching this host now.
fn switching() -> Option<&'static str> {
    ["nx-apply.service", "nx-converge.service"]
        .into_iter()
        .find(|unit| {
            Command::new("systemctl")
                .args(["is-active", "--quiet", unit])
                .stdin(Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        })
}

fn locked() -> bool {
    let Ok(id) = std::env::var("XDG_SESSION_ID") else {
        return false;
    };
    Command::new("loginctl")
        .args(["show-session", &id, "-p", "LockedHint", "--value"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "yes")
}

/// Everything the tab shows. Blocks for up to [`REMOTE_TIMEOUT`] and more;
/// call it off the main thread.
pub fn read_snapshot() -> Snapshot {
    if let Some(name) = fixture_name() {
        return fixture(&name);
    }
    let flake = flake_dir();
    let running_script = version_script(CURRENT);
    let running = running_script.as_deref().and_then(configuration_revision);
    let staged = {
        let current = std::fs::canonicalize(CURRENT).ok();
        let profile = std::fs::canonicalize(PROFILE).ok();
        (current.is_some() && profile.is_some() && current != profile).then(|| {
            version_script(PROFILE)
                .as_deref()
                .and_then(configuration_revision)
        })
    };
    let (origin, origin_source) = origin(&flake);

    let (behind, new_commits) = match (&running, &origin) {
        (Some(r), Some(o)) if r == o => (Some(0), Vec::new()),
        (Some(r), Some(o)) if has_commit(&flake, r) && has_commit(&flake, o) => (
            git(&flake, &["rev-list", "--count", &format!("{r}..{o}")])
                .and_then(|n| n.trim().parse().ok()),
            log_between(&flake, r, o),
        ),
        // origin moved past the last fetch: no count, but the subjects up to
        // the fetched ref are still news.
        (Some(r), Some(_)) => (None, log_between(&flake, r, "refs/remotes/origin/main")),
        _ => (None, Vec::new()),
    };

    let shells = Shells {
        running: BUILD_REV.and_then(full_rev),
        staged: staged.clone().flatten().and_then(|r| shell_at(&flake, &r)),
        origin: origin.as_deref().and_then(|o| shell_at(&flake, o)),
    };
    let shell_commits = match (&shells.running, &shells.origin) {
        (Some(r), Some(o)) if r != o => log_between(&shell_dir(), r, o),
        _ => Vec::new(),
    };

    Snapshot {
        host: read("/proc/sys/kernel/hostname")
            .map(|h| h.trim().to_string())
            .unwrap_or_default(),
        nixos_version: running_script.as_deref().and_then(nixos_version),
        kernel: read("/proc/sys/kernel/osrelease").map(|k| k.trim().to_string()),
        uptime_s: read("/proc/uptime").as_deref().and_then(uptime),
        disks: ["/", "/nix"].into_iter().filter_map(disk).collect(),
        memory: read("/proc/meminfo").as_deref().and_then(meminfo),
        running,
        staged,
        origin,
        origin_source,
        behind,
        new_commits,
        shells,
        shell_commits,
        switching: switching(),
        locked: locked(),
    }
}

// ── Fixtures ────────────────────────────────────────────────────────────

pub fn fixture_name() -> Option<String> {
    std::env::var("SWAYPPLET_SYSTEM_FIXTURE")
        .ok()
        .filter(|s| !s.is_empty())
}

const REV_A: &str = "98c70da6d63e744c8adec7cac59de0be2b33a812";
const REV_B: &str = "1a5231a0c1f9e7d3b2a4c5d6e7f8091a2b3c4d5e";
const SHELL_A: &str = "3a815c4e0d9b8a7f6e5d4c3b2a1908f7e6d5c4b3";
const SHELL_B: &str = "7374675a1b2c3d4e5f60718293a4b5c6d7e8f901";

/// A canned snapshot for the render harness and the tests. Unknown names
/// get the "behind" one.
pub fn fixture(name: &str) -> Snapshot {
    let commit = |hash: &str, subject: &str| Commit {
        hash: hash.to_string(),
        subject: subject.to_string(),
    };
    let mut s = Snapshot {
        host: "meros-laptop".to_string(),
        nixos_version: Some("26.11.20260925.e94cb15".to_string()),
        kernel: Some("7.2.7".to_string()),
        uptime_s: Some(3 * 86_400 + 4 * 3600 + 1200),
        disks: vec![
            Disk {
                mount: "/",
                free: 182 << 30,
                total: 476 << 30,
            },
            Disk {
                mount: "/nix",
                free: 182 << 30,
                total: 476 << 30,
            },
        ],
        memory: Some((21 << 30, 31 << 30)),
        running: Some(REV_A.to_string()),
        staged: None,
        origin: Some(REV_B.to_string()),
        origin_source: OriginSource::Remote,
        behind: Some(3),
        new_commits: vec![
            commit(
                "1a5231a",
                "chore(flake): bump swaypplet to 7374675, its rewritten history",
            ),
            commit(
                "34f8e3c",
                "feat(nx): converge stages the generation when a locked session would lose its locker, and every new shell says so through nx pending until the host is switched or rebooted",
            ),
            commit(
                "2a3f471",
                "feat: swaypplet owns display profiles; kanshi retires",
            ),
        ],
        shells: Shells {
            running: Some(SHELL_A.to_string()),
            staged: None,
            origin: Some(SHELL_B.to_string()),
        },
        shell_commits: vec![
            commit("7374675", "docs: a README with screenshots"),
            commit(
                "b71ecea",
                "fix(ui): the presence dot sits on the current user's ring",
            ),
        ],
        switching: None,
        locked: false,
    };
    match name {
        "in-sync" => {
            s.running = s.origin.clone();
            s.behind = Some(0);
            s.new_commits.clear();
            s.shells.running = s.shells.origin.clone();
            s.shell_commits.clear();
        }
        "staged" => {
            s.staged = Some(Some(REV_B.to_string()));
            s.shells.staged = s.shells.origin.clone();
        }
        "applying" => s.switching = Some("nx-apply.service"),
        _ => {}
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCRIPT: &str = r#"#! /nix/store/x-bash/bin/bash
case "$1" in
  --configuration-revision)
    ;;
  --json)
    cat <<EOF
{"configurationRevision":"98c70da6d63e744c8adec7cac59de0be2b33a812","nixosVersion":"26.11.20260925.e94cb15","nixpkgsRevision":"e94cb152ed51bd6e24eb4a41f1460252beb52cd2"}
EOF
    ;;
esac
"#;

    #[test]
    fn the_version_script_gives_its_commit_and_version() {
        assert_eq!(configuration_revision(SCRIPT).as_deref(), Some(REV_A));
        assert_eq!(
            nixos_version(SCRIPT).as_deref(),
            Some("26.11.20260925.e94cb15")
        );
    }

    #[test]
    fn a_dirty_build_has_no_commit() {
        let dirty = r#"{"nixosVersion":"26.11","nixpkgsRevision":"e94cb152ed51bd6e24eb4a41f1460252beb52cd2"}"#;
        assert_eq!(configuration_revision(dirty), None);
        assert_eq!(nixos_version(dirty).as_deref(), Some("26.11"));
        let odd = r#"{"configurationRevision":"98c70da-dirty","nixosVersion":"26.11"}"#;
        assert_eq!(configuration_revision(odd), None);
        assert_eq!(configuration_revision("#!/bin/sh\necho hi\n"), None);
    }

    #[test]
    fn the_lock_names_the_shell_commit() {
        let lock = format!(
            r#"{{"nodes":{{"swaypplet":{{"locked":{{"rev":"{SHELL_B}","type":"github"}}}},"root":{{}}}}}}"#
        );
        assert_eq!(locked_rev(&lock, "swaypplet").as_deref(), Some(SHELL_B));
        assert_eq!(locked_rev(&lock, "leaves"), None);
        assert_eq!(locked_rev("not json", "swaypplet"), None);
    }

    #[test]
    fn ls_remote_is_read_strictly() {
        let out = format!("{REV_B}\trefs/heads/main\n");
        assert_eq!(ls_remote_rev(&out).as_deref(), Some(REV_B));
        assert_eq!(
            ls_remote_rev(&format!("{REV_B}\trefs/heads/mainline\n")),
            None
        );
        assert_eq!(ls_remote_rev("fatal: could not read from remote\n"), None);
        assert_eq!(ls_remote_rev(""), None);
    }

    #[test]
    fn log_lines_split_on_the_unit_separator() {
        let c = commits("1a5231a\u{1f}chore: a | b\n34f8e3c\u{1f}style: fmt\nbroken\n");
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].hash, "1a5231a");
        assert_eq!(c[0].subject, "chore: a | b");
    }

    #[test]
    fn proc_files() {
        let mem = "MemTotal:       32000000 kB\nMemFree:  100 kB\nMemAvailable:   16000000 kB\n";
        assert_eq!(meminfo(mem), Some((16_000_000 * 1024, 32_000_000 * 1024)));
        assert_eq!(meminfo("MemTotal: 1 kB\n"), None);
        assert_eq!(uptime("273600.52 1000000.00\n"), Some(273_600));
        assert_eq!(uptime("nope"), None);
    }

    #[test]
    fn labels() {
        assert_eq!(uptime_label(59), "0 min");
        assert_eq!(uptime_label(3 * 3600 + 5 * 60), "3 h 5 min");
        assert_eq!(uptime_label(3 * 86_400 + 4 * 3600 + 1200), "3 d 4 h");
        assert_eq!(bytes_label(512), "512 B");
        assert_eq!(bytes_label(182 << 30), "182.0 GiB");
        assert_eq!(bytes_label(1536 << 20), "1.5 GiB");
        assert_eq!(short(REV_A), "98c70da");
        assert_eq!(short("abc"), "abc");
    }

    #[test]
    fn each_fixture_maps_to_its_verdict() {
        assert_eq!(verdict(&fixture("behind")), Verdict::Behind(Some(3)));
        assert_eq!(verdict(&fixture("in-sync")), Verdict::InSync);
        assert_eq!(verdict(&fixture("staged")), Verdict::Staged);
        assert_eq!(
            verdict(&fixture("applying")),
            Verdict::Switching("nx-apply.service")
        );
    }

    #[test]
    fn a_running_commit_origin_lacks_is_diverged_and_missing_facts_are_unknown() {
        let mut s = fixture("behind");
        s.behind = Some(0);
        assert_eq!(verdict(&s), Verdict::Diverged);
        s.behind = None;
        assert_eq!(verdict(&s), Verdict::Behind(None));
        s.origin = None;
        assert_eq!(verdict(&s), Verdict::Unknown);
        s.origin = Some(REV_B.to_string());
        s.running = None;
        assert_eq!(verdict(&s), Verdict::Unknown);
    }

    #[test]
    fn nothing_switches_while_locked_or_while_a_switch_runs() {
        let behind = Verdict::Behind(Some(1));
        let a = actions(behind, false, false);
        assert!(a.apply && a.update && a.reason.is_none());

        let a = actions(behind, true, false);
        assert!(!a.apply && !a.update);
        assert_eq!(a.reason, Some(LOCKED_REASON));

        let a = actions(behind, false, true);
        assert!(!a.apply && !a.update);
        let a = actions(Verdict::Switching("nx-apply.service"), false, false);
        assert!(!a.apply && !a.update);
    }

    #[test]
    fn in_sync_offers_update_and_not_apply() {
        let a = actions(Verdict::InSync, false, false);
        assert!(!a.apply && a.update);
        assert!(a.reason.is_some());
        let a = actions(Verdict::Staged, false, false);
        assert!(a.apply && a.update);
    }
}
