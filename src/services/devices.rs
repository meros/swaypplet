//! Names a person reads for a device, and the names they gave.
//!
//! Every kind of device the shell shows (network adapters, audio outputs
//! and inputs, displays, input devices) has an automatic name, worked out
//! by its own service from what the device says about itself, and may have
//! one the person chose with Rename. The chosen ones live in the settings
//! file's user-only `devices` section, keyed by [`DeviceKey`]: something
//! that follows the device itself rather than the port it is plugged into.
//! Bluetooth is the exception: BlueZ keeps a per-device `Alias` of its own,
//! and Rename writes that (`services::bluez::set_alias`).
//!
//! The technical name (interface, connector, node description, sway name)
//! is never replaced, only moved: callers put it in the subtitle or the
//! tooltip so it stays findable.

/// What a stored name is keyed by.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DeviceKey {
    /// A network adapter's permanent MAC address.
    Net(String),
    /// A PipeWire node's `node.name`, stable across replugs and reboots.
    Audio(String),
    /// A display's make, model and serial (`displays::identity`).
    Display(String),
    /// Sway's input identifier, `vendor:product:name`.
    Input(String),
}

impl DeviceKey {
    /// The key in the settings file: `net:f4:a8:0d:5b:70:33`.
    pub fn key(&self) -> String {
        match self {
            DeviceKey::Net(mac) => format!("net:{}", mac.to_ascii_lowercase()),
            DeviceKey::Audio(node) => format!("audio:{node}"),
            DeviceKey::Display(id) => format!("display:{id}"),
            DeviceKey::Input(id) => format!("input:{id}"),
        }
    }
}

/// What the person reads: the name they gave the device, else `auto`.
/// Reads the live settings, so main thread only.
pub fn display_name(key: Option<&DeviceKey>, auto: &str) -> String {
    key.and_then(|key| {
        let key = key.key();
        crate::settings::store::with(|s| s.devices.as_ref()?.names.get(&key).cloned())
    })
    .unwrap_or_else(|| auto.to_string())
}

/// Store `name` for the device, or clear it (`name` empty after trimming)
/// to go back to the automatic name. Main thread only.
pub fn rename(key: &DeviceKey, name: &str) {
    let (key, name) = (key.key(), name.trim().to_string());
    crate::settings::store::edit::<crate::settings::store::Devices>(|d| {
        if name.is_empty() {
            d.names.remove(&key);
        } else {
            d.names.insert(key, name);
        }
    });
}

/// Run `cb` when a stored name changes: every surface that shows a device
/// redraws its names on this. The settings notify on every edit (a slider
/// drag is dozens), so this passes on only the edits that moved the
/// `devices` section.
pub fn observe(cb: impl Fn() + 'static) {
    let names = || crate::settings::store::with(|s| s.devices.clone());
    let last = std::cell::RefCell::new(names());
    crate::settings::store::observe(move || {
        let now = names();
        if *last.borrow() != now {
            *last.borrow_mut() = now;
            cb();
        }
    });
}

