//! BlueZ over D-Bus.
//!
//! The Bluetooth section used to drive `bluetoothctl`, which is a REPL wearing
//! a CLI: `bluetoothctl info <MAC>` printed a block of `Key: value` lines, one
//! process per device, and the answers were recovered with `strip_prefix` —
//! including a battery percentage printed as `0x4b (75)` and read by finding
//! the parentheses. Connect and disconnect were worse: they reported success
//! by *matching English* in the output ("Connection successful"), so a
//! translated or reworded message would have been read as a failure.
//!
//! BlueZ's own interface answers all of it in one call. `GetManagedObjects`
//! returns every adapter and device with their properties already typed, so a
//! full refresh is one round trip instead of one process per device plus two.
//!
//! Blocking, like `network::nm`, and for the same reason: the callers are
//! already on worker threads. The quick-settings tile reads and powers the
//! adapter here; the section follows BlueZ's signals through
//! `services::bluetooth`, which shares [`parse`].

use std::collections::HashMap;

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

const SERVICE: &str = "org.bluez";
const IFACE_ADAPTER: &str = "org.bluez.Adapter1";
const IFACE_DEVICE: &str = "org.bluez.Device1";
const IFACE_BATTERY: &str = "org.bluez.Battery1";
const IFACE_OBJECT_MANAGER: &str = "org.freedesktop.DBus.ObjectManager";

/// One device, as the section draws it.
#[derive(Debug, Clone, PartialEq)]
pub struct Device {
    /// Its object path, which every action on it takes.
    pub path: String,
    pub mac: String,
    pub name: String,
    /// BlueZ's `Icon` property (`audio-headset`, `input-keyboard`, …), which
    /// is the same hint `bluetoothctl` was printing.
    pub icon_hint: Option<String>,
    pub connected: bool,
    pub paired: bool,
    pub trusted: bool,
    pub battery: Option<u8>,
    /// Signal strength while discovering; `None` for a device not in range
    /// of this scan.
    pub rssi: Option<i16>,
    /// Whether it has a name of its own: a nearby device that is only an
    /// address is not worth a row.
    pub named: bool,
}

/// Everything one `GetManagedObjects` call yields.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Snapshot {
    pub available: bool,
    pub powered: bool,
    pub discovering: bool,
    pub devices: Vec<Device>,
    /// The adapter every action needs, `/org/bluez/hci0` in practice.
    pub adapter: Option<String>,
}

fn system() -> Result<Connection, String> {
    Connection::system().map_err(|e| format!("system bus: {e}"))
}

fn proxy<'a>(conn: &Connection, path: &'a str, iface: &'a str) -> Result<Proxy<'a>, String> {
    Proxy::new(conn, SERVICE, path, iface).map_err(|e| format!("{iface} at {path}: {e}"))
}

fn as_string(value: &OwnedValue) -> Option<String> {
    value.downcast_ref::<&str>().ok().map(str::to_string)
}

fn as_bool(value: &OwnedValue) -> Option<bool> {
    value.downcast_ref::<bool>().ok()
}

pub(crate) type Managed =
    HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;

/// The whole tree in one call.
///
/// `available: false` covers both "bluetoothd is not running" and "there is
/// no adapter", because the section draws the same thing for each: nothing it
/// can offer.
pub fn snapshot() -> Snapshot {
    let Ok(conn) = system() else {
        return Snapshot::default();
    };
    let Ok(proxy) = proxy(&conn, "/", IFACE_OBJECT_MANAGER) else {
        return Snapshot::default();
    };
    match proxy.call::<_, _, Managed>("GetManagedObjects", &()) {
        Ok(objects) => parse(&objects),
        Err(e) => {
            log::debug!("bluez: GetManagedObjects: {e}");
            Snapshot::default()
        }
    }
}

/// The snapshot out of one `GetManagedObjects` answer. Shared by the
/// blocking read above and the event-driven service
/// (`services::bluetooth`).
pub(crate) fn parse(objects: &Managed) -> Snapshot {
    let mut snapshot = Snapshot::default();

    // The first adapter wins. A machine with two Bluetooth radios exists and
    // is not this one; picking the first is what bluetoothctl did too.
    for (path, interfaces) in objects {
        if let Some(adapter) = interfaces.get(IFACE_ADAPTER) {
            snapshot.available = true;
            snapshot.adapter = Some(path.as_str().to_string());
            snapshot.powered = adapter.get("Powered").and_then(as_bool).unwrap_or(false);
            snapshot.discovering = adapter
                .get("Discovering")
                .and_then(as_bool)
                .unwrap_or(false);
            break;
        }
    }

    if !snapshot.available {
        return snapshot;
    }

    for (path, interfaces) in objects {
        let Some(device) = interfaces.get(IFACE_DEVICE) else {
            continue;
        };
        let Some(mac) = device.get("Address").and_then(as_string) else {
            continue;
        };
        let own = device.get("Name").and_then(as_string);

        snapshot.devices.push(Device {
            path: path.as_str().to_string(),
            // `Alias` is the name the owner may have changed; `Name` is what
            // the device calls itself. Preferring the alias matches every
            // other Bluetooth UI, and the address is the last resort.
            name: device
                .get("Alias")
                .and_then(as_string)
                .or_else(|| own.clone())
                .unwrap_or_else(|| mac.clone()),
            named: own.is_some(),
            icon_hint: device.get("Icon").and_then(as_string),
            connected: device.get("Connected").and_then(as_bool).unwrap_or(false),
            paired: device.get("Paired").and_then(as_bool).unwrap_or(false),
            trusted: device.get("Trusted").and_then(as_bool).unwrap_or(false),
            rssi: device.get("RSSI").and_then(|v| v.downcast_ref::<i16>().ok()),
            // Battery is its own interface, published only by devices that
            // report one.
            battery: interfaces
                .get(IFACE_BATTERY)
                .and_then(|b| b.get("Percentage"))
                .and_then(|v| v.downcast_ref::<u8>().ok()),
            mac,
        });
    }

    // Connected first, then by name: the list's job is to get you to the
    // thing you are already using, and after that to be alphabetical rather
    // than in whatever order the bus enumerated.
    snapshot.devices.sort_by(|a, b| {
        b.connected
            .cmp(&a.connected)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    snapshot
}

pub fn set_powered(on: bool) -> Result<(), String> {
    let conn = system()?;
    let adapter = snapshot().adapter.ok_or("no Bluetooth adapter")?;
    proxy(&conn, &adapter, IFACE_ADAPTER)?
        .set_property("Powered", on)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod live {
    /// Reads the running daemon. Ignored: needs BlueZ on the system bus.
    /// Touches nothing — no power change, no connect, no scan.
    #[test]
    #[ignore]
    fn read_the_adapter() {
        let snapshot = super::snapshot();
        println!(
            "available={} powered={} discovering={} adapter={:?}",
            snapshot.available, snapshot.powered, snapshot.discovering, snapshot.adapter
        );
        for d in &snapshot.devices {
            println!(
                "  {:<18} {:<28} paired={} connected={} battery={:?} icon={:?}",
                d.mac, d.name, d.paired, d.connected, d.battery, d.icon_hint
            );
        }
        println!("{} devices", snapshot.devices.len());
    }
}
