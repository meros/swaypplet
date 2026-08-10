use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use zbus::zvariant::Value;
use zbus::{SignalContext, interface};

use super::store::{self, NotificationStore};
use super::{CloseReason, ImageData, ImageSource, Notification, Urgency};

// ── Events sent from the D-Bus thread to the GTK main thread ─────────────

enum DbusEvent {
    /// The reply channel is a `tokio::sync::oneshot` so the D-Bus side can
    /// *await* the assigned ID instead of blocking its thread on it. See
    /// `notify()` for why blocking here wedges the whole session bus.
    /// Boxed: the notification carries inline image bytes, and an enum
    /// sized for its largest variant would make every `Close` that big too.
    Notify(Box<Notification>, tokio::sync::oneshot::Sender<u32>),
    Close(u32),
}

/// Outgoing signal requested by the GTK thread, forwarded to the D-Bus thread.
/// Uses a `tokio::sync::mpsc` channel because `UnboundedSender::send` is a
/// plain sync call, so the GTK thread can use it without touching the runtime.
enum SignalEvent {
    Closed(u32, u32),
    ActionInvoked(u32, String),
}

/// Thread-safe sender for D-Bus → main thread communication.
struct EventSender {
    tx: std::sync::mpsc::Sender<DbusEvent>,
}

// ── Hint parsing ─────────────────────────────────────────────────────────

/// A string hint. Senders are inconsistent about whether a "string" arrives
/// as one or wrapped in a variant, so both are unwrapped.
fn hint_str(hints: &HashMap<String, Value<'_>>, key: &str) -> Option<String> {
    match unwrap_variant(hints.get(key)?) {
        Value::Str(s) => Some(s.to_string()),
        _ => None,
    }
}

fn hint_bool(hints: &HashMap<String, Value<'_>>, key: &str) -> Option<bool> {
    match unwrap_variant(hints.get(key)?) {
        Value::Bool(b) => Some(*b),
        Value::U8(u) => Some(*u != 0),
        Value::I32(i) => Some(*i != 0),
        _ => None,
    }
}

/// One level of `Value::Value` indirection. `notify-send` and GLib both send
/// hints boxed this way; reading them raw is why several hints looked absent.
fn unwrap_variant<'a>(v: &'a Value<'a>) -> &'a Value<'a> {
    match v {
        Value::Value(inner) => inner,
        other => other,
    }
}

/// A path or a themed icon name, told apart the way the spec does.
fn image_from_str(s: String) -> ImageSource {
    if let Some(rest) = s.strip_prefix("file://") {
        return ImageSource::Path(percent_decode(rest).into());
    }
    if s.starts_with('/') {
        return ImageSource::Path(s.into());
    }
    ImageSource::Named(s)
}

/// Minimal percent-decoding for `file://` URIs. Only the escapes a file name
/// actually produces; anything malformed is left as written rather than
/// guessed at.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(byte) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The spec's `(iiibiiay)` image struct. Rejected rather than trusted when
/// the declared geometry does not match the byte count: the renderer hands
/// this straight to a pixbuf, which reads `rowstride * height` bytes and does
/// not check.
fn hint_image_data(hints: &HashMap<String, Value<'_>>, key: &str) -> Option<ImageSource> {
    let Value::Structure(st) = unwrap_variant(hints.get(key)?) else {
        return None;
    };
    let f = st.fields();
    if f.len() < 7 {
        return None;
    }
    let int = |v: &Value| match v {
        Value::I32(i) => Some(*i),
        Value::U32(u) => Some(*u as i32),
        _ => None,
    };
    let width = int(&f[0])?;
    let height = int(&f[1])?;
    let rowstride = int(&f[2])?;
    let has_alpha = matches!(&f[3], Value::Bool(b) if *b);
    let bits_per_sample = int(&f[4])?;
    let channels = int(&f[5])?;
    let Value::Array(arr) = &f[6] else {
        return None;
    };
    let data: Vec<u8> = arr
        .iter()
        .filter_map(|v| match v {
            Value::U8(b) => Some(*b),
            _ => None,
        })
        .collect();

    if width <= 0 || height <= 0 || rowstride <= 0 || bits_per_sample != 8 {
        return None;
    }
    if !(1..=4).contains(&channels) {
        return None;
    }
    // The last row need not be padded to rowstride, which is why this is a
    // lower bound rather than an equality.
    let needed = (rowstride as i64) * (height as i64 - 1) + (width as i64) * (channels as i64);
    if (data.len() as i64) < needed {
        return None;
    }

    Some(ImageSource::Data(ImageData {
        width,
        height,
        rowstride,
        has_alpha,
        bits_per_sample,
        data,
    }))
}

