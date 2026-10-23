//! The Cast devices on the network, from avahi over D-Bus.
//!
//! Every Cast device (a Chromecast, a TV with Cast built in, a Nest Hub, a
//! speaker, a speaker group) announces `_googlecast._tcp` over mDNS. avahi
//! already listens for mDNS on this machine (nss-mdns resolves `.local`
//! through it), so asking it costs no socket of our own, no second responder
//! on port 5353 and no new crate: zbus is here for NetworkManager and BlueZ
//! already. Parsing `avahi-browse -p` would have been the other way, and it
//! escapes every byte past ASCII as `\DDD` ("Köket" arrives as
//! `K\195\182ket`); the D-Bus answer is the TXT record's own bytes.
//!
//! One browse is a burst, not a watch: a browser is made, the cache it
//! answers from arrives as `ItemNew` up to `AllForNow`, each one is resolved
//! to an address, and the browser is freed. The section asks again when it
//! is opened; nothing runs at rest.

use std::collections::HashMap;
use std::time::Duration;

use zbus::export::futures_util::StreamExt;
use zbus::zvariant::OwnedObjectPath;
use zbus::{Connection, MatchRule, MessageStream, Proxy};

const AVAHI: &str = "org.freedesktop.Avahi";
const SERVER: &str = "org.freedesktop.Avahi.Server";
const BROWSER: &str = "org.freedesktop.Avahi.ServiceBrowser";
const SERVICE_TYPE: &str = "_googlecast._tcp";
/// Any interface.
const IF_UNSPEC: i32 = -1;
/// IPv4 only: the Cast channel is TCP to the device's address, and an IPv6
/// answer is a link-local address that needs its interface to be dialled.
const PROTO_INET: i32 = 0;

/// The `ca` TXT field is a bit set (Chromium's `CastDeviceCapability`).
const CAP_VIDEO_OUT: u32 = 1 << 0;
const CAP_MULTIZONE_GROUP: u32 = 1 << 5;

/// One Cast device, as the section lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    /// The device's own id (`id`), stable across addresses and renames.
    pub id: String,
    /// The name its owner gave it (`fn`): "Vardagsrummet TV".
    pub name: String,
    /// What it is (`md`): "Chromecast", "Google Nest Hub".
    pub model: Option<String>,
    pub address: String,
    /// The Cast channel's port: 8009 for a device, another for a group.
    pub port: u16,
    /// It has a screen (`ca` bit 0). A speaker or a speaker group does not.
    pub video: bool,
    /// A speaker group, not a device.
    pub group: bool,
}

/// TXT entries (`key=value`, as bytes) as a map. A key without `=` is a
/// boolean attribute, kept with an empty value; a value that is not UTF-8
/// keeps what it can.
pub fn parse_txt(entries: &[Vec<u8>]) -> HashMap<String, String> {
    entries
        .iter()
        .filter(|e| !e.is_empty())
        .map(|e| {
            let text = String::from_utf8_lossy(e);
            match text.split_once('=') {
                Some((k, v)) => (k.to_ascii_lowercase(), v.to_string()),
                None => (text.to_ascii_lowercase(), String::new()),
            }
        })
        .collect()
}

impl Device {
    /// A device from one resolved record, or `None` without an id, which
    /// every Cast device sends. `instance` is the mDNS name, the fallback for
    /// a device that sends no `fn`.
    pub fn from_record(
        instance: &str,
        address: &str,
        port: u16,
        txt: &[Vec<u8>],
    ) -> Option<Device> {
        let txt = parse_txt(txt);
        let id = txt.get("id").filter(|s| !s.is_empty())?.clone();
        let caps: u32 = txt.get("ca").and_then(|s| s.parse().ok()).unwrap_or(0);
        let name = txt
            .get("fn")
            .filter(|s| !s.trim().is_empty())
            .cloned()
            .unwrap_or_else(|| instance.to_string());
        Some(Device {
            id,
            name,
            model: txt.get("md").filter(|s| !s.is_empty()).cloned(),
            address: address.to_string(),
            port,
            video: caps & CAP_VIDEO_OUT != 0,
            group: caps & CAP_MULTIZONE_GROUP != 0,
        })
    }
}

