//! Names a person reads for a network adapter, instead of `enp0s13f0u2u1`.
//!
//! Generic rules over sysfs, no per-model table:
//!
//! 1. Wi-Fi is "Wi-Fi"; a wired card on the machine's own PCI bus is
//!    "Ethernet (built-in)".
//! 2. A device on an external PCI path (the kernel marks the bridges behind
//!    a Thunderbolt or USB4 port `removable`) takes the Thunderbolt device's
//!    own name: "Ethernet on <vendor> <device_name>".
//! 3. A USB adapter behind a hub that also carries a Billboard device (USB
//!    class 0x11, which a USB-C dock exposes to say which alternate modes
//!    it runs) is "Ethernet on <vendor> dock". A dock is two hubs, one per
//!    USB generation, on the two root ports of one connector; the billboard
//!    sits on the USB 2 side and a gigabit adapter on the USB 3 side, so
//!    the search crosses to the other port: its `peer` link when the kernel
//!    has one, else the root port wired to the same Type-C `connector`.
//! 4. A USB adapter behind a hub without one is "Ethernet on USB hub"; one
//!    on a root port is "USB Ethernet adapter".
//!
//! The chipset for the subtitle comes from udev's hwdb (`ID_VENDOR_FROM_
//! DATABASE`, `ID_MODEL_FROM_DATABASE`), which is where libnm gets the
//! vendor and product NetworkManager's D-Bus API does not carry, shortened
//! the way GNOME shortens them, conservatively.
//!
//! Pure over a root directory (`/` live, a temporary tree in the tests), and
//! blocking: callers run it on a worker thread with the other reads.

use std::fs;
use std::path::{Path, PathBuf};

/// `NM_DEVICE_TYPE_ETHERNET` and `NM_DEVICE_TYPE_WIFI`.
const ETHERNET: u32 = 1;
const WIFI: u32 = 2;

/// USB class code of a Billboard device.
const BILLBOARD: &str = "11";

/// The automatic name for `iface`, or `None` for a type these rules do not
/// name (the caller shows the kernel name).
pub fn auto_name(root: &Path, iface: &str, device_type: u32) -> Option<String> {
    match device_type {
        WIFI => return Some("Wi-Fi".into()),
        ETHERNET => {}
        _ => return None,
    }
    let device = fs::canonicalize(root.join("sys/class/net").join(iface).join("device")).ok();
    let Some(device) = device else {
        // No backing device: virtual. The callers filter those out; this is
        // the answer if one gets here anyway.
        return None;
    };
    if is_external_pci(root, &device)
        && let Some(tb) = thunderbolt_name(root)
    {
        return Some(format!("Ethernet on {tb}"));
    }
    let chain = usb_chain(&device);
    let Some(top) = chain.first() else {
        return Some("Ethernet (built-in)".into());
    };
    if chain.len() == 1 {
        return Some("USB Ethernet adapter".into());
    }
    match dock_vendor(root, top) {
        Some(Some(vendor)) => Some(format!("Ethernet on {vendor} dock")),
        Some(None) => Some("Ethernet on dock".into()),
        None => Some("Ethernet on USB hub".into()),
    }
}

/// "Realtek RTL8153", from udev's hwdb entry for the interface.
pub fn chipset(root: &Path, iface: &str) -> Option<String> {
    let index = read(&root.join("sys/class/net").join(iface).join("ifindex"))?;
    let db = fs::read_to_string(root.join("run/udev/data").join(format!("n{index}"))).ok()?;
    let prop = |key: &str| {
        db.lines()
            .find_map(|l| l.strip_prefix("E:")?.strip_prefix(key)?.strip_prefix('='))
            .map(str::to_string)
    };
    let vendor = prop("ID_VENDOR_FROM_DATABASE").map(|v| short_vendor(&v));
    let model = prop("ID_MODEL_FROM_DATABASE").map(|m| short_model(&m));
    match (vendor, model) {
        (Some(v), Some(m)) if m.starts_with(&v) => Some(m),
        (Some(v), Some(m)) => Some(format!("{v} {m}")),
        (v, m) => v.or(m).filter(|s| !s.is_empty()),
    }
}

