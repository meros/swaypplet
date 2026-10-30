//! What sends the extra screen somewhere: one [`Backend`] per way of
//! getting pixels from a sway output to a screen across the room.
//!
//! # Built
//!
//! A Cast device with a screen gets [`Cast`], which tries two ways in turn:
//!
//! 1. [`LivePage`], under a second behind. The output becomes one endless
//!    fragmented MP4 (`live`), served with a page that plays it through
//!    Media Source Extensions (`player.html`, `http`). DashCast, a public
//!    receiver app that shows a web page (no registration), is launched and
//!    sent the page's URL (`channel::Channel::show_page`). Measured on the
//!    owner's TV with a prototype of this page: 0.42 s median, glass to
//!    glass, against a synthetic source.
//! 2. [`MediaReceiver`], a few seconds behind, when the first does not
//!    play. The output is encoded to HLS (`hls`), served over HTTP, and the
//!    Default Media Receiver (on every Cast device) is told to play it.
//!
//! The fallback happens when DashCast does not launch (`LAUNCH_ERROR`, or
//! no answer in [`LIVE_TIMING`]'s reply time), when it closes, or when the
//! page has not said "playing" within [`PLAYING_WITHIN`]. The DashCast
//! session is stopped before the Default Media Receiver is launched. A TV
//! that cannot be reached, or that goes away while the page loads, is an
//! error, not a fallback: the second way would fail the same.
//!
//! DashCast is sent `force: true`, as `catt cast_site` does: DashCast
//! navigates the receiver itself to the page. With `force: false` DashCast
//! (an https page) loads ours (plain http) in an iframe, which the browser
//! blocks as mixed content: the TV sat on "loading http..." and never asked
//! for the page (2026-10-23, Vardagsrummet TV). The prototype that worked
//! was cast by `catt`, so it had used `force: true` all along.
//! [`LivePage::force`] is the switch.
//!
//! [`Vnc`] is for a VNC viewer: wayvnc on the output, view only, on
//! localhost.
//!
//! # Not built
//!
//! **Cast mirroring** (the receiver Chrome's "Cast screen" uses). Its
//! sender side is Cast Streaming: an OFFER/ANSWER exchange on the Cast
//! channel, then AES-encrypted RTP over UDP with RTCP feedback (Open
//! Screen's `cast/streaming`), with no Rust implementation to build on. The
//! live page gets most of its latency advantage without it.
//!
//! # What a stream leaves behind
//!
//! Nothing, when stopped: the TV's app is stopped, the encoder ends, the
//! port closes, the segments (or the live stream's log) are deleted, and
//! `mod.rs` removes the output.
//! When the panel dies instead, every child process gets SIGTERM
//! (`children`), and the next panel stops the TV session it finds in the
//! runtime directory ([`sweep`]) and removes the output (`output::sweep`).

use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::channel::{self, Channel, Launched, Timing};
use super::discovery::Device;
use super::{children, hls, http, live};

/// Where a stream goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A Cast device on the network.
    Device(Device),
    /// Whatever viewer connects: a VNC client.
    Viewer,
}

/// A way to send an output somewhere.
pub trait Backend: Sync {
    /// What it is called in the section ("Cast", "VNC").
    fn name(&self) -> &'static str;
    /// Whether it can send to `target` at all.
    fn reaches(&self, target: &Target) -> bool;
    /// Start sending `output` to `target`. Blocking, for a worker: a cast
    /// waits for the first segments and for the TV to load them.
    fn start(&self, output: &str, target: &Target) -> Result<Box<dyn Stream>, String>;
}

/// A stream that is running.
pub trait Stream: Send {
    /// Where to look for it, in words ("localhost:5900, view only").
    fn describe(&self) -> String;
    /// Whether it still runs; false once whatever sends it has ended.
    fn running(&mut self) -> bool;
    /// Why it ended by itself, in words, once [`Stream::running`] is false.
    fn why_ended(&mut self) -> Option<String> {
        None
    }
    /// End it. Idempotent. May block for a few seconds (a cast tells the
    /// TV); call it from a worker.
    fn stop(&mut self);
}

/// DashCast answers a LAUNCH at once on a TV that has it; one that says
/// nothing for this long is treated as not having it.
pub const LIVE_TIMING: Timing = Timing {
    reply: Duration::from_secs(10),
    ..Timing::STANDARD
};
/// From sending DashCast the page to the page saying it plays: loading the
/// page in DashCast's iframe, the first fragments, the decoder.
pub const PLAYING_WITHIN: Duration = Duration::from_secs(30);

static CAST: Cast = Cast {
    live: LivePage {
        port: http::PORT,
        timing: LIVE_TIMING,
        playing_within: PLAYING_WITHIN,
        force: true,
    },
    hls: MediaReceiver {
        port: http::PORT,
        timing: Timing::STANDARD,
    },
};

