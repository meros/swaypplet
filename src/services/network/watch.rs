//! NetworkManager's change signals, as a wake-up for the section.
//!
//! The section used to poll: five D-Bus conversations every 5 s (2 s while
//! something moved) for as long as it was on screen. NetworkManager already
//! announces every change it makes — a property on a device, an access point
//! appearing, a connection's state — so this listens instead, and the
//! section rereads (`snapshot::read`) only when something was said.
//!
//! Filtered on the bus to NetworkManager's own signals and the Bluetooth
//! adapter's (airplane mode covers both). Signals come in bursts during a
//! scan or a join; the reader on the GTK side coalesces them.
//!
//! Runs only while someone holds the [`Watch`]: the section starts one when
//! it is mapped and drops it when it is not, so a closed panel costs
//! nothing, not even a subscription.

use zbus::export::futures_util::StreamExt;

/// A running subscription. Dropping it ends the thread.
pub struct Watch {
    _stop: async_channel::Sender<()>,
}

/// Call `on_change` (through `tx`) after any NetworkManager or Bluetooth
/// adapter signal.
pub fn start(tx: async_channel::Sender<()>) -> Watch {
    let (stop_tx, stop_rx) = async_channel::bounded::<()>(1);
    crate::spawn::spawn_tokio_thread("network-watch", async move {
        if let Err(e) = follow(&tx, &stop_rx).await {
            log::debug!("network: not following NetworkManager: {e}");
        }
    });
    Watch { _stop: stop_tx }
}

async fn follow(
    tx: &async_channel::Sender<()>,
    stop: &async_channel::Receiver<()>,
) -> zbus::Result<()> {
    let conn = zbus::Connection::system().await?;
    let nm = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender("org.freedesktop.NetworkManager")?
        .build();
    let mut nm = zbus::MessageStream::for_match_rule(nm, &conn, Some(256)).await?;
    let bt = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender("org.bluez")?
        .interface("org.freedesktop.DBus.Properties")?
        .member("PropertiesChanged")?
        .path("/org/bluez/hci0")?
        .build();
    let mut bt = zbus::MessageStream::for_match_rule(bt, &conn, Some(16)).await?;
    loop {
        tokio::select! {
            Some(_) = nm.next() => {}
            Some(_) = bt.next() => {}
            // The holder dropped the Watch: the sender is gone.
            _ = stop.recv() => return Ok(()),
            else => return Ok(()),
        }
        // A full channel already has a wake-up queued; this one adds nothing.
        if tx.is_closed() {
            return Ok(());
        }
        let _ = tx.try_send(());
    }
}
