//! The animated wallpaper on battery (`look.wallpaper_on_battery`): keep it
//! playing, slow it to a stop, or fade it to black with the shell in dark
//! mode. Every change is eased, both ways.
//!
//! The wallpaper video is mpvpaper's, a systemd user service this process
//! does not own. The NixOS side starts it with mpv's JSON IPC on
//! `$XDG_RUNTIME_DIR/mpvpaper.sock`, and this service moves its `speed`,
//! `pause` and video track over it:
//!
//! - **pause**: `speed` eases from the player's own speed (its base, read
//!   from it, never assumed) down to mpv's floor of 0.01 over
//!   [`SPEED_SPAN`], fast at first and settling, and only then is `pause`
//!   set. Back on mains, `pause` goes first and the speed climbs from the
//!   floor, slowly at first.
//! - **black**: a black curtain fades in over the video
//!   (`services::wallpaper_curtain`, a layer surface eased on GTK's frame
//!   clock) over [`FADE_SPAN`], with the video playing on at its own
//!   speed under it: only `pause` slows down and speeds up. Once the
//!   curtain is up the player is paused, its video track dropped with
//!   mpv told to draw black without one (so mpvpaper's own buffer, which the
//!   lock screen shows, is black too), sway's background set to solid black,
//!   and the curtain lifted off a picture that is black under it. The shell
//!   goes dark in the same moment the curtain starts, through the theme's
//!   own cross-fade (`theme::set_battery_dark`, an override over the Look
//!   mode rather than a write to it). Back on mains the order reverses: the
//!   curtain goes up opaque over the black, the wallpaper pick goes back,
//!   the video track comes back and the player resumes under it; once mpv
//!   shows a frame again the curtain fades away over the video playing at
//!   its base speed, and the mode is let go in the same moment.
//!
//! The fade is not the player's own (contrast and saturation over IPC, as it
//! once was) because mpv applies those only when it renders a video frame:
//! 15 a second at the wallpaper's half speed, fewer while it slows, so the
//! fade stepped with the video.
//!
//! Every transition starts from what the player says it is at now, read
//! over the socket, and from what this service last told the curtain, so a
//! transition cut off by the opposite one turns around where it is rather
//! than from its end (the curtain from its current opacity, for its share
//! of the span), and a player that restarted (it comes up playing, at its
//! own speed) is taken from where it is to the state wanted.
//!
//! What this service did to a player is kept on the player, in mpv's
//! `user-data`: its base speed, and whether the pause on it is this
//! service's. Both die with the player, so a restarted mpvpaper has neither
//! and a restarted panel finds both. Resuming is only ever undoing this
//! service's own pause: a player paused by anything else (mpvpaper's own
//! `-p` while the screen is off, a person over IPC) stays paused, though its
//! speed still goes back, since nothing else moves it.
//!
//! Three things re-assert the state: a battery change (`services::battery`,
//! UPower), a settings change, and a 30-second tick while the wallpaper is
//! not meant to be playing, for a player that restarted (nothing announces
//! it) or that something else resumed. Polling only then keeps a machine on
//! mains free of the wake-up.
//!
//! The socket runs on a worker thread, one connection per transition: the
//! GTK thread sends it the wanted state and tells it when the curtain is
//! up; the worker sends the curtain what to do. A missing socket is the
//! normal state of a session without the animated wallpaper and is logged
//! at debug only; `black` then still turns the background black.

use std::cell::{Cell, RefCell};
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::services::battery;
use crate::services::wallpaper_curtain::{self, Curtain};
use crate::settings::store::{OnBattery, Wallpaper};

/// How often the state is re-asserted while the wallpaper is not meant to
/// be playing.
const TICK_S: u32 = 30;

/// mpv's lowest speed: it refuses anything under it.
const FLOOR: f64 = 0.01;

/// Slowing to a stop, and starting up again, from the base speed.
const SPEED_SPAN: Duration = Duration::from_millis(2500);

/// The curtain's fade to black and back, the whole way; the speed falls
/// under it over the same span.
const FADE_SPAN: Duration = Duration::from_millis(1500);

/// Between two speed writes: one per refresh of a 60 Hz output.
const STEP: Duration = Duration::from_micros(16_667);

/// How long a picture mpv changed gets to reach the screen: mpvpaper
/// redraws, commits, and the compositor presents. Measured in a nested
/// sway: black about 80 ms after the video track goes.
const SETTLE: Duration = Duration::from_millis(150);

/// The same for a frame of video that just came back, which the curtain
/// starts uncovering slowly anyway.
const SHOWN: Duration = Duration::from_millis(50);

/// The longest wait for mpv to show a frame, or stop showing one, before
/// carrying on regardless. A video track coming back takes about 0.9 s on
/// the 4K loop (seek, decoder, first frame), measured in a nested sway.
const FRAME_WAIT: Duration = Duration::from_secs(3);

/// Between two looks at whether mpv shows a frame.
const POLL: Duration = Duration::from_millis(10);

/// The longest wait for the curtain to say it is opaque. The curtain lands
/// on its own 250 ms after its span even with no frame clock ticking; this
/// is for a GTK thread that never answers at all.
const COVER_WAIT: Duration = Duration::from_secs(10);

/// Below this, two values are the same one.
const EPS: f64 = 1e-3;

/// The player's `user-data` keys this service keeps its own marks under.
const BASE_KEY: &str = "user-data/swaypplet/base-speed";
const HELD_KEY: &str = "user-data/swaypplet/held";

/// What the wallpaper should be doing now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Play,
    Pause,
    Black,
}

