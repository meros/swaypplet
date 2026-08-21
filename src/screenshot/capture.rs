//! Pixels off an output, over `ext-image-copy-capture-v1`.
//!
//! This is the half of a screenshot that `grim` used to be. The protocol is
//! the staging successor to wlr-screencopy, and the compositor advertises both;
//! the ext one is chosen because it is also what a live window thumbnail wants
//! (SHELL_IDEAS item 5), so there is one capture path in the process rather
//! than two.
//!
//! The flow is fixed: ask an output for a capture source, open a session on it,
//! wait for the session to say how big a buffer it needs and in what format,
//! hand it one backed by a memfd, and wait for the frame to go `ready`. All of
//! it blocks, so all of it happens on a worker thread with its own Wayland
//! connection — GDK's connection is in front of the frame clock and has no
//! business waiting on a copy.
//!
//! Cropping is not in the protocol: a session captures its whole source. The
//! region selector's rectangle is applied here, after the fact, which also
//! means a region capture and a full-output capture are the same round trip.

use std::os::fd::{AsFd, OwnedFd};

use wayland_client::protocol::{wl_buffer, wl_output, wl_registry, wl_shm, wl_shm_pool};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, delegate_noop};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::{
    ext_foreign_toplevel_handle_v1::{self, ExtForeignToplevelHandleV1},
    ext_foreign_toplevel_list_v1::{self, ExtForeignToplevelListV1},
};
use wayland_protocols::ext::image_capture_source::v1::client::{
    ext_foreign_toplevel_image_capture_source_manager_v1::ExtForeignToplevelImageCaptureSourceManagerV1,
    ext_image_capture_source_v1::ExtImageCaptureSourceV1,
    ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1,
};
use wayland_protocols::ext::image_copy_capture::v1::client::{
    ext_image_copy_capture_frame_v1::{self, ExtImageCopyCaptureFrameV1},
    ext_image_copy_capture_manager_v1::{self, ExtImageCopyCaptureManagerV1},
    ext_image_copy_capture_session_v1::{self, ExtImageCopyCaptureSessionV1},
};

/// A captured image, unpremultiplied RGBA, tightly packed.
///
/// Tightly packed because everything downstream (GdkTexture, PNG encoding,
/// the annotation canvas) wants it that way, and the compositor's stride is
/// the only place it is not.
#[derive(Clone)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl Image {
    /// The sub-rectangle at `(x, y, w, h)`, clamped to the image.
    ///
    /// Coordinates are buffer pixels, so a caller working in logical
    /// coordinates has already multiplied by the output's scale.
    pub fn crop(&self, x: i32, y: i32, w: u32, h: u32) -> Image {
        let x0 = x.clamp(0, self.width as i32) as u32;
        let y0 = y.clamp(0, self.height as i32) as u32;
        let x1 = (x0 + w).min(self.width);
        let y1 = (y0 + h).min(self.height);
        let (cw, ch) = (x1.saturating_sub(x0), y1.saturating_sub(y0));

        let mut pixels = Vec::with_capacity((cw * ch * 4) as usize);
        for row in y0..y1 {
            let start = ((row * self.width + x0) * 4) as usize;
            pixels.extend_from_slice(&self.pixels[start..start + (cw * 4) as usize]);
        }
        Image {
            width: cw,
            height: ch,
            pixels,
        }
    }


    /// The pixel at `(x, y)` as `(r, g, b)`, for the colour picker.
    pub fn pixel(&self, x: u32, y: u32) -> Option<(u8, u8, u8)> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let i = ((y * self.width + x) * 4) as usize;
        Some((self.pixels[i], self.pixels[i + 1], self.pixels[i + 2]))
    }
}

