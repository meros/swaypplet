//! Clipboard history over `ext-data-control-v1`.
//!
//! # Why in-process
//!
//! The panel's clipboard section used to shell out to `cliphist list`, which
//! only ever has anything to say if something else is running
//! `wl-paste --watch cliphist store`. Nothing was. The database on this
//! machine had not been written to in three months, so the section faithfully
//! showed a snapshot of one afternoon in May and nothing since — a widget
//! wired to a daemon nobody started fails silently and looks like it works.
//!
//! `ext-data-control-v1` is the protocol written for exactly this job: a
//! privileged client watches the selection, reads each new one, and can put
//! one back. Owning it here removes the missing daemon, the `cliphist` and
//! `wl-clipboard` dependencies, and three blocking `Command` round-trips per
//! panel open.
//!
//! # Shape
//!
//! One thread owns a second Wayland connection (GDK keeps its own, and this
//! one blocks on pipe reads that must never sit in front of a frame). It
//! pushes through an `async_channel` into an [`Observed`] on the GTK side,
//! the same shape `sway_ipc` and `bar::tray` use, so consumers connect and
//! read exactly as they do there. Requests go the other way as plain
//! Wayland calls: proxies are `Send`, so setting the selection from the GTK
//! thread is safe as long as the watcher thread is the one dispatching the
//! events that follow.
//!
//! # What is deliberately not here
//!
//! *Persistence.* cliphist writes history to disk, which is how clipboard
//! managers end up with passwords in a file that outlives the session. The
//! ring lives in memory and dies with the panel.
//!
//! *Images.* Text mimes only. An image ring needs a memory cap and an
//! eviction policy of its own, and a preview that is not a string.

use std::cell::OnceCell;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::fd::{AsFd, FromRawFd, OwnedFd};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::service::Observed;

use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, delegate_noop};
use wayland_protocols::ext::data_control::v1::client::{
    ext_data_control_device_v1::{self, ExtDataControlDeviceV1},
    ext_data_control_manager_v1::ExtDataControlManagerV1,
    ext_data_control_offer_v1::{self, ExtDataControlOfferV1},
    ext_data_control_source_v1::{self, ExtDataControlSourceV1},
};

/// Entries kept. The panel renders the first [`VISIBLE_ENTRIES`]; the rest
/// are here so a restore of something older is one scroll away once the
/// section grows a longer list.
const MAX_ENTRIES: usize = 50;

/// How many the panel section shows.
pub const VISIBLE_ENTRIES: usize = 10;

/// Anything larger is a file being moved around, not something a person
/// wants to pick out of a list, and holding it costs the session memory
/// for as long as it runs.
const MAX_BYTES: usize = 1024 * 1024;

/// Single-line preview length, matching what the section rendered before.
const PREVIEW_LEN: usize = 60;

/// Text flavours in descending order of preference. The first one an offer
/// advertises is the one read.
const TEXT_MIMES: &[&str] = &[
    "text/plain;charset=utf-8",
    "text/plain;charset=UTF-8",
    "UTF8_STRING",
    "text/plain",
    "STRING",
    "TEXT",
];

/// The cross-desktop marker password managers put on a secret so clipboard
/// managers skip it. KDE defined it; GNOME, 1Password, KeePassXC and Bitwarden
/// all emit it. Honouring it is the difference between a history widget and a
/// password leak with a scrollbar.
const PASSWORD_HINT_MIME: &str = "x-kde-passwordManagerHint";

// ── Model ───────────────────────────────────────────────────────────────

/// One clipboard entry, kept whole so it can be put back byte for byte.
struct Entry {
    id: u64,
    mime: String,
    bytes: Arc<Vec<u8>>,
    preview: String,
}

/// What the panel needs to draw a row. The bytes stay behind the service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryView {
    pub id: u64,
    pub preview: String,
}

/// The payload a restored entry serves, attached to the source object as its
/// user data so the `send` handler has it without a lookup.
struct Payload {
    mime: String,
    bytes: Arc<Vec<u8>>,
}

