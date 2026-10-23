//! The extra screen as an HLS stream: wf-recorder captures the output and
//! encodes it, ffmpeg cuts it into segments in a directory the HTTP server
//! serves.
//!
//! # Why HLS, and not one endless fragmented MP4
//!
//! A progressive fragmented MP4 (one HTTP response that never ends) could
//! run closer to live, since the player starts on the first fragment it
//! gets. But the Default Media Receiver documents MP4 for files, not for
//! an endless live response; a player that falls behind over one response
//! never catches up; and a TV that reconnects would need the init segment
//! and a keyframe cut for it. HLS is the live format that receiver
//! documents: the playlist names the newest segments, the player joins at
//! the live edge, and a hiccup costs a segment, not the stream. The server
//! only serves files. The price is lag: an HLS player stays about three
//! target durations behind the newest segment, so the segments here are as
//! short as a keyframe interval allows, one second.
//!
//! The segments are MPEG-TS, which the receiver plays without being told
//! the segment format (`hlsVideoSegmentFormat` is needed for fMP4).
//!
//! # The pipeline
//!
//! `wf-recorder --no-damage` asks for every frame, so a still screen still
//! makes segments (by damage only, a still screen makes none and the
//! player stalls), at a constant 30 fps. libx264 `veryfast`/`zerolatency`
//! (no B-frames, no lookahead), a keyframe every 30 frames so every
//! one-second segment starts on one, limited-range yuv420p (a TV's range),
//! capped at 8 Mbit/s for Wi-Fi. ffmpeg copies the video into segments
//! (`temp_file`, so a file is whole when it appears) and keeps the playlist
//! to the last six.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use super::children;

/// Seconds per segment; also the keyframe interval in seconds.
pub const SEGMENT_S: u32 = 1;
pub const FPS: u32 = 30;
pub const PLAYLIST: &str = "stream.m3u8";

/// wf-recorder's arguments for `output`, writing MPEG-TS to stdout.
pub fn recorder_args(output: &str) -> Vec<String> {
    let gop = (FPS * SEGMENT_S).to_string();
    [
        "--output",
        output,
        "--no-damage",
        "--framerate",
        &FPS.to_string(),
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
        &format!("g={gop}"),
        "--codec-param",
        "maxrate=8000000",
        "--codec-param",
        "bufsize=4000000",
        "--muxer",
        "mpegts",
        "-f",
        "pipe:1",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// ffmpeg's arguments: MPEG-TS on stdin, HLS into `dir`.
pub fn segmenter_args(dir: &Path) -> Vec<String> {
    let d = dir.display();
    [
        "-hide_banner",
        "-loglevel",
        "warning",
        "-f",
        "mpegts",
        "-i",
        "pipe:0",
        "-c",
        "copy",
        "-f",
        "hls",
        "-hls_time",
        &SEGMENT_S.to_string(),
        "-hls_list_size",
        "6",
        "-hls_flags",
        "delete_segments+independent_segments+temp_file",
        "-hls_segment_filename",
        &format!("{d}/seg%05d.ts"),
        &format!("{d}/{PLAYLIST}"),
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// How many segments a playlist names.
pub fn segments_in(playlist: &str) -> usize {
    playlist
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .count()
}

/// The two processes, running.
pub struct Encoder {
    pub dir: PathBuf,
    recorder: Child,
    segmenter: Child,
}

impl Encoder {
    /// Start on `output`, writing into `dir` (made here), and wait until the
    /// playlist names `ready` segments: a LOAD before that is a playlist the
    /// player gives up on. The stderr of both goes to `dir/encoder.log`.
    pub fn start(
        output: &str,
        dir: PathBuf,
        ready: usize,
        within: Duration,
    ) -> Result<Encoder, String> {
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let log = std::fs::File::create(dir.join("encoder.log")).map_err(|e| e.to_string())?;
        let mut rec = Command::new("wf-recorder");
        rec.args(recorder_args(output))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(log.try_clone().map_err(|e| e.to_string())?);
        let mut recorder = children::spawn(rec).map_err(|e| missing("wf-recorder", e))?;
        let Some(ts) = recorder.stdout.take() else {
            children::end(&mut recorder, Duration::ZERO);
            return Err("wf-recorder gave no output pipe".into());
        };
        let mut seg = Command::new("ffmpeg");
        seg.args(segmenter_args(&dir))
            .stdin(Stdio::from(ts))
            .stdout(Stdio::null())
            .stderr(log);
        let segmenter = match children::spawn(seg) {
            Ok(c) => c,
            Err(e) => {
                children::end(&mut recorder, Duration::ZERO);
                return Err(missing("ffmpeg", e));
            }
        };
        let mut enc = Encoder {
            dir,
            recorder,
            segmenter,
        };
        let until = Instant::now() + within;
        loop {
            let names = std::fs::read_to_string(enc.dir.join(PLAYLIST))
                .map(|p| segments_in(&p))
                .unwrap_or(0);
            if names >= ready {
                return Ok(enc);
            }
            if !enc.running() {
                let why = enc.log_tail();
                enc.stop();
                return Err(format!("the encoder stopped: {why}"));
            }
            if Instant::now() >= until {
                enc.stop();
                return Err(format!("no stream after {} s", within.as_secs()));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    pub fn running(&mut self) -> bool {
        matches!(self.recorder.try_wait(), Ok(None))
            && matches!(self.segmenter.try_wait(), Ok(None))
    }

    /// The last line either process wrote, for a reason in the view.
    pub fn log_tail(&self) -> String {
        std::fs::read_to_string(self.dir.join("encoder.log"))
            .ok()
            .and_then(|l| {
                l.lines()
                    .rev()
                    .find(|l| !l.trim().is_empty())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "no message".into())
    }

    /// Stop both and remove the directory.
    pub fn stop(&mut self) {
        children::end(&mut self.recorder, Duration::from_secs(1));
        children::end(&mut self.segmenter, Duration::from_secs(1));
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        self.stop();
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

    #[test]
    fn every_segment_starts_on_a_keyframe_and_a_still_screen_still_streams() {
        let a = recorder_args("HEADLESS-3");
        let has = |pair: [&str; 2]| a.windows(2).any(|w| w[0] == pair[0] && w[1] == pair[1]);
        assert!(has(["--output", "HEADLESS-3"]));
        assert!(a.contains(&"--no-damage".to_string()));
        assert!(has(["--codec-param", "g=30"]), "one keyframe per segment");
        assert!(has(["--pixel-format", "yuv420p"]));
        assert!(has(["-f", "pipe:1"]));
    }

    #[test]
    fn segments_are_one_second_whole_files_in_the_directory() {
        let a = segmenter_args(Path::new("/run/user/1000/swaypplet/cast-hls"));
        let s = a.join(" ");
        assert!(s.contains("-c copy"));
        assert!(s.contains("-hls_time 1 "));
        assert!(s.contains("temp_file"));
        assert!(s.ends_with("/run/user/1000/swaypplet/cast-hls/stream.m3u8"));
    }

    #[test]
    fn a_playlist_counts_its_segments() {
        let p = "#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXTINF:1.0,\nseg00002.ts\n#EXTINF:1.0,\nseg00003.ts\n";
        assert_eq!(segments_in(p), 2);
        assert_eq!(segments_in("#EXTM3U\n"), 0);
    }
}
