//! Host side of fast user switching: the logind/systemd/fprintd calls behind
//! `swaypplet switch-user`.
//!
//! This replaces `switchUserCmd` and `gotoGreeterCmd` in
//! `modules/nixos/desktop/wayland.nix`, which shelled out to `loginctl`,
//! `busctl`, `jq` and `systemctl` to do the same things — six independent
//! answers to "is this user logged in" and four copies of the free-VT scan.
//! There is one of each here, and `rows.rs` unit-tests them.
//!
//! Privileges are unchanged from the scripts: no SUID, no root. Activating a
//! session rides `org.freedesktop.login1.chvt` (allowed for active local
//! sessions), starting a greeter rides the `greetd-switch@tty*.service`
//! polkit rule in `wayland.nix`, and the cross-user fprintd query rides the
//! `net.reactivated.fprint.device.setusername` rule in `security/fingerprint.nix`.

use std::collections::HashMap;

use serde::Deserialize;
use zbus::Connection;
use zbus::names::InterfaceName;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

use super::SwitchUser;
use super::rows::{self, Session};

/// Where AccountsService keeps avatars.
const ICON_DIR: &str = "/var/lib/AccountsService/icons";

/// The interface we pull TTY/Class off, as `GetAll` wants it.
fn session_iface() -> zbus::zvariant::Optional<InterfaceName<'static>> {
    Some(InterfaceName::from_static_str_unchecked(
        "org.freedesktop.login1.Session",
    ))
    .into()
}

/// Static host config, written by Nix (`environment.etc`), so adding a user or
/// a spare VT stays a one-line change in `wayland.nix` and never needs a Rust
/// rebuild. Its absence means "this host doesn't do user switching" — which is
/// exactly the right answer on a dev box, where the UI then hides the affordance.
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// Switchable users, in the order the picker shows them.
    pub users: Vec<String>,
    /// VTs a switch greeter may be started on. Each needs a matching
    /// `/etc/greetd/switch-ttyN.toml`.
    #[serde(default)]
    pub spare_vts: Vec<u32>,
}

/// Read the host config, or `None` when this host has no switching set up.
pub fn config() -> Option<Config> {
    let path = std::env::var("SWAYPPLET_SWITCH_USER_CONFIG")
        .unwrap_or_else(|_| "/etc/swaypplet/switch-user.json".into());
    let raw = std::fs::read(&path).ok()?;
    match serde_json::from_slice(&raw) {
        Ok(cfg) => Some(cfg),
        Err(e) => {
            log::warn!("switch-user: bad config at {path}: {e}");
            None
        }
    }
}

// --- D-Bus proxies ---------------------------------------------------------

/// ListSessions row: (session_id, uid, user, seat, object_path).
type SessionRow = (String, u32, String, String, OwnedObjectPath);

#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait Logind {
    fn list_sessions(&self) -> zbus::Result<Vec<SessionRow>>;
    fn get_session(&self, id: &str) -> zbus::Result<OwnedObjectPath>;
    /// `loginctl activate` — the VT switch itself.
    fn activate_session(&self, id: &str) -> zbus::Result<()>;
}

#[zbus::proxy(
    interface = "org.freedesktop.login1.Session",
    default_service = "org.freedesktop.login1"
)]
trait LogindSession {
    /// Emits the Lock signal our own idle daemon listens for.
    fn lock(&self) -> zbus::Result<()>;
}

#[zbus::proxy(
    interface = "org.freedesktop.systemd1.Manager",
    default_service = "org.freedesktop.systemd1",
    default_path = "/org/freedesktop/systemd1"
)]
trait Systemd {
    fn start_unit(&self, name: &str, mode: &str) -> zbus::Result<OwnedObjectPath>;
}

// --- session snapshot ------------------------------------------------------

fn prop(props: &HashMap<String, OwnedValue>, key: &str) -> String {
    props
        .get(key)
        .and_then(|v| <&str>::try_from(&**v).ok())
        .unwrap_or_default()
        .to_string()
}