// ── Service ─────────────────────────────────────────────────────────────

/// Shared between the GTK thread and the watcher thread.
struct Shared {
    conn: Connection,
    manager: ExtDataControlManagerV1,
    device: ExtDataControlDeviceV1,
    qh: QueueHandle<Watcher>,
    history: Mutex<Vec<Entry>>,
    /// The source we own while a restored entry holds the selection. Its
    /// presence is what stops the watcher reading its own paste back: the
    /// compositor would ask this client to write while this thread sits
    /// blocked on the read, which is a deadlock with itself.
    own: Mutex<Option<ExtDataControlSourceV1>>,
    next_id: AtomicU64,
}

/// Clipboard history for the panel. Lives on the GTK thread behind `Rc`;
/// the watcher and the bytes live behind the `Arc`.
pub struct ClipboardService {
    shared: Arc<Shared>,
    state: Observed<Vec<EntryView>>,
}

/// One clipboard per session, so both the panel and the preview harness
/// build their section against the same ring instead of racing two watchers
/// for one selection.
pub fn service() -> Option<Rc<ClipboardService>> {
    thread_local! {
        static SERVICE: OnceCell<Option<Rc<ClipboardService>>> = const { OnceCell::new() };
    }
    SERVICE.with(|cell| cell.get_or_init(ClipboardService::start).clone())
}

impl ClipboardService {
    /// Connect, bind, and start watching. `None` when this is not Wayland or
    /// the compositor does not offer `ext-data-control-v1`, which leaves the
    /// section to say so rather than showing an empty list that looks full.
    fn start() -> Option<Rc<Self>> {
        let conn = Connection::connect_to_env()
            .map_err(|e| log::info!("clipboard: no Wayland connection: {e}"))
            .ok()?;

        let mut queue = conn.new_event_queue::<Bootstrap>();
        let qh_boot = queue.handle();
        let _registry = conn.display().get_registry(&qh_boot, ());
        let mut boot = Bootstrap::default();
        // Two round-trips: the first brings the globals in, the second the
        // seat's own events. Binding needs both before a device exists.
        queue.roundtrip(&mut boot).ok()?;
        queue.roundtrip(&mut boot).ok()?;

        let (manager, seat) = match (boot.manager, boot.seat) {
            (Some(m), Some(s)) => (m, s),
            _ => {
                log::info!(
                    "clipboard: ext_data_control_manager_v1 not offered; history unavailable"
                );
                return None;
            }
        };

        // The watcher gets its own queue so the bootstrap one can be dropped
        // with its short-lived registry.
        let queue = conn.new_event_queue::<Watcher>();
        let qh = queue.handle();
        let device = manager.get_data_device(&seat, &qh, ());

        let (tx, rx) = async_channel::unbounded();
        let shared = Arc::new(Shared {
            conn: conn.clone(),
            manager,
            device,
            qh,
            history: Mutex::new(Vec::new()),
            own: Mutex::new(None),
            next_id: AtomicU64::new(1),
        });

        let watcher = Watcher {
            shared: shared.clone(),
            offers: HashMap::new(),
            tx,
        };
        std::thread::Builder::new()
            .name("swaypplet-clipboard".into())
            .spawn(move || run(queue, watcher))
            .map_err(|e| log::warn!("clipboard: watcher thread: {e}"))
            .ok()?;

        let service = Rc::new(ClipboardService {
            shared,
            state: Observed::new(Vec::new()),
        });

        // The loop holds a strong ref, so the service persists for the
        // process — the same lifetime rule as the other push services.
        let held = service.clone();
        glib::MainContext::default().spawn_local(async move {
            while rx.recv().await.is_ok() {
                held.state.set_if_changed(held.snapshot());
            }
        });

        Some(service)
    }

    /// Newest first, capped at [`VISIBLE_ENTRIES`].
    pub fn entries(&self) -> Vec<EntryView> {
        self.state.with(|v| v.clone())
    }

