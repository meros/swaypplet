//! The Quality tab: every open issue on the repository, an Auto-fix button
//! on each, the fix's progress, and Merge & apply for one that is ready.
//!
//! Nothing works on an issue until Auto-fix is pressed. The press starts
//! `dev/autofix/run.sh --issue <n>` as a transient user unit
//! (`swaypplet-autofix-<n>`), not as a child of this process, so it
//! survives the panel restarting; jobs queue on the runner's lock. The
//! runner's files (`status::Job`) are read every two seconds while the tab
//! is open, which is what the phase line, the log tail and Stop follow.
//!
//! GitHub is asked only while the tab is on screen: once when it maps
//! (unless the last answer is under a minute old), then once a minute. One
//! fetch is two list calls (open issues, the runner's PRs); pictures for a
//! ready fix come from the job's own folder, or once from the asset branch.
//!
//! An issue someone else filed asks before it runs: its text is the
//! agent's input, and anyone can write one on a public repository.
//!
//! Merge & apply squash-merges the PR and runs `nx` as its own unit
//! (`swaypplet-deploy`), because nx switches home-manager, which restarts
//! this process; its log is followed here, and a reopened tab shows how it
//! ended. "Try locally" is not offered: `nx dev` rebuilds the system with
//! sudo, which prompts for a password that nothing here can answer.
//!
//! `SWAYPPLET_QUALITY_FIXTURE=<dir>` reads `issues.json`, `prs.json`, the
//! jobs (`jobs/<n>/`) and the pictures from a directory, for the harness.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk4::prelude::*;

use super::form::{self, section_box};
use crate::quality::body;
use crate::quality::status::{self, Fix, Issue, Job, Pr};
use crate::ui::{self, Text, Tone};

/// How often GitHub is asked while the tab is open.
const POLL: Duration = Duration::from_secs(60);

/// How often the local jobs are read while the tab is open.
const TICK: Duration = Duration::from_secs(2);

/// How many open issues the tab lists.
const LIMIT: u32 = 30;

/// The systemd unit a deploy runs in.
const DEPLOY_UNIT: &str = "swaypplet-deploy";

/// How long an armed button waits for its second press.
const ARMED: Duration = Duration::from_secs(6);

/// A thumbnail's size, in logical pixels.
const THUMB_W: i32 = 240;
const THUMB_H: i32 = 150;

// ── Pure parts ──────────────────────────────────────────────────────────

/// The tone of a fix's status.
pub fn tone(f: &Fix) -> ui::Status {
    match f {
        Fix::None | Fix::Closed | Fix::Stopped | Fix::DryRun => ui::Status::Neutral,
        // In flight is not a warning; the accent phase line under it says
        // what is happening. --warning is not held to a text contrast
        // through the glass (tokens/apca.rs); --fg-muted is.
        Fix::Queued | Fix::Working(_) => ui::Status::Neutral,
        Fix::Ready | Fix::Merged => ui::Status::Success,
        Fix::Failed(_) => ui::Status::Danger,
    }
}

/// "3 min ago", "5 h ago", "2 d ago" from two Unix times.
pub fn age(now: i64, then: i64) -> String {
    let s = (now - then).max(0);
    match s {
        0..60 => "just now".into(),
        60..3600 => format!("{} min ago", s / 60),
        3600..86_400 => format!("{} h ago", s / 3600),
        _ => format!("{} d ago", s / 86_400),
    }
}

/// `tests pass · gate pass` from check.sh's `RESULT` lines.
pub fn checks_line(text: &str) -> Option<String> {
    let parts: Vec<String> = text
        .lines()
        .filter_map(|l| l.trim().strip_prefix("RESULT "))
        .map(|r| r.replace(':', "").to_lowercase())
        .collect();
    (!parts.is_empty()).then(|| parts.join(" \u{00b7} "))
}

/// The deploy log's last line, and the exit code once the wrapper wrote it.
pub fn deploy_state(log: &str) -> (String, Option<i32>) {
    let mut last = String::new();
    let mut code = None;
    for line in log.lines().map(strip_ansi) {
        let line = line.trim().to_string();
        if let Some(c) = line.strip_prefix("deploy-exit: ") {
            code = c.trim().parse().ok();
        } else if !line.is_empty() {
            last = line;
        }
    }
    (last, code)
}

/// The last `n` non-empty lines of a log, without colour codes.
pub fn tail(log: &str, n: usize) -> Vec<String> {
    let lines: Vec<String> = log
        .lines()
        .map(strip_ansi)
        .map(|l| l.trim_end().to_string())
        .filter(|l| !l.trim().is_empty())
        .collect();
    lines[lines.len().saturating_sub(n)..].to_vec()
}

/// "3/5 agent working" → "Step 3 of 5: agent working".
pub fn phase_text(phase: &str) -> String {
    match phase.split_once(' ') {
        Some((step, what)) => match step.split_once('/') {
            Some((a, b)) => format!("Step {a} of {b}: {what}"),
            None => phase.to_string(),
        },
        None => phase.to_string(),
    }
}

