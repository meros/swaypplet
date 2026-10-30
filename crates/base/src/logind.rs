//! logind's D-Bus interfaces, and the session this process belongs to.
//!
//! Shared by the idle manager (`swaypplet_idle::logind`: the sleep inhibitor
//! and the Lock/Unlock signals) and the theme (`swaypplet_ui::theme::locked`:
//! LockedHint).

use zbus::zvariant::{OwnedFd, OwnedObjectPath};

/// ListSessions row: (session_id, uid, user, seat, object_path).
type SessionRow = (String, u32, String, String, OwnedObjectPath);

#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
pub trait Manager {
    fn inhibit(&self, what: &str, who: &str, why: &str, mode: &str) -> zbus::Result<OwnedFd>;
    fn get_session(&self, session_id: &str) -> zbus::Result<OwnedObjectPath>;
    fn list_sessions(&self) -> zbus::Result<Vec<SessionRow>>;
    #[zbus(signal)]
    fn prepare_for_sleep(&self, start: bool) -> zbus::Result<()>;
}

#[zbus::proxy(
    interface = "org.freedesktop.login1.Session",
    default_service = "org.freedesktop.login1"
)]
pub trait Session {
    #[zbus(signal)]
    fn lock(&self) -> zbus::Result<()>;
    #[zbus(signal)]
    fn unlock(&self) -> zbus::Result<()>;
    /// Whether this session owns the seat right now (fast user switching /
    /// VT changes flip it).
    #[zbus(property)]
    fn active(&self) -> zbus::Result<bool>;
    /// logind's record of whether this session is locked. We set it true once
    /// the locker is up and false on a clean unlock; a fresh idle instance
    /// reads it at startup to detect a lock that outlived a service restart
    /// (KillMode took the old locker down with the cgroup).
    #[zbus(property)]
    fn locked_hint(&self) -> zbus::Result<bool>;
    fn set_locked_hint(&self, locked: bool) -> zbus::Result<()>;
}

/// This user's graphical session on logind.
///
/// "auto" resolves to the caller's session or, for processes outside any
/// session (a user service under user@.service), the user's display
/// session. Falls back to scanning for our uid's seated session.
pub async fn session(
    conn: &zbus::Connection,
    manager: &ManagerProxy<'_>,
) -> zbus::Result<SessionProxy<'static>> {
    let session_path = match manager.get_session("auto").await {
        Ok(p) => p,
        Err(e) => {
            log::warn!("logind: GetSession(auto) failed ({e}); scanning sessions");
            find_session(manager).await?
        }
    };
    log::info!("logind: session {}", session_path.as_str());
    SessionProxy::builder(conn)
        .path(session_path)?
        .build()
        .await
}

/// Fallback session lookup: prefer our uid's seated session, else any
/// session of ours.
async fn find_session(manager: &ManagerProxy<'_>) -> zbus::Result<OwnedObjectPath> {
    let uid = unsafe { libc::getuid() };
    let sessions = manager.list_sessions().await?;
    let mine: Vec<_> = sessions.into_iter().filter(|s| s.1 == uid).collect();
    mine.iter()
        .find(|s| !s.3.is_empty())
        .or_else(|| mine.first())
        .map(|s| s.4.clone())
        .ok_or_else(|| zbus::Error::Failure("no logind session for this uid".into()))
}
