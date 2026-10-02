//! The animated wallpaper on battery (`look.wallpaper_on_battery`): keep it
//! playing, slow it to a stop, or fade it to black with the shell in dark
//! mode. Every change is eased, both ways.
//!
//! The wallpaper video is mpvpaper's, a systemd user service this process
//! does not own. The NixOS side starts it with mpv's JSON IPC on
//! `$XDG_RUNTIME_DIR/mpvpaper.sock`, and this service moves four of its
//! properties:
//!
//! - **pause**: `speed` eases from the player's own speed (its base, read
//!   from it, never assumed) down to mpv's floor of 0.01 over
//!   [`SPEED_SPAN`], fast at first and settling, and only then is `pause`
//!   set. Back on mains, `pause` goes first and the speed climbs from the
//!   floor, slowly at first.
//! - **black**: the picture fades to black over [`FADE_SPAN`] while the
//!   speed falls with it (see [`FADE`] for how); then the player is paused, `vid` is set to `no` (mpv
//!   stops decoding; mpvpaper keeps the black frame it last drew), and only
//!   then is sway's background set to solid black, under a video that is
//!   already black, so nothing flashes. The shell goes dark in the same
//!   moment the fade starts, through the theme's own cross-fade
//!   (`theme::set_battery_dark`, an override over the Look mode rather than a
//!   write to it). Back on mains the order reverses: the wallpaper pick goes
//!   back under the still-black video, the mode is let go, the video track
//!   comes back, and the picture and the speed climb back.
//!
//! Every transition starts from what the player says it is at now, read
//! over the socket, so a transition cut off by the opposite one turns
//! around where it is rather than from its end, and a player that restarted
//! (it comes up playing, at its own speed) is taken from where it is to the
//! state wanted.
//!
//! What this service did to a player is kept on the player, in mpv's
//! `user-data`: its base speed, and whether the pause on it is this
//! service's. Both die with the player, so a restarted mpvpaper has neither
//! and a restarted panel finds both. Resuming is only ever undoing this
//! service's own pause: a player paused by anything else (mpvpaper's own
//! `-p` while the screen is off, a person over IPC) stays paused, though its
//! speed and fade still go back, since nothing else moves those.
//!
//! Three things re-assert the state: a battery change (`services::battery`,
//! UPower), a settings change, and a 30-second tick while the wallpaper is
//! not meant to be playing, for a player that restarted (nothing announces
//! it) or that something else resumed. Polling only then keeps a machine on
//! mains free of the wake-up.
//!
//! The socket runs on a worker thread, one connection per transition: the
//! GTK thread only sends it the wanted state. A missing socket is the
//! normal state of a session without the animated wallpaper and is logged
//! at debug only; `black` then still turns the background black.

use std::cell::RefCell;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::services::battery;
use crate::services::power::ChargeState;
use crate::settings::store::{OnBattery, Wallpaper};

/// How often the state is re-asserted while the wallpaper is not meant to
/// be playing.
const TICK_S: u32 = 30;

/// mpv's lowest speed: it refuses anything under it.
const FLOOR: f64 = 0.01;

/// Slowing to a stop, and starting up again, from the base speed.
const SPEED_SPAN: Duration = Duration::from_millis(2500);

/// The fade to black and back, the whole way.
const FADE_SPAN: Duration = Duration::from_millis(1500);

/// Between two property writes: one per refresh of a 60 Hz output. At the
/// twelve a second this started with, the fade showed as visible steps of
/// contrast; a write per refresh is what makes each refresh a new value.
const STEP: Duration = Duration::from_micros(16_667);

/// How long the last fade write gets to reach the screen before the video
/// track is dropped. A paused player redraws its frame only when asked, and
/// with no track there is no frame to redraw: dropped at once, the picture
/// kept is the one before last (measured: grey up to 3 of 255, not black).
const SETTLE: Duration = Duration::from_millis(150);

/// The fade's level at black.
const DARK: f64 = -100.0;