/// nx colours its step headers; the tab shows words.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

// ── Where things are ────────────────────────────────────────────────────

fn fixture() -> Option<PathBuf> {
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
                .unwrap_or_else(|| Path::new(&crate::quality::home()).join(".local/state"));
            base.join("swaypplet-autofix")
        })
}

fn job_dir(n: u64) -> PathBuf {
    runner_state().join("jobs").join(n.to_string())
}

/// The job for issue `n`, if one ever ran here.
fn read_job(n: u64) -> Option<Job> {
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
            Path::new(&crate::quality::home()).join("git/personal/swaypplet/dev/autofix/run.sh")
        })
}

fn unit(n: u64) -> String {
    format!("swaypplet-autofix-{n}")
}

/// Start the runner for issue `n`, as a unit (or, in a dry run, as a plain
/// child with `--dry-run`, so the user's systemd is left alone).
fn start_job(n: u64, allow: Option<&str>) -> Result<(), String> {
    let script = runner();
    if !script.is_file() {
        return Err(format!("no runner at {}", script.display()));
    }
    let mut args = vec!["--issue".to_string(), n.to_string()];
    if let Some(author) = allow {
        args.push("--allow-author".into());
        args.push(author.to_string());
    }
    // Queued at once on screen; the runner rewrites it as it starts.
    let dir = job_dir(n);
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::remove_file(dir.join("result"));
    let _ = std::fs::write(dir.join("phase"), "1/5 queued\n");
    let _ = std::fs::write(dir.join("log"), "");
    if crate::quality::dry_run() {
        args.push("--dry-run".into());
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
    let mut command = std::process::Command::new("systemd-run");
    command.args([
        "--user",
        "--collect",
        "--quiet",
        &format!("--unit={}", unit(n)),
    ]);
    // What the runner needs from this session: the profile's PATH (git,
    // gh, nix, claude, jq), the home, and the SSH agent for the push. The
    // runner takes the agent socket and the GitHub token away from claude.
    for var in [
        "PATH",
        "HOME",
        "USER",
        "SSH_AUTH_SOCK",
        "NIX_PATH",
        "SWAYPPLET_SWAY",
    ] {
        if let Ok(value) = std::env::var(var) {
            command.arg(format!("--setenv={var}={value}"));
        }
    }
    let status = command
        .arg("--")
        .arg(&script)
        .args(&args)
        .status()
        .map_err(|e| format!("systemd-run: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("systemd-run: {status} (is #{n} already running?)"))
    }
}

/// Stop the runner for issue `n`: the unit, or the dry run's process.
fn stop_job(n: u64) {
    if crate::quality::dry_run() {
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
    let _ = std::process::Command::new("systemctl")
        .args(["--user", "stop", "--no-block", &unit(n)])
        .status();
}

/// The programs a job needs on this process's PATH (or the user profile),
/// by name, that are not there.
fn missing_tools() -> Vec<&'static str> {
    ["claude", "jq", "gh", "nix", "git", "systemd-run"]
        .into_iter()
        .filter(|t| crate::quality::gh::which(t).is_none())
        .collect()
}

/// What GitHub says, fetched on a worker thread.
fn fetch() -> Result<(Vec<Issue>, Vec<Pr>), String> {
    if let Some(dir) = fixture() {
        let read = |f: &str| std::fs::read_to_string(dir.join(f)).unwrap_or_default();
        let issues = serde_json::from_str(&read("issues.json")).map_err(|e| e.to_string())?;
        let prs = serde_json::from_str(&read("prs.json")).unwrap_or_default();
        return Ok((issues, prs));
    }
    let issues = crate::quality::gh::issues(None, "open", LIMIT)?;
    let prs = crate::quality::gh::prs(LIMIT)?;
    Ok((issues, prs))
}

/// Where a picture from the asset branch is cached.
fn cached(path: &str) -> PathBuf {
    crate::quality::cache_dir("quality").join(path.replace('/', "_"))
}

/// A picture from the asset branch on disk, downloaded the first time.
fn download(path: &str) -> Result<PathBuf, String> {
    let file = cached(path);
    if file.exists() {
        return Ok(file);
    }
    let bytes = crate::quality::gh::fetch_asset(path)?;
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&file, bytes).map_err(|e| e.to_string())?;
    Ok(file)
}

/// The first before/after pair: the job's own pictures, else the PR's.
enum Pair {
    Local(PathBuf, PathBuf),
    Remote(String, String),
}

fn pair(n: u64, pr: Option<&Pr>) -> Option<Pair> {
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
fn checks(n: u64, pr: Option<&Pr>) -> Option<String> {
    std::fs::read_to_string(job_dir(n).join("check.txt"))
        .ok()
        .and_then(|t| checks_line(&t))
        .or_else(|| checks_line(&pr?.body))
}

// ── The tab ─────────────────────────────────────────────────────────────

/// One issue's block, and the parts that change while it is on screen.
struct Entry {
    issue: Issue,
    pr: Option<Pr>,
    fix: RefCell<Option<Fix>>,
    chip: gtk4::Label,
    /// Rebuilt when the fix's state changes.
    detail: gtk4::Box,
    /// Updated in place on every tick while a job runs.
    phase: RefCell<Option<gtk4::Label>>,
    log: RefCell<Option<gtk4::Label>>,
}

struct State {
    list: gtk4::Box,
    status: gtk4::Label,
    entries: RefCell<Vec<Rc<Entry>>>,
    deploy: gtk4::Box,
    deploy_line: gtk4::Label,
    deploy_bar: gtk4::ProgressBar,
    last_fetch: Cell<Option<Instant>>,
    fetching: Cell<bool>,
    poll: RefCell<Option<glib::SourceId>>,
    tick: RefCell<Option<glib::SourceId>>,
    follow: RefCell<Option<glib::SourceId>>,
    deploying: Cell<bool>,
    /// Whether every program a job needs was found at the last fetch.
    tools_ok: Cell<bool>,
}

pub struct QualityPane {
    root: gtk4::Box,
    state: Rc<State>,
}

impl QualityPane {
    pub fn new() -> Self {
        let root = form::pane();

        let group = section_box(
            "Issues",
            &format!(
                "The open issues on {}. Auto-fix starts an agent on this machine for one issue: it reproduces the problem in the nested harness, fixes it, runs the tests and the frame gate, and opens a PR.",
                crate::quality::repo()
            ),
        );
        let list = ui::vbox(0);
        group.append(&list);

        let deploy = section_box(
            "Deploy",
            "nx after a merge: the personal repos, the flake lock, the build, the switch.",
        );
        let deploy_bar = ui::progress(0.0);
        let deploy_line = ui::text("", Text::Caption, Tone::Muted);
        deploy_line.set_xalign(0.0);
        deploy_line.set_wrap(true);
        deploy_line.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
        ui::set_mono(&deploy_line, true);
        deploy.append(&deploy_bar);
        deploy.append(&deploy_line);
        deploy.set_visible(false);

        let refresh = form::action_button("Refresh", "Ask GitHub now.");
        let file = form::action_button(
            "Report a problem",
            "Close the panel and start a report, as the key binding does.",
        );
        let (footer, status) = form::footer(&[&refresh, &file]);

        root.append(&group);
        root.append(&deploy);
        root.append(&footer);

        let state = Rc::new(State {
            list,
            status,
            entries: RefCell::new(Vec::new()),
            deploy,
            deploy_line,
            deploy_bar,
            last_fetch: Cell::new(None),
            fetching: Cell::new(false),
            poll: RefCell::new(None),
            tick: RefCell::new(None),
            follow: RefCell::new(None),
            deploying: Cell::new(false),
            tools_ok: Cell::new(true),
        });
        state.status.set_text("Not fetched yet");
        form::mark_source(&state.status, true);

        {
            let state = state.clone();
            refresh.connect_clicked(move |_| fetch_now(&state));
        }
        file.connect_clicked(|_| {
            // Out of the way first (SIGUSR1 is the panel's own toggle, and
            // the panel is open), then the report the way the binding
            // starts it, once the panel has faded and cannot be in the shot.
            // SAFETY: raising a signal this process handles (app.rs).
            unsafe {
                libc::raise(libc::SIGUSR1);
            }
            glib::timeout_add_local_once(Duration::from_millis(400), || {
                let exe = std::env::current_exe().unwrap_or_else(|_| "swaypplet".into());
                if let Err(e) = std::process::Command::new(exe).arg("report").spawn() {
                    log::warn!("quality: could not start a report: {e}");
                }
            });
        });

        // GitHub and the jobs, only while the tab is on screen.
        {
            let state = state.clone();
            root.connect_map(move |_| {
                let stale = state.last_fetch.get().is_none_or(|t| t.elapsed() >= POLL);
                if stale {
                    fetch_now(&state);
                }
                let s = state.clone();
                let id = glib::timeout_add_local(POLL, move || {
                    fetch_now(&s);
                    glib::ControlFlow::Continue
                });
                if let Some(old) = state.poll.replace(Some(id)) {
                    crate::spawn::remove_source(old);
                }
                let s = state.clone();
                let id = glib::timeout_add_local(TICK, move || {
                    tick(&s);
                    glib::ControlFlow::Continue
                });
                if let Some(old) = state.tick.replace(Some(id)) {
                    crate::spawn::remove_source(old);
                }
                follow_deploy(&state);
            });
        }
        {
            let state = state.clone();
            root.connect_unmap(move |_| {
                for slot in [&state.poll, &state.tick, &state.follow] {
                    if let Some(id) = slot.take() {
                        crate::spawn::remove_source(id);
                    }
                }
            });
        }

        // Harness hook: the nested session has no pointer, so
        // `SWAYPPLET_QUALITY_APPLY=<pr>` presses Merge & apply for that PR
        // two seconds in. Only in a dry run: it must never merge for real.
        if crate::quality::dry_run()
            && let Some(pr) = std::env::var("SWAYPPLET_QUALITY_APPLY")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
        {
            let state = state.clone();
            glib::timeout_add_local_once(Duration::from_secs(2), move || apply_fix(&state, pr));
        }

        QualityPane { root, state }
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    /// Nothing to re-read from the settings file: the list follows GitHub
    /// and the jobs while mapped.
    pub fn refresh(&self) {
        tick(&self.state);
    }
}

fn fetch_now(state: &Rc<State>) {
    if state.fetching.replace(true) {
        return;
    }
    state.status.set_text("Asking GitHub\u{2026}");
    let state = state.clone();
    crate::spawn::spawn_work(fetch, move |result| {
        state.fetching.set(false);
        state.last_fetch.set(Some(Instant::now()));
        let when = glib::DateTime::now_local()
            .and_then(|t| t.format("%H:%M"))
            .map(|s| s.to_string())
            .unwrap_or_default();
        match result {
            Ok((issues, prs)) => {
                state.status.set_text(&format!(
                    "{} open {} · updated {when}, again in a minute while this tab is open",
                    issues.len(),
                    if issues.len() == 1 { "issue" } else { "issues" }
                ));
                form::mark_source(&state.status, true);
                fill(&state, issues, &prs);
            }
            Err(e) => {
                log::warn!("quality: {e}");
                state.status.set_text(&format!(
                    "Could not reach GitHub at {when}: {}",
                    body::cut(&e, 80)
                ));
                form::mark_source(&state.status, false);
            }
        }
    });
}

fn fill(state: &Rc<State>, issues: Vec<Issue>, prs: &[Pr]) {
    while let Some(child) = state.list.first_child() {
        state.list.remove(&child);
    }
    state.entries.borrow_mut().clear();
    // Checked here rather than by a job that would fail at its first step.
    let missing = missing_tools();
    if !missing.is_empty() {
        let note = caption(
            &format!("Auto-fix needs: {}", missing.join(", ")),
            Tone::Muted,
        );
        ui::set_weight(&note, ui::Weight::Strong);
        state.list.append(&note);
    }
    state.tools_ok.set(missing.is_empty());
    if issues.is_empty() {
        let empty = ui::text(
            "No open issues. Report a problem, and it shows here.",
            Text::Caption,
            Tone::Faint,
        );
        empty.set_xalign(0.0);
        state.list.append(&empty);
        return;
    }
    let now = glib::DateTime::now_utc().map(|t| t.to_unix()).unwrap_or(0);
    for (i, issue) in issues.into_iter().enumerate() {
        if i > 0 {
            state
                .list
                .append(&ui::separator(gtk4::Orientation::Horizontal));
        }
        let pr = prs
            .iter()
            .find(|p| p.issue() == Some(issue.number))
            .cloned();
        let (widget, entry) = entry_widget(issue, pr, now);
        state.list.append(&widget);
        state.entries.borrow_mut().push(entry);
    }
    tick(state);
}

fn entry_widget(issue: Issue, pr: Option<Pr>, now: i64) -> (gtk4::Box, Rc<Entry>) {
    let b = ui::vbox(2);
    b.add_css_class("quality-entry");

    let head = ui::hbox(3);
    let chip = ui::status(ui::Status::Neutral, "Open");
    // On the title's baseline: the chip's type is smaller and bolder.
    chip.set_valign(gtk4::Align::BaselineCenter);
    head.append(&chip);
    let title = ui::text(&issue.title, Text::Body, Tone::Fg);
    title.set_xalign(0.0);
    title.set_hexpand(true);
    title.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    title.set_tooltip_text(Some(&issue.title));
    title.set_valign(gtk4::Align::BaselineCenter);
    head.append(&title);
    let number = format!("#{}", issue.number);
    let open = ui::button_with(ui::Face::Label(&number), ui::Kind::Flat, ui::Size::Small);
    open.set_valign(gtk4::Align::BaselineCenter);
    open.set_tooltip_text(Some("Open the issue on GitHub"));
    {
        let url = issue.url.clone();
        open.connect_clicked(move |_| crate::quality::report::open_uri(&url));
    }
    head.append(&open);
    b.append(&head);

    let created = glib::DateTime::from_iso8601(&issue.created_at, None)
        .map(|t| t.to_unix())
        .unwrap_or(now);
    let mut facts = vec![format!("@{}", issue.author.login)];
    let labels: Vec<&str> = issue.labels.iter().map(|l| l.name.as_str()).collect();
    if !labels.is_empty() {
        facts.push(labels.join(", "));
    }
    facts.push(age(now, created));
    if let Some(pr) = &pr {
        facts.push(format!("PR #{}", pr.number));
    }
    let meta = ui::text(&facts.join(" \u{00b7} "), Text::Caption, Tone::Muted);
    meta.set_xalign(0.0);
    meta.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    b.append(&meta);

    let detail = ui::vbox(2);
    b.append(&detail);

    let entry = Rc::new(Entry {
        issue,
        pr,
        fix: RefCell::new(None),
        chip,
        detail,
        phase: RefCell::new(None),
        log: RefCell::new(None),
    });
    (b, entry)
}

/// Read the jobs; rebuild an entry's detail when its state changed, and
/// move its phase and log lines when it only progressed.
fn tick(state: &Rc<State>) {
    let entries = state.entries.borrow().clone();
    for entry in entries {
        let job = read_job(entry.issue.number);
        let fix = status::fix(job.as_ref(), entry.pr.as_ref());
        let same_kind = entry
            .fix
            .borrow()
            .as_ref()
            .is_some_and(|old| std::mem::discriminant(old) == std::mem::discriminant(&fix));
        if same_kind {
            if let (Fix::Working(phase), Some(label)) = (&fix, &*entry.phase.borrow()) {
                label.set_text(&phase_text(phase));
            }
            if let (Some(job), Some(label)) = (&job, &*entry.log.borrow()) {
                label.set_text(&job.tail.join("\n"));
            }
            continue;
        }
        entry.chip.set_text(&format!("\u{25cf} {}", fix.label()));
        ui::set_status(&entry.chip, tone(&fix));
        build_detail(state, &entry, &fix, job.as_ref());
        entry.fix.replace(Some(fix));
    }
}

fn caption(text: &str, tone: Tone) -> gtk4::Label {
    let l = ui::text(text, Text::Caption, tone);
    l.set_xalign(0.0);
    l
}

fn build_detail(state: &Rc<State>, entry: &Rc<Entry>, fix: &Fix, job: Option<&Job>) {
    let d = &entry.detail;
    while let Some(child) = d.first_child() {
        d.remove(&child);
    }
    entry.phase.replace(None);
    entry.log.replace(None);
    let n = entry.issue.number;
    let actions = ui::hbox(3);
    actions.set_halign(gtk4::Align::Start);

    match fix {
        Fix::Queued | Fix::Working(_) => {
            let line = caption(
                &match fix {
                    Fix::Working(p) => phase_text(p),
                    _ => "Queued behind the job that is running".into(),
                },
                Tone::Accent,
            );
            d.append(&line);
            entry.phase.replace(Some(line));
            let log = caption(
                &job.map(|j| j.tail.join("\n")).unwrap_or_default(),
                Tone::Muted,
            );
            ui::set_mono(&log, true);
            log.set_ellipsize(gtk4::pango::EllipsizeMode::End);
            log.set_lines(2);
            d.append(&log);
            entry.log.replace(Some(log));
            let stop = ui::button("Stop", ui::Kind::Secondary);
            stop.set_tooltip_text(Some("Stop the agent and throw its work away"));
            stop.connect_clicked(move |b| {
                b.set_sensitive(false);
                stop_job(n);
            });
            actions.append(&stop);
        }
        Fix::Ready => {
            if let Some(pair) = pair(n, entry.pr.as_ref()) {
                let row = ui::hbox(3);
                let (before, after) = match pair {
                    Pair::Local(b, a) => (thumb_local("Before", &b), thumb_local("After", &a)),
                    Pair::Remote(b, a) => (thumb_remote("Before", &b), thumb_remote("After", &a)),
                };
                row.append(&before);
                row.append(&after);
                d.append(&row);
            }
            if let Some(line) = checks(n, entry.pr.as_ref()) {
                d.append(&caption(&line, Tone::Success));
            }
            let url = entry
                .pr
                .as_ref()
                .map(|p| p.url.clone())
                .or_else(|| job.and_then(status::ready_url).map(str::to_string));
            if let Some(url) = url {
                let view = ui::button("View PR", ui::Kind::Secondary);
                view.connect_clicked(move |_| crate::quality::report::open_uri(&url));
                actions.append(&view);
            }
            if let Some(pr) = entry.pr.as_ref().map(|p| p.number) {
                let close = ui::button("Close", ui::Kind::Secondary);
                close.set_tooltip_text(Some("Close the PR without merging, and delete its branch"));
                {
                    let state = state.clone();
                    arm(&close, "Close the PR?", move |b| {
                        b.set_sensitive(false);
                        let state = state.clone();
                        crate::spawn::spawn_work(
                            move || crate::quality::gh::close_pr(pr),
                            move |r| {
                                if let Err(e) = r {
                                    log::warn!("quality: {e}");
                                }
                                fetch_now(&state);
                            },
                        );
                    });
                }
                actions.append(&close);
                let apply = ui::button("Merge & apply", ui::Kind::Primary);
                apply.set_tooltip_text(Some(
                    "Squash-merge the PR, then run nx: build, switch, push",
                ));
                apply.set_sensitive(!state.deploying.get());
                {
                    let state = state.clone();
                    arm(&apply, "Merge and deploy?", move |_| apply_fix(&state, pr));
                }
                actions.append(&apply);
            }
        }
        Fix::Failed(why) => {
            let line = caption(
                &format!("The agent gave up: {}", body::cut(why, 140)),
                Tone::Danger,
            );
            line.set_wrap(true);
            line.set_max_width_chars(form::HINT_CHARS);
            d.append(&line);
            actions.append(&autofix_button(state, entry, "Try again"));
        }
        Fix::Merged => {}
        Fix::None | Fix::Stopped | Fix::DryRun | Fix::Closed => {
            if status::needs_confirm(&entry.issue, &crate::quality::owner()) {
                let note = caption(
                    &format!(
                        "Filed by @{}, not you: its text becomes the agent's input, so Auto-fix asks first.",
                        entry.issue.author.login
                    ),
                    Tone::Muted,
                );
                // Muted, which is held to its contrast through the glass
                // over any backdrop; the weight carries the caution.
                ui::set_weight(&note, ui::Weight::Strong);
                d.append(&note);
            }
            if let Fix::DryRun = fix {
                d.append(&caption(
                    "Dry run: stopped where the agent would start",
                    Tone::Muted,
                ));
            }
            actions.append(&autofix_button(state, entry, "Auto-fix"));
        }
    }
    if actions.first_child().is_some() {
        d.append(&actions);
    }
    d.set_visible(d.first_child().is_some());
}

/// Auto-fix, asking first for an issue the owner did not write.
fn autofix_button(state: &Rc<State>, entry: &Rc<Entry>, label: &str) -> gtk4::Button {
    let button = ui::button(label, ui::Kind::Secondary);
    if !state.tools_ok.get() {
        button.set_sensitive(false);
        button.set_tooltip_text(Some("A program Auto-fix needs is missing; see above"));
        return button;
    }
    let n = entry.issue.number;
    let author = entry.issue.author.login.clone();
    let stranger = status::needs_confirm(&entry.issue, &crate::quality::owner());
    button.set_tooltip_text(Some(&if stranger {
        format!("Filed by @{author}, not you: its text becomes the agent's input. Asks first.")
    } else {
        "Start an agent on this issue, on this machine".to_string()
    }));
    let state = state.clone();
    let prompt = format!("Run on @{author}'s text?");
    let go = move |b: &gtk4::Button| {
        b.set_sensitive(false);
        let allow = stranger.then(|| author.clone());
        if let Err(e) = start_job(n, allow.as_deref()) {
            log::warn!("quality: {e}");
            state.status.set_text(&format!("#{n} did not start: {e}"));
            form::mark_source(&state.status, false);
            b.set_sensitive(true);
        }
        tick(&state);
    };
    if stranger {
        arm(&button, &prompt, go);
    } else {
        button.connect_clicked(go);
    }
    button
}

/// A button that asks on its first press (its label becomes `question`,
/// armed) and acts on a second within [`ARMED`].
fn arm(button: &gtk4::Button, question: &str, act: impl Fn(&gtk4::Button) + 'static) {
    let armed = Rc::new(Cell::new(false));
    let question = question.to_string();
    let label = button.label().map(|l| l.to_string()).unwrap_or_default();
    button.connect_clicked(move |b| {
        if !armed.replace(true) {
            b.set_label(&question);
            ui::set_armed(b, true);
            let b = b.clone();
            let armed = armed.clone();
            let label = label.clone();
            glib::timeout_add_local_once(ARMED, move || {
                if armed.replace(false) {
                    b.set_label(&label);
                    ui::set_armed(&b, false);
                }
            });
            return;
        }
        armed.set(false);
        ui::set_armed(b, false);
        b.set_label(&label);
        act(b);
    });
}

fn thumb_frame(label: &str) -> (gtk4::Box, gtk4::Picture) {
    let b = ui::vbox(1);
    let frame = ui::thumb();
    frame.set_halign(gtk4::Align::Start);
    let pic = gtk4::Picture::new();
    // Cover, so the picture reaches the frame's rounded corners.
    pic.set_content_fit(gtk4::ContentFit::Cover);
    pic.set_can_shrink(true);
    pic.set_size_request(THUMB_W, THUMB_H);
    frame.append(&pic);
    b.append(&frame);
    b.append(&caption(label, Tone::Muted));
    (b, pic)
}

fn thumb_local(label: &str, file: &Path) -> gtk4::Box {
    let (b, pic) = thumb_frame(label);
    show_thumb(&pic, file);
    b
}

fn thumb_remote(label: &str, path: &str) -> gtk4::Box {
    let (b, pic) = thumb_frame(label);
    let path = path.to_string();
    crate::spawn::spawn_work(
        move || download(&path),
        move |got| match got {
            Ok(file) => show_thumb(&pic, &file),
            Err(e) => log::warn!("quality: picture: {e}"),
        },
    );
    b
}

/// The picture scaled down on load to the thumbnail's size: a `Picture`
/// asks for its paintable's size, so a full screenshot (or a 2× one) would
/// take the whole row.
fn show_thumb(pic: &gtk4::Picture, file: &Path) {
    match gtk4::gdk_pixbuf::Pixbuf::from_file_at_scale(file, THUMB_W, THUMB_H, true) {
        Ok(pixbuf) => pic.set_paintable(Some(&gtk4::gdk::Texture::for_pixbuf(&pixbuf))),
        Err(e) => log::warn!("quality: {}: {e}", file.display()),
    }
}

// ── Merge & apply ───────────────────────────────────────────────────────

fn deploy_log() -> PathBuf {
    crate::quality::state_dir("deploy.log")
}

fn set_deploying(state: &Rc<State>, on: bool) {
    state.deploying.set(on);
    // The ready entries rebuild with Merge & apply enabled or not.
    for entry in state.entries.borrow().iter() {
        if matches!(*entry.fix.borrow(), Some(Fix::Ready)) {
            entry.fix.replace(None);
        }
    }
    tick(state);
}

/// Merge, then deploy. Everything after the click is on a worker or in the
/// deploy unit.
fn apply_fix(state: &Rc<State>, pr: u64) {
    set_deploying(state, true);
    state.deploy.set_visible(true);
    state.deploy_bar.set_visible(true);
    ui::set_mono(&state.deploy_line, true);
    ui::set_tone(&state.deploy_line, Tone::Muted);
    ui::set_weight(&state.deploy_line, ui::Weight::Regular);
    state
        .deploy_line
        .set_text(&format!("Merging PR #{pr}\u{2026}"));
    ui::set_progress_status(&state.deploy_bar, None);
    let state = state.clone();
    crate::spawn::spawn_work(
        move || {
            // nx commits whatever the tree holds; a half-done edit there
            // must not ride along with a merge pressed in a settings tab.
            // Checked before the merge, so a refusal leaves the PR open.
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
                    form::pretty_path(&flake)
                ));
            }
            crate::quality::gh::merge(pr)?;
            start_deploy(&deploy_log())
        },
        move |result| match result {
            Ok(()) => follow_deploy(&state),
            Err(e) => {
                // A refusal, not a run: plain words, no bar. Muted and strong
                // rather than --warning, which is not held to a text
                // contrast through the glass.
                state.deploy_bar.set_visible(false);
                ui::set_mono(&state.deploy_line, false);
                ui::set_tone(&state.deploy_line, Tone::Muted);
                ui::set_weight(&state.deploy_line, ui::Weight::Strong);
                state
                    .deploy_line
                    .set_text(&format!("Not applied: {}", body::cut(&e, 200)));
                set_deploying(&state, false);
            }
        },
    );
}

/// The nixos checkout nx commits from: `NX_FLAKE`, as nx's lib.sh says.
fn nixos_dir() -> PathBuf {
    std::env::var_os("NX_FLAKE")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(&crate::quality::home()).join("nixos"))
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

/// The deploy command: `SWAYPPLET_DEPLOY` (a shell command line), or `nx`.
fn deploy_command() -> Result<String, String> {
    if let Ok(cmd) = std::env::var("SWAYPPLET_DEPLOY")
        && !cmd.trim().is_empty()
    {
        return Ok(cmd);
    }
    crate::quality::gh::which("nx")
        .map(|p| shell_quote(&p.to_string_lossy()))
        .ok_or_else(|| "nx is not on PATH or in the user profile".to_string())
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Start the deploy, its output into `log`, ending with `deploy-exit: N`.
fn start_deploy(log: &Path) -> Result<(), String> {
    if let Some(dir) = log.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(log, "").map_err(|e| e.to_string())?;
    let cmd = deploy_command()?;
    let script = format!("{cmd}; echo \"deploy-exit: $?\"");
    if crate::quality::dry_run() {
        // Not a unit: a dry run leaves the user's systemd alone, and says
        // what it would have started.
        let shown = "echo 'DRY RUN: nx would run now'; sleep 2; echo '── personal repos: push, fast-forward'; sleep 3; echo '── build every host'; sleep 3; echo 'deploy-exit: 0'";
        let out = std::fs::OpenOptions::new()
            .append(true)
            .open(log)
            .map_err(|e| e.to_string())?;
        std::process::Command::new("sh")
            .args(["-c", shown])
            .stdout(out.try_clone().map_err(|e| e.to_string())?)
            .stderr(out)
            .spawn()
            .map_err(|e| e.to_string())?;
        eprintln!("DRY RUN: would start {DEPLOY_UNIT}: sh -c {script}");
        return Ok(());
    }
    let log = log.to_string_lossy();
    let mut command = std::process::Command::new("systemd-run");
    command.args([
        "--user",
        "--collect",
        "--quiet",
        &format!("--unit={DEPLOY_UNIT}"),
        &format!("--property=StandardOutput=append:{log}"),
        &format!("--property=StandardError=append:{log}"),
    ]);
    // What nx needs from this session: the profile's PATH (git, nix, gh),
    // the SSH agent for pushing the personal repos, and the home.
    for var in ["PATH", "HOME", "SSH_AUTH_SOCK", "USER", "NIX_PATH"] {
        if let Ok(value) = std::env::var(var) {
            command.arg(format!("--setenv={var}={value}"));
        }
    }
    let status = command
        .args(["--", "sh", "-c", &script])
        .status()
        .map_err(|e| format!("systemd-run: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "systemd-run: {status} (is a deploy already running?)"
        ))
    }
}

/// Follow the deploy log once a second while the tab is open.
fn follow_deploy(state: &Rc<State>) {
    let log = deploy_log();
    // A log from over an hour ago is history, not a deploy to show.
    let recent = std::fs::metadata(&log)
        .and_then(|m| m.modified())
        .is_ok_and(|t| t.elapsed().is_ok_and(|a| a < Duration::from_secs(3600)));
    if !recent {
        return;
    }
    state.deploy.set_visible(true);
    let s = state.clone();
    let tick = move || {
        let text = std::fs::read_to_string(&log).unwrap_or_default();
        let (last, code) = deploy_state(&text);
        match code {
            None => {
                s.deploy_line.set_text(if last.is_empty() {
                    "Starting\u{2026}"
                } else {
                    &last
                });
                s.deploy_bar.pulse();
                glib::ControlFlow::Continue
            }
            Some(code) => {
                s.deploy_bar.set_fraction(1.0);
                if code == 0 {
                    s.deploy_line.set_text(&format!(
                        "Deployed. Last step: {}",
                        last.trim_start_matches("\u{2500}\u{2500} ")
                    ));
                    ui::set_progress_status(&s.deploy_bar, Some(ui::Status::Success));
                } else {
                    s.deploy_line.set_text(&format!(
                        "nx failed ({code}) at: {}",
                        last.trim_start_matches("\u{2500}\u{2500} ")
                    ));
                    ui::set_progress_status(&s.deploy_bar, Some(ui::Status::Danger));
                }
                s.follow.replace(None);
                if s.deploying.get() {
                    set_deploying(&s, false);
                }
                glib::ControlFlow::Break
            }
        }
    };
    if tick() == glib::ControlFlow::Break {
        return;
    }
    let id = glib::timeout_add_local(Duration::from_secs(1), tick);
    if let Some(old) = state.follow.replace(Some(id)) {
        crate::spawn::remove_source(old);
    }
}

/// The tab in the settings search (`search.rs`): its groups, which have no
/// settings rows of their own.
#[rustfmt::skip]
pub(super) const SEARCH: &[super::search::Entry] = &[
    super::search::row("Issues", "", "The open issues, and Auto-fix: an agent that prepares a PR for one", &["bug", "report", "crash", "auto-fix", "autofix", "fix", "issue", "github", "quality", "pr", "pull request", "agent", "claude"]),
    super::search::row("Deploy", "", "Merge a ready fix and run nx: build, switch, push", &["merge", "apply", "nx", "deploy", "rebuild"]),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_fix_has_a_tone() {
        assert_eq!(tone(&Fix::None), ui::Status::Neutral);
        assert_eq!(tone(&Fix::Queued), ui::Status::Neutral);
        assert_eq!(tone(&Fix::Working("3/5".into())), ui::Status::Neutral);
        assert_eq!(tone(&Fix::Ready), ui::Status::Success);
        assert_eq!(tone(&Fix::Failed(String::new())), ui::Status::Danger);
        assert_eq!(tone(&Fix::Merged), ui::Status::Success);
        assert_eq!(tone(&Fix::Closed), ui::Status::Neutral);
    }

    #[test]
    fn ages_pick_one_unit() {
        assert_eq!(age(100, 100), "just now");
        assert_eq!(age(100, 200), "just now");
        assert_eq!(age(1000, 100), "15 min ago");
        assert_eq!(age(7300, 100), "2 h ago");
        assert_eq!(age(3 * 86_400 + 5, 0), "3 d ago");
    }

    #[test]
    fn checks_read_the_result_lines() {
        let text = "== cargo test --release\nok\nRESULT tests: pass\n== gate\nRESULT gate: pass\n";
        assert_eq!(checks_line(text).as_deref(), Some("tests pass · gate pass"));
        assert_eq!(checks_line("no results"), None);
    }

    #[test]
    fn a_phase_reads_as_a_step() {
        assert_eq!(
            phase_text("3/5 agent working"),
            "Step 3 of 5: agent working"
        );
        assert_eq!(phase_text("odd"), "odd");
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
    fn the_deploy_log_ends_with_its_exit() {
        let (last, code) = deploy_state("\u{1b}[1m── build\u{1b}[0m\nbuilding x\n");
        assert_eq!((last.as_str(), code), ("building x", None));
        let (last, code) = deploy_state("── switch\nswitched\ndeploy-exit: 0\n");
        assert_eq!((last.as_str(), code), ("switched", Some(0)));
        assert_eq!(deploy_state("deploy-exit: 1").1, Some(1));
    }
}
