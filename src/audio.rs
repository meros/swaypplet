//! Audio state from the sound server, pushed rather than parsed.
//!
//! # Why not `wpctl`
//!
//! The panel's audio section ran `wpctl status` and read its output with a
//! hand-written parser: indentation depth, box-drawing characters, an
//! asterisk marking the default, and volumes fetched by a second call per
//! device. Six call sites, all of them one WirePlumber release away from
//! being wrong, and all of them a blocking process spawn on the way to
//! drawing a slider. It also had no way to know when anything changed, so
//! every open re-read everything.
//!
//! # Why the PulseAudio protocol
//!
//! The obvious replacement is libpipewire, and it is not usable here: the
//! `pipewire` crate generates its bindings with bindgen 0.72, this binary
//! already links `pam-sys` which generates its own with bindgen 0.69, and
//! cargo's feature unification of the shared `clang-sys` leaves the older
//! one unable to load libclang at build time. That is a build-system
//! collision, not a design judgement, and it is not worth a vendored fork.
//!
//! PipeWire's own PulseAudio server (`services.pipewire.pulse.enable`, on
//! since this machine's audio was set up) speaks a protocol with a mature
//! Rust binding and no bindgen at all, and it exposes exactly what this
//! module needs: sinks and sources with volume and mute, per-application
//! streams, the default device, and — the reason item 4 was waiting on this
//! — *source outputs*, which is a recording application by another name.
//!
//! The dependency is real and worth stating: if pulse compatibility is ever
//! turned off, this goes quiet. It is asserted on the nixos side rather than
//! discovered at runtime.
//!
//! # Shape
//!
//! One thread owns the connection and its mainloop; snapshots cross to the
//! GTK side through an `async_channel` into an [`Observed`], the same shape
//! `sway_ipc`, `bar::tray` and `clipboard` use. Commands go the other way
//! through an `mpsc`, applied between mainloop iterations. Nothing polls:
//! the server sends a subscription event, the thread re-reads, and the panel
//! redraws only when the snapshot actually differs.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use libpulse_binding as pulse;
use pulse::callbacks::ListResult;
use pulse::context::subscribe::InterestMaskSet;
use pulse::context::{Context, FlagSet as ContextFlagSet, State as ContextState};
use pulse::mainloop::standard::{IterateResult, Mainloop};
use pulse::proplist::Proplist;
use pulse::volume::{ChannelVolumes, Volume};

use crate::service::{Backoff, Observed};

/// A device's or stream's loudness, as the panel draws it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VolumeState {
    /// 0.0–1.5. Above 1.0 is over-amplification, which the slider marks.
    pub volume: f64,
    pub muted: bool,
}

/// A sink or a source.
#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    /// The server's name (`alsa_output.pci-0000_00_1f.3.analog-stereo`),
    /// which is what selecting a default takes.
    pub id: String,
    pub name: String,
    pub is_default: bool,
    pub volume: VolumeState,
}

/// One application's audio, playing or recording.
#[derive(Clone, Debug, PartialEq)]
pub struct Stream {
    pub index: u32,
    pub name: String,
    pub volume: VolumeState,
    /// True for a source output — an application reading the microphone.
    pub recording: bool,
}

/// Everything the panel and the bar read.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AudioState {
    pub sink: Option<VolumeState>,
    pub source: Option<VolumeState>,
    pub sinks: Vec<Device>,
    pub sources: Vec<Device>,
    /// Playback streams, for the per-application rows.
    pub streams: Vec<Stream>,
    /// Recording streams. Empty is the whole signal the mic indicator needs.
    pub recorders: Vec<Stream>,
    /// The server answered at least once. False means the section draws its
    /// unavailable banner rather than an empty list pretending to be silence.
    pub connected: bool,
}

impl AudioState {
    /// Is something recording right now?
    ///
    /// Read by the hazard lane once the microphone indicator lands; the
    /// tests below are its only other caller today.
    ///
    /// The hazard lane's microphone glyph (BAR_VISION P9/P10) is exactly this
    /// question, and its stand-down is the same list going empty.
    pub fn microphone_in_use(&self) -> bool {
        !self.recorders.is_empty()
    }

