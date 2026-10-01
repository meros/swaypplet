//! Bluetooth, followed by BlueZ's own signals, with a pairing agent.
//!
//! The section used to read a snapshot when the panel opened and poll it
//! every 2 s while a scan ran, so headphones that connected on their own
//! showed up only on the next open, and a scan was ten seconds of timers.
//! This holds one system-bus connection on its own thread instead: every
//! `PropertiesChanged`, `InterfacesAdded` and `InterfacesRemoved` BlueZ
//! sends marks the picture dirty, a burst settles for [`SETTLE`], and one
//! `GetManagedObjects` re-reads it. Nothing polls; with the panel closed
//! and no device changing, the thread sleeps.
//!
//! Actions run as their own tasks, so a connect that takes ten seconds to
//! time out holds nothing else up. What each device is doing (connecting,
//! pairing, a code to confirm, the reason it failed) is [`Op`], part of the
//! state the panel draws, so a failure stays on its row until dismissed.
//!
//! # Pairing
//!
//! Connecting a device that was never paired needs an agent: BlueZ asks it
//! to confirm a six-digit code both sides show, or to show a code the user
//! types on a keyboard. [`Agent`] is served at [`AGENT_PATH`] for the life
//! of the connection but registered with BlueZ only for the length of a
//! pair the user started, so it never answers for another program's
//! pairing and never auto-accepts anything outside one.

use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use zbus::export::futures_util::StreamExt;
use zbus::zvariant::{ObjectPath, OwnedObjectPath};
use zbus::{Connection, MatchRule, MessageStream, Proxy};

use super::bluez::{self, Managed, Snapshot};
use crate::service::{Backoff, Observed};

const SERVICE: &str = "org.bluez";
const IFACE_ADAPTER: &str = "org.bluez.Adapter1";
const IFACE_DEVICE: &str = "org.bluez.Device1";
const IFACE_AGENT_MANAGER: &str = "org.bluez.AgentManager1";
const IFACE_OBJECT_MANAGER: &str = "org.freedesktop.DBus.ObjectManager";
const AGENT_PATH: &str = "/dev/swaypplet/bluetooth/agent";

/// How long a burst of BlueZ signals settles before the one re-read.
const SETTLE: Duration = Duration::from_millis(80);

/// What a device is doing, by address.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    Connecting,
    Disconnecting,
    Pairing,
    Forgetting,
    /// Both sides show this code; the user says whether they match.
    Confirm(String),
    /// Type this code on the device (a keyboard), then Enter.
    Show(String),
    /// Why the last action failed, in words for a person.
    Failed(String),
}

impl Op {
    pub fn busy(&self) -> bool {
        matches!(
            self,
            Op::Connecting | Op::Disconnecting | Op::Pairing | Op::Forgetting
        )
    }
}

/// Everything the section draws.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BtState {
    pub snapshot: Snapshot,
    pub ops: BTreeMap<String, Op>,
}

#[derive(Clone, Debug)]
pub enum Command {
    Power(bool),
    Connect(String),
    Disconnect(String),
    /// Pair, trust, then connect: what a click on a nearby device means.
    Pair(String),
    Forget(String),
    /// Scan while the section is on screen; stop when it leaves.
    Discover(bool),
    /// The user's answer to a code to confirm.
    Answer { mac: String, accept: bool },
    /// Clear a failure off its row.
    Dismiss(String),
    /// Set BlueZ's `Alias` for the device; empty gives it back its own name.
    Rename { mac: String, alias: String },
}

pub struct BluetoothService {
    state: Observed<BtState>,
    commands: Option<async_channel::Sender<Command>>,
}

impl BluetoothService {
    pub fn start() -> Rc<Self> {
        let (state_tx, state_rx) = async_channel::unbounded::<BtState>();
        let (cmd_tx, cmd_rx) = async_channel::unbounded::<Command>();
        crate::spawn::spawn_tokio_thread("bluetooth", run(state_tx, cmd_rx));
        let service = Rc::new(BluetoothService {
            state: Observed::new(BtState::default()),
            commands: Some(cmd_tx),
        });
        let for_recv = service.clone();
        glib::spawn_future_local(async move {
            while let Ok(state) = state_rx.recv().await {
                for_recv.state.set_if_changed(state);
            }
        });
        service
    }

    /// A service that never touches BlueZ: `state` is what it reports and
    /// every command is dropped. For the preview.
    pub fn fixture(state: BtState) -> Rc<Self> {
        Rc::new(BluetoothService {
            state: Observed::new(state),
            commands: None,
        })
    }

