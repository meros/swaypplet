//! What sends the extra screen somewhere: one [`Backend`] per way of
//! getting pixels from a sway output to a screen across the room.
//!
//! # Built
//!
//! - [`Vnc`]: wayvnc on the output, view only, on localhost. Any VNC viewer
//!   shows the screen (through an SSH or tailscale hop from another
//!   machine). It reaches no Cast device.
//!
//! # Not built, and why
//!
//! A Cast device plays what a *receiver app* on it understands, and the
//! sender has to speak the Cast v2 channel (TLS to port 8009, protobuf
//! `CastMessage` frames, JSON payloads, a heartbeat) to launch one. Two
//! receivers ship on every device, so neither needs an app id registered
//! with Google:
//!
//! - **Default Media Receiver** (`CC1AD845`) plays a URL: HLS, DASH or a
//!   file. A backend would encode the output (wf-recorder or a screencopy
//!   capture into ffmpeg, H.264), cut it into one-second HLS segments, serve
//!   them over HTTP from this machine, and `LOAD` the playlist as a LIVE
//!   stream. The receiver buffers a few segments before it plays, so the
//!   picture runs seconds behind: fine for a slideshow or a video, not for
//!   a pointer. It needs a TLS client (none in this crate yet) and an open
//!   TCP port for the segments.
//! - **Cast mirroring** (the receiver Chrome's "Cast screen" uses) is the
//!   low-lag path, a fraction of a second. Its sender side is Cast
//!   Streaming: an OFFER/ANSWER exchange on the Cast channel, then
//!   AES-encrypted RTP over UDP with RTCP feedback (Open Screen's
//!   `cast/streaming`). No Rust implementation exists to build on.
//!   Chrome itself is such a sender; it captures through the screen-cast
//!   portal, so pointing xdg-desktop-portal-wlr at the extra screen
//!   (`output_name`, or a `chooser_cmd` that names it) and choosing "Cast
//!   screen" in Chrome would be the way to use it without writing one.
//!
//! A backend for either reaches [`Target::Device`]; [`for_target`] picks it
//! up and the section's device rows start working. Until then those rows
//! say that no backend reaches them, and nothing pretends to cast.

use std::process::{Child, Command, Stdio};

use super::discovery::Device;

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
    /// What it is called in the section ("VNC").
    fn name(&self) -> &'static str;
    /// Whether it can send to `target` at all.
    fn reaches(&self, target: &Target) -> bool;
    /// Start sending `output` to `target`. Blocking only as long as starting
    /// a process takes.
    fn start(&self, output: &str, target: &Target) -> Result<Box<dyn Stream>, String>;
}

/// A stream that is running.
pub trait Stream: Send {
    /// Where to look for it, in words ("localhost:5900, view only").
    fn describe(&self) -> String;
    /// Whether it still runs; false once whatever sends it has ended.
    fn running(&mut self) -> bool;
    /// End it. Idempotent.
    fn stop(&mut self);
}

/// Every backend built, in the order they are preferred.
static BACKENDS: &[&dyn Backend] = &[&Vnc];

/// The backend that sends to `target`, when one is built.
pub fn for_target(target: &Target) -> Option<&'static dyn Backend> {
    BACKENDS.iter().copied().find(|b| b.reaches(target))
}

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
        let control = std::env::var("XDG_RUNTIME_DIR")
            .ok()
            .map(|d| format!("{d}/swaypplet/wayvnc.sock"));
        if let Some(dir) = control
            .as_deref()
            .and_then(|c| std::path::Path::new(c).parent())
        {
            let _ = std::fs::create_dir_all(dir);
        }
        let mut cmd = Command::new("wayvnc");
        cmd.args(Self::args(output, control.as_deref()))
            .stdin(Stdio::null())
            .stdout(Stdio::null());
        die_with_parent(&mut cmd);
        let child = cmd.spawn().map_err(|e| match e.kind() {
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
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The child gets SIGTERM when the panel dies, so a crash does not leave a
/// VNC server running on an output nobody removes.
fn die_with_parent(cmd: &mut Command) {
    use std::os::unix::process::CommandExt;
    // SAFETY: prctl is async-signal-safe and touches only the child.
    unsafe {
        cmd.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
            Ok(())
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tv() -> Target {
        Target::Device(Device {
            id: "daf6".into(),
            name: "Vardagsrummet TV".into(),
            model: Some("Chromecast".into()),
            address: "192.168.1.107".into(),
            port: 8009,
            video: true,
            group: false,
        })
    }

    #[test]
    fn a_viewer_gets_vnc_and_no_backend_claims_a_cast_device() {
        assert_eq!(for_target(&Target::Viewer).map(|b| b.name()), Some("VNC"));
        // Until a Cast sender is built, a device row has nothing to start.
        assert!(for_target(&tv()).is_none());
    }

    /// The whole lifecycle against a running sway: make the extra screen,
    /// serve it over VNC, read the RFB greeting and the screen's size, stop,
    /// remove. Run it against a nested headless sway, never the session's:
    ///   SWAYSOCK=<nested socket> WAYLAND_DISPLAY=<nested display> \
    ///     cargo test -- --ignored extra_screen_round_trip
    #[test]
    #[ignore = "needs a sway (nested) and wayvnc on PATH"]
    fn extra_screen_round_trip() {
        use std::io::{Read, Write};
        let before = crate::sway::ipc::connect().unwrap().get_outputs().unwrap();
        let name = super::super::output::create().expect("create");
        let after = crate::sway::ipc::connect().unwrap().get_outputs().unwrap();
        let made = after.iter().find(|o| o.name == name).expect("listed");
        let right = before
            .iter()
            .map(|o| o.rect.x + o.rect.width)
            .max()
            .unwrap_or(0);
        assert_eq!(made.rect.x, right, "beside the others, not on top");
        let mut stream = Vnc.start(&name, &Target::Viewer).expect("wayvnc");
        let mut sock = None;
        for _ in 0..50 {
            std::thread::sleep(std::time::Duration::from_millis(100));
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
            (super::super::output::MODE.0, super::super::output::MODE.1)
        );
        assert!(stream.running());
        stream.stop();
        assert!(!stream.running());
        super::super::output::remove(&name).expect("remove");
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