/// Capture one output whole, by connector name (`eDP-1`).
///
/// Blocking. `Err` when the compositor has no such output, does not advertise
/// the capture protocol, or fails the copy.
pub fn output(name: &str) -> Result<Image, String> {
    with_source(|state, qh| {
        let sources = state
            .outputs_manager
            .clone()
            .ok_or("compositor does not advertise ext-output-image-capture-source-v1")?;
        let output = state
            .outputs
            .iter()
            .find(|(_, n)| n.as_deref() == Some(name))
            .map(|(o, _)| o.clone())
            .ok_or_else(|| format!("no output named {name}"))?;
        Ok(sources.create_source(&output, qh, ()))
    })
}


/// The whole dance, once, on a connection of its own.
///
/// A fresh connection per capture rather than one long-lived thread: the
/// toplevel handles a per-window source needs only exist on the connection
/// that bound the list, so a shared connection would have to own both the
/// enumeration and every capture, and serialise them. Two round trips is a
/// cheaper price than that coupling.
fn with_source(
    make_source: impl FnOnce(&Capture, &QueueHandle<Capture>) -> Result<ExtImageCaptureSourceV1, String>,
) -> Result<Image, String> {
    let conn = Connection::connect_to_env().map_err(|e| format!("wayland connect: {e}"))?;
    let display = conn.display();
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    display.get_registry(&qh, ());

    let mut state = Capture::default();
    // Two round trips: the first brings the globals in, the second the
    // per-object `name` and `identifier` events that pick one out.
    for _ in 0..2 {
        queue
            .roundtrip(&mut state)
            .map_err(|e| format!("wayland roundtrip: {e}"))?;
    }

    let manager = state
        .manager
        .clone()
        .ok_or("compositor does not advertise ext-image-copy-capture-v1")?;
    let shm = state.shm.clone().ok_or("compositor has no wl_shm")?;
    let source = make_source(&state, &qh)?;

    let session = manager.create_session(
        &source,
        // Cursors are not part of a screenshot anyone asked for; the pointer
        // is a tool here, not content.
        ext_image_copy_capture_manager_v1::Options::empty(),
        &qh,
        (),
    );

    // The session answers with a size and a set of formats, then `done`.
    dispatch_until(&conn, &mut queue, &mut state, |s| {
        s.constraints.is_some() || s.stopped
    })?;

    // `buffer_constraints` is the compositor saying the buffer no longer
    // matches — the source was resized between the `done` that described it
    // and the capture. It re-sends the constraints with that failure, so the
    // answer is to build the buffer again, once. Spotify hit this every time
    // on this machine; everything else never did.
    let mut attempt = 0;
    let (memory, width, height, format) = loop {
        let Constraints {
            width,
            height,
            format,
        } = state.constraints.clone().ok_or("capture session stopped")?;

        let stride = width * 4;
        let len = (stride * height) as usize;
        let memory = Shm::new(len)?;
        let pool = shm.create_pool(memory.fd.as_fd(), len as i32, &qh, ());
        let buffer = pool.create_buffer(
            0,
            width as i32,
            height as i32,
            stride as i32,
            format,
            &qh,
            (),
        );

        let frame = session.create_frame(&qh, ());
        frame.attach_buffer(&buffer);
        frame.damage_buffer(0, 0, width as i32, height as i32);
        frame.capture();

        dispatch_until(&conn, &mut queue, &mut state, |s| s.frame.is_some())?;
        let outcome = state.frame.take();

        frame.destroy();

        match outcome {
            Some(Ok(())) => {
                buffer.destroy();
                pool.destroy();
                break (memory, width, height, format);
            }
            Some(Err(reason)) => {
                buffer.destroy();
                pool.destroy();
                attempt += 1;
                if attempt > 1 || !reason.contains("BufferConstraints") {
                    return Err(format!("capture failed: {reason}"));
                }
                // The new constraints ride in on the same failure; wait for
                // the `done` that closes them before trying again.
                state.constraints = None;
                dispatch_until(&conn, &mut queue, &mut state, |s| {
                    s.constraints.is_some() || s.stopped
                })?;
            }
            None => return Err("capture resolved to nothing".into()),
        }
    };

    let pixels = to_rgba(memory.as_slice(), width, height, format);

    session.destroy();
    source.destroy();

    Ok(Image {
        width,
        height,
        pixels,
    })
}

