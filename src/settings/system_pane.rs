//! The System tab: which build this host runs against origin/main, what is
//! new, the two nx actions with their progress, and the machine's facts.
//!
//! The facts come from `system_info.rs` and the actions from
//! `system_job.rs`, both on worker threads. The tab reads only while it is
//! on screen: once when it is shown, every [`POLL`] after that, and on
//! Refresh; a running action's log is read every second. Hidden, it holds
//! no timer and does no work. Neither action ever starts on its own.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gtk4::prelude::*;

use super::form::{self, section_box};
use super::system_info::{self as info, OriginSource, Snapshot, Verdict};
use super::system_job::{self as job, Action, Job};
use crate::spawn::{remove_source, spawn_work};
use crate::ui::{self, Kind, Status, Text, Tone};

/// How often the tab re-reads while it is open.
const POLL: Duration = Duration::from_secs(30);
/// How often a running action's log is read.
const JOB_TICK: Duration = Duration::from_secs(1);

struct Facts {
    host: gtk4::Label,
    version: gtk4::Label,
    kernel: gtk4::Label,
    uptime: gtk4::Label,
    root: gtk4::Label,
    nix: gtk4::Label,
    memory: gtk4::Label,
}

struct Pane {
    root: gtk4::Box,
    fixture: Option<String>,
    verdict: gtk4::Label,
    verdict_text: gtk4::Label,
    running: gtk4::Label,
    origin: gtk4::Label,
    staged_row: gtk4::Box,
    staged: gtk4::Label,
    shell: gtk4::Label,
    news: gtk4::Box,
    apply: gtk4::Button,
    update: gtk4::Button,
    reason: gtk4::Label,
    job_box: gtk4::Box,
    job_head: gtk4::Label,
    job_tail: gtk4::Label,
    facts: Facts,
    checked: gtk4::Label,
    snapshot: RefCell<Option<Snapshot>>,
    job: RefCell<Job>,
    reading: Cell<bool>,
    poll: RefCell<Option<glib::SourceId>>,
    job_poll: RefCell<Option<glib::SourceId>>,
    /// The fixture's simulated run: its action and how far it has got.
    fake: Cell<Option<(Action, usize)>>,
}

pub struct SystemPane {
    pane: Rc<Pane>,
}

fn value() -> gtk4::Label {
    let l = ui::text("", Text::Body, Tone::Fg);
    l.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    l.set_selectable(true);
    l
}

fn value_row(group: &gtk4::Box, label: &str) -> (gtk4::Box, gtk4::Label) {
    let v = value();
    let row = form::kind_row(label, &v);
    group.append(&row);
    (row, v)
}

fn commit_row(hash: &str, subject: &str) -> gtk4::Box {
    let row = ui::hbox(3);
    let h = ui::text(hash, Text::Caption, Tone::Faint);
    ui::set_mono(&h, true);
    let s = ui::text(subject, Text::Caption, Tone::Fg);
    s.set_hexpand(true);
    s.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    s.set_tooltip_text(Some(subject));
    row.append(&h);
    row.append(&s);
    row
}

fn verdict_words(v: Verdict) -> (Status, String, &'static str) {
    match v {
        Verdict::Switching(unit) => (
            Status::Neutral,
            "Switching".to_string(),
            if unit.starts_with("nx-converge") {
                "nx-converge is moving this host to origin/main."
            } else {
                "nx-apply is switching this host to origin/main."
            },
        ),
        Verdict::Staged => (
            Status::Warning,
            "A newer build is waiting".to_string(),
            "Apply it now, or reboot.",
        ),
        Verdict::Behind(n) => (
            Status::Warning,
            match n {
                Some(1) => "1 commit behind origin/main".to_string(),
                Some(n) => format!("{n} commits behind origin/main"),
                None => "Behind origin/main".to_string(),
            },
            "Apply now builds origin/main and switches to it.",
        ),
        Verdict::Diverged => (
            Status::Neutral,
            "Runs a commit origin/main does not have".to_string(),
            "A local build that is not pushed yet. Update pushes it.",
        ),
        Verdict::InSync => (
            Status::Success,
            "Up to date".to_string(),
            "This host runs origin/main.",
        ),
        Verdict::Unknown => (
            Status::Neutral,
            "Unknown".to_string(),
            "The running commit or origin/main could not be read.",
        ),
    }
}

