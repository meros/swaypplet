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
//! connection for every window on the card, and one shm buffer per window,
//! reused for every frame.
//!
//! Frames are box-filtered down on the worker before they cross to GTK. A
//! window on a 2x panel is 16 MB a frame, and the card draws it at about
//! 300 px wide; uploading the full buffer at 20 frames a second for five
//! windows would spend more memory bandwidth than everything else on screen.

use std::os::fd::{AsFd, OwnedFd};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use wayland_client::protocol::{wl_buffer, wl_registry, wl_shm, wl_shm_pool};
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

/// One window's pixels, scaled down: premultiplied BGRA, rows `width * 4`
/// bytes apart, which is `gdk::MemoryFormat::B8g8r8a8Premultiplied`.
pub struct Frame {
    /// The `foreign_toplevel_identifier` sway reports for the window.
    pub id: String,
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// A running capture of a set of windows. Dropping it stops the worker, which
/// closes its connection, and the compositor frees every session with it.
pub struct Stream {
    stop: Arc<AtomicBool>,
}

/// A piece of a window, as fractions of it: x, y, width, height in 0..=1.
pub type Crop = (f64, f64, f64, f64);

/// How far a window's frames are cut down before they cross to GTK.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Size {
    /// The longer side at most this many pixels.
    MaxEdge(u32),
    /// Drawn contained in a box this many device pixels: cut by the largest
    /// whole factor that still covers the box, so GTK scales the frame down
    /// by less than 2x and never up. A box and not an edge, because a pin's
    /// windows are drawn at many sizes, and a cut to the picture's edge left
    /// a small window's frame several times the pixels it is drawn at, or a
    /// large one below them.
    Draw(u32, u32),
}

/// One window a stream captures.
#[derive(Clone, Debug, PartialEq)]
pub struct Want {
    /// The `foreign_toplevel_identifier` sway reports for the window.
    pub id: String,
    /// Only this piece of it, cut from the full buffer before scaling, so a
    /// small piece stays as sharp as the window is.
    pub crop: Option<Crop>,
    pub size: Size,
}

impl Stream {
    /// Capture the windows named by `ids` until dropped.
    ///
    /// `max_edge` bounds the longer side of every frame sent, in pixels.
    /// `fps` caps how often one window may send a frame; 0 is no cap, so a
    /// window sends a frame for every one the compositor renders for it.
    pub fn start(
        ids: Vec<String>,
        max_edge: u32,
        fps: u32,
        tx: async_channel::Sender<Frame>,
    ) -> Stream {
        Stream::start_wants(
            ids.into_iter()
                .map(|id| Want {
                    id,
                    crop: None,
                    size: Size::MaxEdge(max_edge),
                })
                .collect(),
            fps,
            tx,
        )
    }

