//! Human presence sensor — the one walk-away signal this machine reports.
//!
//! The Integrated Sensor Hub publishes a HID sensor with usage 0x200011
//! (human presence) as an IIO device named "prox": `in_proximity0_raw` reads 1
//! while someone is detected and 0 once they leave, `in_attention_input`
//! tracks engagement and sits at 100 while facing the machine. Both sample at
//! 10 Hz (measured 2026-08-12, ThinkPad X9-14 Gen 1).
//!
//! Why this sensor carries the feature: the IR camera (Himax HM1092) has no
//! Linux driver, and the power button never reaches the OS at all — the EC
//! consumes the press and raises no ACPI query — so neither is available as a
//! lock trigger. This one needs no vendor driver.
//!
//! The IIO index moves across boots, so the device is resolved by name.
//!
//! Transitions are reported raw. The sensor does its own filtering in the
//! ISH, so debouncing here was a second layer over one that already exists,
//! and it cost the thing presence is for: a 10 s absence hold meant the screen
//! stayed lit for ten seconds after you left, and a 1 s return hold delayed
//! every wake and every face-unlock attempt.
//!
//! An older note here measured 1 → 0 → 0 (attention 100) → 1 across 8 s for a
//! step away and back, which is what the debounce was built for. If a glance
//! aside starts locking the session, that blip is back and the fix is a hold
//! on the *absence* edge only — never on the return, which must stay
//! immediate.
//!
//! # What a read costs
//!
//! Both attributes are plain sysfs files and neither is cheap: a read is a
//! synchronous round trip to the sensor hub that parks the calling thread in
//! uninterruptible sleep for **250–400 ms** (measured 2026-08-13).
//!
//! # Why one reader
//!
//! Three processes wanted presence and all three read the device themselves:
//! the bar at 1 Hz (twice per tick, for the value and the tooltip), the idle
//! manager at 4 Hz, and the lock screen's face engine at 4 Hz while locked.
//! That is around six reads per second asked of hardware that serves about
//! three, so every reader queued behind the others and each read got slower.
//!
//! The bar's share was the visible damage, because its reads ran on the GTK
//! main thread: the panel's main loop was parked for roughly half of every
//! second, and anything arriving in that window waited on it. A volume key
//! routed through `swaypplet-osd` took 281–491 ms to be answered, uniformly,
//! whatever the key actually did.
//!
//! So the sensor gets one owner. The idle manager takes it: it is always
//! running, and it is the process that acts on presence soonest. Everything
//! else subscribes over the session bus and never touches sysfs.
//!
//! # Why not iio-sensor-proxy
//!
//! It is already running, and owning proximity sensors is its whole job,
//! which would have made it the obvious answer. It cannot serve this one.
//! Its poll driver computes `is_near` as `prox > near_level * 1.1` and
//! rejects a `near_level` of 0 as "unset", so the smallest threshold it will
//! accept is effectively 1.1 — and this sensor reads 0 or 1. It is built for
//! analog proximity sensors reading in the hundreds; a binary presence bit
//! can never cross its threshold. The `PROXIMITY_NEAR_LEVEL=5` udev rule on
//! the nixos side is inert for the same reason.
//!
//! # Cadence
//!
//! The ISH does its own hysteresis, so a poll asks about settled state
//! rather than taking a sample to be filtered. The cadence therefore only
//! has to beat human reaction, and its floor is what the hardware serves:
//! below [`POLL`] a reader spends its time queueing rather than sampling.

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

const IIO_DEVICES: &str = "/sys/bus/iio/devices";
const RAW: &str = "in_proximity0_raw";
const ATTENTION: &str = "in_attention_input";

/// Where the owner publishes, and where everyone else listens.
pub const BUS_NAME: &str = "dev.swaypplet.presence";
pub const OBJECT_PATH: &str = "/dev/swaypplet/Presence";

/// Sampling cadence for the owner. One read costs 250–400 ms, so anything
/// shorter than this spends its time queueing.
pub const POLL: Duration = Duration::from_millis(500);