    pub fn connect_change(&self, cb: impl Fn() + 'static) {
        self.state.connect_change(cb);
    }

    fn snapshot(&self) -> Vec<EntryView> {
        let history = self.shared.history.lock().unwrap_or_else(|e| e.into_inner());
        history
            .iter()
            .take(VISIBLE_ENTRIES)
            .map(|e| EntryView {
                id: e.id,
                preview: e.preview.clone(),
            })
            .collect()
    }

    /// Put an entry back on the selection. Unknown ids are a no-op: the row
    /// that named it is from a snapshot, and the ring may have moved on.
    pub fn restore(&self, id: u64) {
        let payload = {
            let history = self.shared.history.lock().unwrap_or_else(|e| e.into_inner());
            match history.iter().find(|e| e.id == id) {
                Some(e) => Payload {
                    mime: e.mime.clone(),
                    bytes: e.bytes.clone(),
                },
                None => return,
            }
        };
        self.shared.set_selection(payload);
    }

    /// Drop everything. The current selection is left alone — clearing the
    /// history is not the same as taking the clipboard away from whoever
    /// owns it, and a Clear that wiped the live selection would surprise.
    pub fn clear(&self) {
        self.shared
            .history
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        self.state.set_if_changed(Vec::new());
    }
}

impl Shared {
    /// Offer `payload` as the selection, replacing anything we already own.
    fn set_selection(&self, payload: Payload) {
        let mime = payload.mime.clone();
        let source = self.manager.create_data_source(&self.qh, Arc::new(payload));
        source.offer(mime);
        self.device.set_selection(Some(&source));
        // Destroy the previous source only after the new one is in place, so
        // the selection never blinks through empty.
        let previous = self
            .own
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .replace(source);
        if let Some(old) = previous {
            old.destroy();
        }
        let _ = self.conn.flush();
    }
}

// ── Watcher thread ──────────────────────────────────────────────────────

fn run(mut queue: wayland_client::EventQueue<Watcher>, mut watcher: Watcher) {
    loop {
        if let Err(e) = queue.blocking_dispatch(&mut watcher) {
            log::warn!("clipboard: watcher stopped: {e}");
            return;
        }
    }
}

struct Watcher {
    shared: Arc<Shared>,
    /// Mimes accumulated per offer between `data_offer` and `selection`.
    offers: HashMap<wayland_client::backend::ObjectId, Vec<String>>,
    tx: async_channel::Sender<()>,
}

impl Watcher {
    /// Read the selection behind `offer` and file it.
    fn take(&mut self, offer: &ExtDataControlOfferV1) {
        let Some(mimes) = self.offers.remove(&offer.id()) else {
            return;
        };
        if mimes.iter().any(|m| m == PASSWORD_HINT_MIME) {
            // The hint's own value says whether this is a secret. Reading it
            // is one extra pipe; guessing from the mime's presence alone
            // would drop ordinary copies made from a password manager.
            let hint = read_mime(&self.shared.conn, offer, PASSWORD_HINT_MIME);
            if hint.as_deref().map(|b| b.trim_ascii()) == Some(b"secret") {
                log::debug!("clipboard: skipping a password-manager secret");
                return;
            }
        }
        let Some(mime) = TEXT_MIMES
            .iter()
            .find(|want| mimes.iter().any(|m| m == *want))
        else {
            return;
        };
        let Some(bytes) = read_mime(&self.shared.conn, offer, mime) else {
            return;
        };
        if bytes.is_empty() || bytes.len() > MAX_BYTES {
            return;
        }
        let Ok(text) = std::str::from_utf8(&bytes) else {
            return;
        };
        if text.trim().is_empty() {
            return;
        }
        self.file(Entry {
            id: self.shared.next_id.fetch_add(1, Ordering::Relaxed),
            mime: (*mime).to_string(),
            preview: preview(text),
            bytes: Arc::new(bytes),
        });
    }

