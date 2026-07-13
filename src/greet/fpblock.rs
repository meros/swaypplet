//! Opportunistic fprintd device claim — the "skip the fingerprint" lever.
//!
//! While the greeter holds the device claim, pam_fprintd in a freshly
//! created greetd session fails its own claim instantly and the PAM stack
//! falls straight through to pam_unix, so a typed password gets its prompt
//! without waiting out a fingerprint timeout. The claim is made as the
//! greeter user itself (empty username): locking the device needs no
//! enrolled prints and no cross-user polkit grant. If fprintd denies it
//! anyway, the handle simply never claims and the password lands after
//! pam_fprintd gives up — degraded but functional.
//!
//! Dropping the [`Handle`] releases the claim. Worker model mirrors
//! `lock::fprint`: a plain thread with a current-thread tokio runtime.

use std::sync::mpsc;
use std::time::Duration;

use crate::lock::fprint::{DeviceProxy, ManagerProxy};

pub struct Handle {
    _tx: mpsc::Sender<()>, // dropping this disconnects the thread's receiver
}

pub fn claim() -> Handle {
    let (tx, rx) = mpsc::channel::<()>();
    std::thread::spawn(move || run(rx));
    Handle { _tx: tx }
}

fn run(rx: mpsc::Receiver<()>) {
    let Ok(rt) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return;
    };
    let Some(device) = rt.block_on(claim_device(&rx)) else {
        return;
    };
    // Hold until the Handle drops (recv errors out once the sender is gone).
    let _ = rx.recv();
    let _ = rt.block_on(device.release());
}

async fn claim_device(rx: &mpsc::Receiver<()>) -> Option<DeviceProxy<'static>> {
    let conn = zbus::Connection::system().await.ok()?;
    let manager = ManagerProxy::new(&conn).await.ok()?;
    let path = manager.get_default_device().await.ok()?;
    let device = DeviceProxy::builder(&conn)
        .path(path)
        .ok()?
        .build()
        .await
        .ok()?;
    // The dying session's pam_fprintd may lag its release — retry briefly.
    for _ in 0..20 {
        match device.claim("").await {
            Ok(()) => return Some(device),
            Err(e) => log::debug!("fp block claim retry: {e}"),
        }
        if matches!(rx.try_recv(), Err(mpsc::TryRecvError::Disconnected)) {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    log::info!("fp block: could not claim reader (degraded password path)");
    None
}