/// Attention is a tooltip readout, not a trigger, and it costs a second full
/// read. It rides every Nth cycle so the owner stays inside its [`POLL`]
/// budget.
const ATTENTION_EVERY: u32 = 4;

/// Cadence for a subscriber with no owner to listen to. Slower than [`POLL`]
/// deliberately: that path exists so a lone process still works, not so it
/// can compete for the device.
const FALLBACK_POLL: Duration = Duration::from_millis(1000);

// ── The device ──────────────────────────────────────────────────────────

/// The sensor itself. Only the owner should hold one; everything else wants
/// [`subscribe`].
pub struct Presence {
    dir: PathBuf,
    /// Last reported state; seeded by the first reading without reporting it.
    state: Option<bool>,
}

impl Presence {
    /// Resolve the sensor by IIO name. `None` on a machine without one, which
    /// leaves every caller on its pre-presence behaviour.
    pub fn detect() -> Option<Self> {
        let entries = match fs::read_dir(IIO_DEVICES) {
            Ok(entries) => entries,
            Err(e) => {
                log::info!("presence: no IIO devices ({e}) — presence rules off");
                return None;
            }
        };
        for entry in entries.flatten() {
            let dir = entry.path();
            let Ok(name) = fs::read_to_string(dir.join("name")) else {
                continue;
            };
            if name.trim() != "prox" || !dir.join(RAW).exists() {
                continue;
            }
            log::info!("presence: sensor at {}", dir.display());
            return Some(Self { dir, state: None });
        }
        log::info!("presence: no IIO device named \"prox\" — presence rules off");
        None
    }

    fn read_i32(&self, file: &str) -> Option<i32> {
        fs::read_to_string(self.dir.join(file))
            .ok()?
            .trim()
            .parse()
            .ok()
    }

    /// Instantaneous reading. Blocks for 250–400 ms; never call it from a
    /// thread that draws anything.
    pub fn read(&self) -> Option<bool> {
        Some(self.read_i32(RAW)? != 0)
    }

    /// Engagement, 0-100, unsmoothed and undebounced. Display only, and as
    /// expensive as [`Self::read`].
    pub fn attention(&self) -> Option<i32> {
        self.read_i32(ATTENTION)
    }

    /// Read once. Returns `Some(new_state)` only on a settled transition, so
    /// callers can drive this from an existing tick without tracking edges.
    /// Report a change in presence, or `None` when nothing changed.
    ///
    /// Edge-triggered: every caller acts on transitions, and the first reading
    /// is the starting state rather than a transition — reporting it would
    /// fire "user back" at every service start, which on the lock screen would
    /// arm a face attempt against whoever locked the machine a moment ago.
    pub fn poll(&mut self) -> Option<bool> {
        let present = self.read_i32(RAW)? != 0;

        if self.state.is_none() {
            self.state = Some(present);
            return None;
        }

        if Some(present) == self.state {
            return None;
        }

        self.state = Some(present);
        Some(present)
    }
}

// ── What subscribers receive ────────────────────────────────────────────

/// A settled change.
///
/// Presence is sent on transitions only, matching [`Presence::poll`], so a
/// subscriber handles it the same whether it arrived over the bus or off the
/// fallback path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Changed(bool),
    /// Engagement. Display only; nothing acts on it.
    Attention(i32),
}

// ── The owner ───────────────────────────────────────────────────────────