/// Every logind session, with the properties the picker needs.
///
/// `ListSessions` alone doesn't carry TTY or class, so each session gets one
/// `GetAll` — a handful of round trips on a machine with a handful of sessions.
/// A session that vanishes mid-scan (logout race) is skipped, not fatal.
async fn snapshot(conn: &Connection) -> zbus::Result<Vec<Session>> {
    let manager = LogindProxy::new(conn).await?;
    let mut out = Vec::new();
    for (id, _uid, user, _seat, path) in manager.list_sessions().await? {
        let props = zbus::fdo::PropertiesProxy::builder(conn)
            .destination("org.freedesktop.login1")?
            .path(path)?
            .build()
            .await?;
        let all = match props.get_all(session_iface()).await {
            Ok(a) => a,
            Err(e) => {
                log::debug!("switch-user: session {id} went away mid-scan: {e}");
                continue;
            }
        };
        out.push(Session {
            session: id,
            user,
            tty: prop(&all, "TTY"),
            class: prop(&all, "Class"),
        });
    }
    Ok(out)
}

// --- fingerprint enrollment ------------------------------------------------

/// Enrollment per user, tri-state (see [`rows::row`]).
///
/// The shell version mapped every fprintd error to "not enrolled". Here only
/// fprintd's own `NoEnrolledPrints` counts as a definite no; anything else
/// (cold daemon, no default device, polkit denial) stays unknown, so the
/// greeter keeps an armed reader instead of tearing it down on a false negative.
async fn fingerprints(conn: &Connection, users: &[String]) -> HashMap<String, Option<bool>> {
    let mut out: HashMap<String, Option<bool>> = users.iter().map(|u| (u.clone(), None)).collect();

    let device = async {
        let mgr = crate::fp::ManagerProxy::new(conn).await.ok()?;
        let path = mgr.get_default_device().await.ok()?;
        crate::fp::DeviceProxy::builder(conn)
            .path(path)
            .ok()?
            .build()
            .await
            .ok()
    }
    .await;
    let Some(device) = device else {
        log::debug!("switch-user: no fprintd device; enrollment unknown");
        return out;
    };

    for user in users {
        let verdict = match device.list_enrolled_fingers(user).await {
            Ok(fingers) => Some(!fingers.is_empty()),
            Err(e) if crate::fp::is_no_enrolled_prints(&e) => Some(false),
            Err(e) => {
                log::debug!("switch-user: enrollment for {user} unknown: {e}");
                None
            }
        };
        out.insert(user.clone(), verdict);
    }
    out
}

/// This user's avatar, when there is a readable one.
fn icon_for(user: &str) -> Option<String> {
    let path = format!("{ICON_DIR}/{user}");
    std::fs::File::open(&path).ok().map(|_| path)
}

// --- the list --------------------------------------------------------------

/// The picker rows for `cfg.users`, in configured order.
pub async fn list(conn: &Connection, cfg: &Config) -> zbus::Result<Vec<SwitchUser>> {
    let sessions = snapshot(conn).await?;
    let fp = fingerprints(conn, &cfg.users).await;
    let current = crate::lock::auth::current_username().unwrap_or_default();
    Ok(cfg
        .users
        .iter()
        .map(|u| {
            rows::row(
                &sessions,
                u,
                &current,
                fp.get(u).copied().flatten(),
                icon_for(u),
            )
        })
        .collect())
}

// --- the actions -----------------------------------------------------------

/// Lock our own session before handing the seat over, so returning to it
/// demands auth. Best-effort by design: the destination we're switching to
/// does its own authentication, so a failure here must never block the switch
/// (the shell version's `|| true`).
async fn lock_own(conn: &Connection) {
    let attempt = async {
        let manager = LogindProxy::new(conn).await?;
        let path = manager.get_session("auto").await?;
        LogindSessionProxy::builder(conn)
            .path(path)?
            .build()
            .await?
            .lock()
            .await
    }
    .await;
    if let Err(e) = attempt {
        log::warn!("switch-user: could not lock own session: {e}");
    }
}

async fn activate(conn: &Connection, session: &str) -> zbus::Result<()> {
    LogindProxy::new(conn)
        .await?
        .activate_session(session)
        .await
}

