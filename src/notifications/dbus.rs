use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use zbus::zvariant::Value;
use zbus::{interface, Connection, SignalContext};

use super::store::{self, NotificationStore};
use super::{CloseReason, Notification, Urgency};

// ── Events sent from the D-Bus thread to the GTK main thread ─────────────

enum DbusEvent {
    Notify(Notification, std::sync::mpsc::Sender<u32>),
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

// ── D-Bus interface struct (must be Send + Sync) ─────────────────────────

/// D-Bus interface implementing `org.freedesktop.Notifications` (spec v1.2).
pub struct NotificationServer {
    sender: Arc<Mutex<EventSender>>,
}

#[interface(name = "org.freedesktop.Notifications")]
impl NotificationServer {
    async fn get_capabilities(&self) -> Vec<String> {
        vec![
            "body".into(),
            "body-markup".into(),
            "actions".into(),
            "persistence".into(),
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
        _app_icon: &str,
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
        };

        // Send to main thread and wait for the assigned ID
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        {
            let sender = self.sender.lock().unwrap();
            let _ = sender.tx.send(DbusEvent::Notify(notif, reply_tx));
        }

        // Wait for the main thread to process and return the ID
        match reply_rx.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(id) => Ok(id),
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

    // Spawn the D-Bus connection on a background thread
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to create tokio runtime for D-Bus");

        rt.block_on(async move {
            match Connection::session().await {
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
                                .interface::<_, NotificationServer>(
                                    "/org/freedesktop/Notifications",
                                )
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
                                        NotificationServer::notification_closed(ctxt, id, reason)
                                            .await
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
                            log::error!(
                                "Is another notification daemon running? (pkill mako)"
                            );
                        }
                    }
                }
                Err(e) => {
                    log::error!("Failed to connect to session bus: {e}");
                }
            }
        });
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
                    let id = store::store_add(&store, notif);
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