/// The company's name without its legal form: "Realtek Semiconductor
/// Corp." is "Realtek", "Intel Corporation" is "Intel".
pub fn short_vendor(vendor: &str) -> String {
    const NOISE: [&str; 14] = [
        "Semiconductor",
        "Corporation",
        "Corp.",
        "Corp",
        "Co.,",
        "Co.",
        "Ltd.",
        "Ltd",
        "Inc.",
        "Inc",
        "Technology",
        "Technologies",
        "Limited",
        "GmbH",
    ];
    let words: Vec<&str> = vendor
        .split_whitespace()
        .map(|w| w.trim_end_matches(','))
        .collect();
    // Cut at the first noise word, but never down to nothing.
    let keep = words
        .iter()
        .position(|w| NOISE.iter().any(|n| n.trim_end_matches(',') == *w))
        .filter(|&i| i > 0)
        .unwrap_or(words.len());
    words[..keep].join(" ")
}

/// The part number without the product category after it: "RTL8153
/// Gigabit Ethernet Adapter" is "RTL8153". A model whose rest has no digit
/// in it (no part number) is kept whole.
pub fn short_model(model: &str) -> String {
    const SUFFIXES: [&str; 8] = [
        " Gigabit Ethernet Adapter",
        " Gigabit Ethernet Controller",
        " Gigabit Network Connection",
        " Ethernet Adapter",
        " Ethernet Controller",
        " Network Adapter",
        " Network Connection",
        " Network Controller",
    ];
    let model = model.trim();
    SUFFIXES
        .iter()
        .find_map(|s| model.strip_suffix(s))
        // What is left must look like a part number, or the model was
        // mostly category ("Gigabit Ethernet Adapter") and stays whole.
        .filter(|rest| rest.chars().any(|c| c.is_ascii_digit()))
        .unwrap_or(model)
        .trim()
        .to_string()
}

/// A sysfs string without its padding: VIA's hubs pad `manufacturer` and
/// `product` with spaces to a fixed width.
fn read(path: &Path) -> Option<String> {
    let s = fs::read_to_string(path).ok()?;
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

/// Is any PCI device on the way to `device` marked `removable`, the kernel's
/// word for "behind an external-facing port"?
fn is_external_pci(root: &Path, device: &Path) -> bool {
    let devices =
        fs::canonicalize(root.join("sys/devices")).unwrap_or_else(|_| root.join("sys/devices"));
    device
        .ancestors()
        .take_while(|p| p.starts_with(&devices) && *p != devices)
        .any(|p| read(&p.join("removable")).as_deref() == Some("removable"))
}

/// "<vendor> <device_name>" of the connected Thunderbolt device. With more
/// than one, the first by route string: sysfs does not say which tunnel a
/// PCI device came through without walking the switch ports, and two
/// docks at once is rare enough to name one of them.
fn thunderbolt_name(root: &Path) -> Option<String> {
    let mut found: Vec<(String, String)> = fs::read_dir(root.join("sys/bus/thunderbolt/devices"))
        .ok()?
        .filter_map(|e| {
            let name = e.ok()?.file_name().to_string_lossy().into_owned();
            // A switch is "<domain>-<route>"; route 0 is the host itself.
            let (domain, route) = name.split_once('-')?;
            let switch = domain.chars().all(|c| c.is_ascii_digit())
                && !route.is_empty()
                && route.chars().all(|c| c.is_ascii_hexdigit())
                && route != "0";
            switch.then_some(name)
        })
        .filter_map(|name| {
            let dir = root.join("sys/bus/thunderbolt/devices").join(&name);
            let device = read(&dir.join("device_name"))?;
            let label = match read(&dir.join("vendor_name")) {
                Some(v) if !device.starts_with(&v) => format!("{v} {device}"),
                _ => device,
            };
            Some((name, label))
        })
        .collect();
    found.sort();
    found.into_iter().next().map(|(_, label)| label)
}

/// The USB devices on the path to `device`, outermost first: for
/// `…/usb2/2-2/2-2.1/2-2.1:1.0` the directories `…/usb2/2-2` and
/// `…/usb2/2-2/2-2.1`. Empty for a device that is not on USB.
fn usb_chain(device: &Path) -> Vec<PathBuf> {
    let mut chain: Vec<PathBuf> = device
        .ancestors()
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| is_usb_device(&n.to_string_lossy()))
        })
        .map(Path::to_path_buf)
        .collect();
    chain.reverse();
    chain
}

/// "2-2.1": bus, dash, port path. Not "usb2" (a root hub) and not
/// "2-2.1:1.0" (an interface).
fn is_usb_device(name: &str) -> bool {
    name.split_once('-').is_some_and(|(bus, ports)| {
        !bus.is_empty()
            && bus.chars().all(|c| c.is_ascii_digit())
            && !ports.is_empty()
            && ports.chars().all(|c| c.is_ascii_digit() || c == '.')
    })
}