// ── D-Bus interface struct (must be Send + Sync) ─────────────────────────

/// D-Bus interface implementing `org.freedesktop.Notifications` (spec v1.2).
pub struct NotificationServer {
    sender: Arc<Mutex<EventSender>>,
}

#[interface(name = "org.freedesktop.Notifications")]
impl NotificationServer {
    async fn get_capabilities(&self) -> Vec<String> {
        // Senders branch on this list, so anything not named here is a
        // feature they will deliberately downgrade before we ever see it.
        vec![
            "body".into(),
            "body-markup".into(),
            "body-images".into(),
            "body-hyperlinks".into(),
            "icon-static".into(),
            "actions".into(),
            "action-icons".into(),
            "persistence".into(),
            // Non-standard, but the tag every OSD-style sender already uses
            // to replace its own notification in place.
            "x-canonical-private-synchronous".into(),
        ]
    }

    #[zbus(out_args("name", "vendor", "version", "spec_version"))]
    async fn get_server_information(&self) -> zbus::fdo::Result<(String, String, String, String)> {
        Ok((
            "swaypplet".into(),
            "swaypplet".into(),
            env!("CARGO_PKG_VERSION").into(),
            "1.2".into(),
        ))
    }

    #[allow(clippy::too_many_arguments)]
    async fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: Vec<String>,
        hints: HashMap<String, Value<'_>>,
        expire_timeout: i32,
    ) -> zbus::fdo::Result<u32> {
        let urgency = hints
            .get("urgency")
            .and_then(|v| match v {
                Value::U8(u) => Some(*u),
                _ => None,
            })
            .map(Urgency::from)
            .unwrap_or(Urgency::Normal);

        let transient = hints
            .get("transient")
            .and_then(|v| match v {
                Value::Bool(b) => Some(*b),
                Value::U8(u) => Some(*u != 0),
                Value::I32(i) => Some(*i != 0),
                _ => None,
            })
            .unwrap_or(false);

        let progress = hints.get("value").and_then(|v| match v {
            Value::I32(i) => Some(*i as u32),
            Value::U32(u) => Some(*u),
            _ => None,
        });

        // Claude stop-notification attribution (vision O2): the nixos-side
        // hook sends `--hint=int:claude-pid:<PID>`; notify-send ints are I32.
        let claude_pid = hints.get("claude-pid").and_then(|v| match v {
            Value::I32(i) => Some(*i),
            _ => None,
        });

        let category = hint_str(&hints, "category");
        let desktop_entry = hint_str(&hints, "desktop-entry");
        // Both spellings are in the wild: the spec says the first, GLib and
        // several toolkits send the second.
        let sync_tag = hint_str(&hints, "x-canonical-private-synchronous")
            .or_else(|| hint_str(&hints, "x-dunst-stack-tag"));
        let resident = hint_bool(&hints, "resident").unwrap_or(false);
        let action_icons = hint_bool(&hints, "action-icons").unwrap_or(false);

        // Spec precedence for the picture: image-data outranks image-path,
        // and `icon_data` is the 1.1 spelling kept for older senders.
        let image = hint_image_data(&hints, "image-data")
            .or_else(|| hint_image_data(&hints, "image_data"))
            .or_else(|| hint_str(&hints, "image-path").map(image_from_str))
            .or_else(|| hint_str(&hints, "image_path").map(image_from_str))
            .or_else(|| hint_image_data(&hints, "icon_data"));

        // The app's own identity, which is a different question from what the
        // notification is about. Falls back to the desktop entry, since a
        // sender that names one is telling us where to find its icon.
        let icon = (!app_icon.is_empty())
            .then(|| image_from_str(app_icon.to_string()))
            .or_else(|| desktop_entry.clone().map(ImageSource::Named));

        // Parse paired action strings: [id, label, id, label, ...]
        let action_pairs: Vec<(String, String)> = actions
            .chunks(2)
            .filter_map(|chunk| {
                if chunk.len() == 2 {
                    Some((chunk[0].clone(), chunk[1].clone()))
                } else {
                    None
                }
            })
            .collect();

        let notif = Notification {
            id: 0, // assigned by store on main thread
            app_name: app_name.to_string(),
            summary: summary.to_string(),
            body: body.to_string(),
            urgency,
            actions: action_pairs,
            expire_timeout,
            timestamp: std::time::SystemTime::now(),
            transient,
            progress,
            replaces_id,
            claude_pid,
            // Resolved by the store at add time (set_task_resolver).
            task: None,
            suppressed: false,
            icon,
            image,
            category,
            sync_tag,
            resident,
            action_icons,
        };

        // Send to main thread and wait for the assigned ID.
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        {
            // Scoped: a std Mutex guard must not be held across the await
            // below, and `send` on an unbounded channel never blocks.
            let sender = self.sender.lock().unwrap();
            let _ = sender.tx.send(DbusEvent::Notify(Box::new(notif), reply_tx));
        }

        // Await — never block. This runs on the object-server dispatch task,
        // which shares its single-threaded runtime with zbus's socket reader.
        // A blocking wait here stops that reader, and because zbus applies
        // backpressure (`broadcast_direct` into a 64-message queue) the
        // connection then stops draining its socket entirely. dbus-broker
        // charges the resulting backlog to the *user's* byte quota, so one
        // stalled reader here starves every peer on the session bus —
        // xdg-desktop-portal included, which is what made GTK apps (our own
        // lock screen among them) block 25 s on the settings portal.
        match tokio::time::timeout(std::time::Duration::from_secs(5), reply_rx).await {
            Ok(Ok(id)) => Ok(id),
            Ok(Err(_)) => Err(zbus::fdo::Error::Failed(
                "Notification store went away".to_string(),
            )),
            Err(_) => Err(zbus::fdo::Error::Failed(
                "Timed out waiting for notification store".to_string(),
            )),
        }
    }

    async fn close_notification(&self, id: u32) -> zbus::fdo::Result<()> {
        let sender = self.sender.lock().unwrap();
        let _ = sender.tx.send(DbusEvent::Close(id));
        Ok(())
    }

    #[zbus(signal)]
    async fn notification_closed(
        emitter: &SignalContext<'_>,
        id: u32,
        reason: u32,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn action_invoked(
        emitter: &SignalContext<'_>,
        id: u32,
        action_key: &str,
    ) -> zbus::Result<()>;
}