/// The list a person picks from: one row per device (a device answers on
/// every interface it is reachable on), screens first, then by name.
pub fn tidy(mut devices: Vec<Device>) -> Vec<Device> {
    devices.sort_by(|a, b| {
        b.video
            .cmp(&a.video)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.id.cmp(&b.id))
    });
    devices.dedup_by(|a, b| a.id == b.id);
    devices
}

/// One `ItemNew`: what `ResolveService` needs back.
type Item = (i32, i32, String, String, String, u32);
/// `ResolveService`'s reply: interface, protocol, name, type, domain, host,
/// address protocol, address, port, TXT, flags.
type Resolved = (
    i32,
    i32,
    String,
    String,
    String,
    String,
    i32,
    String,
    u16,
    Vec<Vec<u8>>,
    u32,
);

/// Browse once, waiting at most `wait` for avahi's cache to be listed.
/// Needs a tokio runtime (zbus runs on it here).
pub async fn browse(wait: Duration) -> Result<Vec<Device>, String> {
    let conn = Connection::system()
        .await
        .map_err(|e| format!("system bus: {e}"))?;
    // Subscribed before the browser exists: avahi can send the first items
    // before the reply that names the browser, and those must not be lost.
    let rule = MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(AVAHI)
        .and_then(|r| r.interface(BROWSER))
        .map_err(|e| e.to_string())?
        .build();
    let mut signals = MessageStream::for_match_rule(rule, &conn, Some(256))
        .await
        .map_err(|e| format!("avahi signals: {e}"))?;
    let server = Proxy::new(&conn, AVAHI, "/", SERVER)
        .await
        .map_err(|e| format!("avahi: {e}"))?;
    let browser: OwnedObjectPath = server
        .call(
            "ServiceBrowserNew",
            &(IF_UNSPEC, PROTO_INET, SERVICE_TYPE, "", 0u32),
        )
        .await
        .map_err(|e| format!("avahi ServiceBrowserNew: {e}"))?;

    let mut items: Vec<Item> = Vec::new();
    let collect = async {
        while let Some(Ok(msg)) = signals.next().await {
            let header = msg.header();
            if header.path().map(|p| p.as_str()) != Some(browser.as_str()) {
                continue;
            }
            match header.member().map(|m| m.as_str()) {
                Some("ItemNew") => {
                    if let Ok(item) = msg.body().deserialize::<Item>() {
                        items.push(item);
                    }
                }
                Some("AllForNow") => break,
                Some("Failure") => {
                    log::info!("cast: avahi browse failed");
                    break;
                }
                _ => {}
            }
        }
    };
    if tokio::time::timeout(wait, collect).await.is_err() {
        log::debug!("cast: no AllForNow in {wait:?}; using what came");
    }
    if let Ok(b) = Proxy::new(&conn, AVAHI, browser.as_str(), BROWSER).await {
        let _: Result<(), _> = b.call("Free", &()).await;
    }

    let mut devices = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (iface, proto, name, kind, domain, _) in items {
        if !seen.insert(name.clone()) {
            continue;
        }
        let resolved: Result<Resolved, _> = server
            .call(
                "ResolveService",
                &(
                    iface,
                    proto,
                    name.as_str(),
                    kind.as_str(),
                    domain.as_str(),
                    PROTO_INET,
                    0u32,
                ),
            )
            .await;
        match resolved {
            Ok((_, _, instance, _, _, _, _, address, port, txt, _)) => {
                devices.extend(Device::from_record(&instance, &address, port, &txt));
            }
            Err(e) => log::debug!("cast: resolving {name}: {e}"),
        }
    }
    Ok(tidy(devices))
}

