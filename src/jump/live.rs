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
        Stream::spawn(
            ids.into_iter().map(|id| (id, None)).collect(),
            max_edge,
            fps,
            tx,
        )
    }

    /// Capture one piece of one window until dropped: every frame is cut to
    /// `crop` at the buffer's full resolution before it is scaled, so a small
    /// piece stays as sharp as the window is.
    pub fn start_region(
        id: String,
        crop: Crop,
        max_edge: u32,
        fps: u32,
        tx: async_channel::Sender<Frame>,
    ) -> Stream {
        Stream::spawn(vec![(id, Some(crop))], max_edge, fps, tx)
    }

    fn spawn(
        ids: Vec<(String, Option<Crop>)>,
        max_edge: u32,
        fps: u32,
        tx: async_channel::Sender<Frame>,
    ) -> Stream {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let spawned = std::thread::Builder::new()
            .name("jump-live".into())
            .spawn(move || {
                let interval = match fps {
                    0 => Duration::ZERO,
                    fps => Duration::from_millis(1000 / u64::from(fps)),
                };
                if let Err(e) = run(&ids, max_edge, interval, &tx, &flag) {
                    log::warn!("jump: live capture: {e}");
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

fn run(
    ids: &[(String, Option<Crop>)],
    max_edge: u32,
    interval: Duration,
    tx: &async_channel::Sender<Frame>,
    stop: &AtomicBool,
) -> Result<(), String> {
    let conn = Connection::connect_to_env().map_err(|e| format!("wayland connect: {e}"))?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    conn.display().get_registry(&qh, ());

    let mut state = State::default();
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

    for (id, crop) in ids {
        let Some(handle) = state
            .toplevels
            .iter()
            .find(|(_, ident)| ident.as_deref() == Some(id.as_str()))
            .map(|(h, _)| h.clone())
        else {
            log::debug!("jump: no toplevel with identifier {id}");
            continue;
        };
        let index = state.sessions.len();
        let source = sources.create_source(&handle, &qh, ());
        let session = manager.create_session(
            &source,
            ext_image_copy_capture_manager_v1::Options::empty(),
            &qh,
            index,
        );
        state
            .sessions
            .push(Session::new(id.clone(), *crop, source, session));
    }

    while !stop.load(Ordering::Relaxed) && !tx.is_closed() {
        let now = Instant::now();
        let mut wake = now + IDLE_POLL;

        for (index, s) in state.sessions.iter_mut().enumerate() {
            if s.dead {
                continue;
            }
            if std::mem::take(&mut s.ready)
                && let Some(buffer) = &s.buffer
            {
                let (width, height, pixels) = downscale(
                    buffer.memory.as_slice(),
                    buffer.width,
                    region(s.crop, buffer.width, buffer.height),
                    buffer.format,
                    max_edge,
                );
                let frame = Frame {
                    id: s.id.clone(),
                    width,
                    height,
                    pixels,
                };
                if tx.try_send(frame).is_err() {
                    return Ok(());
                }
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
            let due = s.last + interval;
            if due > now {
                wake = wake.min(due);
                continue;
            }
            let buffer = s.buffer.as_ref().expect("built above");
            let frame = s.session.create_frame(&qh, index);
            frame.attach_buffer(&buffer.buffer);
            frame.damage_buffer(0, 0, buffer.width as i32, buffer.height as i32);
            frame.capture();
            s.frame = Some(frame);
            // The cap counts from the request, so a compositor that takes a
            // frame's worth of time to answer does not halve the rate.
            s.last = now;
        }

        let timeout = wake.saturating_duration_since(Instant::now());
        dispatch_for(&conn, &mut queue, &mut state, timeout)?;
    }

    for s in &state.sessions {
        if let Some(frame) = &s.frame {
            frame.destroy();
        }
        s.session.destroy();
        s.source.destroy();
    }
    let _ = conn.flush();
    Ok(())
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

/// Box-filter the buffer so its longer side is at most `max_edge`.
///
/// The factor is a whole number, so every output pixel averages a full square
/// of source pixels and the result has no seams. `xrgb` carries no alpha, so
/// it is written opaque; `argb` from the compositor is already premultiplied,
/// and an average of premultiplied pixels stays premultiplied.
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

/// Box-filter the region `(x0, y0, w, h)` of a buffer `full_w` pixels wide
/// so its longer side is at most `max_edge`.
fn downscale(
    src: &[u8],
    full_w: u32,
    (x0, y0, w, h): (u32, u32, u32, u32),
    format: wl_shm::Format,
    max_edge: u32,
) -> (u32, u32, Vec<u8>) {
    let f = w.max(h).div_ceil(max_edge.max(1)).max(1);
    let (ow, oh) = ((w / f).max(1), (h / f).max(1));
    let stride = (full_w * 4) as usize;
    let opaque = matches!(format, wl_shm::Format::Xrgb8888 | wl_shm::Format::Xbgr8888);
    // Both ABGR formats are RGBA in memory; the card wants BGRA.
    let swap = matches!(format, wl_shm::Format::Xbgr8888 | wl_shm::Format::Abgr8888);
    let n = f * f;

    let mut out = vec![0u8; (ow * oh * 4) as usize];
    for oy in 0..oh {
        for ox in 0..ow {
            let mut acc = [0u32; 4];
            for sy in y0 + oy * f..y0 + oy * f + f {
                let row = sy as usize * stride;
                for sx in x0 + ox * f..x0 + ox * f + f {
                    let i = row + sx as usize * 4;
                    acc[0] += u32::from(src[i]);
                    acc[1] += u32::from(src[i + 1]);
                    acc[2] += u32::from(src[i + 2]);
                    acc[3] += u32::from(src[i + 3]);
                }
            }
            let o = ((oy * ow + ox) * 4) as usize;
            let (b, r) = if swap {
                (acc[2], acc[0])
            } else {
                (acc[0], acc[2])
            };
            out[o] = (b / n) as u8;
            out[o + 1] = (acc[1] / n) as u8;
            out[o + 2] = (r / n) as u8;
            out[o + 3] = if opaque { 0xff } else { (acc[3] / n) as u8 };
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
    id: String,
    /// Only this piece of the window is sent.
    crop: Option<Crop>,
    source: ExtImageCaptureSourceV1,
    session: ExtImageCopyCaptureSessionV1,
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
    dead: bool,
}

impl Session {
    fn new(
        id: String,
        crop: Option<Crop>,
        source: ExtImageCaptureSourceV1,
        session: ExtImageCopyCaptureSessionV1,
    ) -> Session {
        Session {
            id,
            crop,
            source,
            session,
            pending_size: None,
            pending_format: None,
            constraints: None,
            buffer: None,
            frame: None,
            ready: false,
            last: Instant::now() - Duration::from_secs(1),
            dead: false,
        }
    }
}

#[derive(Default)]
struct State {
    manager: Option<ExtImageCopyCaptureManagerV1>,
    toplevel_sources: Option<ExtForeignToplevelImageCaptureSourceManagerV1>,
    shm: Option<wl_shm::WlShm>,
    toplevels: Vec<(ExtForeignToplevelHandleV1, Option<String>)>,
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

impl Dispatch<ExtImageCopyCaptureSessionV1, usize> for State {
    fn event(
        state: &mut Self,
        _: &ExtImageCopyCaptureSessionV1,
        event: ext_image_copy_capture_session_v1::Event,
        index: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use ext_image_copy_capture_session_v1::Event;
        let Some(s) = state.sessions.get_mut(*index) else {
            return;
        };
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
                    // loop builds a new one before the next frame.
                    if s.constraints.as_ref() != Some(&next) {
                        s.buffer = None;
                    }
                    s.constraints = Some(next);
                }
                s.pending_size = None;
                s.pending_format = None;
            }
            Event::Stopped => s.dead = true,
            _ => {}
        }
    }
}

impl Dispatch<ExtImageCopyCaptureFrameV1, usize> for State {
    fn event(
        state: &mut Self,
        frame: &ExtImageCopyCaptureFrameV1,
        event: ext_image_copy_capture_frame_v1::Event,
        index: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use ext_image_copy_capture_frame_v1::{Event, FailureReason};
        let Some(s) = state.sessions.get_mut(*index) else {
            return;
        };
        match event {
            Event::Ready => {
                frame.destroy();
                s.frame = None;
                s.ready = true;
            }
            Event::Failed { reason } => {
                frame.destroy();
                s.frame = None;
                match reason.into_result() {
                    // New constraints follow; the `done` that closes them
                    // drops the buffer.
                    Ok(FailureReason::BufferConstraints) => {}
                    Ok(FailureReason::Stopped) => s.dead = true,
                    // Try again after one frame interval.
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
        if let ext_foreign_toplevel_handle_v1::Event::Identifier { identifier } = event
            && let Some(slot) = state
                .toplevels
                .iter_mut()
                .find(|(h, _)| h.id() == handle.id())
        {
            slot.1 = Some(identifier);
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
    fn the_longer_edge_fits_and_the_aspect_holds() {
        let src = vec![0u8; 2560 * 1600 * 4];
        let (w, h, px) = downscale(
            &src,
            2560,
            (0, 0, 2560, 1600),
            wl_shm::Format::Xrgb8888,
            320,
        );
        assert_eq!((w, h), (320, 200));
        assert_eq!(px.len(), 320 * 200 * 4);
    }

    #[test]
    fn each_output_pixel_averages_its_square() {
        // 2x2 XRGB, BGRX in memory: two black pixels, two at blue 200.
        let src = [0, 0, 0, 0, 0, 0, 0, 0, 200, 0, 0, 0, 200, 0, 0, 0];
        let (w, h, px) = downscale(&src, 2, (0, 0, 2, 2), wl_shm::Format::Xrgb8888, 1);
        assert_eq!((w, h), (1, 1));
        assert_eq!(px, vec![100, 0, 0, 0xff]);
    }

    #[test]
    fn abgr_is_swapped_to_bgra_and_keeps_alpha() {
        let src = [10u8, 20, 30, 128];
        let (_, _, px) = downscale(&src, 1, (0, 0, 1, 1), wl_shm::Format::Abgr8888, 4);
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
        let (w, h, px) = downscale(&src, 4, (2, 0, 2, 2), wl_shm::Format::Xrgb8888, 8);
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
        let (w, h, _) = downscale(&src, 10, (0, 0, 10, 6), wl_shm::Format::Argb8888, 320);
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
