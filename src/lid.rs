//! Lid switch, as an event stream.
//!
//! UPower owns the lid — `LidIsPresent` and `LidIsClosed` on
//! `org.freedesktop.UPower`, both with the usual `PropertiesChanged` behind
//! them — and it is already running on this host for battery history and the
//! critical-charge action (modules/nixos/hardware/thinkpad-x9/hibernate.nix).
//! So this costs a subscription and no timer.
//!
//! The alternative was `/proc/acpi/button/lid/LID/state`, which has no
//! change notification at all: reading it means a poll loop, and a poll loop
//! sized to feel instant (250 ms) is 4 wakeups a second, forever, to observe
//! a switch that moves twice a day.
//!
//! Only *edges* are published, and the opening reading is a seed rather than
//! an edge — the same rule presence and resume follow (`crate::presence`,
//! lock/face.rs). A consumer that armed on the seed would fire the moment it
//! started, on a lid that had not moved.

use std::sync::mpsc;

use zbus::export::futures_util::StreamExt;

#[zbus::proxy(
    interface = "org.freedesktop.UPower",
    default_service = "org.freedesktop.UPower",
    default_path = "/org/freedesktop/UPower"
)]
trait UPower {
    /// False on a desktop, and on a laptop whose lid UPower cannot see. The
    /// watcher stops rather than reporting edges that will never come.
    #[zbus(property)]
    fn lid_is_present(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn lid_is_closed(&self) -> zbus::Result<bool>;
}

/// Lid transitions: `true` when it opens, `false` when it closes. One item
/// per edge; the channel is silent until the lid actually moves.
///
/// The thread ends when the receiver drops, when UPower is unreachable, or
/// when there is no lid.
pub fn watch() -> mpsc::Receiver<bool> {
    let (tx, rx) = mpsc::channel();
    crate::spawn::spawn_tokio_thread("lid-watch", async move {
        if let Err(e) = run(&tx).await {
            log::warn!("lid: watcher stopped: {e}");
        }
    });
    rx
}

async fn run(tx: &mpsc::Sender<bool>) -> zbus::Result<()> {
    let conn = zbus::Connection::system().await?;
    let upower = UPowerProxy::new(&conn).await?;

    if !upower.lid_is_present().await? {
        log::info!("lid: no lid on this machine — not watching");
        return Ok(());
    }

    let mut changes = upower.receive_lid_is_closed_changed().await;
    // Seeded from the current state so the first item off the stream — which
    // may be the cache filling in rather than the lid moving — is compared
    // against something real instead of counting as an edge.
    let mut closed = upower.lid_is_closed().await?;
    log::info!("lid: watching (currently {})", state_name(closed));

    while let Some(change) = changes.next().await {
        let now = change.get().await?;
        if now == closed {
            continue;
        }
        closed = now;
        log::info!("lid: {}", state_name(closed));
        if tx.send(!closed).is_err() {
            return Ok(()); // consumer gone
        }
    }
    Ok(())
}

fn state_name(closed: bool) -> &'static str {
    if closed { "closed" } else { "open" }
}