impl SystemPane {
    pub fn new() -> Self {
        let root = form::pane();

        // ── This host's build ──
        let build = section_box(
            "NixOS build",
            "The nixos-config commit this host runs, and origin/main's. The same facts as nx status.",
        );
        let verdict = ui::status(Status::Neutral, "Checking");
        verdict.set_xalign(0.0);
        let verdict_text = ui::text("", Text::Caption, Tone::Muted);
        verdict_text.set_wrap(true);
        verdict_text.set_max_width_chars(form::HINT_CHARS);
        build.append(&verdict);
        build.append(&verdict_text);
        let (_, running) = value_row(&build, "Running");
        let (_, origin) = value_row(&build, "origin/main");
        let (staged_row, staged) = value_row(&build, "Staged");
        staged_row.set_visible(false);
        let (_, shell) = value_row(&build, "swaypplet");

        // ── What's new ──
        let new_group = section_box(
            "What's new",
            "Commits on origin/main that this host does not run yet.",
        );
        let news = ui::vbox(2);
        new_group.append(&news);

        // ── Actions ──
        let actions = section_box(
            "Update this host",
            "Apply now runs nx apply: this host switches to origin/main, no password. Update runs nx: commit ~/nixos, build every host, push, switch. The shell restarts when the switch lands.",
        );
        let apply = ui::button("Apply now", Kind::Primary);
        apply.set_tooltip_text(Some("nx apply: switch this host to origin/main."));
        let update = ui::button("Update", Kind::Secondary);
        update.set_tooltip_text(Some(
            "nx: commit ~/nixos, build every host, push, then switch this host.",
        ));
        let buttons = ui::hbox(3);
        buttons.append(&apply);
        buttons.append(&update);
        actions.append(&buttons);
        let reason = ui::text("", Text::Caption, Tone::Faint);
        reason.set_wrap(true);
        reason.set_max_width_chars(form::HINT_CHARS);
        actions.append(&reason);
        // The well paints; the box inside it holds the padding, since
        // `ui::pad` is margins and would sit outside the frame.
        let job_box = ui::well();
        let job_inner = ui::vbox(1);
        ui::pad(&job_inner, 3);
        job_box.append(&job_inner);
        let job_head = ui::status(Status::Neutral, "");
        job_head.set_xalign(0.0);
        let job_tail = ui::text("", Text::Caption, Tone::Muted);
        ui::set_mono(&job_tail, true);
        job_tail.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        job_tail.set_selectable(true);
        job_inner.append(&job_head);
        job_inner.append(&job_tail);
        job_box.set_visible(false);
        actions.append(&job_box);

        // ── The machine ──
        let machine = section_box("This machine", "Read from /proc and the filesystems.");
        let facts = Facts {
            host: value_row(&machine, "Host").1,
            version: value_row(&machine, "NixOS").1,
            kernel: value_row(&machine, "Kernel").1,
            uptime: value_row(&machine, "Uptime").1,
            root: value_row(&machine, "Disk /").1,
            nix: value_row(&machine, "Disk /nix").1,
            memory: value_row(&machine, "Memory").1,
        };

        let refresh = form::action_button("Refresh", "Read everything again now.");
        let (footer, checked) = form::footer(&[&refresh]);

        root.append(&build);
        root.append(&new_group);
        root.append(&actions);
        root.append(&machine);
        root.append(&footer);

        let pane = Rc::new(Pane {
            root,
            fixture: info::fixture_name(),
            verdict,
            verdict_text,
            running,
            origin,
            staged_row,
            staged,
            shell,
            news,
            apply: apply.clone(),
            update: update.clone(),
            reason,
            job_box,
            job_head,
            job_tail,
            facts,
            checked,
            snapshot: RefCell::new(None),
            job: RefCell::new(Job::Idle),
            reading: Cell::new(false),
            poll: RefCell::new(None),
            job_poll: RefCell::new(None),
            fake: Cell::new(None),
        });

        {
            let p = pane.clone();
            refresh.connect_clicked(move |_| p.read());
        }
        {
            let p = pane.clone();
            apply.connect_clicked(move |_| p.start(Action::Apply));
        }
        {
            // `nx` commits and pushes, so it asks twice.
            let p = pane.clone();
            let armed = Rc::new(Cell::new(false));
            update.connect_clicked(move |b| {
                if armed.replace(false) {
                    ui::set_armed(b, false);
                    b.set_label("Update");
                    p.start(Action::Update);
                    return;
                }
                armed.set(true);
                ui::set_armed(b, true);
                b.set_label("Commit, build, push?");
                let (armed, b) = (armed.clone(), b.clone());
                glib::timeout_add_local_once(Duration::from_secs(4), move || {
                    if armed.replace(false) {
                        ui::set_armed(&b, false);
                        b.set_label("Update");
                    }
                });
            });
        }
        {
            let p = pane.clone();
            pane.root.connect_map(move |_| p.shown());
        }
        {
            let p = pane.clone();
            pane.root.connect_unmap(move |_| p.hidden());
        }

        SystemPane { pane }
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.pane.root
    }

