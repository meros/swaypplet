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
//! through an `mpsc`. Nothing polls: the server sends a subscription event,
//! the thread re-reads, and the panel redraws only when the snapshot
//! actually differs.
//!
//! # Why the mainloop blocks
//!
//! `Mainloop::iterate(false)` returns the instant nothing is ready, so an
//! idle loop around it spins a core. The obvious guard is to sleep when a
//! turn dispatched nothing, and that is what this module used to do: 50 ms.
//! It cost one sleep per request/response round trip on a unix socket that
//! answers in well under a millisecond, four of them per read, and a read
//! per key press. Measured end to end, one volume key took 319 ms to reach
//! the server, and a ten-press burst drained at 210 ms each, so the volume
//! went on sliding for two seconds after the last press.
//!
//! `iterate(true)` blocks in `poll()` instead, which is what the sound
//! server's own clients do. Nothing sleeps and nothing spins. The catch is
//! that a command from the GTK thread is not something the *server* says,
//! so it would sit in the channel until the next unrelated event; a
//! [`Waker`] pipe joins the poll set to break the block immediately.

use std::cell::{Cell, RefCell};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::rc::Rc;
use std::sync::mpsc;

use libpulse_binding as pulse;
use pulse::callbacks::ListResult;
use pulse::context::subscribe::InterestMaskSet;
use pulse::context::{Context, FlagSet as ContextFlagSet, State as ContextState};
use pulse::mainloop::api::Mainloop as MainloopTrait;
use pulse::mainloop::events::io::FlagSet as IoFlagSet;
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
    /// The server's index, which is what *writing* to it takes. Names are
    /// stable across a reconnect and indices are not, so both are kept.
    pub index: u32,
    pub name: String,
    pub is_default: bool,
    /// How many channels its volume has. A write must carry exactly this
    /// many; see [`from_level`].
    pub channels: u8,
    pub volume: VolumeState,
}