/// Start the D-Bus notification server.
///
/// The zbus server runs on a background thread (tokio). Events are forwarded
/// to the GTK main thread via a channel, where the `NotificationStore` is
/// updated (keeping it safely `Rc<RefCell<>>`).
pub fn start_server(store: Rc<RefCell<NotificationStore>>) {
    let (tx, rx) = std::sync::mpsc::channel::<DbusEvent>();
    let (signal_tx, mut signal_rx) = tokio::sync::mpsc::unbounded_channel::<SignalEvent>();

    let sender = Arc::new(Mutex::new(EventSender { tx }));
    let server = NotificationServer { sender };

    crate::spawn::spawn_tokio_thread("notify-dbus", async move {
        // Headroom over zbus's 64-message default: this connection is the
        // session bus's notification daemon, so it sees bursts (a batch of
        // notify calls, name churn) that a busy GTK main thread can briefly
        // fail to keep pace with. A full queue does not merely slow us down
        // — it halts the socket reader, with the session-wide consequences
        // described in `notify()`. Deep queue + non-blocking `notify()` is
        // belt and braces; neither alone is worth relying on.
        let session = match zbus::ConnectionBuilder::session() {
            Ok(b) => b.max_queued(1024).build().await,
            Err(e) => Err(e),
        };
        match session {
            Ok(conn) => {
                if let Err(e) = conn
                    .object_server()
                    .at("/org/freedesktop/Notifications", server)
                    .await
                {
                    log::error!("Failed to register notification interface: {e}");
                    return;
                }

                match conn.request_name("org.freedesktop.Notifications").await {
                    Ok(_) => {
                        log::info!("Notification D-Bus server started");

                        let iface_ref = match conn
                            .object_server()
                            .interface::<_, NotificationServer>("/org/freedesktop/Notifications")
                            .await
                        {
                            Ok(r) => r,
                            Err(e) => {
                                log::error!("Failed to look up notification interface: {e}");
                                return;
                            }
                        };

                        // Emit signals the GTK thread asks for, for as long as it's alive.
                        while let Some(event) = signal_rx.recv().await {
                            let ctxt = iface_ref.signal_context();
                            let result = match event {
                                SignalEvent::Closed(id, reason) => {
                                    NotificationServer::notification_closed(ctxt, id, reason).await
                                }
                                SignalEvent::ActionInvoked(id, key) => {
                                    NotificationServer::action_invoked(ctxt, id, &key).await
                                }
                            };
                            if let Err(e) = result {
                                log::warn!("Failed to emit notification signal: {e}");
                            }
                        }
                    }
                    Err(e) => {
                        log::error!("Failed to acquire org.freedesktop.Notifications: {e}");
                        log::error!("Is another notification daemon running? (pkill mako)");
                    }
                }
            }
            Err(e) => {
                log::error!("Failed to connect to session bus: {e}");
            }
        }
    });

    // Forward store close/action events to the D-Bus thread as outgoing signals.
    {
        let tx = signal_tx.clone();
        store.borrow_mut().connect_close(move |id, reason| {
            let _ = tx.send(SignalEvent::Closed(id, reason as u32));
        });
    }
    {
        store.borrow_mut().connect_action(move |id, key| {
            let _ = signal_tx.send(SignalEvent::ActionInvoked(id, key.to_string()));
        });
    }

    // Poll the channel on the GTK main thread
    glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
        while let Ok(event) = rx.try_recv() {
            match event {
                DbusEvent::Notify(notif, reply_tx) => {
                    let id = store::store_add(&store, *notif);
                    let _ = reply_tx.send(id);
                }
                DbusEvent::Close(id) => {
                    store::store_close(&store, id, CloseReason::CloseCall);
                }
            }
        }
        glib::ControlFlow::Continue
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_uri_becomes_a_path_and_a_bare_name_stays_a_name() {
        assert!(matches!(
            image_from_str("/tmp/shot.png".into()),
            ImageSource::Path(p) if p == std::path::Path::new("/tmp/shot.png")
        ));
        assert!(matches!(
            image_from_str("file:///tmp/a%20b.png".into()),
            ImageSource::Path(p) if p == std::path::Path::new("/tmp/a b.png")
        ));
        assert!(matches!(
            image_from_str("dialog-information".into()),
            ImageSource::Named(n) if n == "dialog-information"
        ));
    }

    #[test]
    fn percent_decoding_leaves_malformed_escapes_alone() {
        assert_eq!(percent_decode("plain"), "plain");
        assert_eq!(percent_decode("a%20b"), "a b");
        // Not an escape, so not decoded — guessing here would corrupt a
        // filename that legitimately contains a percent sign.
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
    }
}