/// Every backend built, in the order they are preferred.
static BACKENDS: &[&dyn Backend] = &[&CAST, &Vnc];

/// The backend that sends to `target`, when one is built.
pub fn for_target(target: &Target) -> Option<&'static dyn Backend> {
    BACKENDS.iter().copied().find(|b| b.reaches(target))
}

fn runtime_dir() -> PathBuf {
    swaypplet_base::paths::runtime_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("swaypplet")
}

// ── Cast: the Default Media Receiver ─────────────────────────────────────

/// A Cast device, sent an HLS stream of the output.
pub struct MediaReceiver {
    /// Where the TV fetches the stream; 0 for any free port (tests).
    pub port: u16,
    pub timing: Timing,
}

/// Segments the playlist names before the TV is told to load it.
const READY_SEGMENTS: usize = 3;
const READY_WITHIN: Duration = Duration::from_secs(12);

/// The address of this machine that `to` is reached from: what the TV
/// fetches the stream from. Asks the routing table; sends nothing.
pub fn address_toward(to: IpAddr) -> Result<IpAddr, String> {
    let any: IpAddr = match to {
        IpAddr::V4(_) => [0, 0, 0, 0].into(),
        IpAddr::V6(_) => std::net::Ipv6Addr::UNSPECIFIED.into(),
    };
    let s = UdpSocket::bind((any, 0)).map_err(|e| e.to_string())?;
    s.connect((to, 9))
        .map_err(|e| format!("no route to {to}: {e}"))?;
    Ok(s.local_addr().map_err(|e| e.to_string())?.ip())
}

/// 128 random bits, as hex: the stream's path.
fn token() -> Result<String, String> {
    use std::io::Read;
    let mut b = [0u8; 16];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut b))
        .map_err(|e| format!("random: {e}"))?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