    /// What is recording, for the indicator's tooltip. Deduplicated: two
    /// streams from one application is a detail nobody needs.
    pub fn recorder_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.recorders.iter().map(|r| r.name.clone()).collect();
        names.sort();
        names.dedup();
        names
    }
}

/// Maximum output volume, as a fraction of nominal. Above 1.0 is software
/// amplification. Defined here rather than in the OSD because this module
/// clamps to it when applying a relative change.
pub const VOLUME_CEILING: f64 = 1.5;

/// What the GTK side can ask the server to do.
#[derive(Clone, Debug)]
pub enum Command {
    SetSinkVolume(f64),
    /// Relative change, clamped to [0, ceiling]. The caller cannot do this
    /// arithmetic itself: its snapshot may name a different device than the
    /// one this command will land on, so the base value would be another
    /// sink's. Only this thread knows the target and its level together.
    AdjustSinkVolume(f64),
    SetSourceVolume(f64),
    ToggleSinkMute,
    ToggleSourceMute,
    SetDefaultSink(String),
    SetDefaultSource(String),
    SetStreamVolume { index: u32, level: f64 },
}

pub struct AudioService {
    state: Observed<AudioState>,
    commands: mpsc::Sender<Command>,
}

impl AudioService {
    /// Connect, and keep reconnecting. Returns immediately; the first
    /// snapshot arrives when the server answers.
    pub fn start() -> Rc<Self> {
        let (tx, rx) = async_channel::unbounded::<AudioState>();
        let (cmd_tx, cmd_rx) = mpsc::channel::<Command>();

        std::thread::Builder::new()
            .name("audio".into())
            .spawn(move || run(&tx, &cmd_rx))
            .map_err(|e| log::error!("audio: could not start thread: {e}"))
            .ok();

        let service = Rc::new(AudioService {
            state: Observed::new(AudioState::default()),
            commands: cmd_tx,
        });

        let for_recv = service.clone();
        glib::spawn_future_local(async move {
            while let Ok(snapshot) = rx.recv().await {
                for_recv.state.set_if_changed(snapshot);
            }
        });

        service
    }

    pub fn connect_change(&self, cb: impl Fn() + 'static) {
        self.state.connect_change(cb);
    }

    pub fn snapshot(&self) -> AudioState {
        self.state.with(Clone::clone)
    }

    /// Fire and forget. A dropped command means the audio thread died, which
    /// the next snapshot's `connected: false` already says.
    pub fn send(&self, command: Command) {
        if self.commands.send(command).is_err() {
            log::warn!("audio: command dropped, thread is gone");
        }
    }
}

// ── The audio thread ────────────────────────────────────────────────────

/// Connect, serve, reconnect. Mirrors `sway_ipc::run`.
fn run(tx: &async_channel::Sender<AudioState>, commands: &mpsc::Receiver<Command>) {
    let mut backoff = Backoff::new();
    loop {
        let started = std::time::Instant::now();
        match session(tx, commands) {
            Ok(()) => return, // receiver gone — the process is shutting down
            Err(e) => {
                // Say so once, so the panel's banner has a reason behind it.
                let _ = tx.send_blocking(AudioState::default());
                let delay = backoff.next_delay(started.elapsed());
                log::warn!("audio: {e}; reconnecting in {delay:?}");
                std::thread::sleep(delay);
            }
        }
    }
}

/// How long a mainloop iteration may block before the command channel and
/// the shutdown check get a turn. Short enough to feel immediate on a
/// slider drag, long enough not to spin.
const ITERATION_TIMEOUT: Duration = Duration::from_millis(50);

