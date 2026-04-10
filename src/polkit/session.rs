//! Resolve the current logind session id used as the polkit Subject.
//!
//! polkit needs an `(sa{sv})` Subject of kind `unix-session` with a
//! `session-id` detail so it can route auth requests to the right agent.

use std::collections::HashMap;
use zbus::zvariant::{OwnedValue, Value};

/// Try `$XDG_SESSION_ID` first (always set by `pam_systemd` in any logind
/// session), then fall back to `logind.GetSessionByPID(self_pid)`.
pub async fn current_session_id(system_bus: &zbus::Connection) -> zbus::Result<String> {
    if let Ok(id) = std::env::var("XDG_SESSION_ID")
        && !id.is_empty()
    {
        return Ok(id);
    }

    let manager = zbus::Proxy::new(
        system_bus,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )
    .await?;

    let pid: u32 = std::process::id();
    let path: zbus::zvariant::OwnedObjectPath =
        manager.call("GetSessionByPID", &(pid,)).await?;

    let session = zbus::Proxy::new(
        system_bus,
        "org.freedesktop.login1",
        path.as_str(),
        "org.freedesktop.login1.Session",
    )
    .await?;

    let id: String = session.get_property("Id").await?;
    Ok(id)
}

/// Build the polkit Subject details map for a `unix-session` subject.
pub fn subject_details(session_id: &str) -> zbus::Result<HashMap<String, OwnedValue>> {
    let mut details = HashMap::new();
    let value = Value::from(session_id.to_string());
    details.insert("session-id".to_string(), OwnedValue::try_from(value)?);
    Ok(details)
}