    /// Capture every window of `wants`, each cut as it asks, until dropped.
    ///
    /// The worker outlives what goes wrong under it. A window whose session
    /// stops (it closed, or sway replaced its capture) is asked for again
    /// with a growing delay, and found again when it comes back under the
    /// same identifier; a lost connection is made again the same way. A
    /// picture fed by a stream therefore never freezes because the stream
    /// died: it freezes only while its window draws nothing.
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
                let mut warned = false;
                while !flag.load(Ordering::Relaxed) && !tx.is_closed() {
                    let started = Instant::now();
                    match run(&wants, interval, &tx, &flag) {
                        Ok(()) => break,
                        // Warn once; a compositor without the protocol
                        // would otherwise say so every few seconds.
                        Err(e) if !warned => {
                            log::warn!("jump: live capture: {e}; trying again");
                            warned = true;
                        }
                        Err(e) => log::debug!("jump: live capture: {e}"),
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

/// The delay after `d` when the retry after `d` failed too.
fn next_backoff(d: Duration) -> Duration {
    (d * 2).clamp(RESTART_MIN, RESTART_MAX)
}

fn run(
    wants: &[Want],
    interval: Duration,
    tx: &async_channel::Sender<Frame>,
    stop: &AtomicBool,
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
    let shm = state.shm.clone().ok_or("compositor has no wl_shm")?;

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
                    log::debug!("jump: no toplevel with identifier {}", s.want.id);
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
                continue;
            }
            if std::mem::take(&mut s.ready)
                && let Some(buffer) = &s.buffer
            {
                let (width, height, pixels) = downscale(
                    buffer.memory.as_slice(),
                    buffer.width,
                    region(s.want.crop, buffer.width, buffer.height),
                    buffer.format,
                    s.want.size,
                );
                batch.push(Frame {
                    id: s.want.id.clone(),
                    width,
                    height,
                    pixels,
                });
                batch_since.get_or_insert(now);
            }
            if s.frame.is_some() {
                continue;
            }
            if s.buffer.is_none() {
                if let Some(c) = s.constraints.clone() {
                    s.buffer = Some(Buffer::new(&shm, &c, &qh)?);
                } else {
                    continue;
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
            frame.attach_buffer(&buffer.buffer);
            frame.damage_buffer(0, 0, buffer.width as i32, buffer.height as i32);
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

/// The size a `w` by `h` region is sent at for `size`, never larger than
/// it is.
fn out_size(w: u32, h: u32, size: Size) -> (u32, u32) {
    match size {
        Size::MaxEdge(edge) => {
            let f = w.max(h).div_ceil(edge.max(1)).max(1);
            ((w / f).max(1), (h / f).max(1))
        }
        // Contained in the box, as `LivePicture` draws it: then GTK draws
        // the frame one pixel to one, and has nothing left to filter.
        Size::Draw(bw, bh) => {
            let s = (f64::from(bw) / f64::from(w.max(1)))
                .min(f64::from(bh) / f64::from(h.max(1)))
                .min(1.0);
            let px = |v: u32| ((f64::from(v) * s).round() as u32).max(1);
            (px(w), px(h))
        }
    }
}

/// How `n` output pixels cover `len` source pixels from `start`: for each,
/// the first source pixel it covers and the share of it each covered one
/// has, summing to 1.
fn spans(start: u32, len: u32, n: u32) -> Vec<(usize, Vec<f32>)> {
    let step = f64::from(len) / f64::from(n.max(1));
    (0..n)
        .map(|o| {
            let (a, b) = (f64::from(o) * step, f64::from(o + 1) * step);
            let first = a.floor() as u32;
            let last = (b.ceil() as u32).clamp(first + 1, len.max(first + 1));
            let weights = (first..last)
                .map(|i| ((b.min(f64::from(i) + 1.0) - a.max(f64::from(i))) / step) as f32)
                .collect();
            ((start + first) as usize, weights)
        })
        .collect()
}

/// Scale the region `(x0, y0, w, h)` of a buffer `full_w` pixels wide down
/// to [`out_size`], every output pixel the average of the source area it
/// covers, fractions of pixels at its edges included.
///
/// An area average and not a whole-factor box then GTK's linear filter:
/// the two in a row blurred a terminal's text in a pin (a 3x box, then
/// 233 px drawn at 200), and a linear filter alone over more than 2x skips
/// pixels and shimmers. `xrgb` carries no alpha, so it is written opaque;
/// `argb` from the compositor is already premultiplied, and an average of
/// premultiplied pixels stays premultiplied.
fn downscale(
    src: &[u8],
    full_w: u32,
    (x0, y0, w, h): (u32, u32, u32, u32),
    format: wl_shm::Format,
    size: Size,
) -> (u32, u32, Vec<u8>) {
    let (ow, oh) = out_size(w, h, size);
    let stride = (full_w * 4) as usize;
    let opaque = matches!(format, wl_shm::Format::Xrgb8888 | wl_shm::Format::Xbgr8888);
    // Both ABGR formats are RGBA in memory; the card wants BGRA.
    let swap = matches!(format, wl_shm::Format::Xbgr8888 | wl_shm::Format::Abgr8888);

    // Across each source row first, then down the columns.
    let across = spans(x0, w, ow);
    let row_w = ow as usize * 4;
    let mut rows = vec![0f32; h as usize * row_w];
    for sy in 0..h as usize {
        let line = (y0 as usize + sy) * stride;
        let dst = &mut rows[sy * row_w..(sy + 1) * row_w];
        for (ox, (first, weights)) in across.iter().enumerate() {
            let mut acc = [0f32; 4];
            for (k, wt) in weights.iter().enumerate() {
                let i = line + (first + k) * 4;
                for c in 0..4 {
                    acc[c] += wt * f32::from(src[i + c]);
                }
            }
            dst[ox * 4..ox * 4 + 4].copy_from_slice(&acc);
        }
    }

    let mut out = vec![0u8; ow as usize * oh as usize * 4];
    for (oy, (first, weights)) in spans(0, h, oh).iter().enumerate() {
        for ox in 0..ow as usize {
            let mut acc = [0f32; 4];
            for (k, wt) in weights.iter().enumerate() {
                let i = (first + k) * row_w + ox * 4;
                for c in 0..4 {
                    acc[c] += wt * rows[i + c];
                }
            }
            let px = |v: f32| v.round().clamp(0.0, 255.0) as u8;
            let (b, r) = if swap {
                (acc[2], acc[0])
            } else {
                (acc[0], acc[2])
            };
            let o = (oy * ow as usize + ox) * 4;
            out[o] = px(b);
            out[o + 1] = px(acc[1]);
            out[o + 2] = px(r);
            out[o + 3] = if opaque { 0xff } else { px(acc[3]) };
        }
    }
    (ow, oh, out)
}

// ── Per-window state ────────────────────────────────────────────────────

#[derive(Clone, PartialEq)]
struct Constraints {
    width: u32,
    height: u32,
    format: wl_shm::Format,
}

struct Buffer {
    memory: Shm,
    pool: wl_shm_pool::WlShmPool,
    buffer: wl_buffer::WlBuffer,
    width: u32,
    height: u32,
    format: wl_shm::Format,
}

impl Buffer {
    fn new(
        shm: &wl_shm::WlShm,
        c: &Constraints,
        qh: &QueueHandle<State>,
    ) -> Result<Buffer, String> {
        let stride = c.width * 4;
        let len = (stride * c.height) as usize;
        let memory = Shm::new(len)?;
        let pool = shm.create_pool(memory.fd.as_fd(), len as i32, qh, ());
        let buffer = pool.create_buffer(
            0,
            c.width as i32,
            c.height as i32,
            stride as i32,
            c.format,
            qh,
            (),
        );
        Ok(Buffer {
            memory,
            pool,
            buffer,
            width: c.width,
            height: c.height,
            format: c.format,
        })
    }
}

impl Drop for Buffer {
    fn drop(&mut self) {
        self.buffer.destroy();
        self.pool.destroy();
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
    pending_format: Option<wl_shm::Format>,
    constraints: Option<Constraints>,
    buffer: Option<Buffer>,
    /// The frame in flight, if any. One at a time per window.
    frame: Option<ExtImageCopyCaptureFrameV1>,
    /// Set when the frame in flight turned `ready`; the loop sends it.
    ready: bool,
    /// When the last frame was asked for, for the frame cap.
    last: Instant,
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
            pending_format: None,
            constraints: None,
            buffer: None,
            frame: None,
            ready: false,
            last: now - Duration::from_secs(1),
        }
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
        self.pending_format = None;
        self.ready = false;
        self.generation = self.generation.wrapping_add(1);
    }

    /// The compositor stopped the capture: the window closed, or sway let
    /// go of what it captured from. Try again after the delay; a window
    /// that is still there, or back under the same identifier, picks up.
    fn stopped(&mut self) {
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
    shm: Option<wl_shm::WlShm>,
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
            name, interface, ..
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
            "wl_shm" => state.shm = Some(registry.bind(name, 1, qh, ())),
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
            Event::ShmFormat { format } => {
                // A format with alpha wins over one without. A translucent
                // terminal captured as `xrgb` comes out black where the desktop
                // shows through it, and in the tile the ground should.
                // swayfx 0.6 offers only `Xbgr8888` for a toplevel, and its X
                // byte is 255 everywhere (measured 2026-09-24), so today this
                // picks that; it is ready for the day an alpha format is
                // offered.
                if let Ok(format) = format.into_result() {
                    let alpha =
                        matches!(format, wl_shm::Format::Argb8888 | wl_shm::Format::Abgr8888);
                    let opaque =
                        matches!(format, wl_shm::Format::Xrgb8888 | wl_shm::Format::Xbgr8888);
                    let had_alpha = matches!(
                        s.pending_format,
                        Some(wl_shm::Format::Argb8888 | wl_shm::Format::Abgr8888)
                    );
                    if alpha && !had_alpha || opaque && s.pending_format.is_none() {
                        s.pending_format = Some(format);
                    }
                }
            }
            Event::Done => {
                if let (Some((width, height)), Some(format)) = (s.pending_size, s.pending_format) {
                    let next = Constraints {
                        width,
                        height,
                        format,
                    };
                    // A resized window: the old buffer no longer fits. The
                    // loop builds a new one before the next frame. A frame
                    // in flight in the old one fails with
                    // `buffer_constraints` and is asked again.
                    if s.constraints.as_ref() != Some(&next) {
                        s.buffer = None;
                    }
                    s.constraints = Some(next);
                }
                s.pending_size = None;
                s.pending_format = None;
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
                s.backoff = RESTART_MIN;
            }
            Event::Failed { reason } => {
                frame.destroy();
                s.frame = None;
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
delegate_noop!(State: ignore wl_shm::WlShm);
delegate_noop!(State: ignore wl_shm_pool::WlShmPool);
delegate_noop!(State: ignore wl_buffer::WlBuffer);

// ── Shared memory ───────────────────────────────────────────────────────

/// An anonymous shared mapping the compositor draws into.
struct Shm {
    fd: OwnedFd,
    ptr: *mut libc::c_void,
    len: usize,
}

impl Shm {
    fn new(len: usize) -> Result<Shm, String> {
        let fd = unsafe { libc::memfd_create(c"swaypplet-jump".as_ptr(), libc::MFD_CLOEXEC) };
        if fd < 0 {
            return Err(format!("memfd_create: {}", std::io::Error::last_os_error()));
        }
        let fd = unsafe { <OwnedFd as std::os::fd::FromRawFd>::from_raw_fd(fd) };
        if unsafe { libc::ftruncate(std::os::fd::AsRawFd::as_raw_fd(&fd), len as i64) } < 0 {
            return Err(format!("ftruncate: {}", std::io::Error::last_os_error()));
        }
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                std::os::fd::AsRawFd::as_raw_fd(&fd),
                0,
            )
        };
        if ptr == libc::MAP_FAILED {
            return Err(format!("mmap: {}", std::io::Error::last_os_error()));
        }
        Ok(Shm { fd, ptr, len })
    }

    fn as_slice(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr.cast::<u8>(), self.len) }
    }
}

impl Drop for Shm {
    fn drop(&mut self) {
        unsafe { libc::munmap(self.ptr, self.len) };
    }
}

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
        assert_eq!(out_size(700, 870, Size::Draw(200, 245)), (197, 245));
        // Wide in a tall box: width binds.
        assert_eq!(out_size(2880, 1800, Size::Draw(800, 800)), (800, 500));
        // Smaller than the box: never scaled up.
        assert_eq!(out_size(100, 60, Size::Draw(400, 250)), (100, 60));
        assert_eq!(out_size(2560, 1600, Size::MaxEdge(320)), (320, 200));
    }

    #[test]
    fn fractional_spans_share_their_edge_pixels() {
        // Three pixels into two: 1.5 each, the middle one split.
        let s = spans(10, 3, 2);
        assert_eq!(s[0].0, 10);
        assert_eq!(s[0].1, vec![2.0 / 3.0, 1.0 / 3.0]);
        assert_eq!(s[1].0, 11);
        assert_eq!(s[1].1, vec![1.0 / 3.0, 2.0 / 3.0]);
        // One to one: a copy.
        assert!(spans(0, 4, 4).iter().all(|(_, w)| w == &vec![1.0]));
    }

    #[test]
    fn an_edge_between_two_colours_averages_by_area() {
        // 3x1 XRGB, blue 0, 90, 180 in BGRX: into 2x1, 1.5 source each.
        let src = [0, 0, 0, 0, 90, 0, 0, 0, 180, 0, 0, 0];
        let (w, h, px) = downscale(
            &src,
            3,
            (0, 0, 3, 1),
            wl_shm::Format::Xrgb8888,
            Size::Draw(2, 1),
        );
        assert_eq!((w, h), (2, 1));
        assert_eq!((px[0], px[4]), (30, 150));
    }

    #[test]
    fn the_longer_edge_fits_and_the_aspect_holds() {
        let src = vec![0u8; 2560 * 1600 * 4];
        let (w, h, px) = downscale(
            &src,
            2560,
            (0, 0, 2560, 1600),
            wl_shm::Format::Xrgb8888,
            Size::MaxEdge(320),
        );
        assert_eq!((w, h), (320, 200));
        assert_eq!(px.len(), 320 * 200 * 4);
    }

    #[test]
    fn each_output_pixel_averages_its_square() {
        // 2x2 XRGB, BGRX in memory: two black pixels, two at blue 200.
        let src = [0, 0, 0, 0, 0, 0, 0, 0, 200, 0, 0, 0, 200, 0, 0, 0];
        let (w, h, px) = downscale(
            &src,
            2,
            (0, 0, 2, 2),
            wl_shm::Format::Xrgb8888,
            Size::MaxEdge(1),
        );
        assert_eq!((w, h), (1, 1));
        assert_eq!(px, vec![100, 0, 0, 0xff]);
    }

    #[test]
    fn abgr_is_swapped_to_bgra_and_keeps_alpha() {
        let src = [10u8, 20, 30, 128];
        let (_, _, px) = downscale(
            &src,
            1,
            (0, 0, 1, 1),
            wl_shm::Format::Abgr8888,
            Size::MaxEdge(4),
        );
        assert_eq!(px, vec![30, 20, 10, 128]);
    }

    #[test]
    fn a_crop_is_cut_from_the_full_buffer_before_scaling() {
        // 4x2 XRGB: the right half is blue 200, the left black.
        let mut src = vec![0u8; 4 * 2 * 4];
        for y in 0..2 {
            for x in 2..4 {
                src[(y * 4 + x) * 4] = 200;
            }
        }
        let (w, h, px) = downscale(
            &src,
            4,
            (2, 0, 2, 2),
            wl_shm::Format::Xrgb8888,
            Size::MaxEdge(8),
        );
        assert_eq!((w, h), (2, 2), "unscaled: the piece is small");
        assert!(
            px.chunks_exact(4).all(|p| p[0] == 200),
            "only the blue half"
        );
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

    #[test]
    fn a_small_window_is_not_scaled_up() {
        let src = vec![7u8; 10 * 6 * 4];
        let (w, h, _) = downscale(
            &src,
            10,
            (0, 0, 10, 6),
            wl_shm::Format::Argb8888,
            Size::MaxEdge(320),
        );
        assert_eq!((w, h), (10, 6));
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
        let stream = super::Stream::start(ids.clone(), 320, 30, tx);
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
