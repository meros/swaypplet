//! Whether the session is locked, from logind's LockedHint.
//!
//! The lock runs in its own process; logind is where it says so. The theme
//! reads it to time a sun-driven mode switch for when nobody is looking
//! (docs/design-system.md §2.1). Followed as a property stream, not polled.

use std::cell::Cell;

thread_local! {
    static LOCKED: Cell<bool> = const { Cell::new(false) };
    /// Called on the main thread when LockedHint moves.
    static ON_CHANGE: std::cell::RefCell<Vec<Box<dyn Fn()>>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Run `cb` whenever the lock state changes: a sun switch waits for the
/// lock, so the lock is one of the theme's triggers.
pub(super) fn on_change(cb: impl Fn() + 'static) {
    ON_CHANGE.with(|c| c.borrow_mut().push(Box::new(cb)));
}

pub(super) fn is_locked() -> bool {
    LOCKED.with(Cell::get)
}

/// For the lock screen's own process, which is the lock: it has no need to
/// ask logind, and asking would leave it unlocked until the answer came.
pub(super) fn assume_locked() {
    LOCKED.with(|l| l.set(true));
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
            if LOCKED.with(|l| l.replace(locked)) != locked {
                ON_CHANGE.with(|c| c.borrow().iter().for_each(|cb| cb()));
            }
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