fn session(
    tx: &async_channel::Sender<AudioState>,
    commands: &mpsc::Receiver<Command>,
) -> Result<(), String> {
    let mut proplist = Proplist::new().ok_or("could not allocate a proplist")?;
    proplist
        .set_str(pulse::proplist::properties::APPLICATION_NAME, "swaypplet")
        .map_err(|_| "could not set the application name")?;

    let mut mainloop = Mainloop::new().ok_or("could not create a mainloop")?;
    let mut context = Context::new_with_proplist(&mainloop, "swaypplet", &proplist)
        .ok_or("could not create a context")?;
    context
        .connect(None, ContextFlagSet::NOFLAGS, None)
        .map_err(|e| format!("connect: {e}"))?;

    // Wait for the handshake before asking anything.
    loop {
        iterate(&mut mainloop)?;
        match context.get_state() {
            ContextState::Ready => break,
            ContextState::Failed | ContextState::Terminated => {
                return Err("connection failed".into());
            }
            _ => {}
        }
    }

    // Any of these means the picture changed; which one it was does not
    // matter, because a re-read is four cheap queries against a local socket.
    let dirty = Rc::new(std::cell::Cell::new(true));
    {
        let dirty = dirty.clone();
        context.set_subscribe_callback(Some(Box::new(move |_, _, _| dirty.set(true))));
        context.subscribe(
            InterestMaskSet::SINK
                | InterestMaskSet::SOURCE
                | InterestMaskSet::SINK_INPUT
                | InterestMaskSet::SOURCE_OUTPUT
                | InterestMaskSet::SERVER,
            |_| {},
        );
    }

    let mut last = AudioState::default();

    loop {
        iterate(&mut mainloop)?;

        if !matches!(context.get_state(), ContextState::Ready) {
            return Err("connection dropped".into());
        }

        while let Ok(command) = commands.try_recv() {
            // Resolve the target from a fresh read, never from `last`. The
            // default sink can change without any subscription event reaching
            // us: a Bluetooth sink appearing fires SINK events and we re-read,
            // but WirePlumber promotes it to default a moment *after* that,
            // and the promotion arrives (if at all) as a SERVER event that
            // pipewire-pulse does not reliably emit. `last` then names the
            // laptop speakers forever, and every volume and mute press goes
            // there while the audio plays on the headphones.
            //
            // Re-reading here costs four queries against a local socket, per
            // key press. That is the same cost the subscription path already
            // pays, and it makes a stale target impossible rather than
            // unlikely.
            match read(&mut mainloop, &context) {
                Ok(fresh) => {
                    apply(&mut context, &fresh, command);
                    last = fresh;
                }
                // A read failure is no reason to swallow the key press; the
                // cached snapshot is the best guess left.
                Err(_) => apply(&mut context, &last, command),
            }
            dirty.set(true);
        }

        if !dirty.replace(false) {
            continue;
        }

        let snapshot = read(&mut mainloop, &context)?;
        if snapshot != last {
            last = snapshot.clone();
            if tx.send_blocking(snapshot).is_err() {
                return Ok(()); // GTK side hung up
            }
        }
    }
}

fn iterate(mainloop: &mut Mainloop) -> Result<(), String> {
    match mainloop.iterate(false) {
        IterateResult::Success(dispatched) => {
            // `iterate(false)` returns immediately, so an idle loop would
            // spin a core. Sleeping only when nothing happened keeps a busy
            // stretch (a slider drag) at full speed.
            if dispatched == 0 {
                std::thread::sleep(ITERATION_TIMEOUT);
            }
            Ok(())
        }
        IterateResult::Quit(_) => Err("mainloop quit".into()),
        IterateResult::Err(e) => Err(format!("mainloop: {e}")),
    }
}

