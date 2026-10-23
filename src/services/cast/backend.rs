//! What sends the extra screen somewhere: one [`Backend`] per way of
//! getting pixels from a sway output to a screen across the room.
//!
//! # Built
//!
//! - [`MediaReceiver`]: a Cast device with a screen. The output is encoded
//!   to HLS (`hls`), served over HTTP from this machine (`http`), and the
//!   device's Default Media Receiver (app `CC1AD845`, on every Cast device,
//!   no registration) is told to play it as a live stream (`channel`). It
//!   runs seconds behind: an HLS player stays about three segments behind
//!   the newest (`hls` says why HLS). Good for a slideshow, a video, a
//!   dashboard; not for following a pointer.
//! - [`Vnc`]: wayvnc on the output, view only, on localhost. Any VNC viewer
//!   shows the screen (through an SSH or tailscale hop from another
//!   machine).
//!
//! # Not built
//!
//! **Cast mirroring** (the receiver Chrome's "Cast screen" uses) is the
//! low-lag path, a fraction of a second. Its sender side is Cast Streaming:
//! an OFFER/ANSWER exchange on the Cast channel, then AES-encrypted RTP over
//! UDP with RTCP feedback (Open Screen's `cast/streaming`), with no Rust
//! implementation to build on. Chrome is such a sender; pointing
//! xdg-desktop-portal-wlr at the extra screen and choosing "Cast screen" in
//! Chrome is the way to that lag without writing one.
//!
//! # What a stream leaves behind
//!
//! Nothing, when stopped: the TV's app is stopped, the encoder ends, the
//! port closes, the segments are deleted, and `mod.rs` removes the output.
//! When the panel dies instead, every child process gets SIGTERM
//! (`children`), and the next panel stops the TV session it finds in the
//! runtime directory ([`sweep`]) and removes the output (`output::sweep`).

use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::channel::{self, Channel, Timing};
use super::discovery::Device;
use super::{children, hls, http};

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

static MEDIA_RECEIVER: MediaReceiver = MediaReceiver {
    port: http::PORT,
    timing: Timing::STANDARD,
};

/// Every backend built, in the order they are preferred.
static BACKENDS: &[&dyn Backend] = &[&MEDIA_RECEIVER, &Vnc];

/// The backend that sends to `target`, when one is built.
pub fn for_target(target: &Target) -> Option<&'static dyn Backend> {
    BACKENDS.iter().copied().find(|b| b.reaches(target))
}

fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
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
        let server = http::serve(dir, token.clone(), self.port, Some(ip))?;
        let url = format!("http://{here}:{}/{token}/{}", server.port, hls::PLAYLIST);

        let mut ch = Channel::open(tv, self.timing)?;
        let app = ch.launch()?;
        if let Err(e) = ch.load(&app, &url, "Extra screen") {
            ch.stop(&app);
            return Err(e);
        }
        record(tv, &app.session_id);
        log::info!("cast: {} plays {url}", device.name);

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
            what: format!("{}, a few seconds behind", device.name),
            encoder: Some(encoder),
            server: Some(server),
            follow: Some(follow),
            stop,
            ended,
        }))
    }
}

struct CastStream {
    what: String,
    encoder: Option<hls::Encoder>,
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
        self.encoder.as_mut().is_some_and(hls::Encoder::running)
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

/// Whether `name` is a segment directory another process left: a panel
/// that died mid-cast. Ours is `cast-hls-<our pid>`.
fn stale_segments(name: &str, ours: u32) -> bool {
    name.strip_prefix("cast-hls-")
        .and_then(|pid| pid.parse::<u32>().ok())
        .is_some_and(|pid| pid != ours)
}

/// Clean up after a panel that died mid-cast: its segments, and the TV
/// session it left playing. Blocking.
pub fn sweep() {
    if let Ok(entries) = std::fs::read_dir(runtime_dir()) {
        for e in entries.flatten() {
            if stale_segments(&e.file_name().to_string_lossy(), std::process::id()) {
                let _ = std::fs::remove_dir_all(e.path());
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
    use crate::services::cast::channel::fake::{After, Fake};
    use crate::services::cast::channel::{NS_MEDIA, NS_RECEIVER};
    use crate::services::cast::output;

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
        let name = output::create().expect("create");
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
            .any(|e| e.file_name().to_string_lossy().starts_with("cast-hls-"));
        assert!(!stale, "segments removed");
        let outputs = crate::sway::ipc::connect().unwrap().get_outputs().unwrap();
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
        let before = crate::sway::ipc::connect().unwrap().get_outputs().unwrap();
        let name = output::create().expect("create");
        let after = crate::sway::ipc::connect().unwrap().get_outputs().unwrap();
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