    /// Push to the front, collapsing a repeat of something already held, and
    /// wake the GTK side.
    fn file(&self, entry: Entry) {
        {
            let mut history = self.shared.history.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(pos) = history.iter().position(|e| e.bytes == entry.bytes) {
                // Re-copying something is a bump, not a second row.
                let existing = history.remove(pos);
                history.insert(0, existing);
            } else {
                history.insert(0, entry);
                history.truncate(MAX_ENTRIES);
            }
        }
        let _ = self.tx.try_send(());
    }
}

/// Ask for one mime and read the pipe to EOF.
///
/// The flush matters: `receive` only reaches the owning client when the
/// connection is written out, and without it this blocks on a pipe whose
/// writer has not been told to write.
fn read_mime(conn: &Connection, offer: &ExtDataControlOfferV1, mime: &str) -> Option<Vec<u8>> {
    let (read_fd, write_fd) = pipe()?;
    offer.receive(mime.to_string(), write_fd.as_fd());
    let _ = conn.flush();
    // Dropping our end is what lets the read see EOF once the writer is done.
    drop(write_fd);

    let mut file = std::fs::File::from(read_fd);
    let mut buf = Vec::new();
    // take() caps a source that keeps writing; one extra byte distinguishes
    // "exactly at the cap" from "over it", which the caller rejects.
    match Read::take(&mut file, MAX_BYTES as u64 + 1).read_to_end(&mut buf) {
        Ok(_) => Some(buf),
        Err(e) => {
            log::debug!("clipboard: read {mime}: {e}");
            None
        }
    }
}

fn pipe() -> Option<(OwnedFd, OwnedFd)> {
    let mut fds = [0 as libc::c_int; 2];
    // CLOEXEC so a spawned child never inherits a clipboard pipe.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        log::warn!("clipboard: pipe2: {}", std::io::Error::last_os_error());
        return None;
    }
    Some(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

/// Single-line, bounded preview. Newlines collapse to spaces so a multi-line
/// copy still occupies one row.
fn preview(text: &str) -> String {
    let single: String = text
        .trim()
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let single = single.split_whitespace().collect::<Vec<_>>().join(" ");
    if single.chars().count() <= PREVIEW_LEN {
        single
    } else {
        let head: String = single.chars().take(PREVIEW_LEN).collect();
        format!("{head}…")
    }
}

// ── Dispatch ────────────────────────────────────────────────────────────

/// Registry pass, thrown away once the manager and seat are bound.
#[derive(Default)]
struct Bootstrap {
    manager: Option<ExtDataControlManagerV1>,
    seat: Option<wl_seat::WlSeat>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for Bootstrap {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        else {
            return;
        };
        if interface == ExtDataControlManagerV1::interface().name {
            state.manager = Some(registry.bind(name, version.min(1), qh, ()));
        } else if interface == wl_seat::WlSeat::interface().name && state.seat.is_none() {
            // First seat wins; a second one is a remote-control seat
            // (ext_transient_seat_v1) with no clipboard of interest.
            state.seat = Some(registry.bind(name, version.min(1), qh, ()));
        }
    }
}

delegate_noop!(Bootstrap: ignore ExtDataControlManagerV1);
delegate_noop!(Bootstrap: ignore wl_seat::WlSeat);

impl Dispatch<ExtDataControlDeviceV1, ()> for Watcher {
    fn event(
        state: &mut Self,
        _: &ExtDataControlDeviceV1,
        event: ext_data_control_device_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            // Announced before its mimes; the offer's own events fill it in.
            ext_data_control_device_v1::Event::DataOffer { id } => {
                state.offers.insert(id.id(), Vec::new());
            }
            ext_data_control_device_v1::Event::Selection { id: Some(offer) } => {
                let owned = state
                    .shared
                    .own
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .is_some();
                if owned {
                    // Our own restore coming back. Reading it would ask this
                    // client to write while this thread blocks on the read.
                    state.offers.remove(&offer.id());
                } else {
                    state.take(&offer);
                }
                offer.destroy();
            }
            // Selection cleared, or the primary selection, which is not
            // history: middle-click text is a gesture, not a copy.
            ext_data_control_device_v1::Event::Selection { id: None } => {}
            ext_data_control_device_v1::Event::PrimarySelection { id: Some(offer) } => {
                state.offers.remove(&offer.id());
                offer.destroy();
            }
            ext_data_control_device_v1::Event::PrimarySelection { id: None } => {}
            ext_data_control_device_v1::Event::Finished => {
                log::info!("clipboard: device finished; history frozen");
            }
            _ => {}
        }
    }

    wayland_client::event_created_child!(Watcher, ExtDataControlDeviceV1, [
        ext_data_control_device_v1::EVT_DATA_OFFER_OPCODE => (ExtDataControlOfferV1, ()),
    ]);
}