/// Everything, in four queries.
fn read(mainloop: &mut Mainloop, context: &Context) -> Result<AudioState, String> {
    let introspect = context.introspect();

    // The server tells us which sink and source are default; the device
    // lists then mark themselves rather than the panel guessing from names.
    let defaults: Rc<RefCell<(String, String)>> =
        Rc::new(RefCell::new((String::new(), String::new())));
    {
        let defaults = defaults.clone();
        let done = Rc::new(std::cell::Cell::new(false));
        let flag = done.clone();
        let op = introspect.get_server_info(move |info| {
            *defaults.borrow_mut() = (
                info.default_sink_name
                    .as_deref()
                    .unwrap_or_default()
                    .to_string(),
                info.default_source_name
                    .as_deref()
                    .unwrap_or_default()
                    .to_string(),
            );
            flag.set(true);
        });
        wait(mainloop, &op, &done)?;
    }
    let (default_sink, default_source) = defaults.borrow().clone();

    let sinks = Rc::new(RefCell::new(Vec::new()));
    {
        let sinks = sinks.clone();
        let done = Rc::new(std::cell::Cell::new(false));
        let flag = done.clone();
        let default = default_sink.clone();
        let op = introspect.get_sink_info_list(move |result| match result {
            ListResult::Item(info) => {
                let id = info.name.as_deref().unwrap_or_default().to_string();
                sinks.borrow_mut().push(Device {
                    is_default: id == default,
                    name: clean_device_name(info.description.as_deref().unwrap_or(&id)),
                    id,
                    volume: VolumeState {
                        volume: to_level(&info.volume),
                        muted: info.mute,
                    },
                });
            }
            ListResult::End | ListResult::Error => flag.set(true),
        });
        wait(mainloop, &op, &done)?;
    }

    let sources = Rc::new(RefCell::new(Vec::new()));
    {
        let sources = sources.clone();
        let done = Rc::new(std::cell::Cell::new(false));
        let flag = done.clone();
        let default = default_source.clone();
        let op = introspect.get_source_info_list(move |result| match result {
            ListResult::Item(info) => {
                // Monitors are the output looped back; they are not
                // microphones and listing them makes the source picker a
                // mirror of the sink picker.
                if info.monitor_of_sink.is_some() {
                    return;
                }
                let id = info.name.as_deref().unwrap_or_default().to_string();
                sources.borrow_mut().push(Device {
                    is_default: id == default,
                    name: clean_device_name(info.description.as_deref().unwrap_or(&id)),
                    id,
                    volume: VolumeState {
                        volume: to_level(&info.volume),
                        muted: info.mute,
                    },
                });
            }
            ListResult::End | ListResult::Error => flag.set(true),
        });
        wait(mainloop, &op, &done)?;
    }

    let streams = Rc::new(RefCell::new(Vec::new()));
    {
        let streams = streams.clone();
        let done = Rc::new(std::cell::Cell::new(false));
        let flag = done.clone();
        let op = introspect.get_sink_input_info_list(move |result| match result {
            ListResult::Item(info) => streams.borrow_mut().push(Stream {
                index: info.index,
                name: stream_name(&info.proplist, info.name.as_deref()),
                volume: VolumeState {
                    volume: to_level(&info.volume),
                    muted: info.mute,
                },
                recording: false,
            }),
            ListResult::End | ListResult::Error => flag.set(true),
        });
        wait(mainloop, &op, &done)?;
    }

    let recorders = Rc::new(RefCell::new(Vec::new()));
    {
        let recorders = recorders.clone();
        let done = Rc::new(std::cell::Cell::new(false));
        let flag = done.clone();
        let op = introspect.get_source_output_info_list(move |result| match result {
            ListResult::Item(info) => {
                // The server's own monitoring streams are not an application
                // listening to the room, and a meter that watched itself
                // would light the indicator permanently.
                let is_monitor = info
                    .proplist
                    .get_str("media.class")
                    .is_some_and(|c| c.contains("Monitor"));
                if is_monitor {
                    return;
                }
                recorders.borrow_mut().push(Stream {
                    index: info.index,
                    name: stream_name(&info.proplist, info.name.as_deref()),
                    volume: VolumeState {
                        volume: to_level(&info.volume),
                        muted: info.mute,
                    },
                    recording: true,
                });
            }
            ListResult::End | ListResult::Error => flag.set(true),
        });
        wait(mainloop, &op, &done)?;
    }

    let sinks = sinks.borrow().clone();
    let sources = sources.borrow().clone();

    Ok(AudioState {
        sink: sinks
            .iter()
            .find(|d| d.is_default)
            .map(|d| d.volume.clone()),
        source: sources
            .iter()
            .find(|d| d.is_default)
            .map(|d| d.volume.clone()),
        sinks,
        sources,
        streams: streams.borrow().clone(),
        recorders: recorders.borrow().clone(),
        connected: true,
    })
}

/// Iterate until an operation finishes or its callback flags the end.
fn wait<T: ?Sized>(
    mainloop: &mut Mainloop,
    operation: &pulse::operation::Operation<T>,
    done: &Rc<std::cell::Cell<bool>>,
) -> Result<(), String> {
    use pulse::operation::State;
    for _ in 0..200 {
        if done.get() {
            return Ok(());
        }
        match operation.get_state() {
            State::Done => return Ok(()),
            State::Cancelled => return Err("query cancelled".into()),
            State::Running => iterate(mainloop)?,
        }
    }
    Err("query never finished".into())
}