impl Backend for MediaReceiver {
    fn name(&self) -> &'static str {
        "Cast"
    }

    fn reaches(&self, target: &Target) -> bool {
        matches!(target, Target::Device(d) if d.video && d.address.parse::<IpAddr>().is_ok())
    }

    fn start(&self, output: &str, target: &Target) -> Result<Box<dyn Stream>, String> {
        let Target::Device(device) = target else {
            return Err("not a Cast device".into());
        };
        let ip: IpAddr = device
            .address
            .parse()
            .map_err(|_| format!("{}: not an address", device.address))?;
        let tv = SocketAddr::new(ip, device.port);
        let here = address_toward(ip)?;

        let dir = runtime_dir().join(format!("cast-hls-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let encoder = hls::Encoder::start(output, dir.clone(), READY_SEGMENTS, READY_WITHIN)?;
        let token = token()?;
        // Dropping what was started undoes it, so `?` from here on is a
        // clean failure: the encoder ends, the port closes.
        let server = http::serve(http::Site::Files(dir), token.clone(), self.port, Some(ip))?;
        let url = format!("http://{here}:{}/{token}/{}", server.port, hls::PLAYLIST);

        let mut ch = Channel::open(tv, self.timing)?;
        let app = ch.launch(channel::DEFAULT_MEDIA_RECEIVER, None)?;
        if let Err(e) = ch.load(&app, &url, "Extra screen") {
            ch.stop(&app);
            return Err(e);
        }
        log::info!("cast: {} plays {url}", device.name);
        cast_stream(
            format!("{}, a few seconds behind", device.name),
            Box::new(encoder),
            server,
            ch,
            app,
            tv,
        )
    }
}

/// What feeds a cast: the HLS segmenter or the live MP4.
trait Encoding: Send {
    fn running(&mut self) -> bool;
    fn log_tail(&self) -> String;
    fn stop(&mut self);
}

impl Encoding for hls::Encoder {
    fn running(&mut self) -> bool {
        hls::Encoder::running(self)
    }
    fn log_tail(&self) -> String {
        hls::Encoder::log_tail(self)
    }
    fn stop(&mut self) {
        hls::Encoder::stop(self)
    }
}

impl Encoding for live::Encoder {
    fn running(&mut self) -> bool {
        live::Encoder::running(self)
    }
    fn log_tail(&self) -> String {
        live::Encoder::log_tail(self)
    }
    fn stop(&mut self) {
        live::Encoder::stop(self)
    }
}

/// The running half of a cast, once the TV plays: the session recorded for
/// [`sweep`], and a thread that follows the TV until it ends or is stopped.
fn cast_stream(
    what: String,
    encoder: Box<dyn Encoding>,
    server: http::Server,
    mut ch: Channel,
    app: Launched,
    tv: SocketAddr,
) -> Result<Box<dyn Stream>, String> {
    record(tv, &app.session_id);
    let stop = Arc::new(AtomicBool::new(false));
    let ended = Arc::new(Mutex::new(None));
    let follow = {
        let (stop, ended) = (stop.clone(), ended.clone());
        std::thread::Builder::new()
            .name("swaypplet-cast".into())
            .spawn(move || {
                if let Err(end) = ch.follow(&app, &|| stop.load(Ordering::Relaxed)) {
                    log::info!("cast: ended: {end:?}");
                    *ended.lock().unwrap_or_else(|e| e.into_inner()) = Some(end.words());
                }
                forget();
            })
            .map_err(|e| format!("cast thread: {e}"))?
    };
    Ok(Box::new(CastStream {
        what,
        encoder: Some(encoder),
        server: Some(server),
        follow: Some(follow),
        stop,
        ended,
    }))
}

// ── Cast: the live page in DashCast ──────────────────────────────────────

/// A Cast device, shown a page that plays the live MP4 (module comment).
pub struct LivePage {
    pub port: u16,
    pub timing: Timing,
    pub playing_within: Duration,
    /// Navigate DashCast to the page instead of framing it.
    pub force: bool,
}

/// Why the live page did not start.
#[derive(Debug, PartialEq, Eq)]
pub enum Failure {
    /// This way did not work; the HLS one may.
    Fallback(String),
    /// The TV cannot be reached; nothing will work.
    Fatal(String),
}

impl LivePage {
    pub fn try_start(&self, output: &str, device: &Device) -> Result<Box<dyn Stream>, Failure> {
        use Failure::{Fallback, Fatal};
        let ip: IpAddr = device
            .address
            .parse()
            .map_err(|_| Fatal(format!("{}: not an address", device.address)))?;
        let tv = SocketAddr::new(ip, device.port);
        let here = address_toward(ip).map_err(Fatal)?;
        let log = runtime_dir().join(format!("cast-live-{}.log", std::process::id()));
        let encoder =
            live::Encoder::start(output, log, Duration::from_secs(8)).map_err(Fallback)?;
        let token = token().map_err(Fatal)?;
        // As in MediaReceiver: dropping what was started undoes it.
        let server = http::serve(
            http::Site::Live(encoder.hub.clone()),
            token.clone(),
            self.port,
            Some(ip),
        )
        .map_err(Fatal)?;
        let url = format!("http://{here}:{}/{token}/player.html", server.port);

        let mut ch = Channel::open(tv, self.timing).map_err(Fatal)?;
        let app = ch
            .launch(channel::DASHCAST, Some(channel::DASHCAST_READY))
            .map_err(Fallback)?;
        let shown = ch.show_page(&app, &url, self.force).map_err(Fallback);
        let playing = shown.and_then(|()| self.wait_playing(&mut ch, &app, &encoder.hub));
        if let Err(e) = playing {
            ch.stop(&app);
            return Err(e);
        }
        log::info!("cast: {} plays {url}", device.name);
        cast_stream(
            format!("{}, under a second behind", device.name),
            Box::new(encoder),
            server,
            ch,
            app,
            tv,
        )
        .map_err(Fatal)
    }

    /// Wait for the page to say it plays, keeping the channel alive.
    fn wait_playing(
        &self,
        ch: &mut Channel,
        app: &Launched,
        hub: &live::Hub,
    ) -> Result<(), Failure> {
        let until = std::time::Instant::now() + self.playing_within;
        while !hub.playing() {
            if std::time::Instant::now() >= until {
                return Err(Failure::Fallback(format!(
                    "the page did not play within {} s",
                    self.playing_within.as_secs()
                )));
            }
            match ch.recv() {
                Ok(Some(m)) => {
                    if let Some(end) = channel::ends(&m, app) {
                        return Err(Failure::Fallback(format!(
                            "DashCast ended: {}",
                            end.words()
                        )));
                    }
                }
                Ok(None) => {}
                Err(e) => return Err(Failure::Fatal(e.words())),
            }
        }
        Ok(())
    }
}

/// A Cast device: the live page, else HLS (module comment).
pub struct Cast {
    pub live: LivePage,
    pub hls: MediaReceiver,
}

impl Backend for Cast {
    fn name(&self) -> &'static str {
        "Cast"
    }

    fn reaches(&self, target: &Target) -> bool {
        self.hls.reaches(target)
    }

    fn start(&self, output: &str, target: &Target) -> Result<Box<dyn Stream>, String> {
        let Target::Device(device) = target else {
            return Err("not a Cast device".into());
        };
        match self.live.try_start(output, device) {
            Ok(s) => Ok(s),
            Err(Failure::Fatal(e)) => Err(e),
            Err(Failure::Fallback(why)) => {
                log::info!("cast: no live page on {} ({why}); HLS instead", device.name);
                self.hls
                    .start(output, target)
                    .map_err(|e| format!("{e} (the low-lag way failed first: {why})"))
            }
        }
    }
}

struct CastStream {
    what: String,
    encoder: Option<Box<dyn Encoding>>,
    server: Option<http::Server>,
    follow: Option<std::thread::JoinHandle<()>>,
    stop: Arc<AtomicBool>,
    ended: Arc<Mutex<Option<String>>>,
}

