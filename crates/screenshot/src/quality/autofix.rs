//! Auto-fix and Merge & apply, without the tab: the runner's job files,
//! starting and stopping a job, what GitHub says, the pictures of a ready
//! fix, and the merge and deploy. Settings → Quality draws all of it.
//!
//! A job is `dev/autofix/run.sh --issue <n>` as a transient user unit
//! (`swaypplet-autofix-<n>`), not a child of the panel, so it survives the
//! panel restarting; jobs queue on the runner's lock. The runner's files
//! (`status::Job`) are what the tab follows.
//!
//! Merge & apply squash-merges the PR and runs `nx` through the one nx
//! runner (`swaypplet_system::nx`), which the System tab uses too: one unit,
//! one log, so neither tab starts a second deploy while the other's runs.
//!
//! `SWAYPPLET_QUALITY_FIXTURE=<dir>` reads `issues.json`, `prs.json`, the
//! jobs (`jobs/<n>/`) and the pictures from a directory, for the harness.
//! Under `SWAYPPLET_DRY_RUN` a job runs as a plain child with `--dry-run`
//! and a deploy plays a canned log, so the user's systemd is left alone.

use std::path::{Path, PathBuf};

use swaypplet_system::nx;
use swaypplet_system::systemd::{Transient, Unit, Verb};

use super::status::{Issue, Job, Pr};
use super::{body, gh};

/// How many open issues the tab lists.
pub const LIMIT: u32 = 30;

// ── Pure parts ──────────────────────────────────────────────────────────

/// `tests pass · gate pass` from check.sh's `RESULT` lines.
pub fn checks_line(text: &str) -> Option<String> {
    let parts: Vec<String> = text
        .lines()
        .filter_map(|l| l.trim().strip_prefix("RESULT "))
        .map(|r| r.replace(':', "").to_lowercase())
        .collect();
    (!parts.is_empty()).then(|| parts.join(" \u{00b7} "))
}

/// The last `n` non-empty lines of a log, without colour codes.
pub fn tail(log: &str, n: usize) -> Vec<String> {
    let lines: Vec<String> = log
        .lines()
        .map(nx::strip_ansi)
        .map(|l| l.trim_end().to_string())
        .filter(|l| !l.trim().is_empty())
        .collect();
    lines[lines.len().saturating_sub(n)..].to_vec()
}

/// What `git status --porcelain` says is uncommitted, briefly: the first
/// three paths and how many more. `None` for a clean tree.
pub fn dirty(porcelain: &str) -> Option<String> {
    let paths: Vec<&str> = porcelain
        .lines()
        .filter_map(|l| l.get(3..))
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    if paths.is_empty() {
        return None;
    }
    let mut out = paths[..paths.len().min(3)].join(", ");
    if paths.len() > 3 {
        out.push_str(&format!(" and {} more", paths.len() - 3));
    }
    Some(out)
}

/// The runner's arguments for issue `n`, with the author whose issue the
/// owner allowed.
fn job_args(n: u64, allow: Option<&str>, dry: bool) -> Vec<String> {
    let mut args = vec!["--issue".to_string(), n.to_string()];
    if let Some(author) = allow {
        args.push("--allow-author".into());
        args.push(author.to_string());
    }
    if dry {
        args.push("--dry-run".into());
    }
    args
}

fn unit(n: u64) -> String {
    format!("swaypplet-autofix-{n}")
}

/// The unit a job for issue `n` runs in.
fn job_unit(n: u64, script: &Path, args: Vec<String>) -> Transient {
    let mut argv = vec![script.to_string_lossy().into_owned()];
    argv.extend(args);
    // What the runner needs from this session: the profile's PATH (git,
    // gh, nix, claude, jq), the home, and the SSH agent for the push. The
    // runner takes the agent socket and the GitHub token away from claude.
    Transient::new(unit(n), argv).env_from_session(&[
        "PATH",
        "HOME",
        "USER",
        "SSH_AUTH_SOCK",
        "NIX_PATH",
        "SWAYPPLET_SWAY",
    ])
}

// ── Where things are ────────────────────────────────────────────────────

pub fn fixture() -> Option<PathBuf> {
    std::env::var_os("SWAYPPLET_QUALITY_FIXTURE").map(PathBuf::from)
}

/// The runner's state folder (run.sh's `STATE`).
fn runner_state() -> PathBuf {
    if let Some(dir) = fixture() {
        return dir;
    }
    std::env::var_os("AUTOFIX_STATE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let base = std::env::var_os("XDG_STATE_HOME")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| Path::new(&super::home()).join(".local/state"));
            base.join("swaypplet-autofix")
        })
}

fn job_dir(n: u64) -> PathBuf {
    runner_state().join("jobs").join(n.to_string())
}