/// Convert the compositor's buffer into unpremultiplied RGBA.
///
/// The two formats a compositor offers for this are byte-order-reversed
/// little-endian words, so both are BGRA in memory; `xrgb8888` simply has no
/// meaningful alpha byte, and a screenshot of an opaque output is opaque.
fn to_rgba(src: &[u8], width: u32, height: u32, format: wl_shm::Format) -> Vec<u8> {
    let mut out = vec![0u8; (width * height * 4) as usize];
    let opaque = matches!(format, wl_shm::Format::Xrgb8888 | wl_shm::Format::Xbgr8888);
    let swap = matches!(format, wl_shm::Format::Xrgb8888 | wl_shm::Format::Argb8888);

    for (dst, px) in out.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
        if swap {
            dst[0] = px[2];
            dst[1] = px[1];
            dst[2] = px[0];
        } else {
            dst[0] = px[0];
            dst[1] = px[1];
            dst[2] = px[2];
        }
        dst[3] = if opaque { 0xff } else { px[3] };
    }
    out
}

/// How long a capture may wait for the compositor before giving up.
///
/// An output always has something to copy. A *window* does not: a toplevel on
/// a workspace nobody is looking at may not be rendered at all, and the
/// protocol's answer to that is silence rather than `failed` — the session is
/// allowed to "wait an indefinite amount of time for the source content to
/// change". Indefinite is exactly what a switcher building eleven thumbnails
/// cannot afford, so the wait is bounded and a thumbnail that does not arrive
/// is simply not drawn.
const CAPTURE_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(400);

/// Dispatch until `ready` or the deadline, whichever comes first.
///
/// `blocking_dispatch` would be the obvious call and has no deadline, so this
/// runs the read cycle by hand: flush, poll the connection's fd with what is
/// left of the budget, then read and dispatch whatever arrived.
fn dispatch_until(
    conn: &Connection,
    queue: &mut wayland_client::EventQueue<Capture>,
    state: &mut Capture,
    ready: impl Fn(&Capture) -> bool,
) -> Result<(), String> {
    let deadline = std::time::Instant::now() + CAPTURE_TIMEOUT;

    loop {
        queue
            .dispatch_pending(state)
            .map_err(|e| format!("wayland dispatch: {e}"))?;
        if ready(state) {
            return Ok(());
        }

        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Err("capture timed out".into());
        }

        conn.flush().map_err(|e| format!("wayland flush: {e}"))?;
        let Some(guard) = conn.prepare_read() else {
            continue; // events arrived between the dispatch and here
        };

        let fd = std::os::fd::AsRawFd::as_raw_fd(&guard.connection_fd());
        let mut poll_fd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let millis = remaining.as_millis().min(i32::MAX as u128) as i32;
        let polled = unsafe { libc::poll(&mut poll_fd, 1, millis) };
        match polled {
            0 => return Err("capture timed out".into()),
            n if n < 0 => {
                let err = std::io::Error::last_os_error();
                if err.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(format!("poll: {err}"));
            }
            _ => {
                guard.read().map_err(|e| format!("wayland read: {e}"))?;
            }
        }
    }
}

// ── Shared memory ───────────────────────────────────────────────────────

/// An anonymous shared mapping handed to the compositor to draw into.
struct Shm {
    fd: OwnedFd,
    ptr: *mut libc::c_void,
    len: usize,
}