    pub fn connect_change(&self, cb: impl Fn() + 'static) {
        self.state.connect_change(cb);
    }

    pub fn state(&self) -> BtState {
        self.state.with(Clone::clone)
    }

    pub fn send(&self, command: Command) {
        match &self.commands {
            Some(tx) => {
                if tx.try_send(command).is_err() {
                    log::warn!("bluetooth: command dropped, thread is gone");
                }
            }
            None => log::info!("bluetooth: fixture ignores {command:?}"),
        }
    }
}

// ── The thread ─────────────────────────────────────────────────────────

async fn run(state_tx: async_channel::Sender<BtState>, commands: async_channel::Receiver<Command>) {
    let mut backoff = Backoff::new();
    loop {
        let started = Instant::now();
        match session(&state_tx, &commands).await {
            Ok(()) => return,
            Err(e) => {
                let _ = state_tx.send(BtState::default()).await;
                let delay = backoff.next_delay(started.elapsed());
                log::warn!("bluetooth: {e}; reconnecting in {delay:?}");
                tokio::time::sleep(delay).await;
            }
        }
    }
}

/// What the tasks and the agent share with the loop.
#[derive(Clone)]
struct Shared {
    conn: Connection,
    ops: Arc<Mutex<BTreeMap<String, Op>>>,
    answers: Arc<Mutex<HashMap<String, tokio::sync::oneshot::Sender<bool>>>>,
    /// Ask the loop to send the state again: an op moved.
    push: tokio::sync::mpsc::UnboundedSender<()>,
}

impl Shared {
    fn set(&self, mac: &str, op: Option<Op>) {
        if let Ok(mut ops) = self.ops.lock() {
            match op {
                Some(op) => ops.insert(mac.to_string(), op),
                None => ops.remove(mac),
            };
        }
        let _ = self.push.send(());
    }
}

async fn session(
    state_tx: &async_channel::Sender<BtState>,
    commands: &async_channel::Receiver<Command>,
) -> Result<(), String> {
    let conn = Connection::system()
        .await
        .map_err(|e| format!("system bus: {e}"))?;
    let (push, mut pushed) = tokio::sync::mpsc::unbounded_channel::<()>();
    let shared = Shared {
        conn: conn.clone(),
        ops: Arc::default(),
        answers: Arc::default(),
        push,
    };
    conn.object_server()
        .at(AGENT_PATH, Agent { shared: shared.clone() })
        .await
        .map_err(|e| format!("agent object: {e}"))?;

    let rule = MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(SERVICE)
        .map_err(|e| e.to_string())?
        .build();
    let mut signals = MessageStream::for_match_rule(rule, &conn, Some(256))
        .await
        .map_err(|e| format!("signals: {e}"))?;

    let mut snapshot = fetch(&conn).await;
    emit(state_tx, &snapshot, &shared).await?;

    loop {
        tokio::select! {
            message = signals.next() => {
                if message.is_none() {
                    return Err("bus closed".into());
                }
                // A connect is a dozen property changes; read once, after.
                let settle = tokio::time::sleep(SETTLE);
                tokio::pin!(settle);
                loop {
                    tokio::select! {
                        _ = &mut settle => break,
                        more = signals.next() => if more.is_none() {
                            return Err("bus closed".into());
                        },
                    }
                }
                snapshot = fetch(&conn).await;
                emit(state_tx, &snapshot, &shared).await?;
            }
            command = commands.recv() => {
                let Ok(command) = command else { return Ok(()) };
                handle(command, &shared, &snapshot);
            }
            _ = pushed.recv() => emit(state_tx, &snapshot, &shared).await?,
        }
    }
}

async fn emit(
    tx: &async_channel::Sender<BtState>,
    snapshot: &Snapshot,
    shared: &Shared,
) -> Result<(), String> {
    let ops = shared.ops.lock().map(|o| o.clone()).unwrap_or_default();
    tx.send(BtState {
        snapshot: snapshot.clone(),
        ops,
    })
    .await
    .map_err(|_| "the panel hung up".to_string())
}

async fn fetch(conn: &Connection) -> Snapshot {
    let Ok(proxy) = Proxy::new(conn, SERVICE, "/", IFACE_OBJECT_MANAGER).await else {
        return Snapshot::default();
    };
    match proxy.call::<_, _, Managed>("GetManagedObjects", &()).await {
        Ok(objects) => bluez::parse(&objects),
        Err(e) => {
            log::debug!("bluetooth: GetManagedObjects: {e}");
            Snapshot::default()
        }
    }
}

