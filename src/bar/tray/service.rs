//! SNI host service — `system-tray`'s tokio client on a dedicated thread
//! (same runtime pattern as `idle/logind.rs`), mirroring the item map to
//! the GTK thread the way `sway_ipc` does: full snapshots per event, not
//! deltas, so a lagged broadcast receiver can never leave the cache stale.
//!
//! `system-tray` requires zbus 5 while the rest of swaypplet sits on
//! zbus 4; Cargo carries both majors and nothing crosses this module's
//! boundary except the crate's own plain-data model types.

use std::rc::Rc;
use std::time::{Duration, Instant};

use system_tray::client::{ActivateRequest, Client};
use system_tray::item::StatusNotifierItem;
use system_tray::menu::TrayMenu;
use tokio::sync::broadcast::error::RecvError;

use crate::service::{Backoff, Observed};

/// One SNI item as the bar renders it.
#[derive(Debug, Clone)]
pub struct TrayItem {
    /// Bus address (unique name) — the stable key for widget reconciliation.
    pub address: String,
    pub sni: StatusNotifierItem,
    /// Parsed DBusMenu layout, once the client has fetched it.
    pub menu: Option<TrayMenu>,
}

enum Cmd {
    Activate(ActivateRequest),
    AboutToShow { address: String, menu_path: String },
}

/// SNI item service; [`Observed`] documents the `Rc` lifetime story.
pub struct TrayService {
    items: Observed<Vec<TrayItem>>,
    cmd: tokio::sync::mpsc::UnboundedSender<Cmd>,
}

impl TrayService {
    pub fn start() -> Rc<Self> {
        let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel();
        let service = Rc::new(Self {
            items: Observed::new(Vec::new()),
            cmd: cmd_tx,
        });

        let (tx, rx) = async_channel::unbounded::<Vec<TrayItem>>();
        crate::spawn::spawn_tokio_thread("tray", run(tx, cmd_rx));

        let for_events = service.clone();
        glib::MainContext::default().spawn_local(async move {
            while let Ok(items) = rx.recv().await {
                for_events.items.set(items);
            }
        });

        service
    }

    pub fn connect_change(&self, cb: impl Fn() + 'static) {
        self.items.connect_change(cb);
    }

    /// Current items, sorted for a stable bar order.
    pub fn items(&self) -> Vec<TrayItem> {
        self.items.with(Clone::clone)
    }

    pub fn item(&self, address: &str) -> Option<TrayItem> {
        self.items
            .with(|items| items.iter().find(|i| i.address == address).cloned())
    }

    /// Left-click activation. Coordinates are a hint the spec allows us to
    /// omit — Wayland has no global coordinates to offer anyway.
    pub fn activate_default(&self, address: &str) {
        let _ = self.cmd.send(Cmd::Activate(ActivateRequest::Default {
            address: address.into(),
            x: 0,
            y: 0,
        }));
    }

    /// Middle-click (secondary) activation.
    pub fn activate_secondary(&self, address: &str) {
        let _ = self.cmd.send(Cmd::Activate(ActivateRequest::Secondary {
            address: address.into(),
            x: 0,
            y: 0,
        }));
    }

    /// "clicked" event for a menu entry.
    pub fn menu_item_clicked(&self, address: &str, menu_path: &str, id: i32) {
        let _ = self.cmd.send(Cmd::Activate(ActivateRequest::MenuItem {
            address: address.into(),
            menu_path: menu_path.into(),
            submenu_id: id,
        }));
    }

    /// Spec's AboutToShow(0) — fired before opening a menu so lazy apps
    /// populate it. A "needs update" answer arrives as a normal layout
    /// event, so the result is not routed back.
    pub fn about_to_show(&self, address: &str, menu_path: &str) {
        let _ = self.cmd.send(Cmd::AboutToShow {
            address: address.into(),
            menu_path: menu_path.into(),
        });
    }
}

// ── Worker (tokio thread) ───────────────────────────────────────────────

async fn run(
    tx: async_channel::Sender<Vec<TrayItem>>,
    mut cmd: tokio::sync::mpsc::UnboundedReceiver<Cmd>,
) {
    let mut backoff = Backoff::new();
    loop {
        let started = Instant::now();
        match session(&tx, &mut cmd).await {
            Ok(()) => return, // receiver gone — process is shutting down
            Err(e) => {
                let delay = backoff.next_delay(started.elapsed());
                log::warn!("tray: {e}; reconnecting in {delay:?}");
                tokio::time::sleep(delay).await;
            }
        }
    }
}

/// One client lifetime. `Ok(())` means the GTK side hung up; `Err` means
/// the bus/stream dropped and the caller should reconnect.
async fn session(
    tx: &async_channel::Sender<Vec<TrayItem>>,
    cmd: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>,
) -> Result<(), system_tray::error::Error> {
    let client = Client::new().await?;
    let mut events = client.subscribe();
    if tx.send(snapshot(&client)).await.is_err() {
        return Ok(());
    }

    loop {
        tokio::select! {
            event = events.recv() => {
                match event {
                    // The event itself is irrelevant — the client's "data"
                    // feature already applied it (menu diffs included) to
                    // the item map we snapshot. That also makes Lagged
                    // harmless: the map is current regardless.
                    Ok(_) | Err(RecvError::Lagged(_)) => {
                        if tx.send(snapshot(&client)).await.is_err() {
                            return Ok(());
                        }
                    }
                    Err(RecvError::Closed) => {
                        return Err(system_tray::error::Error::InvalidData(
                            "tray event stream closed",
                        ));
                    }
                }
            }
            recv = cmd.recv() => match recv {
                // Both calls proxy to apps that may hang; bound them so a
                // stuck app can't wedge event forwarding for good.
                Some(Cmd::Activate(req)) => {
                    match tokio::time::timeout(Duration::from_secs(2), client.activate(req)).await {
                        Ok(Err(e)) => log::warn!("tray: activate failed: {e}"),
                        Err(_) => log::warn!("tray: activate timed out"),
                        Ok(Ok(())) => {}
                    }
                }
                Some(Cmd::AboutToShow { address, menu_path }) => {
                    let call = client.about_to_show_menuitem(address, menu_path, 0);
                    if let Ok(Err(e)) = tokio::time::timeout(Duration::from_secs(1), call).await {
                        log::warn!("tray: AboutToShow failed: {e}");
                    }
                }
                None => return Ok(()), // main loop gone
            }
        }
    }
}

fn snapshot(client: &Client) -> Vec<TrayItem> {
    let map = client.items();
    let map = map.lock().expect("tray item mutex");
    let mut items: Vec<TrayItem> = map
        .iter()
        .map(|(address, (sni, menu))| TrayItem {
            address: address.clone(),
            sni: sni.clone(),
            menu: menu.clone(),
        })
        .collect();
    // App id first (stable across restarts), bus address as tiebreak.
    items.sort_by(|a, b| {
        (a.sni.id.as_str(), a.address.as_str()).cmp(&(b.sni.id.as_str(), b.address.as_str()))
    });
    items
}
