//! logind integration: sleep delay-inhibitor + PrepareForSleep, and the
//! session's Lock/Unlock signals (`loginctl lock-session` → Lock).
//!
//! Same thread pattern as lock::fprint — a dedicated thread hosting a
//! current-thread tokio runtime for zbus, events out via std mpsc, commands
//! in via a tokio channel.
//!
//! The inhibitor is held whenever the system is awake, so suspend always
//! waits for the locker (released by the main loop once the locker is up,
//! re-taken here on PrepareForSleep(false)).

use std::sync::mpsc::Sender;

use zbus::export::futures_util::StreamExt;
use zbus::zvariant::{OwnedFd, OwnedObjectPath};

use super::Ev;

/// ListSessions row: (session_id, uid, user, seat, object_path).
type SessionRow = (String, u32, String, String, OwnedObjectPath);

#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait Manager {
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
trait Session {
    #[zbus(signal)]
    fn lock(&self) -> zbus::Result<()>;
    #[zbus(signal)]
    fn unlock(&self) -> zbus::Result<()>;
    /// Whether this session owns the seat right now (fast user switching /
    /// VT changes flip it).
    #[zbus(property)]
    fn active(&self) -> zbus::Result<bool>;
}

/// Handle for the main loop to command the logind thread.
pub struct Logind {
    tx: tokio::sync::mpsc::UnboundedSender<Cmd>,
}

enum Cmd {
    ReleaseSleepInhibitor,
}

impl Logind {
    pub fn release_sleep_inhibitor(&self) {
        let _ = self.tx.send(Cmd::ReleaseSleepInhibitor);
    }
}

pub fn start(ev: Sender<Ev>) -> Logind {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    std::thread::Builder::new()
        .name("idle-logind".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            if let Err(e) = rt.block_on(run(ev.clone(), rx)) {
                let _ = ev.send(Ev::Fatal(format!("logind: {e}")));
            }
        })
        .expect("spawn idle-logind thread");
    Logind { tx }
}

async fn run(
    ev: Sender<Ev>,
    mut cmd: tokio::sync::mpsc::UnboundedReceiver<Cmd>,
) -> zbus::Result<()> {
    let conn = zbus::Connection::system().await?;
    let manager = ManagerProxy::new(&conn).await?;

    // "auto" resolves to the caller's session or, for processes outside any
    // session (this is a user service under user@.service), the user's
    // display session. Fall back to scanning for our uid's seated session.
    let session_path = match manager.get_session("auto").await {
        Ok(p) => p,
        Err(e) => {
            log::warn!("logind: GetSession(auto) failed ({e}); scanning sessions");
            find_session(&manager).await?
        }
    };
    log::info!("logind: session {}", session_path.as_str());
    let session = SessionProxy::builder(&conn)
        .path(session_path)?
        .build()
        .await?;

    let mut inhibitor = Some(take_inhibitor(&manager).await?);
    let mut prepare = manager.receive_prepare_for_sleep().await?;
    let mut lock_sig = session.receive_lock().await?;
    let mut unlock_sig = session.receive_unlock().await?;
    let mut active_changes = session.receive_active_changed().await;
    if let Ok(a) = session.active().await {
        let _ = ev.send(Ev::SessionActive(a));
    }

    loop {
        tokio::select! {
            Some(change) = active_changes.next() => {
                if let Ok(a) = change.get().await {
                    let _ = ev.send(Ev::SessionActive(a));
                }
            }
            Some(sig) = prepare.next() => {
                let start = sig.args()?.start;
                if !start && inhibitor.is_none() {
                    match take_inhibitor(&manager).await {
                        Ok(fd) => inhibitor = Some(fd),
                        Err(e) => log::warn!("logind: re-take inhibitor failed: {e}"),
                    }
                }
                let _ = ev.send(Ev::Sleep(start));
            }
            Some(_) = lock_sig.next() => { let _ = ev.send(Ev::LockSignal); }
            Some(_) = unlock_sig.next() => { let _ = ev.send(Ev::UnlockSignal); }
            recv = cmd.recv() => match recv {
                Some(Cmd::ReleaseSleepInhibitor) => {
                    if inhibitor.take().is_some() {
                        log::info!("logind: sleep inhibitor released");
                    }
                }
                None => return Ok(()), // main loop gone
            }
        }
    }
}

async fn take_inhibitor(manager: &ManagerProxy<'_>) -> zbus::Result<OwnedFd> {
    let fd = manager
        .inhibit("sleep", "swaypplet", "lock screen before sleep", "delay")
        .await?;
    log::info!("logind: sleep delay-inhibitor taken");
    Ok(fd)
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