/// The setting, on battery; playing otherwise. "On battery" is no charger
/// connected (`BatteryState::on_battery`), not the battery's charge state,
/// which flips at a charge threshold while plugged in.
fn wanted(setting: OnBattery, on_battery: Option<bool>) -> Target {
    if on_battery != Some(true) {
        return Target::Play;
    }
    match setting {
        OnBattery::Keep => Target::Play,
        OnBattery::Pause => Target::Pause,
        OnBattery::Black => Target::Black,
    }
}

/// The player as read off its socket.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Player {
    /// The speed it plays at when left alone.
    base: f64,
    speed: f64,
    paused: bool,
    /// The pause on it is this service's.
    held: bool,
    /// A video track is selected (`vid` is not `no`).
    video: bool,
}

/// A discrete step around the eased part of a transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cmd {
    /// Put the person's wallpaper back on sway's background.
    Restore,
    /// Sway's background to solid black.
    Blackout,
    /// Select the video track again; or none, with mpv drawing black.
    Video(bool),
    /// Pause, or undo this service's pause.
    Pause(bool),
    /// Wait until mpv shows a frame of video (`true`), or has stopped
    /// showing one and drawn black (`false`).
    Frame(bool),
    /// Start the curtain's fade in.
    FadeIn,
    /// Wait until the curtain is up.
    Covered,
    /// The curtain opaque at once, over a picture that is black, and wait
    /// until it is on screen.
    Cover,
    /// Start the curtain's fade away, and let the shell's mode go. Sent on
    /// every transition to a target that is not black, whether a curtain is
    /// up or not: the mode was set dark by the GTK thread when black was
    /// wanted, and this is the one moment it goes back.
    FadeOut,
    /// Take the curtain away at once: the picture under it is black.
    Lift,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Curve {
    /// Fast at first, settling: slowing to a halt.
    Out,
    /// Slow at first, gathering: starting up.
    In,
}

fn ease(curve: Curve, t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    match curve {
        Curve::Out => 1.0 - (1.0 - t).powi(3),
        Curve::In => t.powi(3),
    }
}

/// The speed eased from where it is to where it is going.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Leg {
    from: f64,
    to: f64,
    span: Duration,
    curve: Curve,
}

impl Leg {
    /// `full` is the span over the whole `range`; a shorter distance, as a
    /// transition turned around midway has, takes its share of it.
    fn new(from: f64, to: f64, full: Duration, range: f64, curve: Curve) -> Leg {
        let share = ((to - from).abs() / range.max(EPS)).min(1.0);
        Leg {
            from,
            to,
            span: full.mul_f64(share),
            curve,
        }
    }

    fn moves(&self) -> bool {
        (self.to - self.from).abs() > EPS
    }

    fn at(&self, t: Duration) -> f64 {
        if self.span.is_zero() || t >= self.span {
            return self.to;
        }
        let p = t.as_secs_f64() / self.span.as_secs_f64();
        self.from + (self.to - self.from) * ease(self.curve, p)
    }
}

/// A transition: the discrete steps before, the eased speed, the steps
/// after.
#[derive(Debug, Clone, PartialEq)]
struct Plan {
    before: Vec<Cmd>,
    speed: Leg,
    after: Vec<Cmd>,
}

impl Plan {
    /// Nothing to do but say so: a [`Cmd::FadeOut`] alone is that.
    fn is_empty(&self) -> bool {
        let idle = |cmds: &[Cmd]| cmds.iter().all(|c| *c == Cmd::FadeOut);
        idle(&self.before) && idle(&self.after) && !self.speed.moves()
    }
}

/// From the player as it is, sway's background (black or not) and the
/// curtain (up or not, as last told) to `target`.
fn plan(p: &Player, bg_black: bool, curtain: bool, target: Target) -> Plan {
    // Only "pause" slows the video to a stop and back. Going black, it plays
    // on at its own speed under the curtain and stops once covered; coming
    // back, it plays at its base speed as the curtain lifts.
    let speed_to = match target {
        Target::Play => p.base,
        Target::Pause => FLOOR,
        Target::Black => p.speed,
    };
    let mut before = Vec::new();
    let mut after = Vec::new();
    if target == Target::Black {
        if p.video {
            before.push(Cmd::FadeIn);
            after.push(Cmd::Covered);
        }
        if !p.paused {
            after.push(Cmd::Pause(true));
        }
        if p.video {
            after.push(Cmd::Video(false));
            after.push(Cmd::Frame(false));
        }
        if !bg_black {
            after.push(Cmd::Blackout);
        }
        if p.video || curtain {
            after.push(Cmd::Lift);
        }
    } else {
        // No video means mpv draws black: the curtain goes up over it at
        // once, and everything comes back under it.
        let hidden = !p.video;
        if hidden {
            before.push(Cmd::Cover);
        }
        if bg_black || hidden {
            before.push(Cmd::Restore);
        }
        if hidden {
            before.push(Cmd::Video(true));
        }
        if target == Target::Play && p.paused && p.held {
            before.push(Cmd::Pause(false));
        }
        if hidden {
            before.push(Cmd::Frame(true));
        }
        before.push(Cmd::FadeOut);
        if target == Target::Pause && !p.paused {
            after.push(Cmd::Pause(true));
        }
    }
    let curve = if speed_to > p.speed {
        Curve::In
    } else {
        Curve::Out
    };
    // Back from black, the base speed is set at once, before the curtain
    // lifts; easing it there would be the slow start this mode leaves out.
    let from_black = target == Target::Play && (!p.video || bg_black);
    let full = if from_black {
        Duration::ZERO
    } else {
        SPEED_SPAN
    };
    let speed = Leg::new(p.speed, speed_to, full, p.base - FLOOR, curve);
    Plan {
        before,
        speed,
        after,
    }
}

