//! The Cast v2 channel: what a sender says to a Cast device.
//!
//! TLS to the device's port (8009), then frames: a 4-byte big-endian length
//! and a protobuf `CastMessage` (source, destination, namespace, a JSON
//! payload). The protobuf has six fields that matter, so it is encoded here
//! by hand ([`CastMessage`]) rather than generated.
//!
//! A conversation with the Default Media Receiver:
//!
//! 1. `CONNECT` to `receiver-0` (namespace `tp.connection`).
//! 2. `LAUNCH` of app `CC1AD845` (namespace `receiver`). The device answers
//!    with `RECEIVER_STATUS` until the app runs; its entry carries the
//!    `sessionId` and the `transportId` the media messages go to.
//! 3. `CONNECT` to the transport, then `LOAD` (namespace `media`) of the
//!    stream's URL as a LIVE stream. `MEDIA_STATUS` means it took it;
//!    `LOAD_FAILED`, `LOAD_CANCELLED` or `INVALID_REQUEST` that it did not.
//! 4. While it plays: `PING` every [`Timing::ping`], `PONG` to the
//!    device's pings. Silence for [`Timing::dead_after`] means the device
//!    is gone. A `RECEIVER_STATUS` without our session means someone else
//!    took the TV (another sender, the remote, a different input).
//! 5. To stop: `STOP` of the session, `CLOSE`, and the socket closes.
//!
//! The device's certificate is not checked. A sender proves a device is
//! genuine through the device-auth namespace and Google's device CA, which
//! this does not do: a machine on the LAN posing as a Cast device would be
//! sent the stream's URL. It is the same trust the discovery list has.
//!
//! Blocking; a stream runs it on its own thread.

use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rustls::pki_types::ServerName;
use rustls::{ClientConnection, StreamOwned};
use serde_json::{Value, json};

pub const NS_CONNECTION: &str = "urn:x-cast:com.google.cast.tp.connection";
pub const NS_HEARTBEAT: &str = "urn:x-cast:com.google.cast.tp.heartbeat";
pub const NS_RECEIVER: &str = "urn:x-cast:com.google.cast.receiver";
pub const NS_MEDIA: &str = "urn:x-cast:com.google.cast.media";
/// The Default Media Receiver: on every Cast device, no registration.
pub const DEFAULT_MEDIA_RECEIVER: &str = "CC1AD845";
/// DashCast: a public receiver that shows a web page in an iframe (or, with
/// `force`, navigates to it). The id is pychromecast's `APP_DASHCAST`, which
/// catt launches; it is madmod's fork of the receiver
/// (madmod.github.io/dashcast/receiver.html).
pub const DASHCAST: &str = "84912283";
/// The namespace madmod's receiver listens on. stestagg's original listens
/// on `urn:x-cast:es.offd.dashcast` under another id; the app's own status
/// names its namespaces, and [`Channel::show_page`] prefers what it says.
pub const NS_DASHCAST: &str = "urn:x-cast:com.madmod.dashcast";
/// DashCast's status text once it takes URLs; before it, it drops them
/// (catt waits for it too).
pub const DASHCAST_READY: &str = "Application ready";
const SENDER: &str = "sender-0";
const RECEIVER: &str = "receiver-0";
/// A frame larger than this is not a Cast message: the device's own limit
/// is 64 KiB.
const MAX_FRAME: usize = 64 * 1024;

/// How long things may take.
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    pub connect: Duration,
    /// For the app to launch and the media to load, each.
    pub reply: Duration,
    pub ping: Duration,
    /// Nothing heard for this long: the device is gone.
    pub dead_after: Duration,
    /// How often a waiting read comes up for air.
    pub tick: Duration,
}

impl Timing {
    /// For a real device: pings every 5 s as Chrome sends them, and three
    /// missed answers before it counts as gone.
    pub const STANDARD: Timing = Timing {
        connect: Duration::from_secs(5),
        reply: Duration::from_secs(20),
        ping: Duration::from_secs(5),
        dead_after: Duration::from_secs(16),
        tick: Duration::from_millis(500),
    };
}

// ── The protobuf ─────────────────────────────────────────────────────────

/// `cast_channel.CastMessage`, with a string payload (the only kind these
/// namespaces use).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CastMessage {
    pub source: String,
    pub destination: String,
    pub namespace: String,
    pub payload: String,
}

fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn put_str(out: &mut Vec<u8>, field: u64, s: &str) {
    put_varint(out, (field << 3) | 2);
    put_varint(out, s.len() as u64);
    out.extend_from_slice(s.as_bytes());
}

fn get_varint(buf: &[u8], at: &mut usize) -> Option<u64> {
    let mut v = 0u64;
    for shift in (0..64).step_by(7) {
        let b = *buf.get(*at)?;
        *at += 1;
        v |= u64::from(b & 0x7f) << shift;
        if b < 0x80 {
            return Some(v);
        }
    }
    None
}

impl CastMessage {
    /// Fields 1 (protocol version 0), 2-4, 5 (payload type STRING) and 6.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(64 + self.payload.len());
        put_varint(&mut out, 1 << 3);
        put_varint(&mut out, 0);
        put_str(&mut out, 2, &self.source);
        put_str(&mut out, 3, &self.destination);
        put_str(&mut out, 4, &self.namespace);
        put_varint(&mut out, 5 << 3);
        put_varint(&mut out, 0);
        put_str(&mut out, 6, &self.payload);
        out
    }

    /// The fields this reads; others (a binary payload, a newer field) are
    /// skipped. `None` for bytes that are not a message.
    pub fn decode(buf: &[u8]) -> Option<CastMessage> {
        let mut m = CastMessage::default();
        let mut at = 0;
        while at < buf.len() {
            let key = get_varint(buf, &mut at)?;
            match key & 7 {
                0 => {
                    get_varint(buf, &mut at)?;
                }
                1 => at = at.checked_add(8)?,
                5 => at = at.checked_add(4)?,
                2 => {
                    let len = get_varint(buf, &mut at)? as usize;
                    let end = at.checked_add(len).filter(|e| *e <= buf.len())?;
                    let text = || String::from_utf8_lossy(&buf[at..end]).into_owned();
                    match key >> 3 {
                        2 => m.source = text(),
                        3 => m.destination = text(),
                        4 => m.namespace = text(),
                        6 => m.payload = text(),
                        _ => {}
                    }
                    at = end;
                }
                _ => return None,
            }
        }
        (at == buf.len()).then_some(m)
    }

    pub fn json(&self) -> Value {
        serde_json::from_str(&self.payload).unwrap_or(Value::Null)
    }

    pub fn kind(&self) -> String {
        self.json()["type"].as_str().unwrap_or_default().to_string()
    }
}

/// A frame: the length, then the message.
pub fn frame(m: &CastMessage) -> Vec<u8> {
    let body = m.encode();
    let mut out = (body.len() as u32).to_be_bytes().to_vec();
    out.extend_from_slice(&body);
    out
}

/// Take one whole frame off the front of `buf`, if one is there. `Err` for
/// a length no Cast message has: the stream is out of step.
pub fn unframe(buf: &mut Vec<u8>) -> Result<Option<CastMessage>, String> {
    if buf.len() < 4 {
        return Ok(None);
    }
    let len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
    if len > MAX_FRAME {
        return Err(format!("a frame of {len} bytes"));
    }
    if buf.len() < 4 + len {
        return Ok(None);
    }
    let m = CastMessage::decode(&buf[4..4 + len]).ok_or("a frame that is not a message")?;
    buf.drain(..4 + len);
    Ok(Some(m))
}

// ── TLS ──────────────────────────────────────────────────────────────────

/// Accepts the device's self-signed certificate (see the module comment);
/// signatures are still checked, so the handshake is a real one.
#[derive(Debug)]
struct AnyCertificate(Arc<rustls::crypto::CryptoProvider>);

impl rustls::client::danger::ServerCertVerifier for AnyCertificate {
    fn verify_server_cert(
        &self,
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &[rustls::pki_types::CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

fn client_config() -> Result<Arc<rustls::ClientConfig>, String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| format!("tls: {e}"))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AnyCertificate(provider)))
        .with_no_client_auth();
    Ok(Arc::new(config))
}

// ── The channel ──────────────────────────────────────────────────────────

/// An open channel to one device.
pub struct Channel {
    tls: StreamOwned<ClientConnection, TcpStream>,
    buf: Vec<u8>,
    next_id: u64,
    heard: Instant,
    pinged: Instant,
    timing: Timing,
}