/// The job for issue `n`, if one ever ran here.
pub fn read_job(n: u64) -> Option<Job> {
    let dir = job_dir(n);
    let phase = std::fs::read_to_string(dir.join("phase")).ok()?;
    let result = std::fs::read_to_string(dir.join("result"))
        .ok()
        .map(|r| r.trim().to_string())
        .filter(|r| !r.is_empty());
    let log = std::fs::read_to_string(dir.join("log")).unwrap_or_default();
    let idle_s = std::fs::metadata(dir.join("log"))
        .or_else(|_| std::fs::metadata(dir.join("phase")))
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .map_or(0, |d| d.as_secs());
    Some(Job {
        phase: phase.trim().to_string(),
        result,
        idle_s: if fixture().is_some() { 0 } else { idle_s },
        // The phase headers repeat the phase line above the tail.
        tail: tail(
            &log.lines()
                .filter(|l| !l.contains("\u{2500}\u{2500} "))
                .collect::<Vec<_>>()
                .join("\n"),
            2,
        ),
    })
}

/// The runner script: `SWAYPPLET_AUTOFIX_RUNNER`, or the clone nx builds
/// from.
fn runner() -> PathBuf {
    std::env::var_os("SWAYPPLET_AUTOFIX_RUNNER")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(&super::home()).join("git/personal/swaypplet/dev/autofix/run.sh")
        })
}

// ── Jobs ────────────────────────────────────────────────────────────────

/// Start the runner for issue `n`, as a unit (or, in a dry run, as a plain
/// child with `--dry-run`, so the user's systemd is left alone).
pub fn start_job(n: u64, allow: Option<&str>) -> Result<(), String> {
    let script = runner();
    if !script.is_file() {
        return Err(format!("no runner at {}", script.display()));
    }
    // Queued at once on screen; the runner rewrites it as it starts.
    let dir = job_dir(n);
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::remove_file(dir.join("result"));
    let _ = std::fs::write(dir.join("phase"), "1/5 queued\n");
    let _ = std::fs::write(dir.join("log"), "");
    if super::dry_run() {
        let args = job_args(n, allow, true);
        eprintln!(
            "DRY RUN: would start systemd-run --user --unit={} -- {} {}",
            unit(n),
            script.display(),
            args.join(" ")
        );
        return std::process::Command::new(&script)
            .args(&args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string());
    }
    job_unit(n, &script, job_args(n, allow, false))
        .start()
        .map_err(|e| format!("{e} (is #{n} already running?)"))
}

/// Stop the runner for issue `n`: the unit, or the dry run's process.
pub fn stop_job(n: u64) {
    if super::dry_run() {
        if let Some(pid) = std::fs::read_to_string(job_dir(n).join("pid"))
            .ok()
            .and_then(|p| p.trim().parse::<i32>().ok())
        {
            // The runner leads its own process group (run.sh re-execs
            // under setsid), so the build or the agent under it goes too.
            // SAFETY: a plain kill(2) of the group the runner wrote for us.
            unsafe {
                libc::kill(-pid, libc::SIGTERM);
            }
        }
        return;
    }
    if let Err(e) = Unit::user(unit(n)).queue(Verb::Stop) {
        log::warn!("quality: {e}");
    }
}

/// The programs a job needs on this process's PATH (or the user profile),
/// by name, that are not there.
pub fn missing_tools() -> Vec<&'static str> {
    ["claude", "jq", "gh", "nix", "git", "systemd-run"]
        .into_iter()
        .filter(|t| gh::which(t).is_none())
        .collect()
}

// ── GitHub ──────────────────────────────────────────────────────────────

/// The open issues and the runner's PRs. Blocking; call it on a worker.
pub fn fetch() -> Result<(Vec<Issue>, Vec<Pr>), String> {
    if let Some(dir) = fixture() {
        let read = |f: &str| std::fs::read_to_string(dir.join(f)).unwrap_or_default();
        let issues = serde_json::from_str(&read("issues.json")).map_err(|e| e.to_string())?;
        let prs = serde_json::from_str(&read("prs.json")).unwrap_or_default();
        return Ok((issues, prs));
    }
    let issues = gh::issues(None, "open", LIMIT)?;
    let prs = gh::prs(LIMIT)?;
    Ok((issues, prs))
}

/// Where a picture from the asset branch is cached.
fn cached(path: &str) -> PathBuf {
    super::cache_dir("quality").join(path.replace('/', "_"))
}

/// A picture from the asset branch on disk, downloaded the first time.
/// Blocking.
pub fn download(path: &str) -> Result<PathBuf, String> {
    let file = cached(path);
    if file.exists() {
        return Ok(file);
    }
    let bytes = gh::fetch_asset(path)?;
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&file, bytes).map_err(|e| e.to_string())?;
    Ok(file)
}

/// The first before/after pair: the job's own pictures, else the PR's.
pub enum Pair {
    Local(PathBuf, PathBuf),
    /// Paths on the asset branch, for [`download`].
    Remote(String, String),
}