    /// The panel opened. The map handler does the reading, and only when
    /// this tab is the one on screen.
    pub fn refresh(&self) {
        if self.pane.root.is_mapped() {
            self.pane.read();
        }
    }
}

impl Pane {
    fn shown(self: &Rc<Self>) {
        self.read();
        if self.poll.borrow().is_none() {
            let p = Rc::downgrade(self);
            let id = glib::timeout_add_local(POLL, move || {
                let Some(p) = p.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                p.read();
                glib::ControlFlow::Continue
            });
            self.poll.replace(Some(id));
        }
    }

    fn hidden(&self) {
        if let Some(id) = self.poll.take() {
            remove_source(id);
        }
        if let Some(id) = self.job_poll.take() {
            remove_source(id);
        }
    }

    /// Everything, on a worker thread. A read already out is not doubled.
    fn read(self: &Rc<Self>) {
        if self.reading.replace(true) {
            return;
        }
        let fixture = self.fixture.is_some();
        let p = self.clone();
        spawn_work(
            move || {
                let snapshot = info::read_snapshot();
                let job = if fixture { None } else { Some(job::read()) };
                (snapshot, job)
            },
            move |(snapshot, job)| {
                p.reading.set(false);
                p.snapshot.replace(Some(snapshot));
                let job = job.unwrap_or_else(|| p.fixture_job());
                p.set_job(job);
                p.show_snapshot();
                p.checked.set_text(&format!(
                    "Checked at {} · again every {} s while this tab is open",
                    glib::DateTime::now_local()
                        .and_then(|t| t.format("%H:%M:%S"))
                        .map(|s| s.to_string())
                        .unwrap_or_default(),
                    POLL.as_secs()
                ));
            },
        );
    }

    /// The fixture's job: the simulated run, or for `applying` one that is
    /// half way.
    fn fixture_job(&self) -> Job {
        if let Some((action, ticks)) = self.fake.get() {
            let log = job::parse_log(&job::fixture_log(action, ticks));
            return job::job(Some(log), true);
        }
        if self.fixture.as_deref() == Some("applying") {
            return Job::Running(job::parse_log(&job::fixture_log(Action::Apply, 6)));
        }
        Job::Idle
    }