impl Stream for CastStream {
    fn describe(&self) -> String {
        self.what.clone()
    }

    fn running(&mut self) -> bool {
        self.encoder.as_mut().is_some_and(|e| e.running())
            && self.server.as_ref().is_some_and(http::Server::running)
            && self.follow.as_ref().is_some_and(|t| !t.is_finished())
    }

    fn why_ended(&mut self) -> Option<String> {
        if let Some(why) = self.ended.lock().ok().and_then(|e| e.clone()) {
            return Some(why);
        }
        let enc = self.encoder.as_mut()?;
        (!enc.running()).then(|| format!("The encoder stopped: {}", enc.log_tail()))
    }

    fn stop(&mut self) {
        // The TV first, while the stream it plays is still there.
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.follow.take() {
            let _ = t.join();
        }
        if let Some(mut s) = self.server.take() {
            s.stop();
        }
        if let Some(mut e) = self.encoder.take() {
            e.stop();
        }
    }
}

impl Drop for CastStream {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The TV session a running cast holds, for [`sweep`].
fn record_path() -> PathBuf {
    runtime_dir().join("cast-session")
}

fn record(tv: SocketAddr, session: &str) {
    let _ = std::fs::create_dir_all(runtime_dir());
    let _ = std::fs::write(record_path(), format!("{tv}\n{session}\n"));
}

fn forget() {
    let _ = std::fs::remove_file(record_path());
}

/// A record's address and session.
fn parse_record(text: &str) -> Option<(SocketAddr, String)> {
    let mut lines = text.lines().map(str::trim);
    let addr = lines.next()?.parse().ok()?;
    let session = lines.next().filter(|s| !s.is_empty())?.to_string();
    Some((addr, session))
}

/// Whether `name` is something another process left: a panel that died
/// mid-cast. Ours are `cast-hls-<our pid>` and `cast-live-<our pid>.log`.
fn stale_segments(name: &str, ours: u32) -> bool {
    name.strip_prefix("cast-hls-")
        .or_else(|| name.strip_prefix("cast-live-")?.strip_suffix(".log"))
        .and_then(|pid| pid.parse::<u32>().ok())
        .is_some_and(|pid| pid != ours)
}

/// Clean up after a panel that died mid-cast: its segments, and the TV
/// session it left playing. Blocking.
pub fn sweep() {
    if let Ok(entries) = std::fs::read_dir(runtime_dir()) {
        for e in entries.flatten() {
            if stale_segments(&e.file_name().to_string_lossy(), std::process::id()) {
                let path = e.path();
                let _ = std::fs::remove_dir_all(&path).or_else(|_| std::fs::remove_file(&path));
            }
        }
    }
    let Some((tv, session)) = std::fs::read_to_string(record_path())
        .ok()
        .as_deref()
        .and_then(parse_record)
    else {
        return;
    };
    forget();
    let quick = Timing {
        reply: Duration::from_secs(3),
        ..Timing::STANDARD
    };
    match channel::stop_remote(tv, &session, quick) {
        Ok(()) => log::info!("cast: stopped the session on {tv} the panel before left"),
        Err(e) => log::info!("cast: the session on {tv} is gone already ({e})"),
    }
}

// ── VNC ──────────────────────────────────────────────────────────────────

/// wayvnc on the extra screen.
///
/// View only (`--disable-input`): the extra screen is part of this seat,
/// and a viewer that could type would type into this session. Bound to
/// localhost, because wayvnc without its TLS and password config lets
/// anyone in who can reach the port; another machine comes in over SSH or
/// tailscale.
pub struct Vnc;

impl Vnc {
    const ADDRESS: &'static str = "127.0.0.1";
    const PORT: u16 = 5900;

    /// The arguments for `output`. Its own control socket, so a wayvnc the
    /// person runs for something else keeps the default one.
    fn args(output: &str, control: Option<&str>) -> Vec<String> {
        let mut args = vec!["--disable-input".to_string(), format!("--output={output}")];
        if let Some(c) = control {
            args.push(format!("--socket={c}"));
        }
        args.push(Self::ADDRESS.to_string());
        args.push(Self::PORT.to_string());
        args
    }
}

impl Backend for Vnc {
    fn name(&self) -> &'static str {
        "VNC"
    }

    fn reaches(&self, target: &Target) -> bool {
        *target == Target::Viewer
    }

    fn start(&self, output: &str, _: &Target) -> Result<Box<dyn Stream>, String> {
        let control = runtime_dir().join("wayvnc.sock");
        let _ = std::fs::create_dir_all(runtime_dir());
        let mut cmd = Command::new("wayvnc");
        cmd.args(Self::args(output, control.to_str()))
            .stdin(Stdio::null())
            .stdout(Stdio::null());
        let child = children::spawn(cmd).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => "wayvnc is not installed".to_string(),
            _ => format!("wayvnc: {e}"),
        })?;
        log::info!("cast: wayvnc on {output}, pid {}", child.id());
        Ok(Box::new(Process {
            child: Some(child),
            what: format!("{}:{}, view only", Self::ADDRESS, Self::PORT),
        }))
    }
}

