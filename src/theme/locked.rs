//! Whether the session is locked, from logind's LockedHint.
//!
//! The lock runs in its own process; logind is where it says so. The theme
//! reads it to time a sun-driven mode switch for when nobody is looking
//! (docs/design-system.md §2.1). Followed as a property stream, not polled.

use std::cell::Cell;

thread_local! {
    static LOCKED: Cell<bool> = const { Cell::new(false) };
}

pub(super) fn is_locked() -> bool {
    LOCKED.with(Cell::get)
}

/// Start following LockedHint, for as long as this process lives.
pub(super) fn follow() {
    let (tx, rx) = async_channel::unbounded::<bool>();
    crate::spawn::spawn_tokio_thread("theme-locked", async move {
        if let Err(e) = run(&tx).await {
            log::debug!("theme: not following the lock state: {e}");
        }
    });
    glib::spawn_future_local(async move {
        while let Ok(locked) = rx.recv().await {
            LOCKED.with(|l| l.set(locked));
        }
    });
}

async fn run(tx: &async_channel::Sender<bool>) -> zbus::Result<()> {
    use zbus::export::futures_util::StreamExt;
    let conn = zbus::Connection::system().await?;
    let manager = crate::idle::logind::ManagerProxy::new(&conn).await?;
    let session = crate::idle::logind::session(&conn, &manager).await?;
    let mut changes = session.receive_locked_hint_changed().await;
    if let Ok(v) = session.locked_hint().await {
        let _ = tx.send(v).await;
    }
    while let Some(change) = changes.next().await {
        if let Ok(v) = change.get().await
            && tx.send(v).await.is_err() {
                break;
            }
    }
    Ok(())
}