/// Apply a command against the last snapshot, which is where the device
/// names and stream indices it needs live.
fn apply(context: &mut Context, state: &AudioState, command: Command) {
    let mut introspect = context.introspect();

    match command {
        Command::SetSinkVolume(level) => {
            if let Some(device) = state.sinks.iter().find(|d| d.is_default) {
                introspect.set_sink_volume_by_name(&device.id, &from_level(level), None);
            }
        }
        Command::AdjustSinkVolume(delta) => {
            if let Some(device) = state.sinks.iter().find(|d| d.is_default) {
                let level = (device.volume.volume + delta).clamp(0.0, VOLUME_CEILING);
                introspect.set_sink_volume_by_name(&device.id, &from_level(level), None);
            }
        }
        Command::SetSourceVolume(level) => {
            if let Some(device) = state.sources.iter().find(|d| d.is_default) {
                introspect.set_source_volume_by_name(&device.id, &from_level(level), None);
            }
        }
        Command::ToggleSinkMute => {
            if let Some(device) = state.sinks.iter().find(|d| d.is_default) {
                introspect.set_sink_mute_by_name(&device.id, !device.volume.muted, None);
            }
        }
        Command::ToggleSourceMute => {
            if let Some(device) = state.sources.iter().find(|d| d.is_default) {
                introspect.set_source_mute_by_name(&device.id, !device.volume.muted, None);
            }
        }
        Command::SetDefaultSink(id) => {
            context.set_default_sink(&id, |_| {});
        }
        Command::SetDefaultSource(id) => {
            context.set_default_source(&id, |_| {});
        }
        Command::SetStreamVolume { index, level } => {
            // A stream's channel count has to match what it is already
            // using, so the existing volumes are the template.
            if let Some(stream) = state.streams.iter().find(|s| s.index == index) {
                let _ = stream;
                introspect.set_sink_input_volume(index, &from_level(level), None);
            }
        }
    }
}

// ── Value conversions ───────────────────────────────────────────────────

/// The server's per-channel volumes as one 0.0–1.5 number.
///
/// Maximum rather than average: a balance set off-centre should not read as
/// quieter than it is, and the slider writes every channel anyway.
pub(crate) fn to_level(volumes: &ChannelVolumes) -> f64 {
    let max = volumes.max().0;
    f64::from(max) / f64::from(Volume::NORMAL.0)
}

/// One level as a stereo pair for the server.
pub(crate) fn from_level(level: f64) -> ChannelVolumes {
    let mut volumes = ChannelVolumes::default();
    volumes.set_len(2);
    let raw = (level.clamp(0.0, 1.5) * f64::from(Volume::NORMAL.0)).round() as u32;
    volumes.set(2, Volume(raw));
    volumes
}

/// The most human name a stream has.
///
/// `application.name` is what the application calls itself ("Spotify");
/// `media.name` is what it is playing, which changes every track and makes
/// a settings row jump. Name first, media only when there is nothing else.
pub(crate) fn stream_name(proplist: &Proplist, fallback: Option<&str>) -> String {
    proplist
        .get_str("application.name")
        .or_else(|| proplist.get_str("application.process.binary"))
        .or_else(|| fallback.map(str::to_string))
        .unwrap_or_else(|| "Unknown".into())
}

