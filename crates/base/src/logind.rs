//! logind's D-Bus interfaces, and the session this process belongs to.
//!
//! The one copy of them. The idle manager (`idle::logind`: the sleep
//! inhibitor and the Lock/Unlock signals), the lock (Unlock, and the
//! fingerprint and face gates), the fingerprint engine and its agent
//! (session-active and sleep gating), user switching (activate, unlock,
//! lock), the polkit agent (its session id) and the theme (LockedHint) all
//! talk to logind through here.

use std::time::Duration;

use zbus::export::futures_util::StreamExt;
use zbus::zvariant::{OwnedFd, OwnedObjectPath};

/// ListSessions row: (session_id, uid, user, seat, object_path).
pub type SessionRow = (String, u32, String, String, OwnedObjectPath);

#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
pub trait Manager {
    /// Delay inhibitor: sleep waits (bounded by logind's InhibitDelayMaxSec)
    /// until every holder closes the returned fd.
    fn inhibit(&self, what: &str, who: &str, why: &str, mode: &str) -> zbus::Result<OwnedFd>;
    /// `"auto"` resolves the caller's own session; a real id resolves that
    /// one. Works for user@.service children, where GetSessionByPID fails
    /// because the PID isn't in any session cgroup.
    fn get_session(&self, session_id: &str) -> zbus::Result<OwnedObjectPath>;
    #[zbus(name = "GetSessionByPID")]
    fn get_session_by_pid(&self, pid: u32) -> zbus::Result<OwnedObjectPath>;
    /// The call [`SessionGate`]'s pid lookup has always made. zbus derives
    /// the wire name from the Rust one, and `get_session_by_pid` became
    /// `GetSessionByPid`, which logind does not have: it answers
    /// UnknownMethod, so a `Client` gate never resolves and settles on
    /// "active", and an `Own` gate only ever resolves through "auto". Kept
    /// as it was, deliberately, until someone decides that the fp-agent
    /// should start releasing the reader behind a backgrounded greeter.
    #[zbus(name = "GetSessionByPid")]
    fn get_session_by_pid_as_gated(&self, pid: u32) -> zbus::Result<OwnedObjectPath>;
    fn list_sessions(&self) -> zbus::Result<Vec<SessionRow>>;
    /// `loginctl activate` — the VT switch itself.
    fn activate_session(&self, session_id: &str) -> zbus::Result<()>;
    /// `loginctl unlock-session`: logind emits Unlock on that session, and
    /// its locker ends. Allowed without polkit for the session's own user.
    fn unlock_session(&self, session_id: &str) -> zbus::Result<()>;
    #[zbus(signal)]
    fn prepare_for_sleep(&self, start: bool) -> zbus::Result<()>;
    /// Current sleep-transition state — `true` between PrepareForSleep(true)
    /// and the matching resume. Read once after subscribing, because the
    /// signal alone only covers *future* transitions.
    #[zbus(property)]
    fn preparing_for_sleep(&self) -> zbus::Result<bool>;
}