/// One speed write: when, and the value.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Step {
    at: Duration,
    speed: f64,
}

/// The eased speed as writes about [`STEP`] apart, its exact end value
/// last.
fn schedule(leg: &Leg) -> Vec<Step> {
    if !leg.moves() {
        return Vec::new();
    }
    let n = ((leg.span.as_secs_f64() / STEP.as_secs_f64()).round() as u32).max(1);
    (1..=n)
        .map(|i| {
            let at = leg.span.mul_f64(f64::from(i) / f64::from(n));
            Step {
                at,
                speed: leg.at(at),
            }
        })
        .collect()
}

// ── The socket ──────────────────────────────────────────────────────────

/// One connection to mpv's JSON IPC: a line out, lines back until the one
/// that answers it (mpv also sends events on the same socket).
struct Ipc {
    stream: UnixStream,
    reader: BufReader<UnixStream>,
    next: u64,
}

impl Ipc {
    fn connect(path: &Path) -> io::Result<Ipc> {
        let stream = UnixStream::connect(path)?;
        stream.set_write_timeout(Some(Duration::from_secs(1)))?;
        stream.set_read_timeout(Some(Duration::from_secs(1)))?;
        let reader = BufReader::new(stream.try_clone()?);
        Ok(Ipc {
            stream,
            reader,
            next: 1,
        })
    }

    /// mpv's answer: its `data` on success, its error string otherwise.
    fn request(&mut self, command: Value) -> io::Result<Result<Value, String>> {
        let id = self.next;
        self.next += 1;
        writeln!(
            self.stream,
            "{}",
            json!({ "command": command, "request_id": id })
        )?;
        let mut line = String::new();
        loop {
            line.clear();
            if self.reader.read_line(&mut line)? == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            let Ok(reply) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if reply.get("request_id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            return Ok(match reply.get("error").and_then(Value::as_str) {
                Some("success") => Ok(reply.get("data").cloned().unwrap_or(Value::Null)),
                other => Err(other.unwrap_or("no error field").to_string()),
            });
        }
    }

    fn get(&mut self, property: &str) -> io::Result<Option<Value>> {
        Ok(self.request(json!(["get_property", property]))?.ok())
    }

    fn set(&mut self, property: &str, value: Value) -> io::Result<()> {
        if let Err(e) = self.request(json!(["set_property", property, value]))? {
            log::debug!("wallpaper-battery: {property} = {value}: {e}");
        }
        Ok(())
    }

    /// Whether mpv shows a frame of video: `video-frame-info` is there
    /// exactly while one is displayed, and turns up the moment a returning
    /// track's first frame does (measured: within a refresh of it reaching
    /// the screen), where `vo-configured` and `video-out-params` outlive the
    /// track.
    fn shows_frame(&mut self) -> io::Result<bool> {
        Ok(self.get("video-frame-info")?.is_some_and(|v| !v.is_null()))
    }
}

/// The player as it is, its base speed marked on it if it was not yet.
fn read(ipc: &mut Ipc) -> io::Result<Player> {
    let number = |v: Option<Value>| v.as_ref().and_then(Value::as_f64);
    let speed = number(ipc.get("speed")?).ok_or_else(|| io::Error::other("no speed"))?;
    let base = match number(ipc.get(BASE_KEY)?) {
        Some(base) => base,
        None if speed > FLOOR + EPS => {
            // Never touched by this service: what it plays at is its base.
            ipc.set(BASE_KEY, json!(speed))?;
            speed
        }
        None => {
            // At the floor with no mark: slowed by something that kept no
            // record (an mpv without user-data). mpv's own default, then.
            log::warn!("wallpaper-battery: player at its floor with no base speed; using 1.0");
            1.0
        }
    };
    let paused = ipc.get("pause")?.and_then(|v| v.as_bool()).unwrap_or(false);
    let held = ipc
        .get(HELD_KEY)?
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let video = ipc.get("vid")? != Some(Value::Bool(false));
    Ok(Player {
        base,
        speed,
        paused,
        held,
        video,
    })
}

// ── The worker ──────────────────────────────────────────────────────────

/// What the GTK thread sends: the state wanted, and the wallpaper to put
/// back when the background leaves black (the pick, or `None` for the
/// config's own).
#[derive(Debug, Clone)]
struct Want {
    target: Target,
    restore: Option<Wallpaper>,
}

/// Into the worker.
#[derive(Debug)]
enum Msg {
    Want(Want),
    /// The curtain asked for under this number is opaque on screen.
    Covered(u64),
}

/// Out of the worker, to the curtain on the GTK thread; the number is the
/// one to answer [`Msg::Covered`] with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ToCurtain(wallpaper_curtain::Cmd, u64);

/// Where a black background this service set is remembered across panel
/// restarts. In the runtime directory, so a new login starts without it, as
/// it starts with swaybg showing the wallpaper.
fn black_marker() -> PathBuf {
    glib::user_runtime_dir()
        .join("swaypplet")
        .join("wallpaper-black")
}

fn remember_black(black: bool) {
    let path = black_marker();
    let result = if black {
        path.parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&path, b""))
    } else {
        match std::fs::remove_file(&path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            other => other,
        }
    };
    if let Err(e) = result {
        log::warn!("wallpaper-battery: {}: {e}", path.display());
    }
}

