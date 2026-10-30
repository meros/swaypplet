//! The Quality tab: every open issue on the repository, an Auto-fix button
//! on each, the fix's progress, and Merge & apply for one that is ready.
//!
//! Nothing works on an issue until Auto-fix is pressed. The press starts
//! the runner for it (`swaypplet_screenshot::quality::autofix`, which holds
//! everything here that is not drawing). The runner's files
//! (`status::Job`) are read every two seconds while the tab is open, which
//! is what the phase line, the log tail and Stop follow.
//!
//! GitHub is asked only while the tab is on screen: once when it maps
//! (unless the last answer is under a minute old), then once a minute. One
//! fetch is two list calls (open issues, the runner's PRs); pictures for a
//! ready fix come from the job's own folder, or once from the asset branch.
//!
//! An issue someone else filed asks before it runs: its text is the
//! agent's input, and anyone can write one on a public repository.
//!
//! Merge & apply squash-merges the PR and runs `nx` as its own unit, the
//! one the System tab's actions run in too (`swaypplet_system::nx`),
//! because nx switches home-manager, which restarts this process; its log
//! is followed here, and a reopened tab shows how it ended. "Try locally" is not offered: `nx dev` rebuilds the system with
//! sudo, which prompts for a password that nothing here can answer.
//!
//! `SWAYPPLET_QUALITY_FIXTURE=<dir>` (`autofix::fixture`) plays the tab
//! from a directory, for the harness.

use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk4::prelude::*;

use swaypplet_screenshot::quality::{autofix, autofix::Pair, body};
use swaypplet_screenshot::quality::{
    status,
    status::{Fix, Issue, Job, Pr},
};
use swaypplet_settings_kit::{form, form::section_box};
use swaypplet_system::nx;
use swaypplet_ui::{
    ui,
    ui::{Text, Tone},
};

/// How often GitHub is asked while the tab is open.
const POLL: Duration = Duration::from_secs(60);

/// How often the local jobs are read while the tab is open.
const TICK: Duration = Duration::from_secs(2);

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
    swaypplet_base::fmt::ago(
        (now - then).max(0) as u64,
        swaypplet_base::fmt::Ago::Floored,
    )
}