/// A stream that is one child process.
struct Process {
    child: Option<Child>,
    what: String,
}

impl Stream for Process {
    fn describe(&self) -> String {
        self.what.clone()
    }

    fn running(&mut self) -> bool {
        self.child
            .as_mut()
            .is_some_and(|c| matches!(c.try_wait(), Ok(None)))
    }

    fn stop(&mut self) {
        if let Some(mut c) = self.child.take() {
            children::end(&mut c, Duration::from_secs(1));
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::fake::{After, Fake};
    use crate::channel::{NS_MEDIA, NS_RECEIVER};
    use crate::output;

    fn device(address: &str, port: u16, video: bool) -> Device {
        Device {
            id: "daf6".into(),
            name: "Vardagsrummet TV".into(),
            model: Some("Chromecast".into()),
            address: address.into(),
            port,
            video,
            group: false,
        }
    }

    #[test]
    fn a_tv_gets_cast_a_viewer_gets_vnc_and_a_speaker_gets_nothing() {
        let tv = Target::Device(device("192.168.1.107", 8009, true));
        assert_eq!(for_target(&tv).map(|b| b.name()), Some("Cast"));
        assert_eq!(for_target(&Target::Viewer).map(|b| b.name()), Some("VNC"));
        let speaker = Target::Device(device("192.168.1.28", 8009, false));
        assert!(for_target(&speaker).is_none());
        let unresolved = Target::Device(device("tv.local", 8009, true));
        assert!(for_target(&unresolved).is_none());
    }

    #[test]
    fn the_stream_is_offered_from_the_address_that_reaches_the_tv() {
        assert_eq!(
            address_toward("127.0.0.1".parse().unwrap()).unwrap(),
            "127.0.0.1".parse::<IpAddr>().unwrap()
        );
        let t = token().unwrap();
        assert_eq!(t.len(), 32);
        assert_ne!(t, token().unwrap());
    }

    #[test]
    fn a_session_record_round_trips() {
        assert_eq!(
            parse_record("192.168.1.107:8009\nsession-7\n"),
            Some(("192.168.1.107:8009".parse().unwrap(), "session-7".into()))
        );
        assert_eq!(parse_record("192.168.1.107:8009\n"), None);
        assert_eq!(parse_record("nonsense\nx\n"), None);
    }

    fn nested() -> (String, SocketAddr) {
        let name = output::create(&Default::default()).expect("create");
        (name, "127.0.0.1:0".parse().unwrap())
    }

    /// The whole cast against a nested headless sway and a fake TV on
    /// localhost: the extra screen is made and encoded, the fake TV is told
    /// to load the URL and fetches it the way a TV would (ffprobe over
    /// HTTP), plays a moment, and Stop leaves nothing: no TV session, no
    /// port, no segments, no output.
    ///   SWAYSOCK=<nested socket> WAYLAND_DISPLAY=<nested display> \
    ///     cargo test -- --ignored cast_round_trip
    #[test]
    #[ignore = "needs a sway (nested), wf-recorder, ffmpeg and ffprobe on PATH"]
    fn cast_round_trip() {
        let (name, _) = nested();
        let probed = Arc::new(Mutex::new(String::new()));
        let p = probed.clone();
        let tv = Fake::start(After::Play, move |url| {
            let out = Command::new("ffprobe")
                .args([
                    "-v",
                    "error",
                    "-show_entries",
                    "stream=codec_name,width,height,pix_fmt",
                ])
                .args(["-of", "compact", url])
                .output()
                .expect("ffprobe");
            *p.lock().unwrap() = String::from_utf8_lossy(&out.stdout).into_owned();
        });
        let backend = MediaReceiver {
            port: 0,
            timing: Timing {
                ping: Duration::from_millis(300),
                ..Timing::STANDARD
            },
        };
        let target = Target::Device(device("127.0.0.1", tv.addr.port(), true));
        let mut stream = backend.start(&name, &target).expect("cast");
        assert!(
            record_path().exists(),
            "the session is recorded while it plays"
        );
        std::thread::sleep(Duration::from_secs(2));
        assert!(stream.running(), "{:?}", stream.why_ended());
        let probe = probed.lock().unwrap().clone();
        assert!(probe.contains("codec_name=h264"), "{probe}");
        assert!(probe.contains("width=1920|height=1080"), "{probe}");
        assert!(probe.contains("pix_fmt=yuv420p"), "{probe}");

        let hls_dir = runtime_dir().join(format!("cast-hls-{}", std::process::id()));
        assert!(hls_dir.exists());
        stream.stop();
        assert!(!stream.running());
        assert!(!hls_dir.exists(), "segments deleted");
        assert!(!record_path().exists(), "record gone");
        let kinds = tv.finish();
        let has = |ns: &str, k: &str| kinds.iter().any(|(n, t)| n == ns && t == k);
        assert!(
            has(NS_MEDIA, "LOAD") && has(NS_RECEIVER, "STOP"),
            "{kinds:?}"
        );
        output::remove(&name).expect("remove");
    }

    /// A TV that is switched off mid-cast ends the stream with a reason,
    /// and the encoder keeps nothing behind once stopped.
    #[test]
    #[ignore = "needs a sway (nested), wf-recorder and ffmpeg on PATH"]
    fn cast_to_a_tv_that_vanishes() {
        let (name, _) = nested();
        let tv = Fake::start(After::Vanish, |_| {});
        let backend = MediaReceiver {
            port: 0,
            timing: Timing::STANDARD,
        };
        let target = Target::Device(device("127.0.0.1", tv.addr.port(), true));
        let mut stream = backend.start(&name, &target).expect("cast");
        tv.join();
        std::thread::sleep(Duration::from_millis(1500));
        assert!(!stream.running());
        let why = stream.why_ended().unwrap_or_default();
        assert!(why.contains("stopped answering"), "{why}");
        stream.stop();
        output::remove(&name).expect("remove");
    }

    #[test]
    fn only_another_processs_segments_are_stale() {
        assert!(stale_segments("cast-hls-41", 42));
        assert!(!stale_segments("cast-hls-42", 42));
        assert!(!stale_segments("cast-output", 42));
        assert!(!stale_segments("cast-hls-x", 42));
        assert!(stale_segments("cast-live-41.log", 42));
        assert!(!stale_segments("cast-live-42.log", 42));
    }

    fn sh(cmd: &str) -> String {
        let out = Command::new("sh").args(["-c", cmd]).output().expect("sh");
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn quick_cast(playing_within: Duration) -> Cast {
        Cast {
            live: LivePage {
                port: 0,
                timing: Timing {
                    reply: Duration::from_secs(5),
                    ..Timing::STANDARD
                },
                playing_within,
                force: false,
            },
            hls: MediaReceiver {
                port: 0,
                timing: Timing::STANDARD,
            },
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("swaypplet-cast-test-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d.join(name)
    }

    /// The live page against a nested headless sway and a fake TV that acts
    /// as the page would: it fetches the page, says it plays, and reads the
    /// live stream (curl), twice, the second time as a reconnect. ffprobe
    /// reads both: H.264 1080p from a keyframe. Stop leaves nothing.
    ///   SWAYSOCK=<nested socket> WAYLAND_DISPLAY=<nested display> \
    ///     cargo test -- --ignored live_page_round_trip
    #[test]
    #[ignore = "needs a sway (nested), wf-recorder, ffmpeg, ffprobe and curl on PATH"]
    fn live_page_round_trip() {
        let (name, _) = nested();
        let (first, again) = (scratch("first.mp4"), scratch("again.mp4"));
        let page = Arc::new(Mutex::new(String::new()));
        let seen = page.clone();
        let (f, a) = (first.clone(), again.clone());
        let tv = Fake::start(After::Play, move |url| {
            let base = url.trim_end_matches("player.html").to_string();
            *seen.lock().unwrap() = sh(&format!("curl -s {url}"));
            sh(&format!(
                "curl -s -X POST --data '{{\"event\":\"playing\"}}' {base}report"
            ));
            let (f, a) = (f.clone(), a.clone());
            std::thread::spawn(move || {
                sh(&format!(
                    "curl -s --max-time 2 -o {} {base}live.mp4",
                    f.display()
                ));
                sh(&format!(
                    "curl -s --max-time 2 -o {} {base}live.mp4",
                    a.display()
                ));
            });
        });
        let cast = quick_cast(PLAYING_WITHIN);
        let target = Target::Device(device("127.0.0.1", tv.addr.port(), true));
        let started = std::time::Instant::now();
        let mut stream = cast.start(&name, &target).expect("cast");
        eprintln!("live page playing after {:?}", started.elapsed());
        assert!(
            stream.describe().contains("under a second"),
            "{}",
            stream.describe()
        );
        assert!(
            page.lock().unwrap().contains("MediaSource"),
            "the page was served"
        );
        std::thread::sleep(Duration::from_millis(4500));
        assert!(stream.running(), "{:?}", stream.why_ended());
        for file in [&first, &again] {
            let probe = sh(&format!(
                "ffprobe -v error -show_entries stream=codec_name,profile,level,width,height,pix_fmt -of compact {}",
                file.display()
            ));
            assert!(probe.contains("codec_name=h264"), "{probe}");
            assert!(probe.contains("width=1920|height=1080"), "{probe}");
            assert!(
                probe.contains("pix_fmt=yuv420p") && probe.contains("level=41"),
                "{probe}"
            );
            let frames = sh(&format!(
                "ffprobe -v error -select_streams v -show_entries frame=pict_type -of csv=p=0 {}",
                file.display()
            ));
            assert_eq!(frames.lines().next(), Some("I"), "starts on a keyframe");
            assert!(
                frames.lines().count() > 30,
                "about 2 s of frames: {}",
                frames.lines().count()
            );
        }
        let log = runtime_dir().join(format!("cast-live-{}.log", std::process::id()));
        assert!(record_path().exists() && log.exists());
        stream.stop();
        assert!(!stream.running());
        assert!(
            !record_path().exists() && !log.exists(),
            "nothing left in the runtime dir"
        );
        let kinds = tv.finish();
        let has = |ns: &str, k: &str| kinds.iter().any(|(n, t)| n == ns && t == k);
        assert!(
            has(NS_RECEIVER, "LAUNCH") && has(NS_RECEIVER, "STOP"),
            "{kinds:?}"
        );
        assert!(!has(NS_MEDIA, "LOAD"), "no fallback: {kinds:?}");
        output::remove(&name).expect("remove");
        let _ = std::fs::remove_dir_all(first.parent().unwrap());
    }

    /// A TV without DashCast gets the HLS stream instead.
    #[test]
    #[ignore = "needs a sway (nested), wf-recorder and ffmpeg on PATH"]
    fn falls_back_to_hls_when_dashcast_is_refused() {
        let (name, _) = nested();
        let urls = Arc::new(Mutex::new(Vec::new()));
        let got = urls.clone();
        let tv = Fake::start_for(After::RefuseDashCast, 2, move |u| {
            got.lock().unwrap().push(u.to_string())
        });
        let target = Target::Device(device("127.0.0.1", tv.addr.port(), true));
        let mut stream = quick_cast(PLAYING_WITHIN)
            .start(&name, &target)
            .expect("cast");
        assert!(
            stream.describe().contains("a few seconds"),
            "{}",
            stream.describe()
        );
        stream.stop();
        let kinds = tv.finish();
        let urls = urls.lock().unwrap().clone();
        assert_eq!(urls.len(), 1, "{urls:?}");
        assert!(urls[0].ends_with("/stream.m3u8"), "{urls:?}");
        assert!(
            kinds.iter().any(|(n, t)| n == NS_MEDIA && t == "LOAD"),
            "{kinds:?}"
        );
        output::remove(&name).expect("remove");
    }

    /// A page that never plays: DashCast is stopped, then HLS.
    #[test]
    #[ignore = "needs a sway (nested), wf-recorder and ffmpeg on PATH"]
    fn falls_back_to_hls_when_the_page_never_plays() {
        let (name, _) = nested();
        let tv = Fake::start_for(After::Play, 2, |_| {});
        let target = Target::Device(device("127.0.0.1", tv.addr.port(), true));
        let started = std::time::Instant::now();
        let mut stream = quick_cast(Duration::from_secs(3))
            .start(&name, &target)
            .expect("cast");
        assert!(started.elapsed() >= Duration::from_secs(3));
        assert!(
            stream.describe().contains("a few seconds"),
            "{}",
            stream.describe()
        );
        stream.stop();
        let kinds: Vec<String> = tv.finish().into_iter().map(|(_, t)| t).collect();
        let at = |k: &str| kinds.iter().position(|t| t == k).unwrap_or(usize::MAX);
        assert!(
            at("STOP") < at("LOAD"),
            "DashCast stopped before HLS: {kinds:?}"
        );
        output::remove(&name).expect("remove");
    }

    /// The live stream and its page for a real browser on this machine:
    /// prints the page's URL, then waits until the page has said it plays
    /// and sent three stats reports (with ?debug=1), at most 120 s.
    ///   SWAYSOCK=… WAYLAND_DISPLAY=… cargo test -- --ignored live_page_for_a_browser --nocapture
    #[test]
    #[ignore = "needs a sway (nested), the encoders, and a browser driven by hand"]
    fn live_page_for_a_browser() {
        let name = output::create(&Default::default()).expect("create");
        let log = scratch("browser.log");
        let enc = live::Encoder::start(&name, log, Duration::from_secs(8)).expect("encoder");
        let hub = enc.hub.clone();
        let server = http::serve(
            http::Site::Live(hub.clone()),
            "tok".into(),
            0,
            Some([127, 0, 0, 1].into()),
        )
        .unwrap();
        let url = format!("http://127.0.0.1:{}/tok/player.html?debug=1", server.port);
        std::fs::write(scratch("url"), &url).unwrap();
        eprintln!("PAGE {url}");
        let until = std::time::Instant::now() + Duration::from_secs(120);
        while !(hub.playing() && hub.stats_reports() >= 3) && std::time::Instant::now() < until {
            std::thread::sleep(Duration::from_millis(200));
        }
        assert!(hub.playing(), "the page never said it plays");
        assert!(
            hub.stats_reports() >= 3,
            "{} stats reports",
            hub.stats_reports()
        );
        eprintln!("STATS {}", hub.last_stats().unwrap_or_default());
        drop(server);
        drop(enc);
        output::remove(&name).expect("remove");
        let _ = std::fs::remove_dir_all(scratch("url").parent().unwrap());
    }

    /// What the next panel does after one was killed mid-cast (run it after
    /// a SIGKILL of `cast_round_trip`): the TV session, the segments and the
    /// extra screen in the records are dealt with, and the records are gone.
    #[test]
    #[ignore = "needs the records a killed cast left"]
    fn sweep_after_a_crash() {
        let left =
            std::fs::read_to_string(runtime_dir().join("cast-output")).expect("output record");
        let name = left.lines().next().unwrap().to_string();
        assert!(record_path().exists(), "session record");
        sweep();
        output::sweep();
        assert!(!record_path().exists());
        let stale = std::fs::read_dir(runtime_dir())
            .unwrap()
            .flatten()
            .any(|e| {
                let n = e.file_name().to_string_lossy().into_owned();
                n.starts_with("cast-hls-") || n.starts_with("cast-live-")
            });
        assert!(!stale, "segments removed");
        let outputs = swaypplet_base::sway::ipc::connect()
            .unwrap()
            .get_outputs()
            .unwrap();
        assert!(outputs.iter().all(|o| o.name != name), "{name} removed");
    }

    /// The VNC lifecycle against a running sway: make the extra screen,
    /// serve it, read the RFB greeting and the screen's size, stop, remove.
    /// wayvnc is started from a worker thread that then exits, as in the
    /// panel, and must outlive it.
    ///   SWAYSOCK=<nested socket> WAYLAND_DISPLAY=<nested display> \
    ///     cargo test -- --ignored extra_screen_round_trip
    #[test]
    #[ignore = "needs a sway (nested) and wayvnc on PATH"]
    fn extra_screen_round_trip() {
        use std::io::{Read, Write};
        let before = swaypplet_base::sway::ipc::connect()
            .unwrap()
            .get_outputs()
            .unwrap();
        let name = output::create(&Default::default()).expect("create");
        let after = swaypplet_base::sway::ipc::connect()
            .unwrap()
            .get_outputs()
            .unwrap();
        let made = after.iter().find(|o| o.name == name).expect("listed");
        let right = before
            .iter()
            .map(|o| o.rect.x + o.rect.width)
            .max()
            .unwrap_or(0);
        assert_eq!(made.rect.x, right, "beside the others, not on top");
        let for_worker = name.clone();
        let mut stream = std::thread::spawn(move || Vnc.start(&for_worker, &Target::Viewer))
            .join()
            .unwrap()
            .expect("wayvnc");
        let mut sock = None;
        for _ in 0..50 {
            std::thread::sleep(Duration::from_millis(100));
            if let Ok(s) = std::net::TcpStream::connect(("127.0.0.1", Vnc::PORT)) {
                sock = Some(s);
                break;
            }
        }
        let mut s = sock.expect("wayvnc listens");
        let mut greeting = [0u8; 12];
        s.read_exact(&mut greeting).unwrap();
        assert_eq!(&greeting, b"RFB 003.008\n");
        // Version, then security type None, then a shared ClientInit.
        s.write_all(b"RFB 003.008\n").unwrap();
        let mut n = [0u8; 1];
        s.read_exact(&mut n).unwrap();
        let mut types = vec![0u8; n[0] as usize];
        s.read_exact(&mut types).unwrap();
        assert!(types.contains(&1), "security types {types:?}");
        s.write_all(&[1]).unwrap();
        let mut result = [0u8; 4];
        s.read_exact(&mut result).unwrap();
        assert_eq!(result, [0; 4]);
        s.write_all(&[1]).unwrap();
        let mut size = [0u8; 4];
        s.read_exact(&mut size).unwrap();
        let (w, h) = (
            u16::from_be_bytes([size[0], size[1]]),
            u16::from_be_bytes([size[2], size[3]]),
        );
        assert_eq!(
            (u32::from(w), u32::from(h)),
            (output::MODE.0, output::MODE.1)
        );
        assert!(stream.running());
        stream.stop();
        assert!(!stream.running());
        output::remove(&name).expect("remove");
    }

    #[test]
    fn wayvnc_is_view_only_on_localhost_on_the_extra_screen() {
        let args = Vnc::args("HEADLESS-2", Some("/run/user/1000/swaypplet/wayvnc.sock"));
        assert_eq!(
            args,
            [
                "--disable-input",
                "--output=HEADLESS-2",
                "--socket=/run/user/1000/swaypplet/wayvnc.sock",
                "127.0.0.1",
                "5900"
            ]
        );
    }
}