/// `Some(vendor)` when a Billboard device sits under the hub `top` or under
/// the hub on the other root port of the same connector; `None` when there
/// is none. The vendor is the billboard's `manufacturer`, else its hub's.
fn dock_vendor(root: &Path, top: &Path) -> Option<Option<String>> {
    let mut hubs = vec![top.to_path_buf()];
    if let Some(other) = peer_hub(root, top) {
        hubs.push(other);
    }
    hubs.iter().find_map(|hub| {
        let billboard = find_billboard(hub, 6)?;
        Some(read(&billboard.join("manufacturer")).or_else(|| read(&hub.join("manufacturer"))))
    })
}

/// The device on the other root port of `top`'s connector.
fn peer_hub(root: &Path, top: &Path) -> Option<PathBuf> {
    let name = top.file_name()?.to_string_lossy().into_owned();
    let (bus, ports) = name.split_once('-')?;
    let port = ports.split('.').next()?;
    let roothub = top.parent()?;
    let own = roothub
        .join(format!("{bus}-0:1.0"))
        .join(format!("usb{bus}-port{port}"));
    let other_port = match fs::canonicalize(own.join("peer")) {
        Ok(p) => p,
        Err(_) => same_connector(root, &own)?,
    };
    fs::canonicalize(other_port.join("device")).ok()
}

/// Another root port wired to the same Type-C connector as `port`.
fn same_connector(root: &Path, port: &Path) -> Option<PathBuf> {
    let connector = fs::canonicalize(port.join("connector")).ok()?;
    let own = fs::canonicalize(port).ok()?;
    let buses = fs::read_dir(root.join("sys/bus/usb/devices")).ok()?;
    for bus in buses.flatten() {
        let name = bus.file_name().to_string_lossy().into_owned();
        let Some(n) = name.strip_prefix("usb") else {
            continue;
        };
        let Ok(dir) = fs::canonicalize(bus.path()) else {
            continue;
        };
        let Ok(ports) = fs::read_dir(dir.join(format!("{n}-0:1.0"))) else {
            continue;
        };
        for p in ports.flatten() {
            let path = p.path();
            if !p
                .file_name()
                .to_string_lossy()
                .starts_with(&format!("usb{n}-port"))
            {
                continue;
            }
            let Ok(canon) = fs::canonicalize(&path) else {
                continue;
            };
            if canon != own
                && fs::canonicalize(path.join("connector")).ok().as_ref() == Some(&connector)
            {
                return Some(canon);
            }
        }
    }
    None
}