#[zbus::proxy(
    interface = "org.freedesktop.login1.Session",
    default_service = "org.freedesktop.login1"
)]
pub trait Session {
    /// Ask logind to lock this session: it emits the Lock signal below, which
    /// our own idle daemon listens for. Named apart from the signal, which
    /// shares the D-Bus name.
    #[zbus(name = "Lock")]
    fn request_lock(&self) -> zbus::Result<()>;
    #[zbus(signal)]
    fn lock(&self) -> zbus::Result<()>;
    #[zbus(signal)]
    fn unlock(&self) -> zbus::Result<()>;
    /// logind's id for the session ("2", "c1"), as polkit's `unix-session`
    /// subject wants it.
    #[zbus(property)]
    fn id(&self) -> zbus::Result<String>;
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

/// A session proxy for the object at `path`.
pub async fn session_at(
    conn: &zbus::Connection,
    path: OwnedObjectPath,
) -> zbus::Result<SessionProxy<'static>> {
    SessionProxy::builder(conn).path(path)?.build().await
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
    session_at(conn, session_path).await
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

// --- LockedHint ------------------------------------------------------------

/// Follow this session's LockedHint, for as long as this process lives: the
/// current value first, then every change, on the returned channel.
///
/// The lock runs in its own process; logind is where it says so. Followed as
/// a property stream, not polled. Without the bus the channel simply never
/// says anything.
pub fn watch_locked_hint() -> async_channel::Receiver<bool> {
    let (tx, rx) = async_channel::unbounded::<bool>();
    crate::spawn::spawn_tokio_thread("logind-locked-hint", async move {
        if let Err(e) = follow_locked_hint(&tx).await {
            log::debug!("logind: not following the lock state: {e}");
        }
    });
    rx
}

async fn follow_locked_hint(tx: &async_channel::Sender<bool>) -> zbus::Result<()> {
    let conn = zbus::Connection::system().await?;
    let manager = ManagerProxy::new(&conn).await?;
    let session = session(&conn, &manager).await?;
    let mut changes = session.receive_locked_hint_changed().await;
    if let Ok(v) = session.locked_hint().await {
        let _ = tx.send(v).await;
    }
    while let Some(change) = changes.next().await {
        if let Ok(v) = change.get().await
            && tx.send(v).await.is_err()
        {
            break;
        }
    }
    Ok(())
}

// --- sleep -----------------------------------------------------------------

/// Take a sleep delay-inhibitor as `who`, for `why`; dropping the fd
/// releases it.
pub async fn inhibit_sleep(
    manager: &ManagerProxy<'_>,
    who: &str,
    why: &str,
) -> zbus::Result<OwnedFd> {
    manager.inhibit("sleep", who, why, "delay").await
}

/// Take a sleep delay-inhibitor; dropping the fd releases it. The
/// fingerprint engine holds it exactly while a claim is held, so the
/// PrepareForSleep handler gets to VerifyStop+Release before logind lets the
/// machine suspend. `None` (logind too old, dev container) just means sleep
/// won't wait for us.
pub async fn take_sleep_inhibitor(
    conn: &zbus::Connection,
    who: &str,
    why: &str,
) -> Option<OwnedFd> {
    let take = async { inhibit_sleep(&ManagerProxy::new(conn).await?, who, why).await };
    match take.await {
        Ok(fd) => Some(fd),
        Err(e) => {
            log::warn!("sleep delay-inhibitor unavailable: {e}");
            None
        }
    }
}

/// Push logind's PrepareForSleep state into `tx`, forever: `true` from
/// PrepareForSleep(start) until the matching resume. Claim holders gate on it
/// and release the reader before sleep — a claim held across suspend leaves
/// the synaptics device wedged open inside fprintd (every later Claim fails
/// with "Device ... is already open" until fprintd restarts). Like
/// `watch_session_active`, never returns: on setup failure it settles on
/// `false` (awake) and parks, so receivers can select on `changed()` freely.
pub async fn watch_sleep(conn: zbus::Connection, tx: tokio::sync::watch::Sender<bool>) {
    let setup = async {
        let manager = ManagerProxy::new(&conn).await?;
        let stream = manager.receive_prepare_for_sleep().await?;
        Ok::<_, zbus::Error>((manager, stream))
    }
    .await;
    match setup {
        Ok((manager, mut stream)) => {
            // Seed with the transition already in flight: a locker spawned
            // *by* the sleep transition (the idle daemon locks on
            // PrepareForSleep) starts after the signal fired and would
            // otherwise claim the reader on the way into suspend.
            // Subscribe-then-read is race-free — a transition between the
            // two lands in the stream and overwrites the seed.
            if let Ok(now) = manager.preparing_for_sleep().await {
                let _ = tx.send(now);
            }
            while let Some(sig) = stream.next().await {
                if let Ok(args) = sig.args() {
                    let _ = tx.send(*args.start());
                }
            }
            // Stream ended (system bus gone) — hold the last value.
        }
        Err(e) => {
            log::warn!("sleep watch unavailable, claims will span suspend: {e}");
            let _ = tx.send(false);
        }
    }
    std::future::pending::<()>().await;
}

// --- session-active gating -------------------------------------------------

/// Which logind session a gate follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionGate {
    /// The caller's *own* session (GetSession("auto"), then the pid as a
    /// fallback). Used by the lock, which runs inside the user's session.
    Own(u32),
    /// A specific *client's* session by pid — never "auto". Used by the
    /// fp-agent: it's a root daemon with no session of its own, so it must
    /// gate each greeter on that greeter's session, not the daemon's.
    Client(u32),
}