fn restore(pick: Option<&Wallpaper>) {
    let wallpaper = pick
        .cloned()
        .or_else(crate::settings::wallpaper::system_default);
    let Some(w) = wallpaper else {
        log::warn!("wallpaper-battery: no wallpaper to put back");
        return;
    };
    match crate::settings::wallpaper::apply_blocking(&w) {
        Ok(()) => remember_black(false),
        Err(e) => log::warn!("wallpaper-battery: {e}"),
    }
}

fn blackout() -> bool {
    match crate::sway::ipc::run_command_blocking("output * bg #000000 solid_color") {
        Ok(()) => {
            remember_black(true);
            true
        }
        Err(e) => {
            log::warn!("wallpaper-battery: {e}");
            false
        }
    }
}

/// How a wait ended.
enum Wake {
    /// Its time came.
    Due,
    /// The curtain it waited on is up.
    Covered,
    /// Another target is wanted: turn around to it.
    Turn(Want),
    /// The GTK side is gone.
    Gone,
}

/// Whether a step lets the transition carry on.
enum Go {
    On,
    Turn(Want),
    Stop,
}

struct Worker {
    rx: Receiver<Msg>,
    curtain: Box<dyn Fn(ToCurtain) + Send>,
    /// Sway's background is the compositor's, not the player's, and sway
    /// cannot be asked what it shows: whether this service made it black is
    /// kept here and in [`black_marker`]. The file is what a panel restarted
    /// while black (an `nx` switch restarts it) starts from; without it the
    /// new process never put the wallpaper back.
    bg_black: bool,
    /// The curtain was last told to come up (in or cover), not to go.
    up: bool,
    /// The number of the last curtain asked for.
    seq: u64,
    /// The number of the last curtain that said it is up. Kept rather than
    /// matched only while waiting: the curtain can land before the speed
    /// under it does, while the worker is waiting on the speed's clock.
    covered: Cell<u64>,
}

impl Worker {
    fn tell(&mut self, cmd: wallpaper_curtain::Cmd) -> u64 {
        use wallpaper_curtain::Cmd as C;
        self.seq += 1;
        self.up = matches!(cmd, C::In | C::Cover);
        (self.curtain)(ToCurtain(cmd, self.seq));
        self.seq
    }

    /// Wait until `until`, or until the curtain numbered `covered` is up,
    /// or until another target is wanted. The same target again (a tick, a
    /// settings change beside it) carries on, with the newest wallpaper to
    /// put back.
    fn wait(&self, want: &mut Want, until: Instant, covered: Option<u64>) -> Wake {
        loop {
            let now = Instant::now();
            if now >= until {
                return Wake::Due;
            }
            match self.rx.recv_timeout(until - now) {
                Ok(Msg::Want(next)) if next.target == want.target => want.restore = next.restore,
                Ok(Msg::Want(next)) => return Wake::Turn(next),
                Ok(Msg::Covered(n)) => {
                    self.covered.set(self.covered.get().max(n));
                    if covered.is_some_and(|c| c <= n) {
                        return Wake::Covered;
                    }
                }
                Err(RecvTimeoutError::Timeout) => return Wake::Due,
                Err(RecvTimeoutError::Disconnected) => return Wake::Gone,
            }
        }
    }

    /// Wait for the curtain numbered `n`; a curtain that never answers is
    /// given up on after [`COVER_WAIT`].
    fn await_cover(&self, want: &mut Want, n: u64) -> Go {
        if self.covered.get() >= n {
            return Go::On;
        }
        match self.wait(want, Instant::now() + COVER_WAIT, Some(n)) {
            Wake::Covered => Go::On,
            Wake::Due => {
                log::warn!("wallpaper-battery: the curtain did not answer; carrying on");
                Go::On
            }
            Wake::Turn(next) => Go::Turn(next),
            Wake::Gone => Go::Stop,
        }
    }

    /// Wait until mpv shows a frame (`shown`) or does not, then for it to
    /// reach the screen.
    fn await_frame(&self, ipc: &mut Ipc, want: &mut Want, shown: bool) -> io::Result<Go> {
        let give_up = Instant::now() + FRAME_WAIT;
        while ipc.shows_frame()? != shown {
            if Instant::now() >= give_up {
                log::debug!("wallpaper-battery: no change of frame in {FRAME_WAIT:?}; carrying on");
                break;
            }
            match self.wait(want, Instant::now() + POLL, None) {
                Wake::Due | Wake::Covered => {}
                Wake::Turn(next) => return Ok(Go::Turn(next)),
                Wake::Gone => return Ok(Go::Stop),
            }
        }
        let land = if shown { SHOWN } else { SETTLE };
        Ok(match self.wait(want, Instant::now() + land, None) {
            Wake::Due | Wake::Covered => Go::On,
            Wake::Turn(next) => Go::Turn(next),
            Wake::Gone => Go::Stop,
        })
    }