/// A company's name without its legal form: "Realtek Semiconductor Corp."
/// is "Realtek", "Intel Corporation" is "Intel", "Samsung Display Corp." is
/// "Samsung Display". Cut at the first such word, never down to nothing.
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
    let keep = words
        .iter()
        .position(|w| NOISE.iter().any(|n| n.trim_end_matches(',') == *w))
        .filter(|&i| i > 0)
        .unwrap_or(words.len());
    words[..keep].join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_carry_their_kind() {
        assert_eq!(
            DeviceKey::Net("F4:A8:0D:5B:70:33".into()).key(),
            "net:f4:a8:0d:5b:70:33"
        );
        assert_eq!(
            DeviceKey::Audio("alsa_output.usb-Creative".into()).key(),
            "audio:alsa_output.usb-Creative"
        );
        assert_eq!(
            DeviceKey::Display("Dell Inc.|U2723QE|ABC123".into()).key(),
            "display:Dell Inc.|U2723QE|ABC123"
        );
        assert_eq!(
            DeviceKey::Input("1:1:AT_Translated_Set_2_keyboard".into()).key(),
            "input:1:1:AT_Translated_Set_2_keyboard"
        );
    }

    #[test]
    fn a_device_without_a_key_or_a_name_reads_as_its_automatic_name() {
        assert_eq!(display_name(None, "Wi-Fi"), "Wi-Fi");
        assert_eq!(
            display_name(
                Some(&DeviceKey::Net("00:11:22:33:44:55".into())),
                "Ethernet (built-in)"
            ),
            "Ethernet (built-in)"
        );
    }

    #[test]
    fn legal_forms_are_dropped_from_vendors() {
        assert_eq!(short_vendor("Realtek Semiconductor Corp."), "Realtek");
        assert_eq!(short_vendor("Intel Corporation"), "Intel");
        assert_eq!(short_vendor("Samsung Display Corp."), "Samsung Display");
        assert_eq!(short_vendor("ASIX Electronics Corp."), "ASIX Electronics");
        assert_eq!(
            short_vendor("Hon Hai Precision Ind. Co., Ltd."),
            "Hon Hai Precision Ind."
        );
        assert_eq!(short_vendor("Dell Inc."), "Dell");
        assert_eq!(short_vendor("Lenovo"), "Lenovo");
    }

    /// The automatic name of every audio node, output, input device and
    /// network adapter on this machine, as each rule set computes it.
    /// `cargo test --release live_device_names -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_device_names() {
        use std::process::Command;
        let json = |cmd: &mut Command| -> serde_json::Value {
            let out = cmd.output().expect("run");
            serde_json::from_slice(&out.stdout).unwrap_or_default()
        };

        println!("── audio (pw-dump) ──");
        let dump = json(&mut Command::new("pw-dump"));
        let objects = dump.as_array().cloned().unwrap_or_default();
        let props = |o: &serde_json::Value| o["info"]["props"].clone();
        let card = |id: &serde_json::Value| {
            objects
                .iter()
                .find(|o| &o["id"] == id)
                .and_then(|o| props(o)["device.description"].as_str().map(str::to_string))
        };
        for o in &objects {
            let p = props(o);
            let class = p["media.class"].as_str().unwrap_or_default();
            if class != "Audio/Sink" && class != "Audio/Source" {
                continue;
            }
            let description = p["node.description"].as_str().unwrap_or_default();
            let card_description = card(&p["device.id"]);
            let n = crate::services::audio::NodeNames {
                description,
                nick: p["node.nick"].as_str(),
                card: card_description.as_deref(),
                bluetooth: p["device.api"].as_str() == Some("bluez5"),
            };
            println!(
                "{class:12} {:34} <- {description}",
                crate::services::audio::device_name(&n)
            );
        }

        println!("── outputs (swaymsg) ──");
        let outputs = json(Command::new("swaymsg").args(["-t", "get_outputs", "-r"]));
        for o in outputs.as_array().cloned().unwrap_or_default() {
            let f = |k: &str| o[k].as_str().unwrap_or_default().to_string();
            println!(
                "{:8} {:30} key {}",
                f("name"),
                crate::services::displays::naming::auto_name(&f("name"), &f("make"), &f("model")),
                crate::services::displays::naming::key(
                    &f("name"),
                    &f("make"),
                    &f("model"),
                    &f("serial")
                )
            );
        }

        println!("── inputs (swaymsg) ──");
        let inputs = json(Command::new("swaymsg").args(["-t", "get_inputs", "-r"]));
        let all = crate::services::input::parse_devices(&inputs);
        let listed = crate::services::input::listed(&all);
        for d in &all {
            let shown = listed
                .iter()
                .any(|l| l.identifier == d.identifier && l.kind == d.kind);
            println!(
                "{:6} {:9} {:30} <- {}",
                if shown { "shown" } else { "hidden" },
                d.kind,
                crate::services::input::auto_name(d),
                d.name
            );
        }

        println!("── network ──");
        let root = std::path::Path::new("/");
        for e in std::fs::read_dir("/sys/class/net").unwrap().flatten() {
            let iface = e.file_name().to_string_lossy().into_owned();
            let ty = if e.path().join("wireless").exists() {
                2
            } else {
                1
            };
            if let Some(name) = crate::services::network::naming::auto_name(root, &iface, ty) {
                let chip =
                    crate::services::network::naming::chipset(root, &iface).unwrap_or_default();
                println!("{iface:16} {name:28} {chip}");
            }
        }
    }
}