fn handle(command: Command, shared: &Shared, snapshot: &Snapshot) {
    let path_of = |mac: &str| {
        snapshot
            .devices
            .iter()
            .find(|d| d.mac == mac)
            .map(|d| d.path.clone())
    };
    let adapter = snapshot.adapter.clone();
    let shared = shared.clone();
    match command {
        Command::Answer { mac, accept } => {
            let sender = shared.answers.lock().ok().and_then(|mut a| a.remove(&mac));
            if let Some(sender) = sender {
                let _ = sender.send(accept);
            }
        }
        Command::Dismiss(mac) => {
            let failed = shared
                .ops
                .lock()
                .is_ok_and(|ops| matches!(ops.get(&mac), Some(Op::Failed(_))));
            if failed {
                shared.set(&mac, None);
            }
        }
        Command::Power(on) => {
            let Some(adapter) = adapter else { return };
            tokio::spawn(async move {
                let result = async {
                    Proxy::new(&shared.conn, SERVICE, adapter.as_str(), IFACE_ADAPTER)
                        .await?
                        .set_property("Powered", on)
                        .await
                        .map_err(zbus::Error::from)
                }
                .await;
                if let Err(e) = result {
                    log::warn!("bluetooth: power {on}: {e}");
                }
            });
        }
        Command::Discover(on) => {
            let Some(adapter) = adapter else { return };
            tokio::spawn(async move {
                let method = if on { "StartDiscovery" } else { "StopDiscovery" };
                if let Err(e) = call(&shared.conn, &adapter, IFACE_ADAPTER, method, &()).await {
                    // Already in the state asked for, or the adapter is
                    // off: nothing to tell anyone.
                    log::debug!("bluetooth: {method}: {e}");
                }
            });
        }
        Command::Rename { mac, alias } => {
            let Some(path) = path_of(&mac) else { return };
            tokio::spawn(async move {
                // BlueZ resets the alias to the device's own name when it is
                // set to the empty string.
                let body = (IFACE_DEVICE, "Alias", zbus::zvariant::Value::from(alias.as_str()));
                let result = call(&shared.conn, &path, "org.freedesktop.DBus.Properties", "Set", &body).await;
                if let Err(e) = result {
                    shared.set(&mac, Some(Op::Failed(e)));
                }
            });
        }
        Command::Connect(mac) | Command::Disconnect(mac) | Command::Forget(mac)
            if path_of(&mac).is_none() =>
        {
            shared.set(&mac, Some(Op::Failed("The device is gone.".into())));
        }
        Command::Connect(mac) => {
            let path = path_of(&mac).unwrap_or_default();
            shared.set(&mac, Some(Op::Connecting));
            tokio::spawn(async move {
                let result = call(&shared.conn, &path, IFACE_DEVICE, "Connect", &()).await;
                shared.set(&mac, result.err().map(Op::Failed));
            });
        }
        Command::Disconnect(mac) => {
            let path = path_of(&mac).unwrap_or_default();
            shared.set(&mac, Some(Op::Disconnecting));
            tokio::spawn(async move {
                let result = call(&shared.conn, &path, IFACE_DEVICE, "Disconnect", &()).await;
                shared.set(&mac, result.err().map(Op::Failed));
            });
        }
        Command::Forget(mac) => {
            let (Some(path), Some(adapter)) = (path_of(&mac), adapter) else { return };
            shared.set(&mac, Some(Op::Forgetting));
            tokio::spawn(async move {
                let result = match ObjectPath::try_from(path.as_str()) {
                    Ok(p) => call(&shared.conn, &adapter, IFACE_ADAPTER, "RemoveDevice", &(p,)).await,
                    Err(e) => Err(e.to_string()),
                };
                shared.set(&mac, result.err().map(Op::Failed));
            });
        }
        Command::Pair(mac) => {
            let Some(path) = path_of(&mac) else {
                shared.set(&mac, Some(Op::Failed("The device is gone.".into())));
                return;
            };
            shared.set(&mac, Some(Op::Pairing));
            tokio::spawn(async move {
                let result = pair(&shared, &path).await;
                shared.set(&mac, result.err().map(Op::Failed));
            });
        }
    }
}