/// The mpv properties the fade moves, together, from 0 to [`DARK`].
///
/// Not `brightness`: that adds an offset, so the picture clips to black
/// from the shadows up and is gone two thirds of the way down (measured on
/// the beach loop: mean grey 147 at 0, 26 at -45, black by -65). Contrast scales luma and saturation scales chroma, so both at
/// the same level multiply the picture: grey 147, 111, 74, 37, 0 at 0, -25,
/// -50, -75, -100, an even fade the whole way.
const FADE: [&str; 2] = ["contrast", "saturation"];

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

/// The setting, on battery; playing otherwise. Plugged in at a charge
/// threshold (`Idle`), full, or a state the firmware does not name is not
/// "on battery".
fn wanted(setting: OnBattery, state: Option<ChargeState>) -> Target {
    if state != Some(ChargeState::Discharging) {
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
    fade: f64,
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
    /// Select the video track again, or none.
    Video(bool),
    /// Pause, or undo this service's pause.
    Pause(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Curve {
    /// Fast at first, settling: slowing to a halt.
    Out,
    /// Slow at first, gathering: starting up.
    In,
    /// Both ends gentle: a fade.
    InOut,
}

fn ease(curve: Curve, t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    match curve {
        Curve::Out => 1.0 - (1.0 - t).powi(3),
        Curve::In => t.powi(3),
        Curve::InOut => t * t * (3.0 - 2.0 * t),
    }
}

/// One property eased from where it is to where it is going.
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

/// A transition: the discrete steps before, the eased part, the steps
/// after.
#[derive(Debug, Clone, PartialEq)]
struct Plan {
    before: Vec<Cmd>,
    speed: Leg,
    fade: Leg,
    after: Vec<Cmd>,
}

impl Plan {
    fn is_empty(&self) -> bool {
        self.before.is_empty() && self.after.is_empty() && !self.speed.moves() && !self.fade.moves()
    }
}

/// From the player as it is (and sway's background, black or not) to
/// `target`.
fn plan(p: &Player, bg_black: bool, target: Target) -> Plan {
    let black = target == Target::Black;
    let (speed_to, fade_to) = match target {
        Target::Play => (p.base, 0.0),
        Target::Pause => (FLOOR, 0.0),
        Target::Black => (FLOOR, DARK),
    };
    let mut before = Vec::new();
    if !black && (bg_black || !p.video) {
        // Under the still-black video, before anything of it shows.
        before.push(Cmd::Restore);
    }
    if !black && !p.video {
        before.push(Cmd::Video(true));
    }
    if target == Target::Play && p.paused && p.held {
        before.push(Cmd::Pause(false));
    }
    let curve = if speed_to > p.speed {
        Curve::In
    } else {
        Curve::Out
    };
    // Going black, the speed falls with the fade: nothing shows after it.
    let full = if black { FADE_SPAN } else { SPEED_SPAN };
    let speed = Leg::new(p.speed, speed_to, full, p.base - FLOOR, curve);
    let fade = Leg::new(p.fade, fade_to, FADE_SPAN, -DARK, Curve::InOut);
    let mut after = Vec::new();
    if target != Target::Play && !p.paused {
        after.push(Cmd::Pause(true));
    }
    if black {
        if p.video {
            after.push(Cmd::Video(false));
        }
        if !bg_black {
            after.push(Cmd::Blackout);
        }
    }
    Plan {
        before,
        speed,
        fade,
        after,
    }
}

/// One write of the eased part: when, and the values that moved.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Step {
    at: Duration,
    speed: Option<f64>,
    fade: Option<f64>,
}

/// The eased part as writes about [`STEP`] apart, evenly over the longest
/// leg, each leg's exact end value last and nothing written for a leg once
/// it is there.
fn schedule(plan: &Plan) -> Vec<Step> {
    let legs = [plan.speed, plan.fade].map(|l| l.moves().then_some(l));
    let end = legs.iter().flatten().map(|l| l.span).max();
    let Some(end) = end else {
        return Vec::new();
    };
    let mut last = [None::<f64>; 2];
    let mut steps = Vec::new();
    let n = ((end.as_secs_f64() / STEP.as_secs_f64()).round() as u32).max(1);
    for i in 1..=n {
        let at = end.mul_f64(f64::from(i) / f64::from(n));
        let mut values = [None; 2];
        for (k, leg) in legs.iter().enumerate() {
            let Some(leg) = leg else { continue };
            if last[k] == Some(leg.to) {
                continue;
            }
            let v = leg.at(at);
            last[k] = Some(v);
            values[k] = Some(v);
        }
        if values.iter().any(Option::is_some) {
            steps.push(Step {
                at,
                speed: values[0],
                fade: values[1],
            });
        }
    }
    steps
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
    let fade = number(ipc.get(FADE[0])?).unwrap_or(0.0);
    let paused = ipc.get("pause")?.and_then(|v| v.as_bool()).unwrap_or(false);
    let held = ipc
        .get(HELD_KEY)?
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let video = ipc.get("vid")? != Some(Value::Bool(false));
    Ok(Player {
        base,
        speed,
        fade,
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

fn restore(pick: Option<&Wallpaper>) {
    let wallpaper = pick
        .cloned()
        .or_else(crate::settings::wallpaper::system_default);
    let Some(w) = wallpaper else {
        log::warn!("wallpaper-battery: no wallpaper to put back");
        return;
    };
    if let Err(e) = crate::settings::wallpaper::apply_blocking(&w) {
        log::warn!("wallpaper-battery: {e}");
    }
}

fn blackout() -> bool {
    match crate::sway::ipc::run_command_blocking("output * bg #000000 solid_color") {
        Ok(()) => true,
        Err(e) => {
            log::warn!("wallpaper-battery: {e}");
            false
        }
    }
}

fn run(cmd: Cmd, ipc: &mut Ipc, want: &Want, bg_black: &mut bool) -> io::Result<()> {
    match cmd {
        Cmd::Restore => {
            restore(want.restore.as_ref());
            *bg_black = false;
        }
        Cmd::Blackout => *bg_black = blackout(),
        Cmd::Video(on) => ipc.set("vid", json!(if on { "auto" } else { "no" }))?,
        Cmd::Pause(on) => {
            ipc.set("pause", json!(on))?;
            ipc.set(HELD_KEY, json!(on))?;
        }
    }
    Ok(())
}

/// Take the player to `want`. Returns a newer want for another target if
/// one came in midway, for the caller to turn around to.
fn settle(path: &Path, want: &mut Want, bg_black: &mut bool, rx: &Receiver<Want>) -> Option<Want> {
    let mut ipc = match Ipc::connect(path) {
        Ok(ipc) => ipc,
        Err(e) => {
            log::debug!("wallpaper-battery: no player at {}: {e}", path.display());
            // No video: the background alone.
            match (want.target == Target::Black, *bg_black) {
                (true, false) => *bg_black = blackout(),
                (false, true) => {
                    restore(want.restore.as_ref());
                    *bg_black = false;
                }
                _ => {}
            }
            return None;
        }
    };
    match transition(&mut ipc, want, bg_black, rx) {
        Ok(next) => next,
        Err(e) => {
            log::debug!("wallpaper-battery: {}: {e}", path.display());
            None
        }
    }
}

fn transition(
    ipc: &mut Ipc,
    want: &mut Want,
    bg_black: &mut bool,
    rx: &Receiver<Want>,
) -> io::Result<Option<Want>> {
    let player = read(ipc)?;
    let plan = plan(&player, *bg_black, want.target);
    if plan.is_empty() {
        return Ok(None);
    }
    log::info!(
        "wallpaper-battery: to {:?} from speed {:.2}, fade {:.0}{}",
        want.target,
        player.speed,
        player.fade,
        if player.paused { ", paused" } else { "" },
    );
    for cmd in &plan.before {
        run(*cmd, ipc, want, bg_black)?;
    }
    let start = Instant::now();
    for step in schedule(&plan) {
        let due = start + step.at;
        loop {
            let now = Instant::now();
            if now >= due {
                break;
            }
            match rx.recv_timeout(due - now) {
                // The same target again (a tick, a settings change beside
                // it): carry on, with the newest wallpaper to put back.
                Ok(next) if next.target == want.target => want.restore = next.restore,
                Ok(next) => return Ok(Some(next)),
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return Ok(None),
            }
        }
        if let Some(speed) = step.speed {
            ipc.set("speed", json!(speed))?;
        }
        if let Some(fade) = step.fade {
            for property in FADE {
                ipc.set(property, json!(fade))?;
            }
        }
    }
    if plan.fade.moves() && plan.after.contains(&Cmd::Video(false)) {
        std::thread::sleep(SETTLE);
    }
    for cmd in &plan.after {
        run(*cmd, ipc, want, bg_black)?;
    }
    Ok(None)
}

/// The worker: wanted states in, transitions out, one at a time and the
/// newest first.
fn worker(path: PathBuf, rx: Receiver<Want>) {
    // Sway's background is the compositor's, not the player's: whether it
    // is black is remembered here. A panel restarted while it was black
    // finds the player's video track off, and puts the wallpaper back on
    // that instead.
    let mut bg_black = false;
    let Ok(mut want) = rx.recv() else { return };
    loop {
        while let Ok(newer) = rx.try_recv() {
            want = newer;
        }
        if let Some(next) = settle(&path, &mut want, &mut bg_black, &rx) {
            want = next;
            continue;
        }
        match rx.recv() {
            Ok(next) => want = next,
            Err(_) => return,
        }
    }
}

/// Start following, from the panel process. Does nothing on a machine
/// without a battery, where nothing here can ever be wanted.
pub fn follow() {
    if !battery::start() {
        return;
    }
    let path = glib::user_runtime_dir().join("mpvpaper.sock");
    let (tx, rx) = mpsc::channel::<Want>();
    let spawned = std::thread::Builder::new()
        .name("wallpaper-battery".into())
        .spawn(move || worker(path, rx));
    if let Err(e) = spawned {
        log::warn!("wallpaper-battery: failed to spawn thread: {e}");
        return;
    }

    let want_now = || {
        let (setting, restore) =
            crate::settings::store::with(|s| (s.look().wallpaper_on_battery, s.wallpaper.clone()));
        Want {
            target: wanted(setting, battery::current().map(|b| b.state)),
            restore,
        }
    };
    let tick: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();
    let apply = Rc::new(move || {
        let want = want_now();
        let still = want.target != Target::Play;
        // The shell darkens as the fade starts, through the theme's own
        // cross-fade.
        crate::theme::set_battery_dark(want.target == Target::Black);
        let _ = tx.send(want);
        let mut t = tick.borrow_mut();
        match (still, t.is_some()) {
            (true, false) => {
                let tx = tx.clone();
                *t = Some(glib::timeout_add_seconds_local(TICK_S, move || {
                    let _ = tx.send(want_now());
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
            fade: 0.0,
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
            fade: DARK,
            video: false,
            ..paused_here(base)
        }
    }

    #[test]
    fn only_a_discharging_battery_changes_anything() {
        let on = Some(ChargeState::Discharging);
        assert_eq!(wanted(OnBattery::Keep, on), Target::Play);
        assert_eq!(wanted(OnBattery::Pause, on), Target::Pause);
        assert_eq!(wanted(OnBattery::Black, on), Target::Black);
        for plugged in [
            ChargeState::Charging,
            ChargeState::Full,
            ChargeState::Idle,
            ChargeState::Unknown,
        ] {
            for setting in OnBattery::ALL {
                assert_eq!(wanted(setting, Some(plugged)), Target::Play, "{plugged:?}");
            }
        }
        assert_eq!(wanted(OnBattery::Black, None), Target::Play);
    }

    #[test]
    fn the_curves_run_end_to_end_the_right_way() {
        for curve in [Curve::Out, Curve::In, Curve::InOut] {
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
        assert!((ease(Curve::InOut, 0.5) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn slowing_down_is_thirty_writes_from_the_base_to_the_floor_then_a_pause() {
        let p = plan(&playing(0.5), false, Target::Pause);
        assert!(p.before.is_empty());
        assert_eq!(p.after, vec![Cmd::Pause(true)]);
        assert_eq!(p.speed.span, SPEED_SPAN);
        assert_eq!(p.speed.curve, Curve::Out);
        assert!(!p.fade.moves());
        let steps = schedule(&p);
        assert_eq!(steps.len(), 150);
        assert!(steps.iter().all(|s| s.fade.is_none()));
        let speeds: Vec<f64> = steps.iter().map(|s| s.speed.unwrap()).collect();
        assert!(speeds.windows(2).all(|w| w[1] < w[0]), "{speeds:?}");
        assert_eq!(*speeds.last().unwrap(), FLOOR);
        assert_eq!(steps.last().unwrap().at, SPEED_SPAN);
        // The base is the player's, whatever it is.
        let p = plan(&playing(1.25), false, Target::Pause);
        assert_eq!(p.speed.from, 1.25);
    }

    #[test]
    fn starting_up_undoes_our_pause_first_then_climbs_slowly() {
        let p = plan(&paused_here(0.5), false, Target::Play);
        assert_eq!(p.before, vec![Cmd::Pause(false)]);
        assert!(p.after.is_empty());
        assert_eq!((p.speed.from, p.speed.to), (FLOOR, 0.5));
        assert_eq!(p.speed.curve, Curve::In);
        assert_eq!(p.speed.span, SPEED_SPAN);
        let steps = schedule(&p);
        let first = steps[0].speed.unwrap();
        assert!(first - FLOOR < 0.001, "starts slow: {first}");
    }

    #[test]
    fn a_pause_that_is_not_ours_is_left_alone() {
        let theirs = Player {
            held: false,
            ..paused_here(0.5)
        };
        let p = plan(&theirs, false, Target::Play);
        assert!(p.before.is_empty(), "never resumed: {:?}", p.before);
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
            Target::Pause,
        );
        assert!(p.after.is_empty());
    }

    #[test]
    fn black_fades_and_slows_together_then_stops_and_blacks_the_background() {
        let p = plan(&playing(0.5), false, Target::Black);
        assert!(p.before.is_empty());
        assert_eq!((p.fade.from, p.fade.to), (0.0, DARK));
        assert_eq!(p.fade.span, FADE_SPAN);
        assert_eq!(p.speed.span, FADE_SPAN);
        // The background only after the video is black and stopped.
        assert_eq!(
            p.after,
            vec![Cmd::Pause(true), Cmd::Video(false), Cmd::Blackout]
        );
        let steps = schedule(&p);
        assert_eq!(steps.len(), 90);
        assert!(steps.iter().all(|s| s.speed.is_some() && s.fade.is_some()));
        assert_eq!(steps.last().unwrap().fade, Some(DARK));
    }

    #[test]
    fn back_from_black_the_wallpaper_goes_back_first() {
        let p = plan(&black_here(0.5), true, Target::Play);
        assert_eq!(
            p.before,
            vec![Cmd::Restore, Cmd::Video(true), Cmd::Pause(false)]
        );
        assert!(p.after.is_empty());
        assert_eq!((p.fade.from, p.fade.to), (DARK, 0.0));
        assert_eq!(p.speed.curve, Curve::In);
        // A panel that restarted while black does not know the background
        // is black; the video track being off says so.
        let p = plan(&black_here(0.5), false, Target::Play);
        assert_eq!(p.before[0], Cmd::Restore);
    }

    #[test]
    fn between_pause_and_black_on_battery() {
        // Paused, then black is picked: only the fade moves.
        let p = plan(&paused_here(0.5), false, Target::Black);
        assert!(p.before.is_empty());
        assert!(!p.speed.moves());
        assert!(p.fade.moves());
        assert_eq!(p.after, vec![Cmd::Video(false), Cmd::Blackout]);
        // Black, then pause is picked: the picture comes back, still held.
        let p = plan(&black_here(0.5), true, Target::Pause);
        assert_eq!(p.before, vec![Cmd::Restore, Cmd::Video(true)]);
        assert!(!p.speed.moves());
        assert_eq!(p.fade.to, 0.0);
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
        let p = plan(&midway, false, Target::Play);
        assert!(p.before.is_empty() && p.after.is_empty());
        assert_eq!((p.speed.from, p.speed.to), (0.2, 0.5));
        let share = 0.3 / (0.5 - FLOOR);
        let span = SPEED_SPAN.as_secs_f64() * share;
        assert!((p.speed.span.as_secs_f64() - span).abs() < 1e-6);
        assert_eq!(schedule(&p)[0].speed.map(|s| s > 0.2), Some(true));
        // Fading in from black, cut off by unplugging again: back down from
        // where the fade got to, and nothing to restore twice.
        let fading_in = Player {
            speed: 0.1,
            fade: -40.0,
            paused: false,
            held: false,
            video: true,
            base: 0.5,
        };
        let p = plan(&fading_in, false, Target::Black);
        assert!(p.before.is_empty());
        assert_eq!(p.fade.from, -40.0);
        let span = 0.6 * FADE_SPAN.as_secs_f64();
        assert!((p.fade.span.as_secs_f64() - span).abs() < 1e-6);
        assert_eq!(
            p.after,
            vec![Cmd::Pause(true), Cmd::Video(false), Cmd::Blackout]
        );
    }

    #[test]
    fn a_restarted_player_is_taken_from_where_it_comes_up() {
        // mpvpaper restarted while black: it plays at its own speed, full
        // fade. The background is already black.
        let p = plan(&playing(0.5), true, Target::Black);
        assert!(p.before.is_empty());
        assert!(p.speed.moves() && p.fade.moves());
        assert_eq!(p.after, vec![Cmd::Pause(true), Cmd::Video(false)]);
        // Restarted while a pause was wanted: slowed again from its speed.
        let p = plan(&playing(0.75), false, Target::Pause);
        assert_eq!(p.speed.from, 0.75);
        assert_eq!(p.after, vec![Cmd::Pause(true)]);
    }

    #[test]
    fn settled_is_nothing_to_do() {
        assert!(plan(&playing(0.5), false, Target::Play).is_empty());
        assert!(plan(&paused_here(0.5), false, Target::Pause).is_empty());
        assert!(plan(&black_here(0.5), true, Target::Black).is_empty());
        assert!(schedule(&plan(&playing(0.5), false, Target::Play)).is_empty());
    }

    /// Drive a real player through a script of targets, for the nested-sway
    /// check: `WALLPAPER_SOCK=<mpv socket> WALLPAPER_SCRIPT="pause:4,play:1.2"`
    /// (each target held for that many seconds), with `SWAYSOCK` set for the
    /// background.
    #[test]
    #[ignore]
    fn drive_a_real_player() {
        let path = PathBuf::from(std::env::var("WALLPAPER_SOCK").expect("WALLPAPER_SOCK"));
        let script = std::env::var("WALLPAPER_SCRIPT").expect("WALLPAPER_SCRIPT");
        let (tx, rx) = mpsc::channel::<Want>();
        let worker = std::thread::spawn(move || worker(path, rx));
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
            tx.send(Want {
                target,
                restore: None,
            })
            .unwrap();
            std::thread::sleep(Duration::from_secs_f64(secs.parse().unwrap()));
        }
        drop(tx);
        worker.join().unwrap();
    }
}