/// Take the sensor and publish it on the session bus.
///
/// `None` on hardware without one, which leaves the caller on its
/// timer-only behaviour. Events also come back on the returned channel, so
/// the owner does not have to listen to itself over D-Bus to learn what it
/// just read.
///
/// The channel is an `async_channel` because the three consumers are three
/// different runtimes: the idle manager and the face engine take it with
/// `recv_blocking`/`try_recv` from a plain loop, the bar awaits it on the
/// GTK main context. One transport, no adapters.
pub fn serve() -> Option<async_channel::Receiver<Event>> {
    let sensor = Presence::detect()?;
    let (tx, rx) = async_channel::unbounded();

    crate::spawn::spawn_tokio_thread("presence", async move {
        // Own it, or fall in behind whoever already does, or read it here.
        // A second idle manager losing the name race must not leave this one
        // blind to presence for the rest of the session — that would stop
        // absence locking with nothing but a log line to say so.
        if let Err(e) = own(sensor, &tx).await {
            log::warn!("presence: could not own the sensor ({e}); following instead");
            if let Err(e) = follow(&tx).await {
                log::info!("presence: no owner to follow either ({e}); reading the device here");
                fallback(&tx);
            }
        }
    });
    Some(rx)
}

/// The published state. zbus emits `PropertiesChanged` when the generated
/// `*_changed` methods are called.
struct Published {
    present: bool,
    attention: i32,
}

#[zbus::interface(name = "dev.swaypplet.Presence")]
impl Published {
    #[zbus(property)]
    async fn present(&self) -> bool {
        self.present
    }

    #[zbus(property)]
    async fn attention(&self) -> i32 {
        self.attention
    }
}

async fn own(mut sensor: Presence, events: &async_channel::Sender<Event>) -> zbus::Result<()> {
    // Seed from the device before claiming the name, so the first thing a
    // subscriber reads is the real state rather than a default.
    let present = sensor.read().unwrap_or(false);
    sensor.state = Some(present);
    let attention = sensor.attention().unwrap_or(0);

    let conn = zbus::connection::Builder::session()?
        .name(BUS_NAME)?
        .serve_at(OBJECT_PATH, Published { present, attention })?
        .build()
        .await?;
    log::info!("presence: owning the sensor, published on {BUS_NAME}");

    let iface = conn
        .object_server()
        .interface::<_, Published>(OBJECT_PATH)
        .await?;

    let mut tick: u32 = 0;
    loop {
        tokio::time::sleep(POLL).await;
        tick = tick.wrapping_add(1);

        if let Some(present) = sensor.poll() {
            let mut published = iface.get_mut().await;
            published.present = present;
            published.present_changed(iface.signal_context()).await?;
            drop(published);
            if events.send_blocking(Event::Changed(present)).is_err() {
                return Ok(()); // the consumer is gone, and so is the process
            }
        }

        if tick.is_multiple_of(ATTENTION_EVERY)
            && let Some(attention) = sensor.attention()
        {
            let mut published = iface.get_mut().await;
            if published.attention != attention {
                published.attention = attention;
                published.attention_changed(iface.signal_context()).await?;
                drop(published);
                if events.send_blocking(Event::Attention(attention)).is_err() {
                    return Ok(());
                }
            }
        }
    }
}

// ── Subscribers ─────────────────────────────────────────────────────────