/// What the device said, as far as a running stream cares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ended {
    /// Nothing heard for `dead_after`, or the connection closed.
    Gone(String),
    /// The device runs something else now: our session is not in its
    /// status, or it closed our transport.
    TakenOver,
    /// The player stopped with an error.
    PlayerError(String),
}

impl Ended {
    pub fn words(&self) -> String {
        match self {
            Ended::Gone(why) => format!("The TV stopped answering ({why})."),
            Ended::TakenOver => "The TV is showing something else now.".into(),
            Ended::PlayerError(why) => format!("The TV could not play the stream ({why})."),
        }
    }
}

/// The app running on the device for us.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launched {
    pub session_id: String,
    pub transport_id: String,
    /// The namespaces the app says it listens on.
    pub namespaces: Vec<String>,
}

fn is_timeout(e: &std::io::Error) -> bool {
    matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut)
}

impl Channel {
    /// Connect, shake hands and `CONNECT` to the platform receiver.
    pub fn open(addr: SocketAddr, timing: Timing) -> Result<Channel, String> {
        let tcp = TcpStream::connect_timeout(&addr, timing.connect)
            .map_err(|e| format!("connect to {addr}: {e}"))?;
        tcp.set_read_timeout(Some(timing.tick)).ok();
        tcp.set_write_timeout(Some(timing.connect)).ok();
        tcp.set_nodelay(true).ok();
        let conn = ClientConnection::new(client_config()?, ServerName::IpAddress(addr.ip().into()))
            .map_err(|e| format!("tls: {e}"))?;
        let mut ch = Channel {
            tls: StreamOwned::new(conn, tcp),
            buf: Vec::new(),
            next_id: 1,
            heard: Instant::now(),
            pinged: Instant::now(),
            timing,
        };
        ch.send(RECEIVER, NS_CONNECTION, json!({"type": "CONNECT"}))?;
        Ok(ch)
    }

    pub fn send(&mut self, to: &str, namespace: &str, payload: Value) -> Result<(), String> {
        let m = CastMessage {
            source: SENDER.into(),
            destination: to.into(),
            namespace: namespace.into(),
            payload: payload.to_string(),
        };
        self.tls
            .write_all(&frame(&m))
            .and_then(|()| self.tls.flush())
            .map_err(|e| format!("send: {e}"))
    }

    fn request_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    /// The next message, or `None` after a tick with nothing. Answers the
    /// device's pings, sends ours, and fails once the device has been quiet
    /// for `dead_after`.
    pub fn recv(&mut self) -> Result<Option<CastMessage>, Ended> {
        loop {
            match unframe(&mut self.buf) {
                Ok(Some(m)) => {
                    self.heard = Instant::now();
                    if m.namespace == NS_HEARTBEAT && m.kind() == "PING" {
                        let to = m.source.clone();
                        self.send(&to, NS_HEARTBEAT, json!({"type": "PONG"}))
                            .map_err(Ended::Gone)?;
                        continue;
                    }
                    return Ok(Some(m));
                }
                Ok(None) => {}
                Err(e) => return Err(Ended::Gone(e)),
            }
            if self.pinged.elapsed() >= self.timing.ping {
                self.pinged = Instant::now();
                self.send(RECEIVER, NS_HEARTBEAT, json!({"type": "PING"}))
                    .map_err(Ended::Gone)?;
            }
            if self.heard.elapsed() >= self.timing.dead_after {
                return Err(Ended::Gone(format!(
                    "nothing for {} s",
                    self.timing.dead_after.as_secs()
                )));
            }
            let mut chunk = [0u8; 8192];
            match self.tls.read(&mut chunk) {
                Ok(0) => return Err(Ended::Gone("it closed the connection".into())),
                Ok(n) => self.buf.extend_from_slice(&chunk[..n]),
                Err(e) if is_timeout(&e) => return Ok(None),
                Err(e) => return Err(Ended::Gone(e.to_string())),
            }
        }
    }

    /// Wait up to `timing.reply` for a message `pick` takes, which returns
    /// what to answer with: `Ok` to finish, `Err` to fail.
    fn wait_for<T>(
        &mut self,
        what: &str,
        mut pick: impl FnMut(&CastMessage) -> Option<Result<T, String>>,
    ) -> Result<T, String> {
        let until = Instant::now() + self.timing.reply;
        while Instant::now() < until {
            match self.recv() {
                Ok(Some(m)) => {
                    if let Some(answer) = pick(&m) {
                        return answer;
                    }
                }
                Ok(None) => {}
                Err(e) => return Err(e.words()),
            }
        }
        Err(format!(
            "no answer to {what} in {} s",
            self.timing.reply.as_secs()
        ))
    }

