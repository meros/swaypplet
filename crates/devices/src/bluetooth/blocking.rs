//! BlueZ over D-Bus, one blocking call at a time.
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
//! already on worker threads. The quick-settings tile and airplane mode read
//! and power the adapter here; the section follows BlueZ's signals through
//! the service in [`super`], which shares [`parse`] and powers the adapter
//! through [`set_adapter_powered`].

use std::collections::HashMap;

use swaypplet_base::dbus::{as_string, proxy, system};
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

use super::{IFACE_ADAPTER, IFACE_BATTERY, IFACE_DEVICE, IFACE_OBJECT_MANAGER, SERVICE};

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

fn as_bool(value: &OwnedValue) -> Option<bool> {
    value.downcast_ref::<bool>().ok()
}

pub type Managed = HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;

/// The whole tree in one call.
///
/// `available: false` covers both "bluetoothd is not running" and "there is
/// no adapter", because the section draws the same thing for each: nothing it
/// can offer.
pub fn snapshot() -> Snapshot {
    let Ok(conn) = system() else {
        return Snapshot::default();
    };
    let Ok(proxy) = proxy(&conn, SERVICE, "/", IFACE_OBJECT_MANAGER) else {
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
/// blocking read above and the event-driven service ([`super`]).
pub fn parse(objects: &Managed) -> Snapshot {
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
            rssi: device
                .get("RSSI")
                .and_then(|v| v.downcast_ref::<i16>().ok()),
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

/// Power the first adapter on or off.
pub fn set_powered(on: bool) -> Result<(), String> {
    let adapter = snapshot().adapter.ok_or("no Bluetooth adapter")?;
    set_adapter_powered(&adapter, on)
}

/// Power the adapter at `adapter` (its object path) on or off.
pub fn set_adapter_powered(adapter: &str, on: bool) -> Result<(), String> {
    let conn = system()?;
    proxy(&conn, SERVICE, adapter, IFACE_ADAPTER)?
        .set_property("Powered", on)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::Value;

    fn text(s: &str) -> OwnedValue {
        OwnedValue::try_from(Value::from(s)).expect("owned string")
    }

    fn props(pairs: Vec<(&str, OwnedValue)>) -> HashMap<String, OwnedValue> {
        pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
    }

    fn object(
        objects: &mut Managed,
        path: &str,
        interfaces: Vec<(&str, HashMap<String, OwnedValue>)>,
    ) {
        objects.insert(
            OwnedObjectPath::try_from(path).expect("path"),
            interfaces
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        );
    }

    fn adapter(objects: &mut Managed, powered: bool) {
        object(
            objects,
            "/org/bluez/hci0",
            vec![(
                IFACE_ADAPTER,
                props(vec![
                    ("Powered", OwnedValue::from(powered)),
                    ("Discovering", OwnedValue::from(false)),
                ]),
            )],
        );
    }

    #[test]
    fn no_adapter_is_unavailable_even_with_devices_listed() {
        let mut objects = Managed::new();
        object(
            &mut objects,
            "/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF",
            vec![(
                IFACE_DEVICE,
                props(vec![("Address", text("AA:BB:CC:DD:EE:FF"))]),
            )],
        );
        assert_eq!(parse(&objects), Snapshot::default());
    }

    #[test]
    fn devices_take_the_alias_then_the_name_then_the_address() {
        let mut objects = Managed::new();
        adapter(&mut objects, true);
        object(
            &mut objects,
            "/org/bluez/hci0/dev_01",
            vec![(
                IFACE_DEVICE,
                props(vec![
                    ("Address", text("00:00:00:00:00:01")),
                    ("Name", text("WH-1000XM4")),
                    ("Alias", text("Kitchen headphones")),
                    ("Icon", text("audio-headset")),
                    ("Paired", OwnedValue::from(true)),
                ]),
            )],
        );
        object(
            &mut objects,
            "/org/bluez/hci0/dev_02",
            vec![(
                IFACE_DEVICE,
                props(vec![
                    ("Address", text("00:00:00:00:00:02")),
                    ("RSSI", OwnedValue::from(-60i16)),
                ]),
            )],
        );
        // A device without an address is no device.
        object(
            &mut objects,
            "/org/bluez/hci0/dev_03",
            vec![(IFACE_DEVICE, props(vec![("Name", text("Ghost"))]))],
        );
        let s = parse(&objects);
        assert!(s.available && s.powered && !s.discovering);
        assert_eq!(s.adapter.as_deref(), Some("/org/bluez/hci0"));
        assert_eq!(s.devices.len(), 2);
        let names: Vec<(&str, bool)> = s
            .devices
            .iter()
            .map(|d| (d.name.as_str(), d.named))
            .collect();
        assert_eq!(
            names,
            [("00:00:00:00:00:02", false), ("Kitchen headphones", true)]
        );
        assert_eq!(s.devices[0].rssi, Some(-60));
        assert_eq!(s.devices[1].icon_hint.as_deref(), Some("audio-headset"));
        assert!(s.devices[1].paired && !s.devices[1].connected);
    }

    #[test]
    fn connected_devices_lead_and_the_battery_comes_from_its_own_interface() {
        let mut objects = Managed::new();
        adapter(&mut objects, false);
        object(
            &mut objects,
            "/org/bluez/hci0/dev_01",
            vec![(
                IFACE_DEVICE,
                props(vec![
                    ("Address", text("00:00:00:00:00:01")),
                    ("Name", text("Alpha")),
                ]),
            )],
        );
        object(
            &mut objects,
            "/org/bluez/hci0/dev_02",
            vec![
                (
                    IFACE_DEVICE,
                    props(vec![
                        ("Address", text("00:00:00:00:00:02")),
                        ("Name", text("zulu")),
                        ("Connected", OwnedValue::from(true)),
                    ]),
                ),
                (
                    IFACE_BATTERY,
                    props(vec![("Percentage", OwnedValue::from(75u8))]),
                ),
            ],
        );
        let s = parse(&objects);
        assert!(!s.powered);
        let order: Vec<&str> = s.devices.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(order, ["zulu", "Alpha"]);
        assert_eq!(s.devices[0].battery, Some(75));
        assert_eq!(s.devices[1].battery, None);
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