/// Register the agent, pair, trust, connect; unregister whatever happened.
async fn pair(shared: &Shared, path: &str) -> Result<(), String> {
    let agent = ObjectPath::try_from(AGENT_PATH).map_err(|e| e.to_string())?;
    call(
        &shared.conn,
        "/org/bluez",
        IFACE_AGENT_MANAGER,
        "RegisterAgent",
        &(&agent, "KeyboardDisplay"),
    )
    .await?;
    let result = async {
        call(&shared.conn, path, IFACE_DEVICE, "Pair", &()).await?;
        // Trusted, so it may reconnect by itself next time, as every
        // desktop's pairing does.
        Proxy::new(&shared.conn, SERVICE, path, IFACE_DEVICE)
            .await
            .map_err(|e| e.to_string())?
            .set_property("Trusted", true)
            .await
            .map_err(|e| e.to_string())?;
        call(&shared.conn, path, IFACE_DEVICE, "Connect", &()).await
    }
    .await;
    let _ = call(
        &shared.conn,
        "/org/bluez",
        IFACE_AGENT_MANAGER,
        "UnregisterAgent",
        &(&agent,),
    )
    .await;
    result
}

async fn call<B>(conn: &Connection, path: &str, iface: &str, method: &str, body: &B) -> Result<(), String>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    let proxy = Proxy::new(conn, SERVICE, path, iface)
        .await
        .map_err(|e| e.to_string())?;
    proxy
        .call::<_, _, ()>(method, body)
        .await
        .map_err(|e| match &e {
            zbus::Error::MethodError(name, detail, _) => {
                reason(name.as_str(), detail.as_deref().unwrap_or_default())
            }
            other => reason("", &other.to_string()),
        })
}

/// A BlueZ error in words for a person. BlueZ's names are precise and its
/// messages are for a log (`br-connection-page-timeout`); the row says what
/// happened and, where there is one, what to do.
pub fn reason(name: &str, message: &str) -> String {
    let short = name.rsplit('.').next().unwrap_or_default();
    let m = message.to_ascii_lowercase();
    let text = if m.contains("page-timeout") || m.contains("host is down") || short == "ConnectionAttemptFailed" && m.contains("timeout") {
        "Not responding. Is it on and in range?"
    } else if m.contains("profile-unavailable") || m.contains("protocol not available") {
        "It offers nothing this computer can use."
    } else if short == "AuthenticationCanceled" || m.contains("canceled") || m.contains("cancelled") {
        "Pairing was cancelled."
    } else if short == "AuthenticationTimeout" {
        "Pairing timed out. Try again with the device in pairing mode."
    } else if short.starts_with("Authentication") || short == "Rejected" {
        "Pairing was declined."
    } else if short == "AlreadyExists" {
        "Already paired."
    } else if short == "InProgress" {
        "Already in progress."
    } else if short == "NotReady" {
        "Bluetooth is not ready yet."
    } else if short == "DoesNotExist" || short == "UnknownObject" {
        "The device is gone."
    } else if m.contains("abort-by-local") {
        "The connection was cancelled."
    } else if short == "ConnectionAttemptFailed" {
        "Could not connect."
    } else if !message.is_empty() {
        return capitalised(message);
    } else {
        "Something went wrong."
    };
    text.to_string()
}

fn capitalised(s: &str) -> String {
    let mut chars = s.trim().chars();
    match chars.next() {
        Some(first) => {
            let rest: String = chars.collect();
            let mut out: String = first.to_uppercase().collect();
            out.push_str(rest.trim_end_matches('.'));
            out.push('.');
            out
        }
        None => String::new(),
    }
}

/// The address in a device's object path (`…/dev_AA_BB_CC_DD_EE_FF`).
pub fn mac_of(path: &str) -> Option<String> {
    let tail = path.rsplit('/').next()?.strip_prefix("dev_")?;
    (tail.len() == 17).then(|| tail.replace('_', ":"))
}

/// A six-digit passkey as it is shown on both sides.
pub fn passkey(code: u32) -> String {
    let s = format!("{code:06}");
    format!("{} {}", &s[..3], &s[3..])
}

// ── The agent ──────────────────────────────────────────────────────────

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.bluez.Error")]
enum AgentError {
    #[zbus(error)]
    ZBus(zbus::Error),
    Rejected(String),
}

struct Agent {
    shared: Shared,
}