pub fn pair(n: u64, pr: Option<&Pr>) -> Option<Pair> {
    let shots = job_dir(n).join("out/shots");
    let mut befores: Vec<PathBuf> = std::fs::read_dir(&shots)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .is_some_and(|f| f.to_string_lossy().starts_with("before-"))
        })
        .collect();
    befores.sort();
    for before in befores {
        let name = before
            .file_name()?
            .to_string_lossy()
            .replacen("before-", "after-", 1);
        let after = shots.join(name);
        if after.exists() {
            return Some(Pair::Local(before, after));
        }
    }
    let assets = body::parse_assets(&pr?.body);
    Some(Pair::Remote(
        assets.before.first()?.clone(),
        assets.after.first()?.clone(),
    ))
}

/// The checks line: the job's check.txt, else the PR body's.
pub fn checks(n: u64, pr: Option<&Pr>) -> Option<String> {
    std::fs::read_to_string(job_dir(n).join("check.txt"))
        .ok()
        .and_then(|t| checks_line(&t))
        .or_else(|| checks_line(&pr?.body))
}

// ── Merge & apply ───────────────────────────────────────────────────────

/// The nixos checkout nx commits from: `NX_FLAKE`, as nx's lib.sh says.
fn nixos_dir() -> PathBuf {
    std::env::var_os("NX_FLAKE")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(&super::home()).join("nixos"))
}

/// Merge PR `pr`, then start nx. Blocking: a `git status`, the merge, and
/// the unit's start; call it on a worker. `Err` is a refusal in words.
pub fn merge_and_deploy(pr: u64) -> Result<(), String> {
    // nx commits whatever the tree holds; a half-done edit there must not
    // ride along with a merge pressed in a settings tab. Checked before the
    // merge, so a refusal leaves the PR open.
    let flake = nixos_dir();
    let porcelain = std::process::Command::new("git")
        .arg("-C")
        .arg(&flake)
        .args(["status", "--porcelain"])
        .output()
        .map_err(|e| format!("git status in {}: {e}", flake.display()))?;
    if !porcelain.status.success() {
        return Err(format!("{} is not a git repository", flake.display()));
    }
    if let Some(why) = dirty(&String::from_utf8_lossy(&porcelain.stdout)) {
        return Err(format!(
            "{} has uncommitted changes ({why}); commit or stash them, then apply again",
            super::redact(&flake.display().to_string(), &super::home())
        ));
    }
    gh::merge(pr)?;
    deploy()
}

/// Start `nx` in the one nx unit. A dry run plays a canned run into the
/// same log instead, in the runner's own markers.
fn deploy() -> Result<(), String> {
    if !super::dry_run() {
        return nx::start(nx::Action::Update);
    }
    let log = nx::log_path();
    if let Some(dir) = log.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let out = std::fs::File::create(&log).map_err(|e| e.to_string())?;
    let shown = format!(
        "echo '{}{}'; echo 'DRY RUN: nx would run now'; sleep 2; echo '── personal repos: push, fast-forward'; sleep 3; echo '── build every host'; sleep 3; echo '{}0'",
        nx::START,
        nx::Action::Update.arg(),
        nx::EXIT
    );
    std::process::Command::new("sh")
        .args(["-c", &shown])
        .stdout(out.try_clone().map_err(|e| e.to_string())?)
        .stderr(out)
        .spawn()
        .map_err(|e| e.to_string())?;
    eprintln!("DRY RUN: would start {}: nx", nx::UNIT);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_read_the_result_lines() {
        let text = "== cargo test --release\nok\nRESULT tests: pass\n== gate\nRESULT gate: pass\n";
        assert_eq!(checks_line(text).as_deref(), Some("tests pass · gate pass"));
        assert_eq!(checks_line("no results"), None);
    }

    #[test]
    fn the_log_tail_skips_blanks_and_colour() {
        assert_eq!(
            tail("a\n\n\u{1b}[1m── b\u{1b}[0m\n  \nc\n", 2),
            vec!["── b".to_string(), "c".to_string()]
        );
        assert!(tail("", 2).is_empty());
    }

    #[test]
    fn a_dirty_nixos_tree_is_named_briefly() {
        assert_eq!(dirty(""), None);
        assert_eq!(dirty(" M flake.lock\n").as_deref(), Some("flake.lock"));
        assert_eq!(
            dirty(" M a\n?? b\nA  c\n M d\n M e\n").as_deref(),
            Some("a, b, c and 2 more")
        );
    }

    #[test]
    fn a_job_runs_its_issue_in_its_own_unit() {
        assert_eq!(job_args(7, None, false), ["--issue", "7"]);
        assert_eq!(
            job_args(7, Some("ada"), true),
            ["--issue", "7", "--allow-author", "ada", "--dry-run"]
        );
        let args = job_unit(7, Path::new("/r/run.sh"), job_args(7, None, false)).args();
        assert!(args.contains(&"--unit=swaypplet-autofix-7".to_string()));
        let argv = &args[args.iter().position(|a| a == "--").unwrap() + 1..];
        assert_eq!(argv, ["/r/run.sh", "--issue", "7"]);
    }
}
