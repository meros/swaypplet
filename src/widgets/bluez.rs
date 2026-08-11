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
//! already on worker threads.

use std::collections::HashMap;

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue};

const SERVICE: &str = "org.bluez";
const IFACE_ADAPTER: &str = "org.bluez.Adapter1";
const IFACE_DEVICE: &str = "org.bluez.Device1";
const IFACE_BATTERY: &str = "org.bluez.Battery1";
const IFACE_OBJECT_MANAGER: &str = "org.freedesktop.DBus.ObjectManager";

/// One device, as the section draws it.
#[derive(Debug, Clone, PartialEq)]
pub struct Device {
    pub mac: String,
    pub name: String,
    /// BlueZ's `Icon` property (`audio-headset`, `input-keyboard`, …), which
    /// is the same hint `bluetoothctl` was printing.
    pub icon_hint: Option<String>,
    pub connected: bool,
    pub paired: bool,
    pub battery: Option<u8>,
}

/// Everything one `GetManagedObjects` call yields.
#[derive(Default)]
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

    type Managed = HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;
    let objects: Managed = match proxy.call("GetManagedObjects", &()) {
        Ok(objects) => objects,
        Err(e) => {
            log::debug!("bluez: GetManagedObjects: {e}");
            return Snapshot::default();
        }
    };

    let mut snapshot = Snapshot::default();

    // The first adapter wins. A machine with two Bluetooth radios exists and
    // is not this one; picking the first is what bluetoothctl did too.
    for (path, interfaces) in &objects {
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

    for (_, interfaces) in objects.iter() {
        let Some(device) = interfaces.get(IFACE_DEVICE) else {
            continue;
        };
        let Some(mac) = device.get("Address").and_then(as_string) else {
            continue;
        };

        snapshot.devices.push(Device {
            // `Alias` is the name the owner may have changed; `Name` is what
            // the device calls itself. Preferring the alias matches every
            // other Bluetooth UI, and the address is the last resort.
            name: device
                .get("Alias")
                .or_else(|| device.get("Name"))
                .and_then(as_string)
                .unwrap_or_else(|| mac.clone()),
            icon_hint: device.get("Icon").and_then(as_string),
            connected: device.get("Connected").and_then(as_bool).unwrap_or(false),
            paired: device.get("Paired").and_then(as_bool).unwrap_or(false),
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

/// The object path of a device, which every action needs and the section
/// only ever knows by address.
fn device_path(conn: &Connection, mac: &str) -> Option<String> {
    let proxy = proxy(conn, "/", IFACE_OBJECT_MANAGER).ok()?;
    type Managed = HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;
    let objects: Managed = proxy.call("GetManagedObjects", &()).ok()?;

    objects.into_iter().find_map(|(path, interfaces)| {
        let device = interfaces.get(IFACE_DEVICE)?;
        (device.get("Address").and_then(as_string).as_deref() == Some(mac))
            .then(|| path.as_str().to_string())
    })
}

pub fn set_powered(on: bool) -> Result<(), String> {
    let conn = system()?;
    let adapter = snapshot().adapter.ok_or("no Bluetooth adapter")?;
    proxy(&conn, &adapter, IFACE_ADAPTER)?
        .set_property("Powered", on)
        .map_err(|e| e.to_string())
}

/// Connect or disconnect, reported by whether the call returned — not by
/// reading English out of a message.
pub fn connect(mac: &str) -> Result<(), String> {
    act(mac, "Connect")
}

pub fn disconnect(mac: &str) -> Result<(), String> {
    act(mac, "Disconnect")
}

fn act(mac: &str, method: &str) -> Result<(), String> {
    let conn = system()?;
    let path = device_path(&conn, mac).ok_or_else(|| format!("no device {mac}"))?;
    proxy(&conn, &path, IFACE_DEVICE)?
        .call::<_, _, ()>(method, &())
        .map_err(|e| message(&e))
}

/// Unpair. The adapter owns its devices, so removal goes through it.
pub fn forget(mac: &str) -> Result<(), String> {
    let conn = system()?;
    let snapshot = snapshot();
    let adapter = snapshot.adapter.ok_or("no Bluetooth adapter")?;
    let path = device_path(&conn, mac).ok_or_else(|| format!("no device {mac}"))?;
    let path = ObjectPath::try_from(path.as_str()).map_err(|e| e.to_string())?;

    proxy(&conn, &adapter, IFACE_ADAPTER)?
        .call::<_, _, ()>("RemoveDevice", &(&path,))
        .map_err(|e| message(&e))
}

/// Start or stop scanning for devices.
///
/// Errors are swallowed by the caller on purpose: asking a stopped discovery
/// to stop, or a running one to start, is BlueZ's `InProgress` and means the
/// state is already what was wanted.
pub fn set_discovery(on: bool) -> Result<(), String> {
    let conn = system()?;
    let adapter = snapshot().adapter.ok_or("no Bluetooth adapter")?;
    let method = if on {
        "StartDiscovery"
    } else {
        "StopDiscovery"
    };
    proxy(&conn, &adapter, IFACE_ADAPTER)?
        .call::<_, _, ()>(method, &())
        .map_err(|e| message(&e))
}

/// BlueZ's own message, which is short and specific ("Device not available",
/// "Host is down"), rather than zbus's rendering of the whole call.
fn message(error: &zbus::Error) -> String {
    match error {
        zbus::Error::MethodError(name, detail, _) => detail
            .clone()
            .unwrap_or_else(|| name.as_str().rsplit('.').next().unwrap_or("Failed").into()),
        other => other.to_string(),
    }
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