    fn run(&mut self, cmd: Cmd, ipc: &mut Ipc, want: &mut Want) -> io::Result<Go> {
        use wallpaper_curtain::Cmd as C;
        log::debug!("wallpaper-battery: {cmd:?}");
        match cmd {
            Cmd::Restore => {
                restore(want.restore.as_ref());
                self.bg_black = false;
            }
            Cmd::Blackout => self.bg_black = blackout(),
            Cmd::Video(true) => {
                ipc.set("vid", json!("auto"))?;
                ipc.set("force-window", json!("no"))?;
            }
            Cmd::Video(false) => {
                // With no track and no window forced, mpv draws nothing and
                // mpvpaper keeps the last frame it drew: the video, which
                // the lock screen would then show (measured: grey 147 of
                // 255 after `vid` went). A forced window draws its
                // background instead, black, within about 80 ms.
                ipc.set("background-color", json!("#000000"))?;
                ipc.set("force-window", json!("yes"))?;
                ipc.set("vid", json!("no"))?;
            }
            Cmd::Pause(on) => {
                ipc.set("pause", json!(on))?;
                ipc.set(HELD_KEY, json!(on))?;
            }
            Cmd::Frame(shown) => return self.await_frame(ipc, want, shown),
            Cmd::FadeIn => {
                self.tell(C::In);
            }
            Cmd::Covered => {
                let n = self.seq;
                return Ok(self.await_cover(want, n));
            }
            Cmd::Cover => {
                let n = self.tell(C::Cover);
                return Ok(self.await_cover(want, n));
            }
            Cmd::FadeOut => {
                self.tell(C::Out);
            }
            Cmd::Lift => {
                self.tell(C::Lift);
            }
        }
        Ok(Go::On)
    }

    /// Take the player to `want`. Returns a newer want for another target if
    /// one came in midway, for the caller to turn around to.
    fn settle(&mut self, path: &Path, want: &mut Want) -> Option<Want> {
        use wallpaper_curtain::Cmd as C;
        let mut ipc = match Ipc::connect(path) {
            Ok(ipc) => ipc,
            Err(e) => {
                log::debug!("wallpaper-battery: no player at {}: {e}", path.display());
                // No video: the background alone, and no curtain left up.
                if want.target == Target::Black {
                    if !self.bg_black {
                        self.bg_black = blackout();
                    }
                    if self.up {
                        self.tell(C::Lift);
                    }
                } else {
                    if self.bg_black {
                        restore(want.restore.as_ref());
                        self.bg_black = false;
                    }
                    self.tell(C::Out);
                }
                return None;
            }
        };
        match self.transition(&mut ipc, want) {
            Ok(next) => next,
            Err(e) => {
                log::debug!("wallpaper-battery: {}: {e}", path.display());
                None
            }
        }
    }

    fn transition(&mut self, ipc: &mut Ipc, want: &mut Want) -> io::Result<Option<Want>> {
        macro_rules! step {
            ($go:expr) => {
                match $go {
                    Go::On => {}
                    Go::Turn(next) => return Ok(Some(next)),
                    Go::Stop => return Ok(None),
                }
            };
        }
        let player = read(ipc)?;
        let plan = plan(&player, self.bg_black, self.up, want.target);
        if plan.is_empty() {
            for cmd in plan.before.iter().chain(&plan.after) {
                step!(self.run(*cmd, ipc, want)?);
            }
            return Ok(None);
        }
        log::info!(
            "wallpaper-battery: to {:?} from speed {:.2}{}{}",
            want.target,
            player.speed,
            if player.paused { ", paused" } else { "" },
            if player.video { "" } else { ", no video" },
        );
        for cmd in &plan.before {
            step!(self.run(*cmd, ipc, want)?);
        }
        let start = Instant::now();
        for s in schedule(&plan.speed) {
            match self.wait(want, start + s.at, None) {
                Wake::Due | Wake::Covered => {}
                Wake::Turn(next) => return Ok(Some(next)),
                Wake::Gone => return Ok(None),
            }
            ipc.set("speed", json!(s.speed))?;
        }
        for cmd in &plan.after {
            step!(self.run(*cmd, ipc, want)?);
        }
        Ok(None)
    }
}

/// The worker: wanted states in, transitions out, one at a time and the
/// newest first.
fn worker(path: PathBuf, rx: Receiver<Msg>, curtain: Box<dyn Fn(ToCurtain) + Send>) {
    let mut w = Worker {
        rx,
        curtain,
        bg_black: black_marker().exists(),
        up: false,
        seq: 0,
        covered: Cell::new(0),
    };
    let mut want = loop {
        match w.rx.recv() {
            Ok(Msg::Want(want)) => break want,
            Ok(Msg::Covered(_)) => {}
            Err(_) => return,
        }
    };
    loop {
        while let Ok(msg) = w.rx.try_recv() {
            match msg {
                Msg::Want(newer) => want = newer,
                Msg::Covered(n) => w.covered.set(w.covered.get().max(n)),
            }
        }
        if let Some(next) = w.settle(&path, &mut want) {
            want = next;
            continue;
        }
        want = loop {
            match w.rx.recv() {
                Ok(Msg::Want(next)) => break next,
                Ok(Msg::Covered(n)) => w.covered.set(w.covered.get().max(n)),
                Err(_) => return,
            }
        };
    }
}

thread_local! {
    /// The target `SWAYPPLET_WALLPAPER_SCRIPT` has put in place of the
    /// battery's, while it runs.
    static SCRIPTED: Cell<Option<Target>> = const { Cell::new(None) };
}

/// `SWAYPPLET_WALLPAPER_SCRIPT="black:6,play:6"`: a test hook for the
/// nested-sway check, which has no battery to unplug. Each target in turn
/// stands in for the battery's for that many seconds, through the same path
/// a real unplug takes; the last one stays.
fn script() -> Option<Vec<(Target, f64)>> {
    let raw = std::env::var("SWAYPPLET_WALLPAPER_SCRIPT").ok()?;
    let steps: Option<Vec<_>> = raw
        .split(',')
        .map(|item| {
            let (name, secs) = item.trim().split_once(':')?;
            let target = match name {
                "play" => Target::Play,
                "pause" => Target::Pause,
                "black" => Target::Black,
                _ => return None,
            };
            Some((target, secs.parse::<f64>().ok()?))
        })
        .collect();
    if steps.is_none() {
        log::warn!("wallpaper-battery: SWAYPPLET_WALLPAPER_SCRIPT={raw:?} is not target:seconds,…");
    }
    steps
}

