//! The extra screen as one live fragmented MP4, for a page that plays it
//! with Media Source Extensions (`player.html`).
//!
//! # The pipeline
//!
//! `wf-recorder --no-damage` encodes the output as raw H.264 (Annex B) on
//! its stdout; ffmpeg wraps it, without re-encoding, into fragmented MP4 on
//! its stdout: an init segment, then a fragment every 100 ms
//! (`-frag_duration`), and a new one at every keyframe (`frag_keyframe`),
//! one every 0.5 s (`g=15` at 30 fps). A reader thread cuts that into
//! pieces (`fmp4::Splitter`) and hands them to the [`Hub`].
//!
//! Raw H.264, not MPEG-TS, into ffmpeg: ffmpeg probes an MPEG-TS input for
//! up to 5 s or 5 MB before it writes anything, and a still screen makes
//! about 17 kB/s, so the first fragment came out after the recorder had
//! already stopped (measured in a nested sway, 2026-10-23). An Annex B
//! stream starts with its SPS, which says all ffmpeg needs; the timestamps
//! come from the constant 30 fps wf-recorder sends (`-framerate 30`). With
//! that, the init segment arrives 0.6 s after start and fragments follow
//! every 100 ms.
//!
//! wf-recorder's own MP4 muxer cannot be told to fragment, which is why
//! ffmpeg is in the pipe at all.
//!
//! # The hub
//!
//! Keeps the init segment and passes fragments to whoever is watching. A
//! viewer that connects, or reconnects, gets the init segment and then
//! starts at the next keyframe fragment. Each viewer has a queue of
//! [`QUEUE`] fragments (3 s); one that falls further behind than that is
//! dropped (it would show the past anyway) and reconnects. The reader never
//! waits for a viewer.

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::children;
use super::fmp4::{Piece, Splitter};

pub const FPS: u32 = 30;
/// A keyframe every this many frames: 0.5 s, the longest a joining viewer
/// waits for a picture.
pub const GOP: u32 = 15;
/// Fragment length in microseconds.
pub const FRAGMENT_US: u32 = 100_000;
/// Fragments a viewer may be behind before it is dropped.
pub const QUEUE: usize = 30;

/// One shared piece of the stream.
pub type Bytes = Arc<[u8]>;

struct Viewer {
    tx: SyncSender<Bytes>,
    /// It has had its keyframe and gets every fragment from here on.
    started: bool,
}

#[derive(Default)]
struct State {
    init: Option<Bytes>,
    codec: Option<String>,
    viewers: Vec<Viewer>,
    /// Wall-clock ms at media time 0, the smallest seen over recent
    /// fragments: arrival here minus media time.
    t0_ms: Option<f64>,
    closed: bool,
}

/// The live stream's switchboard.
#[derive(Default)]
pub struct Hub {
    state: Mutex<State>,
    keyed: AtomicBool,
    playing: AtomicBool,
    /// Stats reports from the page's debug view, and the last one.
    stats: std::sync::atomic::AtomicUsize,
    last_stats: Mutex<Option<String>>,
}

fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

impl Hub {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Take one piece from the encoder. Never blocks on a viewer.
    pub fn publish(&self, piece: Piece) {
        let mut s = self.lock();
        match piece {
            Piece::Init { bytes, info } => {
                s.init = Some(bytes.into());
                s.codec = Some(info.codec);
            }
            Piece::Fragment { bytes, key, at } => {
                let anchor = now_ms() - at * 1000.0;
                s.t0_ms = Some(s.t0_ms.map_or(anchor, |t| t.min(anchor)));
                if key {
                    self.keyed.store(true, Ordering::Relaxed);
                }
                let bytes: Bytes = bytes.into();
                s.viewers.retain_mut(|v| {
                    if !v.started {
                        if !key {
                            return true;
                        }
                        v.started = true;
                    }
                    match v.tx.try_send(bytes.clone()) {
                        Ok(()) => true,
                        Err(TrySendError::Full(_)) => {
                            log::info!("cast: a viewer fell {QUEUE} fragments behind; dropped");
                            false
                        }
                        Err(TrySendError::Disconnected(_)) => false,
                    }
                });
            }
        }
    }

    /// The encoder ended: every viewer's stream ends too.
    pub fn close(&self) {
        let mut s = self.lock();
        s.closed = true;
        s.viewers.clear();
    }

    /// Start watching: the init segment, and the fragments from the next
    /// keyframe on. `None` before there is an init segment, or once closed.
    pub fn watch(&self) -> Option<(Bytes, Receiver<Bytes>)> {
        let mut s = self.lock();
        if s.closed {
            return None;
        }
        let init = s.init.clone()?;
        let (tx, rx) = sync_channel(QUEUE);
        s.viewers.push(Viewer { tx, started: false });
        Some((init, rx))
    }

    /// The MSE codec string, once the init segment is in.
    pub fn codec(&self) -> Option<String> {
        self.lock().codec.clone()
    }

    pub fn t0_ms(&self) -> Option<f64> {
        self.lock().t0_ms
    }