/// The deploy log's last line, and the exit code once the runner wrote it.
pub fn deploy_state(log: &str) -> (String, Option<i32>) {
    let log = nx::parse_log(log);
    let last = log.tail.last().map(|l| l.trim().to_string());
    (last.unwrap_or_default(), log.exit)
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
    poll: swaypplet_base::spawn::Timeout,
    tick: swaypplet_base::spawn::Timeout,
    follow: swaypplet_base::spawn::Timeout,
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
                swaypplet_screenshot::quality::repo()
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
            "Close settings and start a report, as the key binding does.",
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
            poll: swaypplet_base::spawn::Timeout::new(),
            tick: swaypplet_base::spawn::Timeout::new(),
            follow: swaypplet_base::spawn::Timeout::new(),
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
                state.poll.every(POLL, move || {
                    fetch_now(&s);
                    glib::ControlFlow::Continue
                });
                let s = state.clone();
                state.tick.every(TICK, move || {
                    tick(&s);
                    glib::ControlFlow::Continue
                });
                follow_deploy(&state);
            });
        }
        {
            let state = state.clone();
            root.connect_unmap(move |_| {
                for slot in [&state.poll, &state.tick, &state.follow] {
                    slot.cancel();
                }
            });
        }

        // Harness hook: the nested session has no pointer, so
        // `SWAYPPLET_QUALITY_APPLY=<pr>` presses Merge & apply for that PR
        // two seconds in. Only in a dry run: it must never merge for real.
        if swaypplet_screenshot::quality::dry_run()
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
    swaypplet_base::spawn::spawn_work(autofix::fetch, move |result| {
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
    ui::clear_children(&state.list);
    state.entries.borrow_mut().clear();
    // Checked here rather than by a job that would fail at its first step.
    let missing = autofix::missing_tools();
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
        open.connect_clicked(move |_| swaypplet_screenshot::quality::report::open_uri(&url));
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
    facts.push(swaypplet_base::fmt::ago(
        now.saturating_sub(created).max(0) as u64,
        swaypplet_base::fmt::Ago::Floored,
    ));
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
        let job = autofix::read_job(entry.issue.number);
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
    ui::clear_children(&d);
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
            ui::arm(&stop, "Stop and discard?", move |b| {
                b.set_sensitive(false);
                autofix::stop_job(n);
            });
            actions.append(&stop);
        }
        Fix::Ready => {
            if let Some(pair) = autofix::pair(n, entry.pr.as_ref()) {
                let row = ui::hbox(3);
                let (before, after) = match pair {
                    Pair::Local(b, a) => (thumb_local("Before", &b), thumb_local("After", &a)),
                    Pair::Remote(b, a) => (thumb_remote("Before", &b), thumb_remote("After", &a)),
                };
                row.append(&before);
                row.append(&after);
                d.append(&row);
            }
            if let Some(line) = autofix::checks(n, entry.pr.as_ref()) {
                d.append(&caption(&line, Tone::Success));
            }
            let url = entry
                .pr
                .as_ref()
                .map(|p| p.url.clone())
                .or_else(|| job.and_then(status::ready_url).map(str::to_string));
            if let Some(url) = url {
                let view = ui::button("View PR", ui::Kind::Secondary);
                view.connect_clicked(move |_| {
                    swaypplet_screenshot::quality::report::open_uri(&url)
                });
                actions.append(&view);
            }
            if let Some(pr) = entry.pr.as_ref().map(|p| p.number) {
                let close = ui::button("Close", ui::Kind::Secondary);
                close.set_tooltip_text(Some("Close the PR without merging, and delete its branch"));
                {
                    let state = state.clone();
                    ui::arm(&close, "Close the PR?", move |b| {
                        b.set_sensitive(false);
                        let state = state.clone();
                        swaypplet_base::spawn::spawn_work(
                            move || swaypplet_screenshot::quality::gh::close_pr(pr),
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
                    ui::arm(&apply, "Merge and deploy?", move |_| apply_fix(&state, pr));
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
            if status::needs_confirm(&entry.issue, &swaypplet_screenshot::quality::owner()) {
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
    let stranger = status::needs_confirm(&entry.issue, &swaypplet_screenshot::quality::owner());
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
        if let Err(e) = autofix::start_job(n, allow.as_deref()) {
            log::warn!("quality: {e}");
            state.status.set_text(&format!("#{n} did not start: {e}"));
            form::mark_source(&state.status, false);
            b.set_sensitive(true);
        }
        tick(&state);
    };
    if stranger {
        ui::arm(&button, &prompt, go);
    } else {
        button.connect_clicked(go);
    }
    button
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
    swaypplet_base::spawn::spawn_work(
        move || autofix::download(&path),
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
    swaypplet_base::spawn::spawn_work(
        move || autofix::merge_and_deploy(pr),
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

/// Follow the deploy log once a second while the tab is open.
fn follow_deploy(state: &Rc<State>) {
    let log = nx::log_path();
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
    state.follow.every(Duration::from_secs(1), tick);
}

/// The tab in the settings search (`search.rs`): its groups, which have no
/// settings rows of their own.
#[rustfmt::skip]
pub const SEARCH: &[swaypplet_settings_kit::search::Entry] = &[
    swaypplet_settings_kit::search::row("Issues", "", "The open issues, and Auto-fix: an agent that prepares a PR for one", &["bug", "report", "crash", "auto-fix", "autofix", "fix", "issue", "github", "quality", "pr", "pull request", "agent", "claude"]),
    swaypplet_settings_kit::search::row("Deploy", "", "Merge a ready fix and run nx: build, switch, push", &["merge", "apply", "nx", "deploy", "rebuild"]),
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
    fn a_phase_reads_as_a_step() {
        assert_eq!(
            phase_text("3/5 agent working"),
            "Step 3 of 5: agent working"
        );
        assert_eq!(phase_text("odd"), "odd");
    }

    #[test]
    fn the_deploy_log_ends_with_its_exit() {
        let (last, code) =
            deploy_state("swaypplet-nx: start sync\n\u{1b}[1m── build\u{1b}[0m\nbuilding x\n");
        assert_eq!((last.as_str(), code), ("building x", None));
        let (last, code) = deploy_state("── switch\nswitched\nswaypplet-nx: exit 0\n");
        assert_eq!((last.as_str(), code), ("switched", Some(0)));
        assert_eq!(deploy_state("swaypplet-nx: exit 1").1, Some(1));
    }
}

#[cfg(test)]
mod search_anchors {
    /// Every entry of `SEARCH` names a row this pane builds, and every row
    /// it builds has an entry.
    #[test]
    fn the_search_table_matches_the_rows() {
        swaypplet_settings_kit::search::anchors::check(
            "quality",
            super::SEARCH,
            &[include_str!("lib.rs")],
            &[],
        );
    }
}