/// [`browse`] from a thread that has no runtime: a worker, or the CLI.
pub fn browse_blocking(wait: Duration) -> Result<Vec<Device>, String> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("runtime: {e}"))?
        .block_on(browse(wait))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn txt(entries: &[&str]) -> Vec<Vec<u8>> {
        entries.iter().map(|e| e.as_bytes().to_vec()).collect()
    }

    /// The records a Chromecast and a speaker on this network sent
    /// (avahi-browse, 2026-10-23), as bytes.
    fn chromecast() -> Vec<Vec<u8>> {
        txt(&[
            "id=daf6af366ecf480ec83565f986bd6b76",
            "cd=6DC708454BE6263DB310CCF150404FA4",
            "rm=2B8C5F2B74D126F6",
            "ve=05",
            "md=Chromecast",
            "ic=/setup/icon.png",
            "fn=Vardagsrummet TV",
            "ca=465413",
            "st=0",
            "bs=FA8FCA57E41E",
            "nf=1",
            "rs=",
        ])
    }

    #[test]
    fn a_chromecast_is_a_screen_with_its_owners_name() {
        let d =
            Device::from_record("Chromecast-daf6", "192.168.1.107", 8009, &chromecast()).unwrap();
        assert_eq!(d.name, "Vardagsrummet TV");
        assert_eq!(d.model.as_deref(), Some("Chromecast"));
        assert_eq!(d.id, "daf6af366ecf480ec83565f986bd6b76");
        assert_eq!((d.address.as_str(), d.port), ("192.168.1.107", 8009));
        assert!(d.video && !d.group);
    }

    #[test]
    fn speakers_and_groups_have_no_screen() {
        // "Köket" as the device sends it: UTF-8, not avahi-browse's \195\182.
        let speaker = Device::from_record(
            "HK-Citation-300",
            "192.168.1.28",
            8009,
            &txt(&["id=af14", "fn=Köket", "md=HK Citation 300", "ca=199172"]),
        )
        .unwrap();
        assert_eq!(speaker.name, "Köket");
        assert!(!speaker.video && !speaker.group);
        let group = Device::from_record(
            "Google-Cast-Group",
            "192.168.1.28",
            32042,
            &txt(&[
                "id=7260",
                "fn=Hela huset",
                "md=Google Cast Group",
                "ca=199204",
            ]),
        )
        .unwrap();
        assert!(!group.video && group.group);
    }

    #[test]
    fn a_record_without_an_id_is_not_a_device_and_fn_falls_back() {
        assert!(Device::from_record("x", "1.2.3.4", 8009, &txt(&["fn=TV"])).is_none());
        let d =
            Device::from_record("Living-Room", "1.2.3.4", 8009, &txt(&["id=1", "fn= "])).unwrap();
        assert_eq!(d.name, "Living-Room");
        assert_eq!(d.model, None);
        assert!(!d.video, "no ca: nothing is promised");
    }

    #[test]
    fn txt_keys_are_case_blind_and_values_keep_their_equals() {
        let t = parse_txt(&txt(&["FN=A=B", "flag", ""]));
        assert_eq!(t.get("fn").map(String::as_str), Some("A=B"));
        assert_eq!(t.get("flag").map(String::as_str), Some(""));
        assert_eq!(t.len(), 2);
        // Bytes that are not UTF-8 do not lose the rest of the record.
        let t = parse_txt(&[b"fn=\xffTV".to_vec(), b"id=9".to_vec()]);
        assert_eq!(t.get("id").map(String::as_str), Some("9"));
        assert!(t.get("fn").unwrap().ends_with("TV"));
    }

    #[test]
    fn the_list_has_one_row_per_device_screens_first() {
        let dev = |id: &str, name: &str, video: bool| Device {
            id: id.into(),
            name: name.into(),
            model: None,
            address: "1.2.3.4".into(),
            port: 8009,
            video,
            group: false,
        };
        let list = tidy(vec![
            dev("s", "Soundbar", false),
            dev("b", "Sovrummet Hub", true),
            dev("a", "Vardagsrummet TV", true),
            dev("b", "Sovrummet Hub", true),
            dev("c", "Ateljé", false),
        ]);
        let names: Vec<&str> = list.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(
            names,
            ["Sovrummet Hub", "Vardagsrummet TV", "Ateljé", "Soundbar"]
        );
    }
}