#[zbus::proxy(
    interface = "dev.swaypplet.Presence",
    default_service = "dev.swaypplet.presence",
    default_path = "/dev/swaypplet/Presence"
)]
trait PresenceOwner {
    #[zbus(property)]
    fn present(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn attention(&self) -> zbus::Result<i32>;
}

/// Listen to whoever owns the sensor.
///
/// Falls back to reading the device here when no owner answers, which is the
/// greeter's situation: it runs as a different user with no idle manager
/// behind it. The fallback still keeps the reads on this thread rather than
/// the caller's, so a UI thread is never parked either way.
pub fn subscribe() -> async_channel::Receiver<Event> {
    let (tx, rx) = async_channel::unbounded();
    crate::spawn::spawn_tokio_thread("presence-sub", async move {
        if let Err(e) = follow(&tx).await {
            log::info!("presence: no owner on the bus ({e}); reading the device here");
            fallback(&tx);
        }
    });
    rx
}

async fn follow(events: &async_channel::Sender<Event>) -> zbus::Result<()> {
    use zbus::export::futures_util::StreamExt;

    let conn = zbus::Connection::session().await?;
    let proxy = PresenceOwnerProxy::new(&conn).await?;

    // Reading a property fails when nobody owns the name, which is what
    // sends us to the fallback rather than waiting on an owner that is
    // never going to arrive.
    let mut present = proxy.present().await?;
    let _ = events.send_blocking(Event::Changed(present));
    if let Ok(attention) = proxy.attention().await {
        let _ = events.send_blocking(Event::Attention(attention));
    }

    let mut presents = proxy.receive_present_changed().await;
    let mut attentions = proxy.receive_attention_changed().await;

    loop {
        tokio::select! {
            Some(change) = presents.next() => {
                let Ok(next) = change.get().await else { continue };
                if next == present {
                    continue;
                }
                present = next;
                if events.send_blocking(Event::Changed(present)).is_err() {
                    return Ok(());
                }
            }
            Some(change) = attentions.next() => {
                let Ok(attention) = change.get().await else { continue };
                if events.send_blocking(Event::Attention(attention)).is_err() {
                    return Ok(());
                }
            }
            // Both streams ended: the owner dropped off the bus. That is a
            // failure to be recovered from by falling back, not a finish.
            else => return Err(zbus::Error::Failure("presence owner went away".into())),
        }
    }
}

/// Read the device here, for want of an owner. Same edge semantics as the
/// bus path, so a caller cannot tell which one it got.
fn fallback(events: &async_channel::Sender<Event>) {
    let Some(mut sensor) = Presence::detect() else {
        return;
    };

    if let Some(present) = sensor.read() {
        sensor.state = Some(present);
        if events.send_blocking(Event::Changed(present)).is_err() {
            return;
        }
    }

    let mut tick: u32 = 0;
    loop {
        std::thread::sleep(FALLBACK_POLL);
        tick = tick.wrapping_add(1);

        if let Some(present) = sensor.poll()
            && events.send_blocking(Event::Changed(present)).is_err()
        {
            return;
        }
        if tick.is_multiple_of(ATTENTION_EVERY)
            && let Some(attention) = sensor.attention()
            && events.send_blocking(Event::Attention(attention)).is_err()
        {
            return;
        }
    }
}

#[cfg(test)]
mod live {
    use super::*;
    use std::time::Instant;

    /// One reader, and everyone else hears it. Ignored: needs a session bus
    /// and the sensor.
    ///
    /// Tolerant about who owns the device: if the idle manager is already
    /// running it holds the name and this only exercises the subscriber,
    /// which is the interesting half either way.
    #[test]
    #[ignore]
    fn a_subscriber_hears_the_owner() {
        let owned = serve().is_some();
        println!("claimed the sensor here: {owned}");

        let events = subscribe();
        let started = Instant::now();
        let deadline = Duration::from_secs(5);

        let mut present = None;
        while started.elapsed() < deadline {
            match events.try_recv() {
                Ok(Event::Changed(state)) => {
                    present = Some(state);
                    break;
                }
                Ok(Event::Attention(_)) => {}
                Err(_) => std::thread::sleep(Duration::from_millis(20)),
            }
        }

        let present = present.expect("no opening state in 5s");
        println!("opening state: present={present} after {:?}", started.elapsed());
    }

    /// The device is only ever touched by the owner. Ignored: needs the
    /// sensor.
    ///
    /// This is the property the whole module exists for, so it is worth
    /// stating as a test rather than a comment: a read costs 250-400 ms, and
    /// the regression was three processes each paying it.
    #[test]
    #[ignore]
    fn a_read_is_as_slow_as_the_comments_claim() {
        let Some(sensor) = Presence::detect() else {
            println!("no sensor here; nothing to measure");
            return;
        };
        let started = Instant::now();
        let value = sensor.read();
        let elapsed = started.elapsed();
        println!("one read: {value:?} in {elapsed:?}");
        assert!(
            elapsed > Duration::from_millis(50),
            "a read got cheap ({elapsed:?}); the one-owner rule may be worth revisiting"
        );
    }
}