/// A Billboard device at or under `dir`, by device class or by an
/// interface's class. Walks real directories only, never a symlink, so it
/// stays inside the hub's own subtree.
fn find_billboard(dir: &Path, depth: u8) -> Option<PathBuf> {
    let name = dir.file_name()?.to_string_lossy().into_owned();
    if is_usb_device(&name) {
        let class = read(&dir.join("bDeviceClass"));
        let interface_class = fs::read_dir(dir).ok()?.flatten().any(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with(&format!("{name}:"))
                && read(&e.path().join("bInterfaceClass")).as_deref() == Some(BILLBOARD)
        });
        if class.as_deref() == Some(BILLBOARD) || interface_class {
            return Some(dir.to_path_buf());
        }
    }
    if depth == 0 {
        return None;
    }
    fs::read_dir(dir).ok()?.flatten().find_map(|e| {
        let is_dir = e.file_type().ok()?.is_dir();
        let child = e.file_name().to_string_lossy().into_owned();
        (is_dir && is_usb_device(&child))
            .then(|| find_billboard(&e.path(), depth - 1))
            .flatten()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    /// A scratch sysfs root, removed when dropped.
    struct Tree(PathBuf);

    impl Tree {
        fn new(name: &str) -> Tree {
            let dir = std::env::temp_dir()
                .join(format!("swaypplet-naming-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Tree(dir)
        }
        fn file(&self, path: &str, content: &str) {
            let p = self.0.join(path);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, content).unwrap();
        }
        fn dir(&self, path: &str) {
            fs::create_dir_all(self.0.join(path)).unwrap();
        }
        fn link(&self, from: &str, to: &str) {
            let p = self.0.join(from);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            symlink(self.0.join(to), p).unwrap();
        }
        /// `sys/class/net/<iface>/device` pointing at `device`.
        fn nic(&self, iface: &str, device: &str) {
            self.dir(device);
            self.link(&format!("sys/class/net/{iface}/device"), device);
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    const XHCI: &str = "sys/devices/pci0000:00/0000:00:0d.0";

    /// This laptop with its USB-C dock: the gigabit adapter on the USB 3
    /// hub (2-2.1), the billboard on the USB 2 hub (3-1.3), the two root
    /// ports tied by one Type-C connector and no `peer` link.
    fn dock(t: &Tree) {
        t.dir("sys/devices/platform/USBC000:00/typec/port1");
        t.dir("sys/devices/platform/USBC000:00/typec/port2");
        let p2 = format!("{XHCI}/usb2/2-0:1.0/usb2-port2");
        let p3 = format!("{XHCI}/usb3/3-0:1.0/usb3-port1");
        t.link(
            &format!("{p2}/connector"),
            "sys/devices/platform/USBC000:00/typec/port1",
        );
        t.link(
            &format!("{p3}/connector"),
            "sys/devices/platform/USBC000:00/typec/port1",
        );
        // An unrelated port on another connector.
        let p5 = format!("{XHCI}/usb3/3-0:1.0/usb3-port5");
        t.link(
            &format!("{p5}/connector"),
            "sys/devices/platform/USBC000:00/typec/port2",
        );
        t.file(
            &format!("{XHCI}/usb2/2-2/manufacturer"),
            "VIA Labs, Inc.         \n",
        );
        t.file(&format!("{XHCI}/usb2/2-2/bDeviceClass"), "09\n");
        t.link(&format!("{p2}/device"), &format!("{XHCI}/usb2/2-2"));
        t.file(&format!("{XHCI}/usb3/3-1/manufacturer"), "Lenovo\n");
        t.file(&format!("{XHCI}/usb3/3-1/bDeviceClass"), "09\n");
        t.link(&format!("{p3}/device"), &format!("{XHCI}/usb3/3-1"));
        t.file(&format!("{XHCI}/usb3/3-1/3-1.3/manufacturer"), "Lenovo\n");
        t.file(&format!("{XHCI}/usb3/3-1/3-1.3/bDeviceClass"), "00\n");
        t.file(
            &format!("{XHCI}/usb3/3-1/3-1.3/3-1.3:1.0/bInterfaceClass"),
            "11\n",
        );
        t.link("sys/bus/usb/devices/usb2", &format!("{XHCI}/usb2"));
        t.link("sys/bus/usb/devices/usb3", &format!("{XHCI}/usb3"));
        t.nic("enp0s13f0u2u1", &format!("{XHCI}/usb2/2-2/2-2.1/2-2.1:1.0"));
    }

    #[test]
    fn wifi_and_a_built_in_card_have_plain_names() {
        let t = Tree::new("plain");
        t.nic("enp0s31f6", "sys/devices/pci0000:00/0000:00:1f.6");
        assert_eq!(auto_name(&t.0, "wlp0s20f3", WIFI).as_deref(), Some("Wi-Fi"));
        assert_eq!(
            auto_name(&t.0, "enp0s31f6", ETHERNET).as_deref(),
            Some("Ethernet (built-in)")
        );
        // A type the rules do not name, and a virtual device.
        assert_eq!(auto_name(&t.0, "tailscale0", 16), None);
        assert_eq!(auto_name(&t.0, "docker0", ETHERNET), None);
    }

    #[test]
    fn a_usb_adapter_on_a_dock_is_named_for_the_docks_billboard_across_the_connector() {
        let t = Tree::new("dock");
        dock(&t);
        assert_eq!(
            auto_name(&t.0, "enp0s13f0u2u1", ETHERNET).as_deref(),
            Some("Ethernet on Lenovo dock")
        );
    }

    #[test]
    fn the_peer_link_wins_over_the_connector_when_the_kernel_has_one() {
        let t = Tree::new("peer");
        dock(&t);
        // Point the connector somewhere useless; the peer link still finds it.
        let p2 = format!("{XHCI}/usb2/2-0:1.0/usb2-port2");
        fs::remove_file(t.0.join(format!("{p2}/connector"))).unwrap();
        t.link(
            &format!("{p2}/peer"),
            &format!("{XHCI}/usb3/3-0:1.0/usb3-port1"),
        );
        assert_eq!(
            auto_name(&t.0, "enp0s13f0u2u1", ETHERNET).as_deref(),
            Some("Ethernet on Lenovo dock")
        );
    }

    #[test]
    fn a_hub_without_a_billboard_and_a_bare_adapter() {
        let t = Tree::new("hub");
        t.file(&format!("{XHCI}/usb2/2-1/bDeviceClass"), "09\n");
        t.nic("enx1", &format!("{XHCI}/usb2/2-1/2-1.4/2-1.4:1.0"));
        t.nic("enx2", &format!("{XHCI}/usb2/2-3/2-3:1.0"));
        assert_eq!(
            auto_name(&t.0, "enx1", ETHERNET).as_deref(),
            Some("Ethernet on USB hub")
        );
        assert_eq!(
            auto_name(&t.0, "enx2", ETHERNET).as_deref(),
            Some("USB Ethernet adapter")
        );
    }

    #[test]
    fn a_card_behind_a_thunderbolt_port_takes_the_docks_name() {
        let t = Tree::new("tb");
        let bridge = "sys/devices/pci0000:00/0000:00:07.0/0000:20:00.0";
        t.file(&format!("{bridge}/removable"), "removable\n");
        t.nic("enp33s0", &format!("{bridge}/0000:21:00.0"));
        t.file("sys/bus/thunderbolt/devices/0-0/device_name", "Laptop\n");
        t.file(
            "sys/bus/thunderbolt/devices/0-1/device_name",
            "ThinkPad Thunderbolt 4 Dock\n",
        );
        t.file("sys/bus/thunderbolt/devices/0-1/vendor_name", "Lenovo\n");
        // A port of the switch, not a switch.
        t.dir("sys/bus/thunderbolt/devices/0-1:1.1");
        assert_eq!(
            auto_name(&t.0, "enp33s0", ETHERNET).as_deref(),
            Some("Ethernet on Lenovo ThinkPad Thunderbolt 4 Dock")
        );
        // The same card on a fixed bridge is built in.
        fs::write(t.0.join(format!("{bridge}/removable")), "fixed\n").unwrap();
        assert_eq!(
            auto_name(&t.0, "enp33s0", ETHERNET).as_deref(),
            Some("Ethernet (built-in)")
        );
    }

    #[test]
    fn the_chipset_comes_from_the_hwdb_shortened() {
        let t = Tree::new("chip");
        t.file("sys/class/net/enp0s13f0u2u1/ifindex", "301\n");
        t.file(
            "run/udev/data/n301",
            "I:1\nE:ID_VENDOR_FROM_DATABASE=Realtek Semiconductor Corp.\n\
             E:ID_MODEL_FROM_DATABASE=RTL8153 Gigabit Ethernet Adapter\n",
        );
        assert_eq!(
            chipset(&t.0, "enp0s13f0u2u1").as_deref(),
            Some("Realtek RTL8153")
        );
        t.file("sys/class/net/eth9/ifindex", "9\n");
        assert_eq!(chipset(&t.0, "eth9"), None);
    }

    #[test]
    fn shortening_drops_legal_forms_and_categories_and_nothing_else() {
        assert_eq!(short_vendor("Realtek Semiconductor Corp."), "Realtek");
        assert_eq!(short_vendor("Intel Corporation"), "Intel");
        assert_eq!(short_vendor("ASIX Electronics Corp."), "ASIX Electronics");
        assert_eq!(
            short_vendor("Hon Hai Precision Ind. Co., Ltd."),
            "Hon Hai Precision Ind."
        );
        assert_eq!(short_vendor("Lenovo"), "Lenovo");
        assert_eq!(short_model("RTL8153 Gigabit Ethernet Adapter"), "RTL8153");
        assert_eq!(
            short_model("Ethernet Connection (16) I219-LM"),
            "Ethernet Connection (16) I219-LM"
        );
        assert_eq!(short_model("BE200 Series Wi-Fi 7"), "BE200 Series Wi-Fi 7");
        // A model that is only the category stays.
        assert_eq!(
            short_model("Gigabit Ethernet Adapter"),
            "Gigabit Ethernet Adapter"
        );
    }

    /// What the rules call every NetworkManager device on this machine.
    /// `cargo test --release live_names -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_names() {
        let root = Path::new("/");
        for e in fs::read_dir("/sys/class/net").unwrap().flatten() {
            let iface = e.file_name().to_string_lossy().into_owned();
            let ty = match read(&e.path().join("type")).as_deref() {
                _ if e.path().join("wireless").exists() => WIFI,
                Some("1") => ETHERNET,
                _ => 0,
            };
            println!(
                "{iface:20} {:28} {}",
                auto_name(root, &iface, ty).unwrap_or_else(|| "-".into()),
                chipset(root, &iface).unwrap_or_else(|| "-".into())
            );
        }
    }
}
