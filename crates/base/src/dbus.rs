//! Blocking system-bus helpers shared by the services that read a daemon
//! from a worker thread (NetworkManager in `devices::network::nm`, BlueZ in
//! `devices::bluetooth::blocking`).

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::OwnedValue;

/// One connection to the system bus, opened per call.
///
/// zbus caches the underlying socket per process, so this is cheaper than it
/// reads; making it a long-lived field would mean handing a `!Send` handle
/// across the worker threads that use it.
pub fn system() -> Result<Connection, String> {
    Connection::system().map_err(|e| format!("system bus: {e}"))
}

/// A proxy on `service`'s object at `path`, with an error that names both.
pub fn proxy<'a>(
    conn: &Connection,
    service: &'a str,
    path: &'a str,
    iface: &'a str,
) -> Result<Proxy<'a>, String> {
    Proxy::new(conn, service, path, iface).map_err(|e| format!("{iface} at {path}: {e}"))
}

/// A `OwnedValue` holding a string, as a `String`.
///
/// zvariant 4's `OwnedValue` is not `Clone`, so the usual `try_from(v.clone())`
/// silently clones the *reference* instead and fails to convert. Going through
/// `downcast_ref` says what is meant and cannot do that.
pub fn as_string(value: &OwnedValue) -> Option<String> {
    value.downcast_ref::<&str>().ok().map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::Value;

    #[test]
    fn a_string_value_reads_as_one_and_anything_else_does_not() {
        let s = OwnedValue::try_from(Value::from("hci0")).expect("owned");
        assert_eq!(as_string(&s).as_deref(), Some("hci0"));
        let n = OwnedValue::from(7u32);
        assert_eq!(as_string(&n), None);
    }
}