/// Start following, from the panel process. Does nothing on a machine
/// without a battery, where nothing here can ever be wanted.
pub fn follow(app: &gtk4::Application) {
    let script = script();
    if !battery::start() && script.is_none() {
        return;
    }
    let path = glib::user_runtime_dir().join("mpvpaper.sock");
    let (tx, rx) = mpsc::channel::<Msg>();
    let (to_gtk, from_worker) = async_channel::unbounded::<ToCurtain>();
    let tell: Box<dyn Fn(ToCurtain) + Send> = Box::new(move |c| {
        let _ = to_gtk.send_blocking(c);
    });
    let spawned = std::thread::Builder::new()
        .name("wallpaper-battery".into())
        .spawn(move || worker(path, rx, tell));
    if let Err(e) = spawned {
        log::warn!("wallpaper-battery: failed to spawn thread: {e}");
        return;
    }

    let curtain = Curtain::new(app, FADE_SPAN.as_secs_f64() * 1000.0);
    // Black is wanted, as the GTK thread last saw it: a curtain told to go
    // by a transition this one has overtaken must not lighten the shell.
    let black_wanted = Rc::new(Cell::new(false));
    {
        let tx = tx.clone();
        let black_wanted = black_wanted.clone();
        glib::spawn_future_local(async move {
            while let Ok(ToCurtain(cmd, n)) = from_worker.recv().await {
                if cmd == wallpaper_curtain::Cmd::Out && !black_wanted.get() {
                    // The picture comes back as the curtain starts going:
                    // the mode goes with it.
                    crate::theme::set_battery_dark(false);
                }
                let tx = tx.clone();
                curtain.apply(cmd, move || {
                    let _ = tx.send(Msg::Covered(n));
                });
            }
        });
    }

    let want_now = || {
        let (setting, restore) =
            crate::settings::store::with(|s| (s.look().wallpaper_on_battery, s.wallpaper.clone()));
        let target = SCRIPTED
            .with(Cell::get)
            .unwrap_or_else(|| wanted(setting, battery::current().map(|b| b.on_battery())));
        Want { target, restore }
    };
    let tick: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();
    let apply = Rc::new(move || {
        let want = want_now();
        let still = want.target != Target::Play;
        let black = want.target == Target::Black;
        black_wanted.set(black);
        // The shell darkens as the curtain starts, through the theme's own
        // cross-fade. It lightens when the worker lets the curtain go.
        if black {
            crate::theme::set_battery_dark(true);
        }
        let _ = tx.send(Msg::Want(want));
        let mut t = tick.borrow_mut();
        match (still, t.is_some()) {
            (true, false) => {
                let tx = tx.clone();
                *t = Some(glib::timeout_add_seconds_local(TICK_S, move || {
                    let _ = tx.send(Msg::Want(want_now()));
                    glib::ControlFlow::Continue
                }));
            }
            (false, true) => {
                if let Some(id) = t.take() {
                    crate::spawn::remove_source(id);
                }
            }
            _ => {}
        }
    });
    if let Some(steps) = script {
        let mut at = Duration::ZERO;
        for (target, secs) in steps {
            let apply = apply.clone();
            glib::timeout_add_local_once(at, move || {
                log::info!("wallpaper-battery: script: {target:?}");
                SCRIPTED.with(|s| s.set(Some(target)));
                apply();
            });
            at += Duration::from_secs_f64(secs);
        }
    }
    apply();
    {
        let apply = apply.clone();
        battery::observe(move || apply());
    }
    crate::settings::store::observe(move || apply());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn playing(base: f64) -> Player {
        Player {
            base,
            speed: base,
            paused: false,
            held: false,
            video: true,
        }
    }

    fn paused_here(base: f64) -> Player {
        Player {
            speed: FLOOR,
            paused: true,
            held: true,
            ..playing(base)
        }
    }

    fn black_here(base: f64) -> Player {
        Player {
            video: false,
            ..paused_here(base)
        }
    }

    /// What going black does after the speed, from a playing player.
    const DOWN: [Cmd; 6] = [
        Cmd::Covered,
        Cmd::Pause(true),
        Cmd::Video(false),
        Cmd::Frame(false),
        Cmd::Blackout,
        Cmd::Lift,
    ];

    #[test]
    fn only_running_on_the_battery_changes_anything() {
        let on = Some(true);
        assert_eq!(wanted(OnBattery::Keep, on), Target::Play);
        assert_eq!(wanted(OnBattery::Pause, on), Target::Pause);
        assert_eq!(wanted(OnBattery::Black, on), Target::Black);
        for setting in OnBattery::ALL {
            assert_eq!(wanted(setting, Some(false)), Target::Play);
            assert_eq!(wanted(setting, None), Target::Play);
        }
    }

    #[test]
    fn the_curves_run_end_to_end_the_right_way() {
        for curve in [Curve::Out, Curve::In] {
            assert_eq!(ease(curve, 0.0), 0.0);
            assert_eq!(ease(curve, 1.0), 1.0);
            let mut last = 0.0;
            for i in 1..=100 {
                let v = ease(curve, f64::from(i) / 100.0);
                assert!(v >= last, "{curve:?} goes back at {i}");
                last = v;
            }
        }
        // Slowing: most of the drop early. Starting: most of the rise late.
        assert!(ease(Curve::Out, 0.5) > 0.8);
        assert!(ease(Curve::In, 0.5) < 0.2);
    }

    #[test]
    fn slowing_down_is_a_write_per_refresh_from_the_base_to_the_floor_then_a_pause() {
        let p = plan(&playing(0.5), false, false, Target::Pause);
        // No curtain to take away, but the mode is let go all the same.
        assert_eq!(p.before, vec![Cmd::FadeOut]);
        assert_eq!(p.after, vec![Cmd::Pause(true)]);
        assert_eq!(p.speed.span, SPEED_SPAN);
        assert_eq!(p.speed.curve, Curve::Out);
        let steps = schedule(&p.speed);
        assert_eq!(steps.len(), 150);
        let speeds: Vec<f64> = steps.iter().map(|s| s.speed).collect();
        assert!(speeds.windows(2).all(|w| w[1] < w[0]), "{speeds:?}");
        assert_eq!(*speeds.last().unwrap(), FLOOR);
        assert_eq!(steps.last().unwrap().at, SPEED_SPAN);
        // The base is the player's, whatever it is.
        let p = plan(&playing(1.25), false, false, Target::Pause);
        assert_eq!(p.speed.from, 1.25);
    }

    #[test]
    fn starting_up_undoes_our_pause_first_then_climbs_slowly() {
        let p = plan(&paused_here(0.5), false, false, Target::Play);
        assert_eq!(p.before, vec![Cmd::Pause(false), Cmd::FadeOut]);
        assert!(p.after.is_empty());
        assert_eq!((p.speed.from, p.speed.to), (FLOOR, 0.5));
        assert_eq!(p.speed.curve, Curve::In);
        assert_eq!(p.speed.span, SPEED_SPAN);
        let steps = schedule(&p.speed);
        let first = steps[0].speed;
        assert!(first - FLOOR < 0.001, "starts slow: {first}");
    }

    #[test]
    fn a_pause_that_is_not_ours_is_left_alone() {
        let theirs = Player {
            held: false,
            ..paused_here(0.5)
        };
        let p = plan(&theirs, false, false, Target::Play);
        assert_eq!(p.before, vec![Cmd::FadeOut], "never resumed");
        // Its speed is still ours to give back.
        assert!(p.speed.moves());
        // Already paused by someone else, going to Pause: no pause to send,
        // so none is held.
        let p = plan(
            &Player {
                paused: true,
                ..playing(0.5)
            },
            false,
            false,
            Target::Pause,
        );
        assert!(p.after.is_empty());
    }

    #[test]
    fn black_is_the_curtain_over_the_playing_video_then_a_stop_under_it() {
        let p = plan(&playing(0.5), false, false, Target::Black);
        assert_eq!(p.before, vec![Cmd::FadeIn]);
        // The video plays on at its speed while the curtain comes down.
        assert!(!p.speed.moves());
        assert!(schedule(&p.speed).is_empty());
        // Everything after waits for the curtain; the background only once
        // mpv draws black, and the curtain lifts off black.
        assert_eq!(p.after, DOWN.to_vec());
    }

    #[test]
    fn back_from_black_the_picture_returns_under_the_curtain_first() {
        let p = plan(&black_here(0.5), true, false, Target::Play);
        assert_eq!(
            p.before,
            vec![
                Cmd::Cover,
                Cmd::Restore,
                Cmd::Video(true),
                Cmd::Pause(false),
                Cmd::Frame(true),
                Cmd::FadeOut,
            ]
        );
        assert!(p.after.is_empty());
        // Back at its base speed at once, not a slow start: that is the
        // pause mode's.
        assert_eq!(p.speed.to, 0.5);
        assert!(p.speed.span.is_zero());
        // A panel that restarted while black does not know the background
        // is black; the video track being off says so.
        let p = plan(&black_here(0.5), false, false, Target::Play);
        assert_eq!(&p.before[..2], &[Cmd::Cover, Cmd::Restore]);
    }

    #[test]
    fn between_pause_and_black_on_battery() {
        // Paused, then black is picked: only the curtain moves.
        let p = plan(&paused_here(0.5), false, false, Target::Black);
        assert_eq!(p.before, vec![Cmd::FadeIn]);
        assert!(!p.speed.moves());
        assert_eq!(
            p.after,
            vec![
                Cmd::Covered,
                Cmd::Video(false),
                Cmd::Frame(false),
                Cmd::Blackout,
                Cmd::Lift,
            ]
        );
        // Black, then pause is picked: the picture comes back, still held.
        let p = plan(&black_here(0.5), true, false, Target::Pause);
        assert_eq!(
            p.before,
            vec![
                Cmd::Cover,
                Cmd::Restore,
                Cmd::Video(true),
                Cmd::Frame(true),
                Cmd::FadeOut,
            ]
        );
        assert!(!p.speed.moves());
        assert!(p.after.is_empty());
    }

    #[test]
    fn a_transition_cut_off_turns_around_where_it_is() {
        // Slowing down, cut off at 0.2 by mains: back up from 0.2, for the
        // share of the span that distance is.
        let midway = Player {
            speed: 0.2,
            ..playing(0.5)
        };
        let p = plan(&midway, false, false, Target::Play);
        assert_eq!(p.before, vec![Cmd::FadeOut]);
        assert!(p.after.is_empty());
        assert_eq!((p.speed.from, p.speed.to), (0.2, 0.5));
        let share = 0.3 / (0.5 - FLOOR);
        let span = SPEED_SPAN.as_secs_f64() * share;
        assert!((p.speed.span.as_secs_f64() - span).abs() < 1e-6);
        assert!(schedule(&p.speed)[0].speed > 0.2);
        // Going black, cut off by mains with the curtain half up: the
        // curtain goes back from where it is (the curtain's own business),
        // nothing to restore, nothing to cover.
        let p = plan(&midway, false, true, Target::Play);
        assert_eq!(p.before, vec![Cmd::FadeOut]);
        // Coming back from black, cut off by unplugging again with the
        // video back and the curtain going: it comes up again from where it
        // got to, and nothing is restored twice.
        let coming_back = Player {
            speed: 0.1,
            ..playing(0.5)
        };
        let p = plan(&coming_back, false, false, Target::Black);
        assert_eq!(p.before, vec![Cmd::FadeIn]);
        assert_eq!(p.after, DOWN.to_vec());
        // Cut off after the video went but before the curtain lifted: it
        // still lifts.
        let p = plan(&black_here(0.5), true, true, Target::Black);
        assert_eq!(p.after, vec![Cmd::Lift]);
    }

    #[test]
    fn a_restarted_player_is_taken_from_where_it_comes_up() {
        // mpvpaper restarted while black: it plays at its own speed, its
        // video on. The background is already black.
        let p = plan(&playing(0.5), true, false, Target::Black);
        assert_eq!(p.before, vec![Cmd::FadeIn]);
        assert!(!p.speed.moves());
        assert_eq!(
            p.after,
            vec![
                Cmd::Covered,
                Cmd::Pause(true),
                Cmd::Video(false),
                Cmd::Frame(false),
                Cmd::Lift,
            ]
        );
        // Restarted while a pause was wanted: slowed again from its speed.
        let p = plan(&playing(0.75), false, false, Target::Pause);
        assert_eq!(p.speed.from, 0.75);
        assert_eq!(p.after, vec![Cmd::Pause(true)]);
    }

    #[test]
    fn settled_is_nothing_to_do() {
        assert!(plan(&playing(0.5), false, false, Target::Play).is_empty());
        assert!(plan(&paused_here(0.5), false, false, Target::Pause).is_empty());
        assert!(plan(&black_here(0.5), true, false, Target::Black).is_empty());
        let p = plan(&playing(0.5), false, false, Target::Play);
        assert!(schedule(&p.speed).is_empty());
    }

    #[test]
    fn a_curtain_up_before_the_speed_is_down_is_not_waited_for_again() {
        let (tx, rx) = mpsc::channel::<Msg>();
        let mut w = Worker {
            rx,
            curtain: Box::new(|_| {}),
            bg_black: false,
            up: false,
            seq: 0,
            covered: Cell::new(0),
        };
        let n = w.tell(wallpaper_curtain::Cmd::In);
        let mut want = Want {
            target: Target::Black,
            restore: None,
        };
        // The curtain lands while the worker waits on the speed's clock.
        tx.send(Msg::Covered(n)).unwrap();
        assert!(matches!(
            w.wait(&mut want, Instant::now() + POLL, None),
            Wake::Due
        ));
        let started = Instant::now();
        assert!(matches!(w.await_cover(&mut want, n), Go::On));
        assert!(started.elapsed() < Duration::from_millis(100));
        // A newer curtain is not taken for up by an older answer.
        let newer = w.tell(wallpaper_curtain::Cmd::Cover);
        assert!(w.covered.get() < newer);
    }

    /// Drive a real player through a script of targets, without a curtain
    /// (one that is up at once), for a check of the worker alone:
    /// `WALLPAPER_SOCK=<mpv socket> WALLPAPER_SCRIPT="pause:4,play:1.2"`
    /// (each target held for that many seconds), with `SWAYSOCK` set for the
    /// background. The curtain itself needs the panel:
    /// `SWAYPPLET_WALLPAPER_SCRIPT`.
    #[test]
    #[ignore]
    fn drive_a_real_player() {
        let path = PathBuf::from(std::env::var("WALLPAPER_SOCK").expect("WALLPAPER_SOCK"));
        let script = std::env::var("WALLPAPER_SCRIPT").expect("WALLPAPER_SCRIPT");
        let (tx, rx) = mpsc::channel::<Msg>();
        let ack: mpsc::Sender<Msg> = tx.clone();
        let curtain: Box<dyn Fn(ToCurtain) + Send> = Box::new(move |ToCurtain(cmd, n)| {
            eprintln!("curtain: {cmd:?}");
            let _ = ack.send(Msg::Covered(n));
        });
        let worker = std::thread::spawn(move || worker(path, rx, curtain));
        let start = Instant::now();
        for item in script.split(',') {
            let (name, secs) = item.split_once(':').expect("target:seconds");
            let target = match name {
                "play" => Target::Play,
                "pause" => Target::Pause,
                "black" => Target::Black,
                other => panic!("no target {other}"),
            };
            eprintln!("t={:.2}s -> {target:?}", start.elapsed().as_secs_f64());
            tx.send(Msg::Want(Want {
                target,
                restore: None,
            }))
            .unwrap();
            std::thread::sleep(Duration::from_secs_f64(secs.parse().unwrap()));
        }
        // The curtain's sender keeps the channel open: the worker is left
        // to the end of the process rather than joined.
        drop(worker);
    }
}
