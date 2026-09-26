//! Backup readout for the Helm's System State & Power sheet.
//!
//! The bar's segment (`bar/backup.rs`) carries one glyph and a tooltip. This
//! is the same snapshot spelled out: a row per job with when it last ran,
//! how much it held and which snapshot that was, then what the repository
//! costs on meros-server and what is left beside it.
//!
//! "Back up now" starts both units. It needs no authentication prompt
//! because the nixos side grants this user `start` on exactly those two
//! units (`modules/nixos/services/backup.nix`). It passes `--no-block`,
//! because `systemctl start` on a `Type=oneshot` unit waits for the unit to
//! finish and a backup is minutes long, and it still goes out on a worker
//! thread so the polkit round trip cannot stall a frame.

use std::process::Command;
use std::rc::Rc;

use gtk4::prelude::*;

use crate::backup::{self, BackupStatusService};
use crate::spawn::spawn_work;
use crate::ui;

const UNITS: [&str; 2] = [
    "restic-backups-home.service",
    "restic-backups-locked.service",
];

pub struct BackupSection {
    root: gtk4::Box,
    service: Rc<BackupStatusService>,
    verdict: gtk4::Label,
    rows: gtk4::Box,
    target: gtk4::Label,
    run_now: gtk4::Button,
}

impl BackupSection {
    pub fn new(service: &Rc<BackupStatusService>) -> Self {
        let root = ui::group(2);
        root.add_css_class("section-group");

        let header = ui::hbox(3);
        let title = ui::heading("Backup");
        title.set_hexpand(true);
        header.append(&title);
        let verdict = ui::status(ui::Status::Neutral, "");
        header.append(&verdict);
        root.append(&header);

        let rows = ui::vbox(1);
        root.append(&rows);

        let target = ui::text("", ui::Text::Caption, ui::Tone::Faint);
        root.append(&target);

        let run_now = ui::button("Back up now", ui::Kind::Secondary);
        run_now.set_halign(gtk4::Align::Start);
        // The status file turns to `running` at ExecStartPre, which is what
        // re-enables the button later; this only stops a second click in the
        // seconds before that lands.
        run_now.connect_clicked(|btn| {
            btn.set_sensitive(false);
            spawn_work(start_units, |()| {});
        });
        root.append(&run_now);

        let section = Self {
            root,
            service: service.clone(),
            verdict,
            rows,
            target,
            run_now,
        };
        section.render();

        // Redraw on every change rather than only on panel open: a stale
        // readout behind a closed sheet is how a panel starts lying, and the
        // service fires only when a status file actually moved.
        let service = service.clone();
        let verdict = section.verdict.clone();
        let rows = section.rows.clone();
        let target = section.target.clone();
        let run_now = section.run_now.clone();
        section.service.connect_change(move || {
            draw(&service, &verdict, &rows, &target, &run_now);
        });

        section
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    /// Called on panel open, like every other section.
    pub fn refresh(&self) {
        self.render();
    }

    fn render(&self) {
        draw(
            &self.service,
            &self.verdict,
            &self.rows,
            &self.target,
            &self.run_now,
        );
    }
}

fn draw(
    service: &Rc<BackupStatusService>,
    verdict: &gtk4::Label,
    rows: &gtk4::Box,
    target: &gtk4::Label,
    run_now: &gtk4::Button,
) {
    let snapshot = service.snapshot();
    let tier = snapshot.tier();

    ui::set_status(verdict, status(tier));
    verdict.set_label(&format!("{} {}", tier.icon(), label(tier)));

    while let Some(child) = rows.first_child() {
        rows.remove(&child);
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    for job in &snapshot.jobs {
        rows.append(&job_row(&job.job, &backup::describe(job, now)));
    }

    match snapshot.jobs.iter().max_by_key(|j| j.ended) {
        Some(job) if job.repo_bytes > 0 => {
            target.set_visible(true);
            target.set_label(&format!(
                "meros-server   {} used · {} free",
                backup::bytes(job.repo_bytes),
                backup::bytes(job.target_avail_bytes)
            ));
        }
        _ => target.set_visible(false),
    }

    run_now.set_sensitive(!snapshot.jobs.iter().any(crate::backup::Job::running));
}

/// The verdict's status tone: healthy, attention, or nothing to say yet.
fn status(tier: backup::Tier) -> ui::Status {
    match tier {
        backup::Tier::Ok => ui::Status::Success,
        backup::Tier::Stale | backup::Tier::Failed => ui::Status::Warning,
        backup::Tier::Running | backup::Tier::Unknown => ui::Status::Neutral,
    }
}

fn label(tier: backup::Tier) -> &'static str {
    match tier {
        backup::Tier::Ok => "up to date",
        backup::Tier::Running => "running",
        backup::Tier::Stale => "out of date",
        backup::Tier::Failed => "failed",
        backup::Tier::Unknown => "no run recorded",
    }
}

fn job_row(name: &str, detail: &str) -> gtk4::Box {
    let r = ui::row("", name, "");
    let when = gtk4::Label::builder()
        .label(detail)
        .xalign(1.0)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .build();
    r.end.append(&when);
    r.root
}

/// Both units, one call. systemd queues the second behind the first through
/// the ordering the units already declare.
fn start_units() {
    let mut cmd = Command::new("systemctl");
    cmd.arg("start").arg("--no-block");
    for unit in UNITS {
        cmd.arg(unit);
    }
    match cmd.status() {
        Ok(status) if status.success() => {}
        Ok(status) => log::warn!("backup: systemctl start exited {status}"),
        Err(e) => log::warn!("backup: systemctl start: {e}"),
    }
}