    fn start(self: &Rc<Self>, action: Action) {
        if self.job.borrow().running() {
            return;
        }
        if self.fixture.is_some() {
            self.fake.set(Some((action, 0)));
            self.set_job(self.fixture_job());
            self.show_actions();
            self.watch_job();
            return;
        }
        self.apply.set_sensitive(false);
        self.update.set_sensitive(false);
        let p = self.clone();
        spawn_work(
            move || job::start(action),
            move |result| match result {
                Ok(()) => {
                    p.set_job(Job::Running(job::Log {
                        action: Some(action),
                        ..Default::default()
                    }));
                    p.show_actions();
                    p.watch_job();
                }
                Err(e) => {
                    log::warn!("system: {} did not start: {e}", action.command());
                    p.job_box.set_visible(true);
                    ui::set_status(&p.job_head, Status::Danger);
                    p.job_head
                        .set_text(&format!("\u{25cf} {} did not start", action.command()));
                    p.job_tail.set_text(&e);
                    p.show_actions();
                }
            },
        );
    }

    /// Read the log every [`JOB_TICK`] until the run ends or the tab goes.
    fn watch_job(self: &Rc<Self>) {
        if self.job_poll.borrow().is_some() || !self.root.is_mapped() {
            return;
        }
        let p = Rc::downgrade(self);
        let busy = Rc::new(Cell::new(false));
        let id = glib::timeout_add_local(JOB_TICK, move || {
            let Some(p) = p.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if let Some((action, ticks)) = p.fake.get() {
                p.fake.set(Some((action, ticks + 1)));
                let now = p.fixture_job();
                let over = !now.running();
                p.set_job(now);
                if over {
                    p.fake.set(None);
                    p.job_poll.replace(None);
                    p.show_actions();
                    return glib::ControlFlow::Break;
                }
                return glib::ControlFlow::Continue;
            }
            if busy.replace(true) {
                return glib::ControlFlow::Continue;
            }
            let (q, busy) = (p.clone(), busy.clone());
            spawn_work(job::read, move |now| {
                busy.set(false);
                let over = !now.running();
                q.set_job(now);
                if over {
                    if let Some(id) = q.job_poll.take() {
                        remove_source(id);
                    }
                    // The switch moved what the rest of the tab says.
                    q.read();
                }
            });
            glib::ControlFlow::Continue
        });
        self.job_poll.replace(Some(id));
    }

    fn set_job(self: &Rc<Self>, now: Job) {
        let command = |log: &job::Log| log.action.map_or("nx", Action::command);
        let (status, head, log) = match &now {
            Job::Idle => {
                self.job_box.set_visible(false);
                self.job.replace(now);
                return;
            }
            Job::Running(log) => (
                Status::Neutral,
                match log.step.as_deref() {
                    // `nx apply`'s one step is called apply.
                    Some(step) if !command(log).ends_with(step) => {
                        format!("{} · {step}", command(log))
                    }
                    Some(_) => format!("{} · running", command(log)),
                    None => format!("{} · starting", command(log)),
                },
                log,
            ),
            Job::Done(log) if log.exit == Some(0) => {
                (Status::Success, format!("{} finished", command(log)), log)
            }
            Job::Done(log) => (
                Status::Danger,
                format!(
                    "{} failed{} (exit {})",
                    command(log),
                    log.step
                        .as_deref()
                        .map(|s| format!(" at {s}"))
                        .unwrap_or_default(),
                    log.exit.unwrap_or(-1)
                ),
                log,
            ),
            Job::Lost(log) => (
                Status::Warning,
                format!("{} stopped without a result", command(log)),
                log,
            ),
        };
        self.job_box.set_visible(true);
        ui::set_status(&self.job_head, status);
        self.job_head.set_text(&format!("\u{25cf} {head}"));
        self.job_tail.set_text(&log.tail.join("\n"));
        let running = now.running();
        self.job.replace(now);
        if running {
            self.watch_job();
        }
        self.show_actions();
    }

    fn show_actions(&self) {
        let snapshot = self.snapshot.borrow();
        let Some(s) = snapshot.as_ref() else {
            return;
        };
        let v = info::verdict(s);
        let job_running = self.job.borrow().running();
        let a = info::actions(v, s.locked, job_running);
        self.apply.set_sensitive(a.apply);
        self.update.set_sensitive(a.update);
        ui::set_button_kind(
            &self.apply,
            if a.apply {
                Kind::Primary
            } else {
                Kind::Secondary
            },
        );
        self.reason.set_text(a.reason.unwrap_or(""));
        // Our own run says it is running in the log's head already.
        self.reason.set_visible(a.reason.is_some() && !job_running);
        ui::set_tone(
            &self.reason,
            if s.locked { Tone::Warning } else { Tone::Faint },
        );
    }

