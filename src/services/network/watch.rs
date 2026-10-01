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

/// Call `on_change` (through `tx`) when a device is added, removed, or
/// changes state, and for nothing else: the bar's banned-adapter hazard
/// (`blocked`), which runs for the life of the process and so cannot take
/// [`start`]'s every-signal filter (a scan alone is dozens of signals).
///
/// A ban flips `Managed`, and NM moves the device into or out of the
/// unmanaged state when it does, so `StateChanged` covers it without
/// subscribing to the device's property chatter. Lives as long as the
/// receiving end of `tx`.
pub fn devices(tx: async_channel::Sender<()>) {
    crate::spawn::spawn_tokio_thread("network-devices", async move {
        if let Err(e) = follow_devices(&tx).await {
            log::debug!("network: not following NetworkManager's devices: {e}");
        }
    });
}

async fn follow_devices(tx: &async_channel::Sender<()>) -> zbus::Result<()> {
    let conn = zbus::Connection::system().await?;
    let rule =
        |iface: &'static str, member: &'static str| -> zbus::Result<zbus::MatchRule<'static>> {
            Ok(zbus::MatchRule::builder()
                .msg_type(zbus::message::Type::Signal)
                .sender("org.freedesktop.NetworkManager")?
                .interface(iface)?
                .member(member)?
                .build())
        };
    let stream = |r| zbus::MessageStream::for_match_rule(r, &conn, Some(16));
    let mut state = stream(rule(
        "org.freedesktop.NetworkManager.Device",
        "StateChanged",
    )?)
    .await?;
    let mut added = stream(rule("org.freedesktop.NetworkManager", "DeviceAdded")?).await?;
    let mut removed = stream(rule("org.freedesktop.NetworkManager", "DeviceRemoved")?).await?;
    loop {
        tokio::select! {
            Some(_) = state.next() => {}
            Some(_) = added.next() => {}
            Some(_) = removed.next() => {}
            else => return Ok(()),
        }
        if tx.is_closed() {
            return Ok(());
        }
        let _ = tx.try_send(());
    }
}