/// One logind lookup a [`SessionGate`] resolves through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lookup {
    Auto,
    ByPid(u32),
}

impl SessionGate {
    /// The lookups to try, in order; the first that answers wins.
    fn lookups(self) -> Vec<Lookup> {
        match self {
            SessionGate::Own(pid) => vec![Lookup::Auto, Lookup::ByPid(pid)],
            SessionGate::Client(pid) => vec![Lookup::ByPid(pid)],
        }
    }
}

const RESOLVE_ATTEMPTS: u32 = 5;
const RESOLVE_BACKOFF: Duration = Duration::from_millis(500);

/// Resolve the logind session proxy for `gate` (see [`SessionGate`]).
pub async fn resolve_session(
    conn: &zbus::Connection,
    gate: SessionGate,
) -> Result<SessionProxy<'static>, String> {
    let manager = ManagerProxy::new(conn).await.map_err(|e| e.to_string())?;
    let mut errors = Vec::new();
    for lookup in gate.lookups() {
        let answer = match lookup {
            Lookup::Auto => manager
                .get_session("auto")
                .await
                .map_err(|e| format!("GetSession(auto): {e}")),
            Lookup::ByPid(pid) => manager
                .get_session_by_pid_as_gated(pid)
                .await
                .map_err(|e| format!("GetSessionByPID: {e}")),
        };
        match answer {
            Ok(path) => return session_at(conn, path).await.map_err(|e| e.to_string()),
            Err(e) => errors.push(e),
        }
    }
    Err(errors.join("; "))
}

/// Push `gate`'s logind-session Active state into `tx`, forever. Never
/// returns: when the session can't be resolved (dev box, session gone) it
/// settles on `true` and parks, so receivers can rely on the channel staying
/// open and select on `changed()` without a busy-loop guard.
pub async fn watch_session_active(
    conn: zbus::Connection,
    gate: SessionGate,
    tx: tokio::sync::watch::Sender<bool>,
) {
    // Resolve once, with bounded retry: session registration can race the
    // locker/agent starting (fast VT handoff, cold boot). Giving up too early
    // is what left an inactive locker holding the fingerprint reader.
    let mut session = None;
    for attempt in 0..RESOLVE_ATTEMPTS {
        match resolve_session(&conn, gate).await {
            Ok(s) => {
                session = Some(s);
                break;
            }
            Err(e) => {
                let last = attempt + 1 == RESOLVE_ATTEMPTS;
                log::log!(
                    if last {
                        log::Level::Warn
                    } else {
                        log::Level::Debug
                    },
                    "session resolve attempt {}/{RESOLVE_ATTEMPTS} failed ({e})",
                    attempt + 1
                );
                if !last {
                    tokio::time::sleep(RESOLVE_BACKOFF).await;
                }
            }
        }
    }
    match session {
        Some(session) => {
            let mut stream = session.receive_active_changed().await;
            let _ = tx.send(session.active().await.unwrap_or(true));
            while let Some(change) = stream.next().await {
                if let Ok(v) = change.get().await {
                    let _ = tx.send(v);
                }
            }
            // Session object vanished (logged out) — hold the last value.
        }
        None => {
            log::warn!("no logind session to gate on after retries; treating as active");
            let _ = tx.send(true);
        }
    }
    std::future::pending::<()>().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_session_asks_auto_first_then_the_pid() {
        assert_eq!(
            SessionGate::Own(42).lookups(),
            [Lookup::Auto, Lookup::ByPid(42)]
        );
    }

    #[test]
    fn a_client_session_is_never_resolved_through_auto() {
        // "auto" would answer with the root daemon's own (absent) session:
        // the agent gates each greeter on that greeter's pid.
        assert_eq!(SessionGate::Client(7).lookups(), [Lookup::ByPid(7)]);
    }
}
