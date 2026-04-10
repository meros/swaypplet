//! D-Bus implementation of `org.freedesktop.PolicyKit1.AuthenticationAgent`.
//!
//! Mirrors the threading model of `notifications/dbus.rs`: a dedicated
//! background thread runs a current-thread tokio runtime hosting the zbus
//! object server. RPC calls are forwarded to the GTK main thread over a
//! `std::sync::mpsc` channel; the main thread fulfils each request and
//! signals completion through a `tokio::sync::oneshot::Sender` carried in
//! the event payload.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::oneshot;
use zbus::interface;
use zbus::zvariant::OwnedValue;

use super::session;

/// One identity polkit is willing to accept for this auth request.
/// Right now we only handle `unix-user`; group identities are rare and the
/// upstream agents don't really handle them either.
#[derive(Debug, Clone)]
pub struct ResolvedIdentity {
    pub username: String,
    pub uid: u32,
}

/// A polkit auth request, normalised for the GTK main thread.
#[derive(Debug, Clone)]
pub struct AuthRequest {
    pub action_id: String,
    pub message: String,
    pub icon_name: String,
    pub details: HashMap<String, String>,
    pub cookie: String,
    pub identities: Vec<ResolvedIdentity>,
}

/// Outcome reported back from the GTK main thread once the user has
/// either authenticated, given up, or hit an unrecoverable error.
#[derive(Debug)]
pub enum AuthOutcome {
    /// Auth succeeded. polkit-agent-helper-1 has already called
    /// `AuthenticationAgentResponse2`; we just need to return Ok().
    Success,
    /// User cancelled. Return an error so polkit knows to deny.
    Cancelled,
    /// Something went wrong (helper missing, spawn failed, etc.).
    Error(String),
}

/// Events sent from the zbus thread → GTK main thread.
pub enum AgentEvent {
    Begin {
        request: AuthRequest,
        reply: oneshot::Sender<AuthOutcome>,
    },
    Cancel {
        cookie: String,
    },
}

/// The zbus interface object. Holds only a Send+Sync handle to the event
/// channel — all real work happens on the main thread.
pub struct Agent {
    sender: Arc<Mutex<std::sync::mpsc::Sender<AgentEvent>>>,
}

#[interface(name = "org.freedesktop.PolicyKit1.AuthenticationAgent")]
impl Agent {
    /// polkit calls this when something needs authorisation. The call
    /// blocks (from polkit's POV) until we respond — that's how we report
    /// success/failure.
    async fn begin_authentication(
        &self,
        action_id: String,
        message: String,
        icon_name: String,
        details: HashMap<String, String>,
        cookie: String,
        identities: Vec<(String, HashMap<String, OwnedValue>)>,
    ) -> zbus::fdo::Result<()> {
        log::info!(
            "polkit BeginAuthentication: action={action_id} cookie={cookie} \
             identities={}",
            identities.len()
        );

        let resolved: Vec<ResolvedIdentity> = identities
            .iter()
            .filter_map(|(kind, ident_details)| {
                if kind != "unix-user" {
                    return None;
                }
                let uid_value = ident_details.get("uid")?;
                let uid = u32::try_from(&**uid_value).ok()?;
                let username = lookup_username(uid)
                    .unwrap_or_else(|| format!("uid {uid}"));
                Some(ResolvedIdentity { username, uid })
            })
            .collect();

        if resolved.is_empty() {
            return Err(zbus::fdo::Error::Failed(
                "no usable unix-user identities in auth request".into(),
            ));
        }

        let (reply_tx, reply_rx) = oneshot::channel();
        let request = AuthRequest {
            action_id,
            message,
            icon_name,
            details,
            cookie,
            identities: resolved,
        };

        if self
            .sender
            .lock()
            .unwrap()
            .send(AgentEvent::Begin {
                request,
                reply: reply_tx,
            })
            .is_err()
        {
            return Err(zbus::fdo::Error::Failed(
                "polkit dialog dispatch channel closed".into(),
            ));
        }

        match reply_rx.await {
            Ok(AuthOutcome::Success) => Ok(()),
            Ok(AuthOutcome::Cancelled) => Err(zbus::fdo::Error::Failed(
                "authentication cancelled by user".into(),
            )),
            Ok(AuthOutcome::Error(msg)) => Err(zbus::fdo::Error::Failed(msg)),
            Err(_) => Err(zbus::fdo::Error::Failed(
                "polkit dialog dropped reply channel".into(),
            )),
        }
    }

