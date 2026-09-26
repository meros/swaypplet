//! The battery as an observed value, moved by UPower's change signals.
//!
//! The panel's power section and the bar's battery pill used to read sysfs
//! on their own 30-second timers: two wake-ups a minute each, forever, and a
//! plug or unplug that took up to 30 seconds to show. UPower already watches
//! the battery (and the charger's uevents) and says when anything moved, on
//! its DisplayDevice: one subscription here, one sysfs read per change, and
//! every widget observes the result.
//!
//! UPower's `TimeToEmpty` and `TimeToFull` ride along: it smooths them over
//! minutes, which beats `energy / power_now` from one instant.
//!
//! Without UPower on the bus (a minimal system), a 60-second sysfs poll
//! stands in, and says so in the log once.

use std::cell::Cell;

use crate::service::Observed;
use crate::services::power::{self, BatteryState};

#[zbus::proxy(
    interface = "org.freedesktop.UPower.Device",
    default_service = "org.freedesktop.UPower",
    default_path = "/org/freedesktop/UPower/devices/DisplayDevice"
)]
trait Device {
    #[zbus(property)]
    fn is_present(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn time_to_empty(&self) -> zbus::Result<i64>;
    #[zbus(property)]
    fn time_to_full(&self) -> zbus::Result<i64>;
}

thread_local! {
    static STATE: Observed<Option<BatteryState>> = Observed::new(None);
    static STARTED: Cell<bool> = const { Cell::new(false) };
}

/// The battery as last read; `None` on a machine without one, or before the
/// first reading.
pub fn current() -> Option<BatteryState> {
    STATE.with(|s| s.with(Clone::clone))
}

/// Call `cb` on the main thread after every change.
pub fn observe(cb: impl Fn() + 'static) {
    STATE.with(|s| s.connect_change(cb));
}

/// Start following the battery, once per process. Returns whether this
/// machine has one.
pub fn start() -> bool {
    let Some(path) = power::find_battery_path() else {
        return false;
    };
    if STARTED.with(|s| s.replace(true)) {
        return true;
    }
    // The first reading synchronously, so the bar is right on its first frame.
    STATE.with(|s| s.set(power::read_battery(&path)));

    let (tx, rx) = async_channel::unbounded::<Option<BatteryState>>();
    crate::spawn::spawn_tokio_thread("battery-upower", async move {
        if let Err(e) = follow(&path, &tx).await {
            log::info!("battery: UPower not followed ({e}); reading sysfs every 60 s");
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                if tx.send(power::read_battery(&path)).await.is_err() {
                    return;
                }
            }
        }
    });
    glib::spawn_future_local(async move {
        while let Ok(reading) = rx.recv().await {
            // A failed read keeps the last one: sysfs hiccups around suspend.
            if reading.is_some() {
                STATE.with(|s| s.set(reading));
            }
        }
    });
    true
}

/// One read per UPower change on the DisplayDevice, sysfs for the numbers
/// and UPower for the estimates.
async fn follow(
    path: &str,
    tx: &async_channel::Sender<Option<BatteryState>>,
) -> zbus::Result<()> {
    use zbus::export::futures_util::StreamExt;
    let conn = zbus::Connection::system().await?;
    let device = DeviceProxy::new(&conn).await?;
    device.is_present().await?;
    let props = zbus::fdo::PropertiesProxy::builder(&conn)
        .destination("org.freedesktop.UPower")?
        .path("/org/freedesktop/UPower/devices/DisplayDevice")?
        .build()
        .await?;
    let mut changes = props.receive_properties_changed().await?;
    loop {
        let secs = |v: zbus::Result<i64>| v.ok().filter(|s| *s > 0).map(|s| s as u64);
        let reading = match power::read_battery(path) {
            Some(mut b) => {
                b.upower_to_empty_s = secs(device.time_to_empty().await);
                b.upower_to_full_s = secs(device.time_to_full().await);
                Some(b)
            }
            None => None,
        };
        if tx.send(reading).await.is_err() {
            return Ok(());
        }
        if changes.next().await.is_none() {
            return Err(zbus::Error::Failure("UPower's signal stream ended".into()));
        }
    }
}