    /// Whether a keyframe fragment has come through: a viewer can start.
    pub fn ready(&self) -> bool {
        self.keyed.load(Ordering::Relaxed) && self.lock().init.is_some()
    }

    #[cfg(test)]
    pub fn viewers(&self) -> usize {
        self.lock().viewers.len()
    }

    /// The page said a frame is on screen.
    pub fn set_playing(&self) {
        if !self.playing.swap(true, Ordering::Relaxed) {
            log::info!("cast: the page is playing");
        }
    }

    pub fn playing(&self) -> bool {
        self.playing.load(Ordering::Relaxed)
    }

    pub fn count_stats(&self, report: &str) {
        self.stats.fetch_add(1, Ordering::Relaxed);
        *self.last_stats.lock().unwrap_or_else(|e| e.into_inner()) = Some(report.to_string());
    }

    #[cfg(test)]
    /// The last stats report, as the page sent it.
    pub fn last_stats(&self) -> Option<String> {
        self.last_stats
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    #[cfg(test)]
    /// How many stats reports the page sent.
    pub fn stats_reports(&self) -> usize {
        self.stats.load(Ordering::Relaxed)
    }
}

/// wf-recorder's arguments: raw H.264 of `output` on stdout.
pub fn recorder_args(output: &str) -> Vec<String> {
    let s = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let mut a = s(&["--output", output, "--no-damage", "--framerate"]);
    a.push(FPS.to_string());
    a.extend(s(&[
        "--codec",
        "libx264",
        "--filter",
        "scale=out_range=tv",
        "--pixel-format",
        "yuv420p",
        "--codec-param",
        "color_range=tv",
        "--codec-param",
        "preset=veryfast",
        "--codec-param",
        "tune=zerolatency",
        "--codec-param",
        "bf=0",
        "--codec-param",
        "profile=high",
        "--codec-param",
        "level=4.1",
        "--codec-param",
        "maxrate=6000000",
        "--codec-param",
        "bufsize=3000000",
        "--codec-param",
    ]));
    a.push(format!("g={GOP}"));
    a.extend(s(&["--muxer", "h264", "-f", "pipe:1"]));
    a
}

/// ffmpeg's arguments: raw H.264 in, fragmented MP4 out, both pipes.
pub fn muxer_args() -> Vec<String> {
    [
        "-hide_banner",
        "-loglevel",
        "warning",
        "-fflags",
        "nobuffer",
        "-probesize",
        "2048",
        "-analyzeduration",
        "0",
        "-f",
        "h264",
        "-framerate",
        &FPS.to_string(),
        "-i",
        "pipe:0",
        "-c",
        "copy",
        "-f",
        "mp4",
        "-movflags",
        "empty_moov+default_base_moof+frag_keyframe",
        "-frag_duration",
        &FRAGMENT_US.to_string(),
        "-flush_packets",
        "1",
        "pipe:1",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// The two processes and the reader, running.
pub struct Encoder {
    pub hub: Arc<Hub>,
    recorder: Child,
    muxer: Child,
    reader: Option<std::thread::JoinHandle<()>>,
    log: std::path::PathBuf,
}

impl Encoder {
    /// Start on `output`, and wait until a viewer could start: the init
    /// segment and a keyframe fragment. Both stderrs go to `log`.
    pub fn start(
        output: &str,
        log: std::path::PathBuf,
        within: Duration,
    ) -> Result<Encoder, String> {
        if let Some(dir) = log.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let err = std::fs::File::create(&log).map_err(|e| format!("{}: {e}", log.display()))?;
        let mut rec = Command::new("wf-recorder");
        rec.args(recorder_args(output))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(err.try_clone().map_err(|e| e.to_string())?);
        let mut recorder = children::spawn(rec).map_err(|e| missing("wf-recorder", e))?;
        let Some(h264) = recorder.stdout.take() else {
            children::end(&mut recorder, Duration::ZERO);
            return Err("wf-recorder gave no output pipe".into());
        };
        let mut mux = Command::new("ffmpeg");
        mux.args(muxer_args())
            .stdin(Stdio::from(h264))
            .stdout(Stdio::piped())
            .stderr(err);
        let mut muxer = match children::spawn(mux) {
            Ok(c) => c,
            Err(e) => {
                children::end(&mut recorder, Duration::ZERO);
                return Err(missing("ffmpeg", e));
            }
        };
        let hub = Arc::new(Hub::default());
        let out = muxer.stdout.take();
        let reader = {
            let hub = hub.clone();
            std::thread::Builder::new()
                .name("swaypplet-cast-mp4".into())
                .spawn(move || {
                    if let Some(out) = out {
                        read(out, &hub);
                    }
                    hub.close();
                })
                .map_err(|e| format!("reader thread: {e}"))?
        };
        let mut enc = Encoder {
            hub,
            recorder,
            muxer,
            reader: Some(reader),
            log,
        };
        let until = Instant::now() + within;
        while !enc.hub.ready() {
            if !enc.running() {
                let why = enc.log_tail();
                enc.stop();
                return Err(format!("the encoder stopped: {why}"));
            }
            if Instant::now() >= until {
                enc.stop();
                return Err(format!("no stream after {} s", within.as_secs()));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        Ok(enc)
    }

    pub fn running(&mut self) -> bool {
        matches!(self.recorder.try_wait(), Ok(None))
            && matches!(self.muxer.try_wait(), Ok(None))
            && self.reader.as_ref().is_some_and(|r| !r.is_finished())
    }

    pub fn log_tail(&self) -> String {
        std::fs::read_to_string(&self.log)
            .ok()
            .and_then(|l| {
                l.lines()
                    .rev()
                    .find(|l| !l.trim().is_empty())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "no message".into())
    }

    /// Stop both processes; the reader ends with the pipe and closes the
    /// hub, which ends every viewer's stream.
    pub fn stop(&mut self) {
        children::end(&mut self.recorder, Duration::from_secs(1));
        children::end(&mut self.muxer, Duration::from_secs(1));
        if let Some(r) = self.reader.take() {
            let _ = r.join();
        }
        self.hub.close();
        let _ = std::fs::remove_file(&self.log);
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        self.stop();
    }
}

fn read(mut out: impl Read, hub: &Hub) {
    let mut split = Splitter::default();
    let mut chunk = vec![0u8; 64 * 1024];
    loop {
        let n = match out.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        match split.push(&chunk[..n]) {
            Ok(pieces) => pieces.into_iter().for_each(|p| hub.publish(p)),
            Err(e) => {
                log::warn!("cast: the MP4 stream: {e}");
                return;
            }
        }
    }
}

fn missing(what: &str, e: std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::NotFound => format!("{what} is not installed"),
        _ => format!("{what}: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cast::fmp4::InitInfo;

    fn init() -> Piece {
        Piece::Init {
            bytes: b"INIT".to_vec(),
            info: InitInfo {
                codec: "avc1.640029".into(),
                timescale: 15360,
                trex_flags: 0,
            },
        }
    }

    fn frag(n: u8, key: bool) -> Piece {
        Piece::Fragment {
            bytes: vec![n],
            key,
            at: f64::from(n) / 10.0,
        }
    }

    #[test]
    fn a_viewer_gets_the_init_then_starts_on_a_keyframe() {
        let hub = Hub::default();
        assert!(hub.watch().is_none(), "nothing to watch before the init");
        hub.publish(init());
        hub.publish(frag(0, true));
        let (first, rx) = hub.watch().unwrap();
        assert_eq!(&*first, b"INIT");
        for (n, key) in [(1, false), (2, false), (5, true), (6, false)] {
            hub.publish(frag(n, key));
        }
        let got: Vec<u8> = rx.try_iter().map(|b| b[0]).collect();
        assert_eq!(got, [5, 6], "nothing before the keyframe");
        assert_eq!(hub.codec().as_deref(), Some("avc1.640029"));
        assert!(hub.ready());
    }

    #[test]
    fn a_viewer_that_falls_behind_is_dropped_and_may_come_back() {
        let hub = Hub::default();
        hub.publish(init());
        let (_, slow) = hub.watch().unwrap();
        let (_, quick) = hub.watch().unwrap();
        hub.publish(frag(0, true));
        for _ in 0..QUEUE + 5 {
            hub.publish(frag(1, false));
            // The quick one keeps up.
            while quick.try_recv().is_ok() {}
        }
        assert_eq!(
            hub.viewers(),
            1,
            "the slow one is gone, the quick one stays"
        );
        assert_eq!(slow.try_iter().count(), QUEUE, "it had a full queue");
        assert!(slow.recv().is_err(), "and then its stream ended");
        // A reconnect is a new viewer and starts at the next keyframe.
        let (_, again) = hub.watch().unwrap();
        hub.publish(frag(2, false));
        hub.publish(frag(3, true));
        assert_eq!(again.try_iter().map(|b| b[0]).collect::<Vec<_>>(), [3]);
    }

    #[test]
    fn closing_ends_every_stream_and_refuses_new_ones() {
        let hub = Hub::default();
        hub.publish(init());
        let (_, rx) = hub.watch().unwrap();
        hub.close();
        assert!(rx.recv().is_err());
        assert!(hub.watch().is_none());
    }

    #[test]
    fn a_still_screen_streams_on_a_keyframe_every_half_second() {
        let a = recorder_args("HEADLESS-4");
        let has = |pair: [&str; 2]| a.windows(2).any(|w| w[0] == pair[0] && w[1] == pair[1]);
        assert!(a.contains(&"--no-damage".to_string()));
        assert!(has(["--codec-param", "g=15"]));
        assert!(has(["--muxer", "h264"]), "raw H.264: no MPEG-TS probe");
        let m = muxer_args().join(" ");
        assert!(m.contains("-f h264 -framerate 30 -i pipe:0 -c copy"));
        assert!(m.contains("frag_keyframe") && m.contains("-frag_duration 100000"));
    }
}