impl Shm {
    fn new(len: usize) -> Result<Shm, String> {
        // Sealed against resize: the compositor maps this and would fault if
        // the client shrank it under them.
        let fd = unsafe { libc::memfd_create(c"swaypplet-capture".as_ptr(), libc::MFD_CLOEXEC) };
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

// ── Wayland plumbing ────────────────────────────────────────────────────

#[derive(Clone)]
struct Constraints {
    width: u32,
    height: u32,
    format: wl_shm::Format,
}

#[derive(Default)]
struct Capture {
    manager: Option<ExtImageCopyCaptureManagerV1>,
    outputs_manager: Option<ExtOutputImageCaptureSourceManagerV1>,
    toplevels_manager: Option<ExtForeignToplevelImageCaptureSourceManagerV1>,
    shm: Option<wl_shm::WlShm>,
    outputs: Vec<(wl_output::WlOutput, Option<String>)>,
    /// Every window the compositor lists, with the identifier sway also
    /// reports in its tree.
    toplevels: Vec<(ExtForeignToplevelHandleV1, Option<String>)>,
    /// Filled in as the session reports, committed on `done`.
    pending_size: Option<(u32, u32)>,
    pending_format: Option<wl_shm::Format>,
    constraints: Option<Constraints>,
    stopped: bool,
    /// `None` until the frame resolves either way.
    frame: Option<Result<(), String>>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for Capture {
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
            "ext_output_image_capture_source_manager_v1" => {
                state.outputs_manager = Some(registry.bind(name, 1, qh, ()));
            }
            "ext_foreign_toplevel_image_capture_source_manager_v1" => {
                state.toplevels_manager = Some(registry.bind(name, 1, qh, ()));
            }
            "ext_foreign_toplevel_list_v1" => {
                let _: ExtForeignToplevelListV1 = registry.bind(name, 1, qh, ());
            }
            "wl_shm" => state.shm = Some(registry.bind(name, 1, qh, ())),
            "wl_output" => {
                // Version 4 for the `name` event: connector names are how GDK
                // monitors and sway outputs are matched everywhere else here,
                // and geometry's make/model cannot do it.
                let output: wl_output::WlOutput = registry.bind(name, 4, qh, ());
                state.outputs.push((output, None));
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_output::WlOutput, ()> for Capture {
    fn event(
        state: &mut Self,
        output: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Name { name } = event
            && let Some(slot) = state
                .outputs
                .iter_mut()
                .find(|(o, _)| o.id() == output.id())
        {
            slot.1 = Some(name);
        }
    }
}

impl Dispatch<ExtImageCopyCaptureSessionV1, ()> for Capture {
    fn event(
        state: &mut Self,
        _: &ExtImageCopyCaptureSessionV1,
        event: ext_image_copy_capture_session_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use ext_image_copy_capture_session_v1::Event;
        match event {
            Event::BufferSize { width, height } => state.pending_size = Some((width, height)),
            Event::ShmFormat { format } => {
                // Several formats may arrive; the first understood one wins,
                // and both candidates are the same bytes in a different order.
                if state.pending_format.is_none()
                    && let Ok(format) = format.into_result()
                    && matches!(
                        format,
                        wl_shm::Format::Xrgb8888
                            | wl_shm::Format::Argb8888
                            | wl_shm::Format::Xbgr8888
                            | wl_shm::Format::Abgr8888
                    )
                {
                    state.pending_format = Some(format);
                }
            }
            Event::Done => {
                if let (Some((width, height)), Some(format)) =
                    (state.pending_size, state.pending_format)
                {
                    state.constraints = Some(Constraints {
                        width,
                        height,
                        format,
                    });
                }
            }
            Event::Stopped => state.stopped = true,
            _ => {}
        }
    }
}

impl Dispatch<ExtImageCopyCaptureFrameV1, ()> for Capture {
    fn event(
        state: &mut Self,
        _: &ExtImageCopyCaptureFrameV1,
        event: ext_image_copy_capture_frame_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use ext_image_copy_capture_frame_v1::Event;
        match event {
            Event::Ready => state.frame = Some(Ok(())),
            Event::Failed { reason } => {
                state.frame = Some(Err(format!("{reason:?}")));
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtForeignToplevelListV1, ()> for Capture {
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

    wayland_client::event_created_child!(Capture, ExtForeignToplevelListV1, [
        ext_foreign_toplevel_list_v1::EVT_TOPLEVEL_OPCODE => (ExtForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ExtForeignToplevelHandleV1, ()> for Capture {
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

delegate_noop!(Capture: ignore ExtImageCopyCaptureManagerV1);
delegate_noop!(Capture: ignore ExtOutputImageCaptureSourceManagerV1);
delegate_noop!(Capture: ignore ExtForeignToplevelImageCaptureSourceManagerV1);
delegate_noop!(Capture: ignore ExtImageCaptureSourceV1);
delegate_noop!(Capture: ignore wl_shm::WlShm);
delegate_noop!(Capture: ignore wl_shm_pool::WlShmPool);
delegate_noop!(Capture: ignore wl_buffer::WlBuffer);

/// The mapping is only ever read after the compositor signals `ready`, and
/// the pointer never leaves the thread that made it.
unsafe impl Send for Shm {}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(width: u32, height: u32, rgba: [u8; 4]) -> Image {
        Image {
            width,
            height,
            pixels: rgba.repeat((width * height) as usize),
        }
    }

    #[test]
    fn a_crop_takes_the_rectangle_asked_for() {
        let mut img = solid(4, 4, [0, 0, 0, 255]);
        // Mark (x=2, y=1) so the crop's origin is checkable, not just its
        // size: row 1 of a 4-wide image, third pixel, red channel.
        let i = (4 + 2) * 4;
        img.pixels[i] = 200;

        let cropped = img.crop(2, 1, 2, 2);
        assert_eq!((cropped.width, cropped.height), (2, 2));
        assert_eq!(cropped.pixel(0, 0), Some((200, 0, 0)));
    }

    #[test]
    fn a_crop_running_off_the_edge_is_clamped_not_panicking() {
        let img = solid(4, 4, [1, 2, 3, 255]);
        let cropped = img.crop(3, 3, 10, 10);
        assert_eq!((cropped.width, cropped.height), (1, 1));
        assert_eq!(cropped.pixel(0, 0), Some((1, 2, 3)));
    }

    #[test]
    fn a_crop_entirely_outside_is_empty() {
        let img = solid(4, 4, [0, 0, 0, 255]);
        let cropped = img.crop(9, 9, 2, 2);
        assert_eq!((cropped.width, cropped.height), (0, 0));
        assert!(cropped.pixels.is_empty());
    }



    #[test]
    fn xrgb_arrives_byte_reversed_and_opaque() {
        // One pixel, little-endian XRGB8888: B, G, R, X.
        let src = [10u8, 20, 30, 0];
        let out = to_rgba(&src, 1, 1, wl_shm::Format::Xrgb8888);
        assert_eq!(out, vec![30, 20, 10, 255]);
    }

    #[test]
    fn abgr_keeps_its_byte_order_and_its_alpha() {
        let src = [10u8, 20, 30, 128];
        let out = to_rgba(&src, 1, 1, wl_shm::Format::Abgr8888);
        assert_eq!(out, vec![10, 20, 30, 128]);
    }
}

#[cfg(test)]
mod live {
    /// Grabs the real session's output. Ignored: needs a compositor.

    #[test]
    #[ignore]
    fn capture_an_output() {
        let name = std::env::var("CAPTURE_OUTPUT").unwrap_or_else(|_| "eDP-1".into());
        let img = super::output(&name).expect("capture");
        let mut ppm = format!("P6\n{} {}\n255\n", img.width, img.height).into_bytes();
        ppm.extend(img.pixels.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]));
        std::fs::write("/tmp/capture.ppm", ppm).unwrap();
        println!("captured {}x{}", img.width, img.height);
    }
}