    async fn cancel_authentication(&self, cookie: String) -> zbus::fdo::Result<()> {
        log::info!("polkit CancelAuthentication: cookie={cookie}");
        let _ = self
            .sender
            .lock()
            .unwrap()
            .send(AgentEvent::Cancel { cookie });
        Ok(())
    }
}

/// Spin up the zbus server, register the agent interface at
/// `/dev/swaypplet/PolkitAgent`, and call
/// `RegisterAuthenticationAgent` on the polkit Authority. Returns the
/// receiving half of the event channel — to be polled on the GTK main
/// thread via `glib::timeout_add_local`.
pub fn start() -> std::sync::mpsc::Receiver<AgentEvent> {
    let (tx, rx) = std::sync::mpsc::channel::<AgentEvent>();
    let sender = Arc::new(Mutex::new(tx));

    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("polkit agent: failed to create tokio runtime");

        rt.block_on(async move {
            if let Err(e) = run(sender).await {
                log::error!("polkit agent thread exited: {e}");
            }
        });
    });

    rx
}

const AGENT_OBJECT_PATH: &str = "/dev/swaypplet/PolkitAgent";

async fn run(sender: Arc<Mutex<std::sync::mpsc::Sender<AgentEvent>>>) -> zbus::Result<()> {
    let conn = zbus::Connection::system().await?;
    let agent = Agent { sender };

    conn.object_server().at(AGENT_OBJECT_PATH, agent).await?;

    let session_id = session::current_session_id(&conn).await?;
    log::info!("polkit agent: registering for session {session_id}");

    let authority = zbus::Proxy::new(
        &conn,
        "org.freedesktop.PolicyKit1",
        "/org/freedesktop/PolicyKit1/Authority",
        "org.freedesktop.PolicyKit1.Authority",
    )
    .await?;

    let subject_kind = "unix-session".to_string();
    let subject_details = session::subject_details(&session_id)?;
    let subject = (subject_kind, subject_details);

    let locale = std::env::var("LANG").unwrap_or_else(|_| "en_US.UTF-8".into());
    let object_path = AGENT_OBJECT_PATH.to_string();

    if let Err(e) = authority
        .call::<_, _, ()>(
            "RegisterAuthenticationAgent",
            &(&subject, locale.as_str(), object_path.as_str()),
        )
        .await
    {
        log::error!(
            "polkit RegisterAuthenticationAgent failed: {e} — \
             is another agent already registered?"
        );
        return Err(e);
    }

    log::info!("polkit agent registered at {AGENT_OBJECT_PATH}");

    // Park forever — when the process exits the bus closes and polkit
    // automatically forgets us.
    std::future::pending::<()>().await;
    Ok(())
}

/// Resolve a uid to a login name via getpwuid_r.
fn lookup_username(uid: u32) -> Option<String> {
    use std::ffi::CStr;
    let mut buf = vec![0u8; 1024];
    let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
    let mut result: *mut libc::passwd = std::ptr::null_mut();
    let rc = unsafe {
        libc::getpwuid_r(
            uid as libc::uid_t,
            &mut pwd,
            buf.as_mut_ptr() as *mut _,
            buf.len(),
            &mut result,
        )
    };
    if rc != 0 || result.is_null() {
        return None;
    }
    unsafe {
        if pwd.pw_name.is_null() {
            return None;
        }
        Some(CStr::from_ptr(pwd.pw_name).to_string_lossy().into_owned())
    }
}