/// Strip the audio controller's model name off a device description.
///
/// Moved here from the old `wpctl` parser unchanged: the names come from
/// ALSA either way, and "Lunar Lake-M HD Audio Controller Speaker" is the
/// hardware's name for itself rather than anything a person picking an
/// output needs to read.
pub fn clean_device_name(name: &str) -> String {
    const PREFIXES: &[&str] = &[
        "Alder Lake PCH-P High Definition Audio ",
        "Alder Lake-S PCH High Definition Audio ",
        "Alderlake-S HD Audio ",
        "Broadwell-U Audio Controller ",
        "Cannon Lake PCH cAVS ",
        "Comet Lake PCH cAVS ",
        "Ice Lake-LP Smart Sound Technology Audio Controller ",
        "Jasper Lake HD Audio ",
        "Lunar Lake-M HD Audio Controller ",
        "Meteor Lake-P HD Audio Controller ",
        "Raptor Lake-P/U/H cAVS ",
        "Skylake Audio Controller ",
        "Tiger Lake-LP Smart Sound Technology Audio Controller ",
        "Tiger Lake-H HD Audio Controller ",
        "Wildcat Point-LP High Definition Audio Controller ",
        "Intel High Definition Audio ",
        "Intel HD Audio ",
        "AMD Rembrandt Radeon High Definition Audio ",
        "AMD Renoir Radeon High Definition Audio ",
        "AMD Navi 21/23 HDMI/DP Audio ",
        "AMD Navi 31 HDMI/DP Audio ",
        "Raven/Raven2/Fenghuang HDMI/DP Audio ",
        "Starship/Matisse HD Audio Controller ",
        "AMD High Definition Audio ",
        "AMD HD Audio ",
        "NVIDIA High Definition Audio ",
        "NVIDIA HD Audio ",
    ];

    for prefix in PREFIXES {
        if let Some(rest) = name.strip_prefix(prefix) {
            let cleaned = rest.trim();
            if !cleaned.is_empty() {
                return cleaned.to_string();
            }
        }
    }

    name.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_level_survives_the_round_trip_to_the_server_and_back() {
        for level in [0.0, 0.25, 0.5, 1.0, 1.5] {
            let back = to_level(&from_level(level));
            assert!((back - level).abs() < 0.001, "{level} came back as {back}");
        }
    }

    #[test]
    fn a_level_above_the_over_amplification_ceiling_is_clamped() {
        assert!((to_level(&from_level(9.0)) - 1.5).abs() < 0.001);
    }

    #[test]
    fn controller_names_are_stripped_from_device_descriptions() {
        assert_eq!(
            clean_device_name("Lunar Lake-M HD Audio Controller Speaker"),
            "Speaker"
        );
        assert_eq!(clean_device_name("  Jabra Evolve 65  "), "Jabra Evolve 65");
    }

    #[test]
    fn a_name_that_is_only_a_prefix_keeps_the_original() {
        // Stripping would leave nothing, and an unnamed row is worse than a
        // long one.
        assert_eq!(clean_device_name("Intel HD Audio "), "Intel HD Audio");
    }

    #[test]
    fn the_microphone_indicator_follows_the_recorder_list() {
        let mut state = AudioState::default();
        assert!(!state.microphone_in_use());

        state.recorders.push(Stream {
            index: 1,
            name: "Google Chrome".into(),
            volume: VolumeState::default(),
            recording: true,
        });
        assert!(state.microphone_in_use());
        assert_eq!(state.recorder_names(), vec!["Google Chrome"]);
    }

    #[test]
    fn one_application_recording_twice_is_named_once() {
        let stream = |name: &str, index| Stream {
            index,
            name: name.into(),
            volume: VolumeState::default(),
            recording: true,
        };
        let state = AudioState {
            recorders: vec![stream("Chrome", 1), stream("Chrome", 2), stream("Zoom", 3)],
            ..Default::default()
        };
        assert_eq!(state.recorder_names(), vec!["Chrome", "Zoom"]);
    }
}

#[cfg(test)]
mod live {
    /// Reads the running sound server. Ignored: needs a session.
    #[test]
    #[ignore]
    fn read_the_session() {
        let (tx, rx) = std::sync::mpsc::channel();
        let (_cmd_tx, cmd_rx) = std::sync::mpsc::channel::<super::Command>();
        let (atx, arx) = async_channel::unbounded();

        std::thread::spawn(move || super::run(&atx, &cmd_rx));
        std::thread::spawn(move || {
            let _ = tx.send(arx.recv_blocking());
        });

        let state = rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("no snapshot in 5s")
            .expect("channel closed");

        println!("connected={}", state.connected);
        println!("default sink: {:?}", state.sink);
        println!("default source: {:?}", state.source);
        for d in &state.sinks {
            println!(
                "  sink   {} default={} {:?}",
                d.name, d.is_default, d.volume
            );
        }
        for d in &state.sources {
            println!(
                "  source {} default={} {:?}",
                d.name, d.is_default, d.volume
            );
        }
        for s in &state.streams {
            println!("  stream {} {:?}", s.name, s.volume);
        }
        println!(
            "mic in use: {} {:?}",
            state.microphone_in_use(),
            state.recorder_names()
        );
        assert!(state.connected, "did not connect");
    }
}
