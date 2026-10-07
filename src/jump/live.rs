//! Live window pixels for the jump card, over `ext-image-copy-capture-v1`.
//!
//! The screenshot module captures one frame and closes the session. This keeps
//! the session open and asks for the next frame as soon as the last one is
//! ready, and the compositor answers only when the window has damage. An idle
//! terminal therefore costs one frame, and a playing video costs up to
//! [`Stream`]'s frame cap.
//!
//! Everything runs on one worker thread with its own Wayland connection, for
//! the reason `screenshot::capture` gives: the toplevel handles a capture
//! source needs only exist on the connection that bound the list. One
//! connection for every window on the card, and one dmabuf per window,
//! reused for every frame.
//!
//! Frames never touch the CPU. sway copies the window into a dmabuf on its
//! own GPU, and the worker averages it there into a small dmabuf that GTK
//! draws one pixel to one (`gpu.rs`). There is no second path: a window
//! sway offers no dmabuf for, or a session without a GPU context, gets no
//! frames, and its picture keeps the app icon it starts with (`card.rs`);
//! the journal says why. A screenshot, which wants the pixels in memory,
//! is taken by `screenshot::capture::window`, not here.

use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use wayland_client::protocol::{wl_buffer, wl_registry};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, delegate_noop};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::{
    ext_foreign_toplevel_handle_v1::{self, ExtForeignToplevelHandleV1},
    ext_foreign_toplevel_list_v1::{self, ExtForeignToplevelListV1},
};
use wayland_protocols::ext::image_capture_source::v1::client::{
    ext_foreign_toplevel_image_capture_source_manager_v1::ExtForeignToplevelImageCaptureSourceManagerV1,
    ext_image_capture_source_v1::ExtImageCaptureSourceV1,
};
use wayland_protocols::ext::image_copy_capture::v1::client::{
    ext_image_copy_capture_frame_v1::{self, ExtImageCopyCaptureFrameV1},
    ext_image_copy_capture_manager_v1::{self, ExtImageCopyCaptureManagerV1},
    ext_image_copy_capture_session_v1::{self, ExtImageCopyCaptureSessionV1},
};
use wayland_protocols::wp::linux_dmabuf::zv1::client::{
    zwp_linux_buffer_params_v1::{self, ZwpLinuxBufferParamsV1},
    zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
};

use super::gpu::{self, Gpu, GpuFrame};

/// One window's frame, averaged to the size its picture is drawn at.
pub struct Frame {
    /// The `foreign_toplevel_identifier` sway reports for the window.
    pub id: String,
    pub width: u32,
    pub height: u32,
    pub buffer: GpuFrame,
}

/// A running capture of a set of windows. Dropping it stops the worker, which
/// closes its connection, and the compositor frees every session with it.
pub struct Stream {
    stop: Arc<AtomicBool>,
}

/// A piece of a window, as fractions of it: x, y, width, height in 0..=1.
pub type Crop = (f64, f64, f64, f64);

/// One window a stream captures.
#[derive(Clone, Debug, PartialEq)]
pub struct Want {
    /// The `foreign_toplevel_identifier` sway reports for the window.
    pub id: String,
    /// Only this piece of it, cut from the full buffer before scaling, so a
    /// small piece stays as sharp as the window is.
    pub crop: Option<Crop>,
    /// The box its picture is drawn in, in device pixels. The frame is
    /// that box's size where the window's shape fills it, and contained in
    /// it otherwise, never larger than the window: GTK then draws it one
    /// pixel to one.
    pub size: (u32, u32),
}