    /// Launch app `app_id` and connect to it. With `ready`, wait until the
    /// app's status text says so as well (DashCast drops what it is sent
    /// before it is ready).
    pub fn launch(&mut self, app_id: &str, ready: Option<&str>) -> Result<Launched, String> {
        let id = self.request_id();
        self.send(
            RECEIVER,
            NS_RECEIVER,
            json!({"type": "LAUNCH", "appId": app_id, "requestId": id}),
        )?;
        let launched = self.wait_for("LAUNCH", |m| {
            let v = m.json();
            match v["type"].as_str() {
                Some("RECEIVER_STATUS") => our_app(&v, app_id, ready).map(Ok),
                Some("LAUNCH_ERROR") => Some(Err(format!(
                    "the TV would not start app {app_id}: {}",
                    v["reason"].as_str().unwrap_or("no reason")
                ))),
                _ => None,
            }
        })?;
        self.send(
            &launched.transport_id,
            NS_CONNECTION,
            json!({"type": "CONNECT"}),
        )?;
        Ok(launched)
    }

    /// Load `url`, an HLS playlist, as a live stream titled `title`.
    pub fn load(&mut self, app: &Launched, url: &str, title: &str) -> Result<(), String> {
        let id = self.request_id();
        self.send(
            &app.transport_id,
            NS_MEDIA,
            load_request(id, &app.session_id, url, title),
        )?;
        self.wait_for("LOAD", |m| {
            let v = m.json();
            if v["requestId"].as_u64() != Some(id) {
                return None;
            }
            match v["type"].as_str() {
                Some("MEDIA_STATUS") => Some(Ok(())),
                Some(kind @ ("LOAD_FAILED" | "LOAD_CANCELLED" | "INVALID_REQUEST")) => {
                    Some(Err(format!("the TV refused the stream ({kind})")))
                }
                _ => None,
            }
        })
    }

    /// Have DashCast show `url`. `force` navigates the receiver to it
    /// instead of loading it in an iframe. DashCast does not answer; the
    /// page says itself when it plays.
    pub fn show_page(&mut self, app: &Launched, url: &str, force: bool) -> Result<(), String> {
        let ns = app
            .namespaces
            .iter()
            .find(|n| n.contains("dashcast"))
            .map_or(NS_DASHCAST, String::as_str)
            .to_string();
        self.send(&app.transport_id, &ns, show_request(url, force))
    }

    /// Follow the session until it ends or `stop` says to stop. `Ok` when
    /// stopped by us (the app is stopped and the channel closed), `Err`
    /// with why it ended otherwise.
    pub fn follow(&mut self, app: &Launched, stop: &dyn Fn() -> bool) -> Result<(), Ended> {
        loop {
            if stop() {
                self.stop(app);
                return Ok(());
            }
            let Some(m) = self.recv()? else { continue };
            if let Some(end) = ends(&m, app) {
                return Err(end);
            }
        }
    }

    /// Stop our app on the device and close the channel. Best effort: the
    /// device may already be gone.
    pub fn stop(&mut self, app: &Launched) {
        let id = self.request_id();
        let _ = self.send(
            RECEIVER,
            NS_RECEIVER,
            json!({"type": "STOP", "sessionId": app.session_id, "requestId": id}),
        );
        let _ = self.send(&app.transport_id, NS_CONNECTION, json!({"type": "CLOSE"}));
        let _ = self.send(RECEIVER, NS_CONNECTION, json!({"type": "CLOSE"}));
        self.tls.conn.send_close_notify();
        let _ = self.tls.flush();
    }
}

/// Stop session `session_id` on the device at `addr`, if it still runs:
/// what a panel that restarted does about the cast it left.
pub fn stop_remote(addr: SocketAddr, session_id: &str, timing: Timing) -> Result<(), String> {
    let mut ch = Channel::open(addr, timing)?;
    let id = ch.request_id();
    ch.send(
        RECEIVER,
        NS_RECEIVER,
        json!({"type": "STOP", "sessionId": session_id, "requestId": id}),
    )?;
    ch.send(RECEIVER, NS_CONNECTION, json!({"type": "CLOSE"}))
}

