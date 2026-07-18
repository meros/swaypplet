//! Shared access to the host `SWAYPPLET_SWITCH_USER_CMD` fast-user-switch
//! helper.
//!
//! `--list` prints one JSON object per configured user (stable configured
//! order); a bare username argument performs the switch (the helper locks the
//! current session first, then activates the target's session or a greeter);
//! no argument is the legacy cycle. Every entry point degrades to `None` /
//! best-effort so callers keep their pre-switcher behaviour on a non-NixOS dev
//! box where the env var is unset or the command misbehaves.

use serde::Deserialize;

/// One configured user, as emitted by `swaypplet-switch-user --list`.
#[derive(Debug, Clone, Deserialize)]
pub struct SwitchUser {
    pub user: String,
    /// This is the invoking user — never offered as a switch target.
    #[serde(default)]
    pub current: bool,
    /// Has a live graphical session (selecting it resumes that session).
    #[serde(default)]
    pub logged_in: bool,
    /// fprintd enrollment present for this user.
    #[serde(default)]
    pub fingerprint: bool,
    /// Readable avatar image path, or `None`.
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub vt: Option<u32>,
    #[serde(default)]
    pub session: Option<String>,
}

/// The switch-user command path, when the host exposes one.
pub fn cmd() -> Option<String> {
    std::env::var("SWAYPPLET_SWITCH_USER_CMD")
        .ok()
        .filter(|c| !c.is_empty())
}

/// Run `<cmd> --list` and parse it. Blocking — call on a background thread.
/// Returns `None` when the env var is unset, the command fails, or the output
/// doesn't parse, so every caller can fall back to legacy behaviour.
pub fn list() -> Option<Vec<SwitchUser>> {
    let cmd = cmd()?;
    let out = std::process::Command::new(&cmd)
        .arg("--list")
        .output()
        .map_err(|e| log::warn!("switch-user --list spawn error: {e}"))
        .ok()?;
    if !out.status.success() {
        log::warn!(
            "switch-user --list failed ({}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
        return None;
    }
    match serde_json::from_slice::<Vec<SwitchUser>>(&out.stdout) {
        Ok(users) => Some(users),
        Err(e) => {
            log::warn!("switch-user --list parse error: {e}");
            None
        }
    }
}

/// Switch to `user` (fire-and-forget). The helper locks the current session
/// and activates the target's session or a greeter; switching to self is a
/// no-op upstream, but callers should not offer it anyway.
pub fn switch_to(cmd: &str, user: &str) {
    if let Err(e) = std::process::Command::new(cmd).arg(user).spawn() {
        log::error!("failed to spawn switch-user {user}: {e}");
    }
}

/// Legacy cycle (no argument) — the fallback when `--list` is unavailable.
pub fn cycle(cmd: &str) {
    if let Err(e) = std::process::Command::new(cmd).spawn() {
        log::error!("failed to spawn switch-user: {e}");
    }
}