impl Dispatch<ExtDataControlOfferV1, ()> for Watcher {
    fn event(
        state: &mut Self,
        offer: &ExtDataControlOfferV1,
        event: ext_data_control_offer_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_data_control_offer_v1::Event::Offer { mime_type } = event
            && let Some(mimes) = state.offers.get_mut(&offer.id())
        {
            mimes.push(mime_type);
        }
    }
}

impl Dispatch<ExtDataControlSourceV1, Arc<Payload>> for Watcher {
    fn event(
        state: &mut Self,
        source: &ExtDataControlSourceV1,
        event: ext_data_control_source_v1::Event,
        payload: &Arc<Payload>,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_data_control_source_v1::Event::Send { mime_type, fd } => {
                if mime_type != payload.mime {
                    return;
                }
                // Off-thread: a reader that takes its time must not stall
                // the watcher, and a pipe buffer is 64 KiB.
                let bytes = payload.bytes.clone();
                std::thread::Builder::new()
                    .name("swaypplet-clip-send".into())
                    .spawn(move || {
                        let mut file = std::fs::File::from(fd);
                        if let Err(e) = file.write_all(&bytes) {
                            log::debug!("clipboard: serving selection: {e}");
                        }
                    })
                    .map_err(|e| log::warn!("clipboard: send thread: {e}"))
                    .ok();
            }
            ext_data_control_source_v1::Event::Cancelled => {
                // Someone else took the selection. Letting go here is what
                // re-arms the watcher: with no source of our own, the next
                // selection event is somebody else's and gets read.
                let mut own = state.shared.own.lock().unwrap_or_else(|e| e.into_inner());
                if own.as_ref().is_some_and(|s| s == source) {
                    own.take();
                }
                source.destroy();
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_is_one_bounded_line() {
        assert_eq!(preview("  hello  "), "hello");
        assert_eq!(preview("two\nlines"), "two lines");
        assert_eq!(preview("tabs\tand\r\nnewlines"), "tabs and newlines");
        let long = "x".repeat(PREVIEW_LEN + 20);
        let cut = preview(&long);
        assert_eq!(cut.chars().count(), PREVIEW_LEN + 1); // + the ellipsis
        assert!(cut.ends_with('…'));
    }

    #[test]
    fn preview_counts_characters_not_bytes() {
        // Truncating on bytes would split these and produce invalid UTF-8.
        let text = "åäö".repeat(40);
        let cut = preview(&text);
        assert_eq!(cut.chars().count(), PREVIEW_LEN + 1);
    }

    #[test]
    fn text_mimes_are_ordered_utf8_first() {
        // The picker takes the first advertised in this order, so a source
        // offering both must not hand us the unlabelled flavour.
        let offered = ["text/plain", "text/plain;charset=utf-8"];
        let picked = TEXT_MIMES
            .iter()
            .find(|want| offered.iter().any(|m| m == *want));
        assert_eq!(picked, Some(&"text/plain;charset=utf-8"));
    }
}