impl Stream {
    /// Capture every window of `wants`, each cut as it asks, until dropped.
    ///
    /// The worker outlives what goes wrong under it. A window whose session
    /// stops (it closed, or sway replaced its capture) is asked for again
    /// with a growing delay, and found again when it comes back under the
    /// same identifier; a lost connection is made again the same way. A
    /// picture fed by a stream therefore never freezes because the stream
    /// died: it freezes only while its window draws nothing.
    ///
    /// `fps` caps how often one window may send a frame; 0 is no cap, so a
    /// window sends a frame for every one the compositor renders for it.
    pub fn start_wants(wants: Vec<Want>, fps: u32, tx: async_channel::Sender<Frame>) -> Stream {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let spawned = std::thread::Builder::new()
            .name("jump-live".into())
            .spawn(move || {
                let interval = match fps {
                    0 => Duration::ZERO,
                    fps => Duration::from_millis(1000 / u64::from(fps)),
                };
                let mut backoff = RESTART_MIN;
                let ids: Vec<&str> = wants.iter().map(|w| w.id.as_str()).collect();
                log::info!("jump: capture start {ids:?} at {fps} fps");
                // Made on the first dmabuf device sway names, and kept across
                // reconnects: the context belongs to this thread.
                let mut gpu = GpuState::Untried;
                while !flag.load(Ordering::Relaxed) && !tx.is_closed() {
                    let started = Instant::now();
                    match run(&wants, interval, &tx, &flag, &mut gpu) {
                        Ok(()) => break,
                        // Every time: the backoff below spaces them out to
                        // one in 5 s at most, and a picture that froze needs
                        // the reason in the journal.
                        Err(e) => log::info!("jump: capture {ids:?}: {e}; again in {backoff:?}"),
                    }
                    if started.elapsed() > Duration::from_secs(10) {
                        backoff = RESTART_MIN;
                    }
                    let until = Instant::now() + backoff;
                    while Instant::now() < until && !flag.load(Ordering::Relaxed) {
                        std::thread::sleep(IDLE_POLL);
                    }
                    backoff = next_backoff(backoff);
                }
                let why = if flag.load(Ordering::Relaxed) {
                    "dropped"
                } else {
                    "nobody listening"
                };
                log::info!("jump: capture end {ids:?} ({why})");
            });
        if let Err(e) = spawned {
            log::warn!("jump: live capture thread: {e}");
        }
        Stream { stop }
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// How long the worker sleeps on the socket when nothing is due. It is also
/// how long a dropped [`Stream`] can take to notice.
const IDLE_POLL: Duration = Duration::from_millis(50);
/// The first delay before a stopped window, or a lost connection, is tried
/// again, and the longest one: each failure in a row doubles it.
const RESTART_MIN: Duration = Duration::from_millis(100);
const RESTART_MAX: Duration = Duration::from_secs(5);
/// How long a window goes without a frame before the journal hears of it.
const STALL: Duration = Duration::from_secs(10);
/// How soon a frame that found every output buffer on screen tries again:
/// GTK lets one go after its next render, well within a refresh.
const SLOT_RETRY: Duration = Duration::from_millis(4);

/// The delay after `d` when the retry after `d` failed too.
fn next_backoff(d: Duration) -> Duration {
    (d * 2).clamp(RESTART_MIN, RESTART_MAX)
}

/// The worker's GPU context, if it has one.
enum GpuState {
    /// The context could not be made, or GTK refused its frames.
    Off,
    /// Not needed yet.
    Untried,
    On(Rc<Gpu>),
}

impl GpuState {
    /// The context for sway's dmabuf device `dev`, made on first use.
    fn for_device(&mut self, dev: u64) -> Option<Rc<Gpu>> {
        if matches!(self, GpuState::Untried) {
            *self = match Gpu::open(dev) {
                Ok(g) => GpuState::On(Rc::new(g)),
                Err(e) => {
                    log::info!("jump: no gpu frames: {e}");
                    GpuState::Off
                }
            };
        }
        match self {
            GpuState::On(g) if g.serves(dev) && gpu::usable() => Some(g.clone()),
            _ => None,
        }
    }
}

fn run(
    wants: &[Want],
    interval: Duration,
    tx: &async_channel::Sender<Frame>,
    stop: &AtomicBool,
    gpu: &mut GpuState,
) -> Result<(), String> {
    let conn = Connection::connect_to_env().map_err(|e| format!("wayland connect: {e}"))?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    conn.display().get_registry(&qh, ());

    let mut state = State::default();
    // Every window's requests fall on one grid of `interval` steps, so the
    // windows of one picture answer together and the picture repaints once
    // per step rather than once per window.
    let epoch = Instant::now();
    // The first round trip brings the globals, the second the toplevels'
    // `identifier` events.
    for _ in 0..2 {
        queue
            .roundtrip(&mut state)
            .map_err(|e| format!("wayland roundtrip: {e}"))?;
    }
    let manager = state
        .manager
        .clone()
        .ok_or("compositor does not advertise ext-image-copy-capture-v1")?;
    let sources = state
        .toplevel_sources
        .clone()
        .ok_or("compositor does not advertise ext-foreign-toplevel-image-capture-source-v1")?;
    let dmabuf = state
        .dmabuf
        .clone()
        .ok_or("compositor does not advertise zwp_linux_dmabuf_v1 3")?;

    state.sessions = wants.iter().cloned().map(Session::new).collect();

    // Frames finished this step, held until every window asked in it has
    // answered or half a step has gone by. Windows answer on different
    // refreshes even when asked together, and a picture sent one window at
    // a time repaints once per window.
    let mut batch: Vec<Frame> = Vec::new();
    let mut batch_since: Option<Instant> = None;

    while !stop.load(Ordering::Relaxed) && !tx.is_closed() {
        let now = Instant::now();
        let mut wake = now + IDLE_POLL;
        // A toplevel came or went since the last pass: a window waiting for
        // its identifier looks again now instead of after its delay.
        let toplevels_changed = std::mem::take(&mut state.toplevels_changed);

        for (index, s) in state.sessions.iter_mut().enumerate() {
            if !s.stalled && now.saturating_duration_since(s.last_frame) > STALL {
                s.stalled = true;
                log::info!(
                    "jump: capture {}: no frame for {:?} ({})",
                    s.want.id,
                    STALL,
                    s.waiting_on(now)
                );
                // Sway answers a frame only on new damage, and has been seen
                // to stop answering one for a window that went on drawing.
                // A new session gets the window's current frame at once, so
                // a window that was only idle shows what it already showed
                // and one that was stuck catches up. Once per stall: a
                // frame resets `stalled`, and a window that never answers
                // again is not reopened in a loop.
                if s.capture.is_some() {
                    s.release();
                    s.retry_at = now;
                }
            }
            if s.capture.is_none() {
                if now < s.retry_at && !toplevels_changed {
                    wake = wake.min(s.retry_at);
                    continue;
                }
                let handle = state
                    .toplevels
                    .iter()
                    .find(|(_, ident)| ident.as_deref() == Some(s.want.id.as_str()))
                    .map(|(h, _)| h.clone());
                let Some(handle) = handle else {
                    // The first miss of a run; the retries after it say
                    // nothing new.
                    if s.backoff == RESTART_MIN {
                        log::info!("jump: capture {}: no such window yet", s.want.id);
                    }
                    s.retry_at = now + s.backoff;
                    s.backoff = next_backoff(s.backoff);
                    wake = wake.min(s.retry_at);
                    continue;
                };
                let source = sources.create_source(&handle, &qh, ());
                let session = manager.create_session(
                    &source,
                    ext_image_copy_capture_manager_v1::Options::empty(),
                    &qh,
                    (index, s.generation),
                );
                s.capture = Some((source, session));
                log::info!("jump: capture {}: session open", s.want.id);
                continue;
            }
            if s.ready
                && let Some(buffer) = &mut s.buffer
            {
                match buffer.frame(&s.want) {
                    Made::Frame(frame) => {
                        s.ready = false;
                        batch.push(frame);
                        batch_since.get_or_insert(now);
                    }
                    // Every output buffer is still GTK's. The frame stays in
                    // the capture buffer, and no new capture is asked for
                    // until it is out: sway answers one only on new damage,
                    // so a window that just went idle would otherwise keep
                    // showing an older frame until it next drew.
                    Made::Busy => {
                        wake = wake.min(now + SLOT_RETRY);
                        continue;
                    }
                    Made::Failed => s.ready = false,
                }
            }
            if s.frame.is_some() || !gpu::usable() {
                continue;
            }
            if s.buffer.is_none() {
                let Some(c) = s.constraints.clone() else {
                    continue;
                };
                if s.no_buffer.is_some() {
                    continue;
                }
                let made = match gpu.for_device(c.device) {
                    Some(g) => Buffer::new(g, &dmabuf, &c, &qh),
                    None => Err("no gpu context on sway's device".into()),
                };
                match made {
                    Ok(b) => s.buffer = Some(b),
                    // Said once; the window's picture keeps its icon until
                    // sway sends new constraints.
                    Err(e) => {
                        log::info!("jump: capture {}: no frames: {e}", s.want.id);
                        s.no_buffer = Some(e);
                        continue;
                    }
                }
            }
            let due = next_step(epoch, s.last, interval);
            if due > now {
                wake = wake.min(due);
                continue;
            }
            let (_, session) = s.capture.as_ref().expect("checked above");
            let buffer = s.buffer.as_ref().expect("built above");
            let frame = session.create_frame(&qh, (index, s.generation));
            let (width, height) = buffer.size();
            frame.attach_buffer(&buffer.buffer);
            frame.damage_buffer(0, 0, width as i32, height as i32);
            frame.capture();
            s.frame = Some(frame);
            // The cap counts from the request, so a compositor that takes a
            // frame's worth of time to answer does not halve the rate.
            s.last = now;
        }

        if let Some(since) = batch_since {
            let asked: Vec<Instant> = state
                .sessions
                .iter()
                .filter(|s| s.frame.is_some())
                .map(|s| s.last)
                .collect();
            if holds(now, since, interval, &asked) {
                wake = wake.min(since + interval / 2);
            } else {
                for frame in batch.drain(..) {
                    if tx.try_send(frame).is_err() {
                        return Ok(());
                    }
                }
                batch_since = None;
            }
        }

        let timeout = wake.saturating_duration_since(Instant::now());
        dispatch_for(&conn, &mut queue, &mut state, timeout)?;
    }

    for s in &mut state.sessions {
        s.release();
    }
    let _ = conn.flush();
    Ok(())
}

/// Whether a batch that began at `since` waits longer for the windows
/// still out: those in `asked` (when each window with a frame in flight
/// asked for it).
///
/// It waits at most half a step, and only for a window asked less than a
/// step ago. A window asked longer ago is idle: the compositor answers only
/// when it draws, which can be never, and a batch that waited for it held
/// every other window's frame back by half a step, every step.
fn holds(now: Instant, since: Instant, interval: Duration, asked: &[Instant]) -> bool {
    now < since + interval / 2
        && asked
            .iter()
            .any(|&a| now.saturating_duration_since(a) < interval)
}

/// The first step of the grid from `epoch` in `interval`s that is after
/// `last`; `last` itself when there is no cap.
fn next_step(epoch: Instant, last: Instant, interval: Duration) -> Instant {
    // No cap, or never asked yet: the first picture should not wait.
    if interval.is_zero() || last < epoch {
        return last;
    }
    let step = interval.as_nanos();
    let since = last.saturating_duration_since(epoch).as_nanos();
    let n = since / step + 1;
    epoch + Duration::from_nanos((n * step) as u64)
}

/// Flush, wait up to `timeout` for the socket, read and dispatch.
fn dispatch_for(
    conn: &Connection,
    queue: &mut wayland_client::EventQueue<State>,
    state: &mut State,
    timeout: Duration,
) -> Result<(), String> {
    queue
        .dispatch_pending(state)
        .map_err(|e| format!("wayland dispatch: {e}"))?;
    conn.flush().map_err(|e| format!("wayland flush: {e}"))?;
    let Some(guard) = conn.prepare_read() else {
        return Ok(());
    };
    let fd = std::os::fd::AsRawFd::as_raw_fd(&guard.connection_fd());
    let mut poll_fd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    let millis = timeout.as_millis().min(i32::MAX as u128) as i32;
    let polled = unsafe { libc::poll(&mut poll_fd, 1, millis) };
    if polled > 0 {
        guard.read().map_err(|e| format!("wayland read: {e}"))?;
    } else if polled < 0 {
        let err = std::io::Error::last_os_error();
        if err.kind() != std::io::ErrorKind::Interrupted {
            return Err(format!("poll: {err}"));
        }
    }
    queue
        .dispatch_pending(state)
        .map_err(|e| format!("wayland dispatch: {e}"))?;
    Ok(())
}

/// The pixels of `crop` in a `w` by `h` buffer, as x, y, width, height;
/// the whole buffer without one. At least one pixel each way.
fn region(crop: Option<Crop>, w: u32, h: u32) -> (u32, u32, u32, u32) {
    let Some((fx, fy, fw, fh)) = crop else {
        return (0, 0, w, h);
    };
    let (wf, hf) = (f64::from(w), f64::from(h));
    let x = ((fx * wf).round() as u32).min(w.saturating_sub(1));
    let y = ((fy * hf).round() as u32).min(h.saturating_sub(1));
    let rw = ((fw * wf).round() as u32).clamp(1, w - x);
    let rh = ((fh * hf).round() as u32).clamp(1, h - y);
    (x, y, rw, rh)
}

/// The size a `w` by `h` region is sent at to be drawn contained in a
/// `bw` by `bh` box, never larger than it is.
pub(super) fn out_size(w: u32, h: u32, (bw, bh): (u32, u32)) -> (u32, u32) {
    let s = (f64::from(bw) / f64::from(w.max(1)))
        .min(f64::from(bh) / f64::from(h.max(1)))
        .min(1.0);
    let px = |v: u32| ((f64::from(v) * s).round() as u32).max(1);
    (px(w), px(h))
}

// ── Per-window state ────────────────────────────────────────────────────

/// What sway can copy a window into: its size, and the dmabuf format the
/// shader reads that it offered, on its device.
#[derive(Clone, Debug, PartialEq)]
struct Constraints {
    width: u32,
    height: u32,
    /// The device number of sway's render node.
    device: u64,
    fourcc: u32,
    modifiers: Vec<u64>,
}

/// What averaging a captured frame gave.
enum Made {
    Frame(Frame),
    /// Every output buffer is still on screen; try the same frame again.
    Busy,
    Failed,
}

/// A window's dmabuf, which sway copies it into on the GPU, and the output
/// buffers its frames are averaged into.
struct Buffer {
    gpu: Rc<Gpu>,
    capture: Option<gpu::CaptureBuffer>,
    out: gpu::Pool,
    buffer: wl_buffer::WlBuffer,
}

impl Buffer {
    fn new(
        gpu: Rc<Gpu>,
        dmabuf: &ZwpLinuxDmabufV1,
        c: &Constraints,
        qh: &QueueHandle<State>,
    ) -> Result<Buffer, String> {
        let capture = gpu.capture_buffer(c.width, c.height, c.fourcc, &c.modifiers)?;
        // The modifier decides the cost: a linear buffer took the shader
        // 2.2 ms a frame at 2x, Intel's Tile4 0.6 ms (gpu.rs, the bench).
        log::info!(
            "jump: capture dmabuf {}x{} {:#x} modifier {:#x}",
            c.width,
            c.height,
            c.fourcc,
            capture.modifier
        );
        let params = dmabuf.create_params(qh, ());
        for (plane, (fd, offset, stride)) in capture.planes().enumerate() {
            params.add(
                fd,
                plane as u32,
                offset,
                stride,
                (capture.modifier >> 32) as u32,
                capture.modifier as u32,
            );
        }
        let buffer = params.create_immed(
            c.width as i32,
            c.height as i32,
            c.fourcc,
            zwp_linux_buffer_params_v1::Flags::empty(),
            qh,
            (),
        );
        params.destroy();
        Ok(Buffer {
            gpu,
            capture: Some(capture),
            out: gpu::Pool::default(),
            buffer,
        })
    }

    fn size(&self) -> (u32, u32) {
        self.capture
            .as_ref()
            .map_or((0, 0), |c| (c.width, c.height))
    }

    /// The frame sway just copied in, averaged to what `want` asks.
    fn frame(&mut self, want: &Want) -> Made {
        let (width, height) = self.size();
        let cut = region(want.crop, width, height);
        let (ow, oh) = out_size(cut.2, cut.3, want.size);
        let Some(capture) = self.capture.as_ref() else {
            return Made::Failed;
        };
        match self.gpu.downscale(capture, cut, (ow, oh), &mut self.out) {
            Ok(Some(buffer)) => Made::Frame(Frame {
                id: want.id.clone(),
                width: ow,
                height: oh,
                buffer,
            }),
            Ok(None) => Made::Busy,
            // Logged, and the frame let go: the next one tries again.
            Err(e) => {
                log::info!("jump: capture {}: gpu frame: {e}", want.id);
                Made::Failed
            }
        }
    }
}

impl Drop for Buffer {
    fn drop(&mut self) {
        self.buffer.destroy();
        if let Some(capture) = self.capture.take() {
            self.gpu.release_capture(capture);
        }
        self.gpu.release_pool(&mut self.out);
    }
}

struct Session {
    want: Want,
    /// The capture source and session while the window is being captured;
    /// `None` until its toplevel is found, and again after it stopped.
    capture: Option<(ExtImageCaptureSourceV1, ExtImageCopyCaptureSessionV1)>,
    /// Bumped whenever the capture is let go. Its session's and frames'
    /// events carry the number they were made under, and an event from an
    /// older one is ignored rather than landing on the new capture.
    generation: u32,
    /// When to look for the toplevel again, while `capture` is `None`.
    retry_at: Instant,
    /// The delay after the next failed try; back to the start on a frame.
    backoff: Duration,
    pending_size: Option<(u32, u32)>,
    pending_device: Option<u64>,
    /// Every dmabuf format offered in this round, with its modifiers.
    pending_dmabuf: Vec<(u32, Vec<u64>)>,
    constraints: Option<Constraints>,
    /// Why this window has no buffer, once that was said; cleared by new
    /// constraints.
    no_buffer: Option<String>,
    buffer: Option<Buffer>,
    /// The frame in flight, if any. One at a time per window.
    frame: Option<ExtImageCopyCaptureFrameV1>,
    /// Set when the frame in flight turned `ready`; the loop sends it.
    ready: bool,
    /// When the last frame was asked for, for the frame cap.
    last: Instant,
    /// When the last frame arrived, for the stall line in the journal.
    last_frame: Instant,
    /// The stall line was written, and no frame has come since.
    stalled: bool,
}

impl Session {
    fn new(want: Want) -> Session {
        let now = Instant::now();
        Session {
            want,
            capture: None,
            generation: 0,
            retry_at: now,
            backoff: RESTART_MIN,
            pending_size: None,
            pending_device: None,
            pending_dmabuf: Vec::new(),
            constraints: None,
            no_buffer: None,
            buffer: None,
            frame: None,
            ready: false,
            last: now - Duration::from_secs(1),
            last_frame: now,
            stalled: false,
        }
    }

    /// What a window without frames is waiting on, for the journal. A
    /// frame asked for and unanswered is the compositor's (it answers only
    /// when the window draws, so an idle window lands here too); anything
    /// else is this side's.
    fn waiting_on(&self, now: Instant) -> String {
        if self.capture.is_none() {
            return "no session: the window is not found or sway stopped it".into();
        }
        let Some(c) = &self.constraints else {
            return "session open, sway sent no dmabuf the shader reads".into();
        };
        if let Some(why) = &self.no_buffer {
            return format!("no buffer: {why}");
        }
        if self.frame.is_some() {
            return format!(
                "frame asked {:?} ago, unanswered; buffer {}x{}",
                now.saturating_duration_since(self.last),
                c.width,
                c.height
            );
        }
        format!(
            "nothing asked; buffer {}x{} {}",
            c.width,
            c.height,
            if self.buffer.is_some() { "built" } else { "not built" }
        )
    }

    /// Let go of the capture and everything made for it.
    fn release(&mut self) {
        if let Some(frame) = self.frame.take() {
            frame.destroy();
        }
        if let Some((source, session)) = self.capture.take() {
            session.destroy();
            source.destroy();
        }
        self.buffer = None;
        self.constraints = None;
        self.pending_size = None;
        self.pending_device = None;
        self.pending_dmabuf.clear();
        self.no_buffer = None;
        self.ready = false;
        self.generation = self.generation.wrapping_add(1);
    }

    /// The compositor stopped the capture: the window closed, or sway let
    /// go of what it captured from. Try again after the delay; a window
    /// that is still there, or back under the same identifier, picks up.
    fn stopped(&mut self) {
        log::info!(
            "jump: capture {}: stopped by sway; again in {:?}",
            self.want.id,
            self.backoff
        );
        self.release();
        self.retry_at = Instant::now() + self.backoff;
        self.backoff = next_backoff(self.backoff);
    }

    /// Whether an event made under `generation` is this capture's.
    fn current(&self, generation: u32) -> bool {
        self.capture.is_some() && self.generation == generation
    }
}

#[derive(Default)]
struct State {
    manager: Option<ExtImageCopyCaptureManagerV1>,
    toplevel_sources: Option<ExtForeignToplevelImageCaptureSourceManagerV1>,
    dmabuf: Option<ZwpLinuxDmabufV1>,
    toplevels: Vec<(ExtForeignToplevelHandleV1, Option<String>)>,
    /// A toplevel was named or closed since the loop last looked.
    toplevels_changed: bool,
    sessions: Vec<Session>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        else {
            return;
        };
        match interface.as_str() {
            "ext_image_copy_capture_manager_v1" => {
                state.manager = Some(registry.bind(name, 1, qh, ()));
            }
            "ext_foreign_toplevel_image_capture_source_manager_v1" => {
                state.toplevel_sources = Some(registry.bind(name, 1, qh, ()));
            }
            "ext_foreign_toplevel_list_v1" => {
                let _: ExtForeignToplevelListV1 = registry.bind(name, 1, qh, ());
            }
            // Version 3 has `create_immed`; the formats come from the
            // capture session, not from this global.
            "zwp_linux_dmabuf_v1" if version >= 3 => {
                state.dmabuf = Some(registry.bind(name, 3, qh, ()));
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtImageCopyCaptureSessionV1, (usize, u32)> for State {
    fn event(
        state: &mut Self,
        _: &ExtImageCopyCaptureSessionV1,
        event: ext_image_copy_capture_session_v1::Event,
        &(index, generation): &(usize, u32),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use ext_image_copy_capture_session_v1::Event;
        let Some(s) = state.sessions.get_mut(index) else {
            return;
        };
        if !s.current(generation) {
            return;
        }
        match event {
            Event::BufferSize { width, height } => s.pending_size = Some((width, height)),
            Event::DmabufDevice { device } => {
                s.pending_device = device
                    .get(..8)
                    .and_then(|b| b.try_into().ok())
                    .map(u64::from_ne_bytes);
            }
            Event::DmabufFormat { format, modifiers } => {
                let modifiers = modifiers
                    .chunks_exact(8)
                    .map(|b| u64::from_ne_bytes(b.try_into().expect("8 bytes")))
                    .collect();
                s.pending_dmabuf.push((format, modifiers));
            }
            Event::Done => {
                // Alpha over none (gpu::rank), so a translucent window keeps
                // its transparency where sway offers a format with it.
                let best = std::mem::take(&mut s.pending_dmabuf)
                    .into_iter()
                    .filter_map(|(f, m)| gpu::rank(f).map(|r| (r, f, m)))
                    .min_by_key(|(r, _, _)| *r);
                if let (Some((width, height)), Some(device), Some((_, fourcc, modifiers))) =
                    (s.pending_size, s.pending_device, best)
                {
                    let next = Constraints {
                        width,
                        height,
                        device,
                        fourcc,
                        modifiers,
                    };
                    // A resized window: the old buffer no longer fits. The
                    // loop builds a new one before the next frame. A frame
                    // in flight in the old one fails with
                    // `buffer_constraints` and is asked again.
                    if s.constraints.as_ref() != Some(&next) {
                        log::info!(
                            "jump: capture {}: buffer {width}x{height} {fourcc:#x}, {} modifiers",
                            s.want.id,
                            next.modifiers.len()
                        );
                        s.buffer = None;
                        s.no_buffer = None;
                        // A frame waiting for an output buffer was in the old
                        // capture buffer; the new one holds nothing yet.
                        s.ready = false;
                    }
                    s.constraints = Some(next);
                }
                s.pending_size = None;
                s.pending_device = None;
                s.pending_dmabuf.clear();
            }
            Event::Stopped => s.stopped(),
            _ => {}
        }
    }
}

impl Dispatch<ExtImageCopyCaptureFrameV1, (usize, u32)> for State {
    fn event(
        state: &mut Self,
        frame: &ExtImageCopyCaptureFrameV1,
        event: ext_image_copy_capture_frame_v1::Event,
        &(index, generation): &(usize, u32),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use ext_image_copy_capture_frame_v1::{Event, FailureReason};
        let Some(s) = state.sessions.get_mut(index) else {
            return;
        };
        if !s.current(generation) {
            // A frame of a capture already let go; `release` destroyed the
            // one in flight, so this is only a late event.
            return;
        }
        match event {
            Event::Ready => {
                frame.destroy();
                s.frame = None;
                s.ready = true;
                let now = Instant::now();
                if std::mem::take(&mut s.stalled) {
                    log::info!(
                        "jump: capture {}: frames again after {:?}",
                        s.want.id,
                        now.saturating_duration_since(s.last_frame)
                    );
                }
                s.last_frame = now;
                s.backoff = RESTART_MIN;
            }
            Event::Failed { reason } => {
                frame.destroy();
                s.frame = None;
                log::info!("jump: capture {}: frame failed: {reason:?}", s.want.id);
                match reason.into_result() {
                    // New constraints come before this; the `done` that
                    // closed them dropped the buffer.
                    Ok(FailureReason::BufferConstraints) => {}
                    Ok(FailureReason::Stopped) => s.stopped(),
                    // Try again on the next step.
                    _ => s.last = Instant::now(),
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtForeignToplevelListV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ExtForeignToplevelListV1,
        event: ext_foreign_toplevel_list_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_foreign_toplevel_list_v1::Event::Toplevel { toplevel } = event {
            state.toplevels.push((toplevel, None));
        }
    }

    wayland_client::event_created_child!(State, ExtForeignToplevelListV1, [
        ext_foreign_toplevel_list_v1::EVT_TOPLEVEL_OPCODE => (ExtForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ExtForeignToplevelHandleV1, ()> for State {
    fn event(
        state: &mut Self,
        handle: &ExtForeignToplevelHandleV1,
        event: ext_foreign_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_foreign_toplevel_handle_v1::Event::Identifier { identifier } => {
                if let Some(slot) = state
                    .toplevels
                    .iter_mut()
                    .find(|(h, _)| h.id() == handle.id())
                {
                    slot.1 = Some(identifier);
                    state.toplevels_changed = true;
                }
            }
            // A closed toplevel is gone for good; its handle is only ours
            // to free.
            ext_foreign_toplevel_handle_v1::Event::Closed => {
                state.toplevels.retain(|(h, _)| h.id() != handle.id());
                handle.destroy();
                state.toplevels_changed = true;
            }
            _ => {}
        }
    }
}

delegate_noop!(State: ignore ExtImageCopyCaptureManagerV1);
delegate_noop!(State: ignore ExtForeignToplevelImageCaptureSourceManagerV1);
delegate_noop!(State: ignore ExtImageCaptureSourceV1);
delegate_noop!(State: ignore wl_buffer::WlBuffer);
delegate_noop!(State: ignore ZwpLinuxDmabufV1);
delegate_noop!(State: ignore ZwpLinuxBufferParamsV1);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_land_on_one_grid() {
        let epoch = Instant::now();
        let step = Duration::from_millis(50);
        // Two windows last asked at different moments of one step are next
        // asked at the same moment.
        let a = next_step(epoch, epoch + Duration::from_millis(3), step);
        let b = next_step(epoch, epoch + Duration::from_millis(41), step);
        assert_eq!(a, b);
        assert_eq!(a, epoch + step);
        // A request on a step waits for the next one, not zero time.
        assert_eq!(next_step(epoch, epoch + step, step), epoch + step * 2);
        // No cap: due at once.
        assert_eq!(next_step(epoch, epoch, Duration::ZERO), epoch);
    }

    #[test]
    fn a_window_that_never_answers_holds_a_batch_one_step_at_most() {
        let epoch = Instant::now();
        let step = Duration::from_millis(33);
        let at = |ms| epoch + Duration::from_millis(ms);
        // Asked with the rest this step, not answered yet: wait for it.
        assert!(holds(at(105), at(104), step, &[at(99)]));
        // Half a step on, the batch goes without it.
        assert!(!holds(at(121), at(104), step, &[at(99)]));
        // Asked a step or more ago: idle, not waited for at all.
        assert!(!holds(at(105), at(104), step, &[at(60)]));
        // Nothing out: go.
        assert!(!holds(at(105), at(104), step, &[]));
        // No cap: never held.
        assert!(!holds(at(105), at(104), Duration::ZERO, &[at(105)]));
    }

    #[test]
    fn restarts_back_off_to_a_ceiling() {
        let mut d = RESTART_MIN;
        let mut seen = vec![d];
        for _ in 0..10 {
            d = next_backoff(d);
            seen.push(d);
        }
        assert_eq!(seen[1], RESTART_MIN * 2);
        assert!(seen.windows(2).all(|w| w[1] >= w[0]));
        assert_eq!(*seen.last().unwrap(), RESTART_MAX);
    }

    #[test]
    fn a_frame_is_sent_at_the_size_it_is_drawn() {
        // A 700x870 window drawn in a 200x245 box: height binds.
        assert_eq!(out_size(700, 870, (200, 245)), (197, 245));
        // Wide in a tall box: width binds.
        assert_eq!(out_size(2880, 1800, (800, 800)), (800, 500));
        // Smaller than the box: never scaled up.
        assert_eq!(out_size(100, 60, (400, 250)), (100, 60));
    }

    #[test]
    fn a_crop_in_fractions_becomes_buffer_pixels() {
        // A 2x buffer of a 1440x900 window: the fractions do not care.
        assert_eq!(
            region(Some((0.25, 0.5, 0.5, 0.25)), 2880, 1800),
            (720, 900, 1440, 450)
        );
        assert_eq!(region(None, 2880, 1800), (0, 0, 2880, 1800));
        // A crop running off the edge keeps at least a pixel, and stays in.
        assert_eq!(region(Some((1.0, 1.0, 0.5, 0.5)), 100, 100), (99, 99, 1, 1));
    }
}

#[cfg(test)]
mod session {
    //! Against the real session. Ignored: needs a compositor.
    //!
    //! `JUMP_LIVE_IDS` is a comma-separated list of sway
    //! `foreign_toplevel_identifier`s; every window is captured for three
    //! seconds and the test prints how many frames each one sent.

    #[test]
    #[ignore]
    fn windows_keep_sending_frames() {
        let ids: Vec<String> = std::env::var("JUMP_LIVE_IDS")
            .expect("JUMP_LIVE_IDS")
            .split(',')
            .map(str::to_string)
            .collect();
        let (tx, rx) = async_channel::unbounded();
        let wants = ids
            .iter()
            .map(|id| super::Want {
                id: id.clone(),
                crop: None,
                size: (640, 400),
            })
            .collect();
        let stream = super::Stream::start_wants(wants, 30, tx);
        std::thread::sleep(std::time::Duration::from_secs(3));
        drop(stream);
        let mut counts = std::collections::HashMap::<String, (usize, u32, u32)>::new();
        while let Ok(frame) = rx.try_recv() {
            let e = counts.entry(frame.id).or_default();
            *e = (e.0 + 1, frame.width, frame.height);
        }
        for id in ids {
            let (n, w, h) = counts.get(&id).copied().unwrap_or_default();
            println!("{id}: {n} frames, {w}x{h}");
        }
    }
}
