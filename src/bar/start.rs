//! Start button — the swaypplet logo glyph at the bar's far left, opening
//! the start-menu panel.
//!
//! Hosted in-process (bar inside the panel app) the caller hands a direct
//! `panel.toggle()` hook. Standalone (`swaypplet bar` during the waybar
//! migration) the panel is another process, so [`toggle_panel_fallback`]
//! replicates the `swaypplet-toggle` wrapper (package.nix): SIGUSR1 at the
//! pid from the panel's pid file, guarded by a /proc comm check so a
//! recycled pid never gets a default-fatal USR1, else launch the panel.

use std::rc::Rc;

use gtk4::prelude::*;

use crate::icons;

pub fn build(toggle_panel: Rc<dyn Fn()>) -> gtk4::Button {
    let btn = gtk4::Button::builder()
        .label(icons::START)
        .css_classes(["bar-start"])
        .build();
    btn.connect_clicked(move |_| toggle_panel());
    btn
}

/// Cross-process panel toggle for the standalone bar. The two /proc-sized
/// reads run inline on the GTK thread — they are pseudo-files, not disk I/O.
pub fn toggle_panel_fallback() {
    if let Some(pid) = read_panel_pid() {
        // SAFETY: plain kill(2) with a constant signal; no memory involved.
        unsafe { libc::kill(pid, libc::SIGUSR1) };
        return;
    }
    // No live panel — launch one, like swaypplet-toggle does. Same binary,
    // no args: argv routing falls through to app::run.
    let exe = std::env::current_exe().unwrap_or_else(|_| "swaypplet".into());
    if let Err(e) = std::process::Command::new(&exe).spawn() {
        log::warn!("start button: launching panel {exe:?} failed: {e}");
    }
}

/// The panel's pid, only if that pid still belongs to a live swaypplet
/// process (comm check — the pid file can outlive a crashed panel and the
/// pid can be recycled by an unrelated process).
fn read_panel_pid() -> Option<i32> {
    let pid = std::fs::read_to_string(crate::app::pid_file_path())
        .ok()?
        .trim()
        .parse::<i32>()
        .ok()?;
    let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    // The installed binary is wrapped by makeBinaryWrapper, so the running
    // panel's executable is .swaypplet-wrapped and comm, which is the
    // executable filename cut to 15 characters, reads ".swaypplet-wrap".
    // Requiring the bare name rejected every real panel. Accepting the
    // wrapper prefix as well stays narrow enough that an unrelated process
    // holding a recycled pid still fails the check and is spared the signal.
    let comm = comm.trim();
    (comm == "swaypplet" || comm.starts_with(".swaypplet")).then_some(pid)
}