/// Start a switch greeter on `vt`. Fails when the host has no
/// `/etc/greetd/switch-ttyN.toml` for it, which is how a spare-VT list wider
/// than the configured greeters stays harmless.
async fn start_greeter(conn: &Connection, vt: u32) -> zbus::Result<()> {
    SystemdProxy::new(conn)
        .await?
        .start_unit(&format!("greetd-switch@tty{vt}.service"), "replace")
        .await
        .map(|_| ())
}

/// Land on a greeter: reuse an idle one, else start one on a free spare VT.
///
/// The idle-greeter activation can lose a race with that greeter exiting, so
/// a failure there falls through to starting a fresh one rather than giving up.
async fn to_greeter(conn: &Connection, cfg: &Config, sessions: &[Session]) -> Result<(), String> {
    if let Some(g) = rows::idle_greeter(sessions) {
        match activate(conn, &g.session).await {
            Ok(()) => return Ok(()),
            Err(e) => log::warn!("switch-user: idle greeter {} went away: {e}", g.session),
        }
    }
    let Some(vt) = rows::free_vt(sessions, &cfg.spare_vts) else {
        return Err("no idle greeter or free VT".into());
    };
    start_greeter(conn, vt)
        .await
        .map_err(|e| format!("could not start a greeter on tty{vt}: {e}"))
}

/// `--greeter`: go to the login screen. Picking the user is the greeter's job.
pub async fn goto_greeter(conn: &Connection, cfg: &Config) -> Result<(), String> {
    let sessions = snapshot(conn).await.map_err(|e| e.to_string())?;
    lock_own(conn).await;
    to_greeter(conn, cfg, &sessions).await
}

/// `<user>`: resume that user's session, else put a greeter in front of them.
pub async fn switch_to(conn: &Connection, cfg: &Config, target: &str) -> Result<(), String> {
    if !cfg.users.iter().any(|u| u == target) {
        return Err(format!("unknown user: {target}"));
    }
    // Switching to yourself is a no-op — the picker shows the current user too.
    if crate::lock::auth::current_username().as_deref() == Some(target) {
        return Ok(());
    }
    let sessions = snapshot(conn).await.map_err(|e| e.to_string())?;
    if let Some(sess) = rows::session_for(&sessions, target) {
        lock_own(conn).await;
        return activate(conn, &sess.session)
            .await
            .map_err(|e| format!("could not activate {}: {e}", sess.session));
    }
    lock_own(conn).await;
    to_greeter(conn, cfg, &sessions).await
}

/// No argument: cycle. Next live session above ours, else a greeter for a
/// still-logged-out user, else wrap to the lowest. With two users that's a
/// plain toggle.
pub async fn cycle(conn: &Connection, cfg: &Config) -> Result<(), String> {
    let sessions = snapshot(conn).await.map_err(|e| e.to_string())?;
    let my_vt = own_vt(conn).await;
    lock_own(conn).await;

    if let Some(next) = rows::next_above(&sessions, my_vt) {
        return activate(conn, &next.session)
            .await
            .map_err(|e| format!("could not activate {}: {e}", next.session));
    }
    // Top of the cycle: a logged-out user's greeter slot comes before wrapping.
    if rows::anyone_logged_out(&sessions, &cfg.users)
        && let Some(vt) = rows::free_vt(&sessions, &cfg.spare_vts)
        && start_greeter(conn, vt).await.is_ok()
    {
        return Ok(());
    }
    let Some(wrap) = rows::wrap_target(&sessions, my_vt) else {
        return Err("no other session or free VT to switch to".into());
    };
    activate(conn, &wrap.session)
        .await
        .map_err(|e| format!("could not activate {}: {e}", wrap.session))
}

/// The VT we're on, or 0 when we aren't on one (ssh, serial) — which makes
/// every VT count as "above us" and the cycle start from the bottom.
async fn own_vt(conn: &Connection) -> u32 {
    let vt = async {
        let manager = LogindProxy::new(conn).await.ok()?;
        let path = manager.get_session("auto").await.ok()?;
        let props = zbus::fdo::PropertiesProxy::builder(conn)
            .destination("org.freedesktop.login1")
            .ok()?
            .path(path)
            .ok()?
            .build()
            .await
            .ok()?;
        let all = props.get_all(session_iface()).await.ok()?;
        prop(&all, "TTY").strip_prefix("tty")?.parse().ok()
    }
    .await;
    vt.unwrap_or(0)
}