/// App `app_id`'s entry in a `RECEIVER_STATUS`, once it has a transport
/// (and, with `ready`, that status text).
fn our_app(status: &Value, app_id: &str, ready: Option<&str>) -> Option<Launched> {
    let a = status["status"]["applications"]
        .as_array()?
        .iter()
        .find(|a| a["appId"] == app_id)?;
    if ready.is_some_and(|r| a["statusText"] != r) {
        return None;
    }
    Some(Launched {
        session_id: a["sessionId"].as_str()?.to_string(),
        transport_id: a["transportId"].as_str()?.to_string(),
        namespaces: a["namespaces"]
            .as_array()
            .map(|n| {
                n.iter()
                    .filter_map(|n| n["name"].as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
    })
}

/// DashCast's message: the page, not reloaded on a timer.
pub fn show_request(url: &str, force: bool) -> Value {
    json!({"url": url, "force": force, "reload": false, "reload_time": 0})
}

/// Whether `m` ends the session `app`.
pub fn ends(m: &CastMessage, app: &Launched) -> Option<Ended> {
    let v = m.json();
    match (m.namespace.as_str(), v["type"].as_str()) {
        (NS_RECEIVER, Some("RECEIVER_STATUS")) => {
            let ours = v["status"]["applications"].as_array().is_some_and(|apps| {
                apps.iter()
                    .any(|a| a["sessionId"] == app.session_id.as_str())
            });
            (!ours).then_some(Ended::TakenOver)
        }
        (NS_CONNECTION, Some("CLOSE")) if m.source == app.transport_id => Some(Ended::TakenOver),
        (NS_MEDIA, Some("MEDIA_STATUS")) => v["status"].as_array()?.iter().find_map(|s| {
            (s["playerState"] == "IDLE" && s["idleReason"] == "ERROR")
                .then(|| Ended::PlayerError("player error".into()))
        }),
        _ => None,
    }
}

/// The `LOAD` for an HLS playlist with MPEG-TS segments, as a live stream.
pub fn load_request(id: u64, session_id: &str, url: &str, title: &str) -> Value {
    json!({
        "type": "LOAD",
        "requestId": id,
        "sessionId": session_id,
        "autoplay": true,
        "media": {
            "contentId": url,
            "contentUrl": url,
            "contentType": "application/x-mpegurl",
            "streamType": "LIVE",
            "metadata": {"metadataType": 0, "title": title},
        },
    })
}

// ── A fake receiver, for tests ───────────────────────────────────────────

/// A Cast device on localhost: TLS with a fresh self-signed certificate,
/// the same framing, and the answers a Default Media Receiver gives.
#[cfg(test)]
pub mod fake {
    use super::*;
    use std::net::TcpListener;
    use std::sync::Mutex;

    /// How the fake behaves after the media is loaded.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum After {
        /// Plays until STOP; answers pings.
        Play,
        /// Drops the connection: the TV was switched off.
        Vanish,
        /// Stays connected but never says anything again.
        Silent,
        /// Another sender takes the TV.
        TakenOver,
        /// Refuses to launch any app.
        Refuse,
        /// Refuses DashCast only, and plays media: a TV without DashCast.
        RefuseDashCast,
    }

    pub struct Fake {
        pub addr: SocketAddr,
        /// Every message the sender sent, in order.
        pub heard: Arc<Mutex<Vec<CastMessage>>>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl Fake {
        /// Start it; `on_load` gets the URL of every LOAD, and of every page
        /// DashCast is sent, as the TV would fetch it.
        pub fn start(after: After, on_load: impl Fn(&str) + Send + 'static) -> Fake {
            Fake::start_for(after, 1, on_load)
        }

        /// [`Fake::start`] for `connections` connections, one after the
        /// other: a sender that falls back opens a second one.
        pub fn start_for(
            after: After,
            connections: usize,
            on_load: impl Fn(&str) + Send + 'static,
        ) -> Fake {
            let ck = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
            let key = rustls::pki_types::PrivateKeyDer::Pkcs8(
                rustls::pki_types::PrivatePkcs8KeyDer::from(ck.key_pair.serialize_der()),
            );
            let config = rustls::ServerConfig::builder_with_provider(Arc::new(
                rustls::crypto::ring::default_provider(),
            ))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![ck.cert.der().clone()], key)
            .unwrap();
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let heard = Arc::new(Mutex::new(Vec::new()));
            let log = heard.clone();
            let config = Arc::new(config);
            let thread = std::thread::spawn(move || {
                for _ in 0..connections {
                    let Ok((tcp, _)) = listener.accept() else {
                        return;
                    };
                    tcp.set_read_timeout(Some(Duration::from_secs(30))).ok();
                    let conn = rustls::ServerConnection::new(config.clone()).unwrap();
                    let mut tls = StreamOwned::new(conn, tcp);
                    serve(&mut tls, after, &log, &on_load);
                }
            });
            Fake {
                addr,
                heard,
                thread: Some(thread),
            }
        }

        /// Wait for the connection to end, then the (namespace, type) of
        /// everything heard on it.
        pub fn finish(self) -> Vec<(String, String)> {
            let heard = self.heard.clone();
            self.join();
            heard
                .lock()
                .unwrap()
                .iter()
                .map(|m| (m.namespace.clone(), m.kind()))
                .collect()
        }

        pub fn join(mut self) {
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }

    fn say(tls: &mut impl Write, from: &str, ns: &str, v: Value) -> bool {
        let m = CastMessage {
            source: from.into(),
            destination: SENDER.into(),
            namespace: ns.into(),
            payload: v.to_string(),
        };
        tls.write_all(&frame(&m)).and_then(|()| tls.flush()).is_ok()
    }

    fn status(apps: Value) -> Value {
        json!({"type": "RECEIVER_STATUS", "requestId": 0, "status": {"applications": apps}})
    }

    fn serve(
        tls: &mut StreamOwned<rustls::ServerConnection, TcpStream>,
        after: After,
        log: &Mutex<Vec<CastMessage>>,
        on_load: &dyn Fn(&str),
    ) {
        let app = |id: &str, text: &str| {
            json!([{
                "appId": id, "sessionId": "session-7", "transportId": "web-7",
                "statusText": text, "namespaces": [{"name": NS_MEDIA}, {"name": NS_DASHCAST}],
            }])
        };
        let mut buf = Vec::new();
        let mut loaded = false;
        loop {
            while let Ok(Some(m)) = unframe(&mut buf) {
                log.lock().unwrap().push(m.clone());
                let v = m.json();
                let ok = match (m.namespace.as_str(), m.kind().as_str()) {
                    (NS_HEARTBEAT, "PING") if !(loaded && after == After::Silent) => {
                        say(tls, &m.destination, NS_HEARTBEAT, json!({"type": "PONG"}))
                    }
                    (NS_RECEIVER, "LAUNCH")
                        if after == After::Refuse
                            || (after == After::RefuseDashCast && v["appId"] == DASHCAST) =>
                    {
                        say(
                            tls,
                            RECEIVER,
                            NS_RECEIVER,
                            json!({"type": "LAUNCH_ERROR", "requestId": v["requestId"], "reason": "NOT_FOUND"}),
                        )
                    }
                    (NS_RECEIVER, "LAUNCH") => {
                        // First "launching" without the app, as devices do;
                        // DashCast then says it is starting before it is
                        // ready.
                        let id = v["appId"].as_str().unwrap_or_default();
                        say(tls, RECEIVER, NS_RECEIVER, status(json!([])))
                            && (id != DASHCAST
                                || say(
                                    tls,
                                    RECEIVER,
                                    NS_RECEIVER,
                                    status(app(id, "Application is starting")),
                                ))
                            && say(tls, RECEIVER, NS_RECEIVER, status(app(id, DASHCAST_READY)))
                    }
                    (NS_MEDIA | NS_DASHCAST, kind) if kind == "LOAD" || v["url"].is_string() => {
                        let url = v["media"]["contentId"].as_str().or(v["url"].as_str());
                        on_load(url.unwrap_or_default());
                        loaded = true;
                        let ok = m.namespace != NS_MEDIA
                            || say(
                                tls,
                                "web-7",
                                NS_MEDIA,
                                json!({"type": "MEDIA_STATUS", "requestId": v["requestId"],
                                       "status": [{"playerState": "BUFFERING"}]}),
                            );
                        match after {
                            After::Vanish => return,
                            After::TakenOver => say(
                                tls,
                                RECEIVER,
                                NS_RECEIVER,
                                status(json!([{
                                    "appId": "OTHER", "sessionId": "theirs", "transportId": "web-9",
                                }])),
                            ),
                            _ => ok,
                        }
                    }
                    (NS_RECEIVER, "STOP") => {
                        say(tls, RECEIVER, NS_RECEIVER, status(json!([])));
                        true
                    }
                    (NS_CONNECTION, "CLOSE") if m.destination == RECEIVER => return,
                    _ => true,
                };
                if !ok {
                    return;
                }
            }
            let mut chunk = [0u8; 8192];
            match tls.read(&mut chunk) {
                Ok(0) | Err(_) => return,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::{After, Fake};
    use super::*;

    fn quick() -> Timing {
        Timing {
            connect: Duration::from_secs(2),
            reply: Duration::from_secs(3),
            ping: Duration::from_millis(200),
            dead_after: Duration::from_millis(900),
            tick: Duration::from_millis(50),
        }
    }

    #[test]
    fn a_message_round_trips_and_matches_protobufs_bytes() {
        let m = CastMessage {
            source: "sender-0".into(),
            destination: "receiver-0".into(),
            namespace: NS_CONNECTION.into(),
            payload: r#"{"type":"CONNECT"}"#.into(),
        };
        let bytes = m.encode();
        // protocol_version = 0, then source_id = "sender-0".
        assert_eq!(&bytes[..4], &[0x08, 0x00, 0x12, 0x08]);
        assert_eq!(&bytes[4..12], b"sender-0");
        assert_eq!(CastMessage::decode(&bytes), Some(m.clone()));
        let mut stream = frame(&m);
        stream.extend_from_slice(&frame(&m)[..5]);
        assert_eq!(unframe(&mut stream).unwrap(), Some(m));
        assert_eq!(unframe(&mut stream).unwrap(), None, "half a frame waits");
        assert_eq!(stream.len(), 5);
    }

    #[test]
    fn unknown_fields_are_skipped_and_garbage_is_refused() {
        let mut bytes = CastMessage {
            payload: "x".repeat(200),
            ..Default::default()
        }
        .encode();
        // A field 7 (binary payload) and a fixed64 field 9.
        bytes.extend_from_slice(&[0x3a, 0x02, 0xff, 0xfe, 0x49, 1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(CastMessage::decode(&bytes).unwrap().payload.len(), 200);
        assert_eq!(CastMessage::decode(&[0x12, 0x09, b'a']), None);
        let mut huge = (MAX_FRAME as u32 + 1).to_be_bytes().to_vec();
        assert!(unframe(&mut huge).is_err());
    }

    #[test]
    fn the_load_is_a_live_hls_stream() {
        let v = load_request(3, "s", "http://10.0.0.2:8891/t/stream.m3u8", "Extra screen");
        assert_eq!(v["media"]["streamType"], "LIVE");
        assert_eq!(v["media"]["contentType"], "application/x-mpegurl");
        assert_eq!(v["sessionId"], "s");
        assert_eq!(v["autoplay"], true);
    }

    #[test]
    fn launch_load_play_and_stop_against_a_fake_tv() {
        let urls = Arc::new(std::sync::Mutex::new(Vec::new()));
        let got = urls.clone();
        let tv = Fake::start(After::Play, move |u| {
            got.lock().unwrap().push(u.to_string())
        });
        let mut ch = Channel::open(tv.addr, quick()).unwrap();
        let app = ch.launch(DEFAULT_MEDIA_RECEIVER, None).unwrap();
        assert_eq!(app.transport_id, "web-7");
        ch.load(&app, "http://127.0.0.1:1/t/stream.m3u8", "Extra screen")
            .unwrap();
        // Play for a few heartbeats, then stop.
        let started = Instant::now();
        let stop = || started.elapsed() > Duration::from_millis(700);
        assert_eq!(ch.follow(&app, &stop), Ok(()));
        tv.join_after_close();
        assert_eq!(*urls.lock().unwrap(), ["http://127.0.0.1:1/t/stream.m3u8"]);
    }

    impl Fake {
        fn join_after_close(self) {
            let kinds = self.finish();
            let has = |ns: &str, k: &str| kinds.iter().any(|(n, t)| n == ns && t == k);
            assert_eq!(kinds[0], (NS_CONNECTION.to_string(), "CONNECT".to_string()));
            assert!(
                has(NS_RECEIVER, "LAUNCH") && has(NS_MEDIA, "LOAD"),
                "{kinds:?}"
            );
            assert!(has(NS_HEARTBEAT, "PING"), "pings while it plays: {kinds:?}");
            assert!(has(NS_RECEIVER, "STOP"), "{kinds:?}");
            assert_eq!(kinds.last().unwrap().1, "CLOSE");
        }
    }

    fn ended_by(after: After) -> Ended {
        let tv = Fake::start(after, |_| {});
        let mut ch = Channel::open(tv.addr, quick()).unwrap();
        let app = ch.launch(DEFAULT_MEDIA_RECEIVER, None).unwrap();
        ch.load(&app, "http://127.0.0.1:1/x", "x").unwrap();
        let end = ch.follow(&app, &|| false).unwrap_err();
        drop(ch);
        tv.join();
        end
    }

    #[test]
    fn a_tv_that_is_switched_off_ends_the_stream() {
        assert!(matches!(ended_by(After::Vanish), Ended::Gone(_)));
    }

    #[test]
    fn a_tv_that_stops_answering_ends_the_stream() {
        let Ended::Gone(why) = ended_by(After::Silent) else {
            panic!("not gone")
        };
        assert!(why.contains("nothing for"), "{why}");
    }

    #[test]
    fn another_sender_taking_the_tv_ends_the_stream() {
        assert_eq!(ended_by(After::TakenOver), Ended::TakenOver);
    }

    #[test]
    fn a_refused_connection_is_an_error_not_a_hang() {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        assert!(Channel::open(port, quick()).is_err());
    }

    #[test]
    fn dashcast_is_launched_once_ready_and_sent_the_page() {
        let urls = Arc::new(std::sync::Mutex::new(Vec::new()));
        let got = urls.clone();
        let tv = Fake::start(After::Play, move |u| {
            got.lock().unwrap().push(u.to_string())
        });
        let mut ch = Channel::open(tv.addr, quick()).unwrap();
        let app = ch.launch(DASHCAST, Some(DASHCAST_READY)).unwrap();
        assert!(
            app.namespaces.iter().any(|n| n == NS_DASHCAST),
            "{:?}",
            app.namespaces
        );
        ch.show_page(&app, "http://127.0.0.1:1/t/player.html", false)
            .unwrap();
        let started = Instant::now();
        assert_eq!(
            ch.follow(&app, &|| started.elapsed() > Duration::from_millis(300)),
            Ok(())
        );
        let heard = tv.heard.clone();
        tv.join();
        let heard = heard.lock().unwrap();
        let page = heard
            .iter()
            .find(|m| m.namespace == NS_DASHCAST)
            .expect("the page was sent");
        assert_eq!(page.destination, "web-7", "to the app, not the platform");
        assert_eq!(
            page.json(),
            show_request("http://127.0.0.1:1/t/player.html", false)
        );
        assert_eq!(page.json()["force"], false);
        assert_eq!(*urls.lock().unwrap(), ["http://127.0.0.1:1/t/player.html"]);
        let launch = heard.iter().find(|m| m.kind() == "LAUNCH").unwrap();
        assert_eq!(launch.json()["appId"], DASHCAST);
    }

    #[test]
    fn a_tv_without_dashcast_says_so_at_launch() {
        let tv = Fake::start(After::Refuse, |_| {});
        let mut ch = Channel::open(tv.addr, quick()).unwrap();
        let err = ch.launch(DASHCAST, Some(DASHCAST_READY)).unwrap_err();
        assert!(err.contains("NOT_FOUND"), "{err}");
        drop(ch);
        tv.join();
    }

    #[test]
    fn a_status_counts_only_for_the_app_asked_for_and_once_ready() {
        let status = json!({"status": {"applications": [{
            "appId": DASHCAST, "sessionId": "s", "transportId": "t",
            "statusText": "Application is starting", "namespaces": [{"name": NS_DASHCAST}],
        }]}});
        assert!(our_app(&status, DASHCAST, Some(DASHCAST_READY)).is_none());
        assert!(our_app(&status, DASHCAST, None).is_some());
        assert!(our_app(&status, DEFAULT_MEDIA_RECEIVER, None).is_none());
    }
}