impl Agent {
    fn mac(device: &OwnedObjectPath) -> Result<String, AgentError> {
        mac_of(device.as_str()).ok_or_else(|| AgentError::Rejected("unknown device".into()))
    }
}

#[zbus::interface(name = "org.bluez.Agent1")]
impl Agent {
    fn release(&self) {}

    /// Legacy PIN pairing (pre-2.1 devices). Declined: there is no field for
    /// it, and such devices pair with a fixed PIN from their manual.
    fn request_pin_code(&self, _device: OwnedObjectPath) -> Result<String, AgentError> {
        Err(AgentError::Rejected("PIN pairing is not supported".into()))
    }

    fn display_pin_code(&self, device: OwnedObjectPath, pincode: String) -> Result<(), AgentError> {
        self.shared.set(&Self::mac(&device)?, Some(Op::Show(pincode)));
        Ok(())
    }

    fn request_passkey(&self, _device: OwnedObjectPath) -> Result<u32, AgentError> {
        Err(AgentError::Rejected("passkey entry is not supported".into()))
    }

    /// A keyboard: the user types this on it.
    fn display_passkey(&self, device: OwnedObjectPath, passkey: u32, _entered: u16) -> Result<(), AgentError> {
        self.shared.set(&Self::mac(&device)?, Some(Op::Show(self::passkey(passkey))));
        Ok(())
    }

    /// Both sides show a code; the row asks the user whether they match.
    async fn request_confirmation(&self, device: OwnedObjectPath, passkey: u32) -> Result<(), AgentError> {
        let mac = Self::mac(&device)?;
        let (tx, rx) = tokio::sync::oneshot::channel();
        if let Ok(mut answers) = self.shared.answers.lock() {
            answers.insert(mac.clone(), tx);
        }
        self.shared.set(&mac, Some(Op::Confirm(self::passkey(passkey))));
        let accepted = rx.await.unwrap_or(false);
        self.shared.set(&mac, Some(Op::Pairing));
        if accepted {
            Ok(())
        } else {
            Err(AgentError::Rejected("the codes did not match".into()))
        }
    }

    /// "Just works" pairing the user started from the row.
    fn request_authorization(&self, _device: OwnedObjectPath) -> Result<(), AgentError> {
        Ok(())
    }

    /// Only registered during a pair the user started, so a service on that
    /// device is the user's choice.
    fn authorize_service(&self, _device: OwnedObjectPath, _uuid: String) -> Result<(), AgentError> {
        Ok(())
    }

    fn cancel(&self) {
        if let Ok(mut answers) = self.shared.answers.lock() {
            answers.clear();
        }
        let confirming: Vec<String> = self
            .shared
            .ops
            .lock()
            .map(|ops| {
                ops.iter()
                    .filter(|(_, op)| matches!(op, Op::Confirm(_) | Op::Show(_)))
                    .map(|(mac, _)| mac.clone())
                    .collect()
            })
            .unwrap_or_default();
        for mac in confirming {
            self.shared.set(&mac, Some(Op::Pairing));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_read_as_what_happened() {
        assert_eq!(
            reason("org.bluez.Error.Failed", "br-connection-page-timeout"),
            "Not responding. Is it on and in range?"
        );
        assert_eq!(
            reason("org.bluez.Error.Failed", "br-connection-profile-unavailable"),
            "It offers nothing this computer can use."
        );
        assert_eq!(reason("org.bluez.Error.AuthenticationFailed", ""), "Pairing was declined.");
        assert_eq!(reason("org.bluez.Error.AuthenticationCanceled", ""), "Pairing was cancelled.");
        assert_eq!(reason("org.bluez.Error.InProgress", "In Progress"), "Already in progress.");
        assert_eq!(reason("org.bluez.Error.NotReady", "Resource Not Ready"), "Bluetooth is not ready yet.");
        // Anything unknown keeps BlueZ's own words, as a sentence.
        assert_eq!(reason("org.bluez.Error.Failed", "input/output error"), "Input/output error.");
        assert_eq!(reason("", ""), "Something went wrong.");
    }

    #[test]
    fn a_device_path_carries_its_address() {
        assert_eq!(
            mac_of("/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF").as_deref(),
            Some("AA:BB:CC:DD:EE:FF")
        );
        assert_eq!(mac_of("/org/bluez/hci0"), None);
    }

    #[test]
    fn a_passkey_is_six_digits_in_two_groups() {
        assert_eq!(passkey(42), "000 042");
        assert_eq!(passkey(123456), "123 456");
    }
}
