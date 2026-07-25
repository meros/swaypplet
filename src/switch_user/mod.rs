//! Fast user switching, both sides of it.
//!
//! `swaypplet switch-user` is the host command:
//!
//!   `--list`     JSON of configured users + session state, for UIs that draw
//!                a session-aware picker (the panel rail, the lock screen, the
//!                greeter).
//!   `--greeter`  go to the login screen: an idle greeter if one exists, else
//!                a fresh one on a free spare VT.
//!   `<user>`     resume that user's live session, else put a greeter in front
//!                of them.
//!   (none)       cycle to the next session.
//!
//! Every mode locks the current session first. [`host`] does the D-Bus work,
//! [`rows`] holds the pure logic behind it, and this module is what the rest
//! of swaypplet calls.
//!
//! In-process vs. spawned: [`list`] runs the query directly, since a picker
//! that has to fork/exec to draw itself is a picker that gets left out of the
//! lock screen. The actions spawn `swaypplet switch-user …` and forget it —
//! they lock our own session and hand the seat away, so there is nothing left
//! to report back to, and a detached child keeps the CLI honest as a
//! from-a-getty escape hatch.
//!
//! Everything degrades to `None` / best-effort: on a host with no
//! `/etc/swaypplet/switch-user.json` (any dev box), [`available`] is false and
//! callers hide the affordance instead of offering a switch that can't happen.

pub mod host;
pub mod rows;

use serde::{Deserialize, Serialize};

/// One configured user, as emitted by `swaypplet switch-user --list`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwitchUser {
    pub user: String,
    /// This is the invoking user — never offered as a switch target.
    #[serde(default)]
    pub current: bool,
    /// Has a live graphical session (selecting it resumes that session).
    #[serde(default)]
    pub logged_in: bool,
    /// fprintd enrollment present for this user. `None` when the host
    /// couldn't determine it (fprintd cold / no default device yet) — a
    /// distinct state from a definite "no enrolled prints" (`Some(false)`),
    /// so the greeter can keep the reader armed on unknown rather than tearing
    /// it down on a false negative.
    #[serde(default)]
    pub fingerprint: Option<bool>,
    /// Readable avatar image path, or `None`.
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub vt: Option<u32>,
    #[serde(default)]
    pub session: Option<String>,
}

/// Whether this host does user switching at all.
pub fn available() -> bool {
    host::config().is_some()
}

/// Query the configured users and their session state. Blocking — call it on
/// a background thread. `None` when the host isn't configured for switching or
/// the bus query fails, so every caller can fall back to legacy behaviour.
pub fn list() -> Option<Vec<SwitchUser>> {
    let cfg = host::config()?;
    block_on(async move {
        let conn = zbus::Connection::system().await?;
        host::list(&conn, &cfg).await
    })
    .map_err(|e| log::warn!("switch-user --list failed: {e}"))
    .ok()
}

/// Switch to `user` (fire-and-forget). Locks the current session, then
/// activates the target's session or a greeter. Switching to self is a no-op
/// upstream, but callers should not offer it anyway.
pub fn switch_to(user: &str) {
    spawn(&[user]);
}

/// Jump to a greeter (fire-and-forget) — picking the target user is the
/// greeter's job.
pub fn to_greeter() {
    spawn(&["--greeter"]);
}

/// Cycle to the next session (fire-and-forget).
pub fn cycle() {
    spawn(&[]);
}

/// Re-exec ourselves as the host command and forget about it.
fn spawn(args: &[&str]) {
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => {
            log::error!("switch-user: cannot find own binary: {e}");
            return;
        }
    };
    if let Err(e) = std::process::Command::new(exe)
        .arg("switch-user")
        .args(args)
        .spawn()
    {
        log::error!("switch-user: failed to spawn {args:?}: {e}");
    }
}

/// Run one blocking D-Bus round on a private runtime. Safe from any thread
/// that isn't already inside a tokio runtime, which is every caller here: the
/// actions run in their own process and `list` runs on a worker thread.
fn block_on<T>(fut: impl Future<Output = zbus::Result<T>>) -> zbus::Result<T> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| zbus::Error::Failure(format!("runtime: {e}")))?
        .block_on(fut)
}

/// `swaypplet switch-user [--list|--greeter|<user>]`. Never returns.
pub fn run(mut args: impl Iterator<Item = String>) -> ! {
    let Some(cfg) = host::config() else {
        eprintln!("swaypplet switch-user: this host has no switch-user config");
        std::process::exit(1);
    };
    let arg = args.next();

    let outcome = block_on(async move {
        let conn = zbus::Connection::system().await?;
        Ok(match arg.as_deref() {
            Some("--list") => match host::list(&conn, &cfg).await {
                Ok(users) => serde_json::to_string_pretty(&users)
                    .map(|json| println!("{json}"))
                    .map_err(|e| format!("cannot serialise list: {e}")),
                Err(e) => Err(e.to_string()),
            },
            Some("--greeter") => host::goto_greeter(&conn, &cfg).await,
            Some(user) => host::switch_to(&conn, &cfg, user).await,
            None => host::cycle(&conn, &cfg).await,
        })
    });

    match outcome {
        Ok(Ok(())) => std::process::exit(0),
        Ok(Err(e)) => {
            eprintln!("swaypplet switch-user: {e}");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("swaypplet switch-user: system bus: {e}");
            std::process::exit(1);
        }
    }
}