    fn show_snapshot(&self) {
        {
            let snapshot = self.snapshot.borrow();
            let Some(s) = snapshot.as_ref() else {
                return;
            };
            let (status, head, text) = verdict_words(info::verdict(s));
            ui::set_status(&self.verdict, status);
            self.verdict.set_text(&format!("\u{25cf} {head}"));
            self.verdict_text.set_text(&match s.origin_source {
                OriginSource::LastFetch => format!(
                    "{text} origin/main is as of the last fetch: the remote did not answer."
                ),
                _ => text.to_string(),
            });

            self.running.set_text(
                s.running
                    .as_deref()
                    .map_or("unknown (built from a working tree)", info::short),
            );
            self.origin.set_text(&match (&s.origin, s.origin_source) {
                (Some(o), OriginSource::LastFetch) => {
                    format!("{} (last fetch)", info::short(o))
                }
                (Some(o), _) => info::short(o).to_string(),
                (None, _) => "unknown".to_string(),
            });
            self.staged_row.set_visible(s.staged.is_some());
            if let Some(staged) = &s.staged {
                self.staged.set_text(&match staged {
                    Some(r) => format!("{}, not running yet", info::short(r)),
                    None => "a working-tree build, not running yet".to_string(),
                });
            }
            let mut shell = vec![format!(
                "{} running",
                s.shells.running.as_deref().map_or("dev build", info::short)
            )];
            if let Some(r) = &s.shells.staged {
                shell.push(format!("{} staged", info::short(r)));
            }
            if let Some(r) = &s.shells.origin {
                shell.push(format!("{} on origin/main", info::short(r)));
            }
            self.shell.set_text(&shell.join(" · "));

            self.show_news(s);
            self.show_facts(s);
        }
        self.show_actions();
    }

    fn show_news(&self, s: &Snapshot) {
        while let Some(child) = self.news.first_child() {
            self.news.remove(&child);
        }
        if s.new_commits.is_empty() && s.shell_commits.is_empty() {
            let text = match info::verdict(s) {
                Verdict::InSync => "Nothing: this host runs origin/main.",
                _ => "No commits to list: the running commit or origin/main is not in ~/nixos.",
            };
            self.news
                .append(&ui::text(text, Text::Caption, Tone::Faint));
            return;
        }
        for (name, list) in [
            ("nixos-config", &s.new_commits),
            ("swaypplet", &s.shell_commits),
        ] {
            if list.is_empty() {
                continue;
            }
            self.news.append(&ui::overline(name, Tone::Faint));
            for c in list {
                self.news.append(&commit_row(&c.hash, &c.subject));
            }
        }
    }

    fn show_facts(&self, s: &Snapshot) {
        let or_unknown = |v: Option<String>| v.unwrap_or_else(|| "unknown".to_string());
        let f = &self.facts;
        f.host.set_text(&s.host);
        f.version.set_text(&or_unknown(s.nixos_version.clone()));
        f.kernel.set_text(&or_unknown(s.kernel.clone()));
        f.uptime
            .set_text(&or_unknown(s.uptime_s.map(info::uptime_label)));
        for (label, mount) in [(&f.root, "/"), (&f.nix, "/nix")] {
            label.set_text(&or_unknown(s.disks.iter().find(|d| d.mount == mount).map(
                |d| {
                    format!(
                        "{} free of {}",
                        info::bytes_label(d.free),
                        info::bytes_label(d.total)
                    )
                },
            )));
        }
        f.memory
            .set_text(&or_unknown(s.memory.map(|(avail, total)| {
                format!(
                    "{} available of {}",
                    info::bytes_label(avail),
                    info::bytes_label(total)
                )
            })));
    }
}