/// One application's audio, playing or recording.
#[derive(Clone, Debug, PartialEq)]
pub struct Stream {
    pub index: u32,
    pub name: String,
    /// As on [`Device`]: the channel count a volume write must match.
    pub channels: u8,
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

/// The write end of the audio thread's wakeup pipe.
///
/// The thread blocks in `poll()` waiting on the server's socket. A queued
/// command is invisible there, so one byte down this pipe joins the poll
/// set and ends the block. The payload is meaningless; arrival is the
/// whole message.
type Waker = RefCell<UnixStream>;

pub struct AudioService {
    state: Observed<AudioState>,
    commands: mpsc::Sender<Command>,
    /// `None` only if the pipe could not be made, which costs latency
    /// rather than correctness: commands then wait for the next event.
    wake: Option<Waker>,
}

impl AudioService {
    /// Connect, and keep reconnecting. Returns immediately; the first
    /// snapshot arrives when the server answers.
    pub fn start() -> Rc<Self> {
        let (tx, rx) = async_channel::unbounded::<AudioState>();
        let (cmd_tx, cmd_rx) = mpsc::channel::<Command>();

        // Non-blocking on the read side so draining it in the mainloop
        // callback stops at the last byte instead of parking the thread.
        let (wake_tx, wake_rx) = UnixStream::pair()
            .and_then(|(tx, rx)| {
                rx.set_nonblocking(true)?;
                Ok((tx, rx))
            })
            .map_err(|e| {
                log::error!("audio: no wakeup pipe ({e}); commands wait for the next event");
            })
            .ok()
            .unzip();

        std::thread::Builder::new()
            .name("audio".into())
            .spawn(move || run(&tx, &cmd_rx, wake_rx.as_ref()))
            .map_err(|e| log::error!("audio: could not start thread: {e}"))
            .ok();

        let service = Rc::new(AudioService {
            state: Observed::new(AudioState::default()),
            commands: cmd_tx,
            wake: wake_tx.map(RefCell::new),
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
            return;
        }
        // Queue first, wake second. The other order would let the thread
        // wake to an empty channel and go back to sleep with work pending.
        if let Some(wake) = &self.wake
            && let Err(e) = wake.borrow_mut().write_all(&[0])
        {
            log::warn!("audio: could not wake the audio thread: {e}");
        }
    }
}

// ── The audio thread ────────────────────────────────────────────────────

/// Connect, serve, reconnect. Mirrors `sway_ipc::run`.
fn run(
    tx: &async_channel::Sender<AudioState>,
    commands: &mpsc::Receiver<Command>,
    wake: Option<&UnixStream>,
) {
    let mut backoff = Backoff::new();
    loop {
        let started = std::time::Instant::now();
        match session(tx, commands, wake) {
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

fn session(
    tx: &async_channel::Sender<AudioState>,
    commands: &mpsc::Receiver<Command>,
    wake: Option<&UnixStream>,
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

    // The command channel's other half in the poll set.
    //
    // The callback drains the pipe rather than leaving the byte for the
    // loop below, because `poll()` is level triggered and an unread byte
    // would make every nested `wait()` return instantly and spin. Draining
    // spends the signal, though, and the nested `wait()` inside a `read()`
    // is where it is most likely to be spent: the loop would then go back
    // to blocking with a command already queued and nothing left to say so.
    // `woken` is that record, and it is what the loop consults before it
    // decides to block.
    let woken = Rc::new(Cell::new(false));
    let _wake_event = wake
        .and_then(|w| w.try_clone().ok())
        .and_then(|mut reader| {
            let fd = reader.as_raw_fd();
            let woken = woken.clone();
            mainloop.new_io_event(
                fd,
                IoFlagSet::INPUT,
                Box::new(move |_, _, _| {
                    let mut discard = [0u8; 64];
                    while reader.read(&mut discard).is_ok_and(|read| read > 0) {}
                    woken.set(true);
                }),
            )
        });

    let mut last = AudioState::default();

    loop {
        // Block only with nothing already pending, on either count.
        //
        // `woken` means a command is queued: a sender puts it in the
        // channel before it writes the byte, so a set flag means it is
        // there now. `dirty` means a re-read is owed — and it is owed
        // precisely when the subscription event landed inside the last
        // `read()`, whose result was then taken a moment too early to show
        // the change. Blocking on either would park the thread until some
        // unrelated event happened to arrive, which on a quiet system is
        // not soon. That is what made a volume key intermittently do
        // nothing visible at all.
        if !woken.get() && !dirty.get() {
            iterate(&mut mainloop)?;
        }

        if !matches!(context.get_state(), ContextState::Ready) {
            return Err("connection dropped".into());
        }

        // Cleared before the drain, never after: the callback only runs
        // inside `iterate`, so nothing can set it between these two lines.
        woken.set(false);
        let batch = Batch::collect(commands);
        if !batch.is_empty() {
            apply(&mut mainloop, &mut context, &last, &batch);
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

/// One turn of the mainloop, blocking until there is something to do.
///
/// `poll()` returns on server traffic, on a queued command (the waker's
/// pipe is in the set), and on the socket hanging up, so a server that
/// dies surfaces as a state change on the next turn rather than a hang.
fn iterate(mainloop: &mut Mainloop) -> Result<(), String> {
    match mainloop.iterate(true) {
        IterateResult::Success(_) => Ok(()),
        IterateResult::Quit(_) => Err("mainloop quit".into()),
        IterateResult::Err(e) => Err(format!("mainloop: {e}")),
    }
}

/// Everything, in four queries.
fn read(mainloop: &mut Mainloop, context: &Context) -> Result<AudioState, String> {
    let introspect = context.introspect();

    // The server tells us which sink and source are default; the device
    // lists then mark themselves rather than the panel guessing from names.
    let (default_sink, default_source) = default_names(mainloop, context)?;

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
                    index: info.index,
                    channels: info.volume.len(),
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
                    index: info.index,
                    channels: info.volume.len(),
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
                channels: info.volume.len(),
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
                    channels: info.volume.len(),
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

/// One mainloop turn's worth of commands, folded into a single intent.
///
/// A held volume key repeats at roughly 30 Hz, and the speakers' own knob
/// is a rotary encoder that beats that comfortably. Applying such a burst
/// one command at a time is what made the level keep sliding for seconds
/// after the last press: each one was its own round trip, and they queued.
/// Folded, the burst becomes one write carrying the summed delta, so the
/// volume lands where the last press asked and nothing trails it.
#[derive(Default)]
struct Batch {
    /// An absolute level supersedes anything queued before it. `sink_delta`
    /// still applies on top: a slider drag and a key press can share a turn.
    sink_level: Option<f64>,
    sink_delta: f64,
    /// Toggles cancel in pairs, which is what an even number of presses
    /// means.
    sink_mute_toggle: bool,
    source_level: Option<f64>,
    source_mute_toggle: bool,
    default_sink: Option<String>,
    default_source: Option<String>,
    /// Last write per stream wins; a drag emits one per motion event.
    stream_levels: Vec<(u32, f64)>,
}

impl Batch {
    fn collect(commands: &mpsc::Receiver<Command>) -> Self {
        let mut batch = Self::default();
        while let Ok(command) = commands.try_recv() {
            match command {
                Command::SetSinkVolume(level) => {
                    batch.sink_level = Some(level);
                    batch.sink_delta = 0.0;
                }
                Command::AdjustSinkVolume(delta) => batch.sink_delta += delta,
                Command::SetSourceVolume(level) => batch.source_level = Some(level),
                Command::ToggleSinkMute => batch.sink_mute_toggle = !batch.sink_mute_toggle,
                Command::ToggleSourceMute => batch.source_mute_toggle = !batch.source_mute_toggle,
                Command::SetDefaultSink(id) => batch.default_sink = Some(id),
                Command::SetDefaultSource(id) => batch.default_source = Some(id),
                Command::SetStreamVolume { index, level } => {
                    match batch.stream_levels.iter_mut().find(|(i, _)| *i == index) {
                        Some(slot) => slot.1 = level,
                        None => batch.stream_levels.push((index, level)),
                    }
                }
            }
        }
        batch
    }

    fn touches_sink(&self) -> bool {
        self.sink_level.is_some() || self.sink_delta != 0.0 || self.sink_mute_toggle
    }

    fn touches_source(&self) -> bool {
        self.source_level.is_some() || self.source_mute_toggle
    }

    fn is_empty(&self) -> bool {
        !self.touches_sink()
            && !self.touches_source()
            && self.default_sink.is_none()
            && self.default_source.is_none()
            && self.stream_levels.is_empty()
    }
}

/// A device resolved at the moment a command is applied to it.
struct Target {
    index: u32,
    channels: u8,
    level: f64,
    muted: bool,
}

/// Apply one folded batch.
///
/// `last` is consulted only for stream channel counts, where an index is an
/// index and cannot go stale the way "the default device" can.
fn apply(mainloop: &mut Mainloop, context: &mut Context, last: &AudioState, batch: &Batch) {
    // Picking a default comes first: anything else in this turn means the
    // device the user just chose.
    if let Some(id) = &batch.default_sink {
        context.set_default_sink(id, |_| {});
    }
    if let Some(id) = &batch.default_source {
        context.set_default_source(id, |_| {});
    }

    if batch.touches_sink() {
        match resolve_default_sink(mainloop, context) {
            Ok(sink) => {
                let mut introspect = context.introspect();
                if batch.sink_level.is_some() || batch.sink_delta != 0.0 {
                    let level = (batch.sink_level.unwrap_or(sink.level) + batch.sink_delta)
                        .clamp(0.0, VOLUME_CEILING);
                    introspect.set_sink_volume_by_index(
                        sink.index,
                        &from_level(level, sink.channels),
                        None,
                    );
                }
                if batch.sink_mute_toggle {
                    introspect.set_sink_mute_by_index(sink.index, !sink.muted, None);
                }
            }
            Err(e) => log::warn!("audio: no default sink to apply to: {e}"),
        }
    }

    if batch.touches_source() {
        match resolve_default_source(mainloop, context) {
            Ok(source) => {
                let mut introspect = context.introspect();
                if let Some(level) = batch.source_level {
                    introspect.set_source_volume_by_index(
                        source.index,
                        &from_level(level, source.channels),
                        None,
                    );
                }
                if batch.source_mute_toggle {
                    introspect.set_source_mute_by_index(source.index, !source.muted, None);
                }
            }
            Err(e) => log::warn!("audio: no default source to apply to: {e}"),
        }
    }

    let mut introspect = context.introspect();
    for &(index, level) in &batch.stream_levels {
        let Some(stream) = last.streams.iter().find(|s| s.index == index) else {
            continue; // gone since the snapshot; the next one will agree
        };
        introspect.set_sink_input_volume(index, &from_level(level, stream.channels), None);
    }
}

/// Which sink and source the server currently calls default.
fn default_names(mainloop: &mut Mainloop, context: &Context) -> Result<(String, String), String> {
    let introspect = context.introspect();
    let names = Rc::new(RefCell::new((String::new(), String::new())));
    let done = Rc::new(Cell::new(false));
    let (out, flag) = (names.clone(), done.clone());
    let op = introspect.get_server_info(move |info| {
        *out.borrow_mut() = (
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
    let names = names.borrow().clone();
    Ok(names)
}

/// The default sink, read at the moment of use rather than taken from the
/// last snapshot.
///
/// The default can move without the panel hearing about it in time:
/// WirePlumber promotes a sink that has just appeared, and a key press in
/// that window would otherwise drive the laptop speakers while the sound
/// plays on the headphones. Two queries against a local socket, about a
/// millisecond with a blocking mainloop. It was four queries and 200 ms
/// before, which is what this module's history is mostly about.
fn resolve_default_sink(mainloop: &mut Mainloop, context: &Context) -> Result<Target, String> {
    let (name, _) = default_names(mainloop, context)?;
    if name.is_empty() {
        return Err("the server names no default sink".into());
    }
    let found = Rc::new(RefCell::new(None));
    let done = Rc::new(Cell::new(false));
    let (out, flag) = (found.clone(), done.clone());
    let introspect = context.introspect();
    let op = introspect.get_sink_info_by_name(&name, move |result| match result {
        ListResult::Item(info) => {
            *out.borrow_mut() = Some(Target {
                index: info.index,
                channels: info.volume.len(),
                level: to_level(&info.volume),
                muted: info.mute,
            });
        }
        ListResult::End | ListResult::Error => flag.set(true),
    });
    wait(mainloop, &op, &done)?;
    let target = found.borrow_mut().take();
    target.ok_or_else(|| format!("no sink named {name}"))
}

/// The default source. See [`resolve_default_sink`].
fn resolve_default_source(mainloop: &mut Mainloop, context: &Context) -> Result<Target, String> {
    let (_, name) = default_names(mainloop, context)?;
    if name.is_empty() {
        return Err("the server names no default source".into());
    }
    let found = Rc::new(RefCell::new(None));
    let done = Rc::new(Cell::new(false));
    let (out, flag) = (found.clone(), done.clone());
    let introspect = context.introspect();
    let op = introspect.get_source_info_by_name(&name, move |result| match result {
        ListResult::Item(info) => {
            *out.borrow_mut() = Some(Target {
                index: info.index,
                channels: info.volume.len(),
                level: to_level(&info.volume),
                muted: info.mute,
            });
        }
        ListResult::End | ListResult::Error => flag.set(true),
    });
    wait(mainloop, &op, &done)?;
    let target = found.borrow_mut().take();
    target.ok_or_else(|| format!("no source named {name}"))
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

/// One level as the server's per-channel volumes.
///
/// The channel count comes from the device being written to rather than
/// being assumed stereo. Both a mono USB speaker and a 5.1 HDMI sink are
/// reachable from this machine, and a two-channel volume is not the volume
/// that was asked for on either.
pub(crate) fn from_level(level: f64, channels: u8) -> ChannelVolumes {
    let channels = channels.clamp(1, ChannelVolumes::CHANNELS_MAX);
    let mut volumes = ChannelVolumes::default();
    volumes.set_len(channels);
    let raw = (level.clamp(0.0, VOLUME_CEILING) * f64::from(Volume::NORMAL.0)).round() as u32;
    volumes.set(channels, Volume(raw));
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
        for channels in [1, 2, 6] {
            for level in [0.0, 0.25, 0.5, 1.0, 1.5] {
                let back = to_level(&from_level(level, channels));
                assert!(
                    (back - level).abs() < 0.001,
                    "{level} on {channels}ch came back as {back}"
                );
            }
        }
    }

    #[test]
    fn a_write_carries_the_device_s_own_channel_count() {
        // A mismatch is not a rounding error: the server takes the volume
        // as written, and a stereo pair aimed at a 5.1 sink leaves four
        // channels untouched.
        for channels in [1, 2, 6, 8] {
            assert_eq!(from_level(0.5, channels).len(), channels);
        }
        assert_eq!(from_level(0.5, 0).len(), 1, "a device has at least one");
    }

    #[test]
    fn a_level_above_the_over_amplification_ceiling_is_clamped() {
        assert!((to_level(&from_level(9.0, 2)) - 1.5).abs() < 0.001);
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
            channels: 1,
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
            channels: 1,
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

        std::thread::spawn(move || super::run(&atx, &cmd_rx, None));
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

    /// How long a volume key takes to reach the server. Ignored: needs a
    /// session, and it moves the volume (it puts it back).
    ///
    /// The number this exists to defend: with the old sleep-driven mainloop
    /// this path measured 319 ms for one press, and a ten-press burst
    /// drained at ~210 ms each.
    #[test]
    #[ignore]
    fn a_volume_command_reaches_the_server_promptly() {
        use std::time::{Duration, Instant};

        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<super::Command>();
        let (atx, arx) = async_channel::unbounded();
        let (wake_tx, wake_rx) = std::os::unix::net::UnixStream::pair().expect("pipe");
        wake_rx.set_nonblocking(true).expect("nonblocking");

        std::thread::spawn(move || super::run(&atx, &cmd_rx, Some(&wake_rx)));

        let first = arx
            .recv_blocking()
            .expect("channel closed")
            .sink
            .expect("no default sink");

        let send = |command| {
            cmd_tx.send(command).expect("thread gone");
            use std::io::Write;
            (&wake_tx).write_all(&[0]).expect("wake");
        };

        // One press.
        let started = Instant::now();
        send(super::Command::AdjustSinkVolume(0.05));
        let mut elapsed = None;
        let mut seen = 0;
        while started.elapsed() < Duration::from_secs(2) {
            match arx.try_recv() {
                Ok(state) => {
                    seen += 1;
                    if state.sink.is_some_and(|s| s.volume != first.volume) {
                        elapsed = Some(started.elapsed());
                        break;
                    }
                }
                Err(async_channel::TryRecvError::Empty) => {
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(async_channel::TryRecvError::Closed) => break,
            }
        }
        let elapsed = elapsed.unwrap_or_else(|| {
            panic!("the server never moved from {:?} in 2s ({seen} snapshots seen)", first.volume)
        });
        println!("one press reached the server in {elapsed:?}");

        // A burst, which is what a key repeat or the speakers' knob sends.
        // Ten steps must land as ten steps, not trail in one at a time.
        let burst = Instant::now();
        for _ in 0..10 {
            send(super::Command::AdjustSinkVolume(-0.05));
        }
        std::thread::sleep(Duration::from_millis(400));
        while arx.try_recv().is_ok() {}
        println!("ten-press burst settled within {:?}", burst.elapsed());

        send(super::Command::SetSinkVolume(first.volume));
        std::thread::sleep(Duration::from_millis(300));

        assert!(
            elapsed < Duration::from_millis(50),
            "a volume key took {elapsed:?} to reach the server"
        );
    }
}
