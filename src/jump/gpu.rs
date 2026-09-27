//! Live frames averaged on the GPU, from the compositor's buffer to GTK's
//! texture without a pixel crossing to the CPU.
//!
//! The capture is a dmabuf on sway's GPU, so sway's copy of the window stays
//! on the GPU, and one fragment shader pass averages it into a small dmabuf
//! that GTK draws as it is. A 2x window (2900x1736 into 800x479) takes
//! 0.6 ms with a Tile4 capture buffer (`bench`). The CPU average this
//! replaced took 23 ms, and sway's readback of the full frame into shared
//! memory, on its main thread, came on top.
//!
//! The shader computes the exact area average: every output pixel the mean
//! of the source area under it, the texels cut at its edges weighted by the
//! part inside, in linear light, premultiplied alpha kept. No mipmaps and
//! no bilinear taps: both average a different area than the one the pixel
//! covers. The tests check it against a plain reference in `f64`.
//!
//! A context is made on the worker thread and lives with it; EGL contexts
//! belong to one thread. There is no CPU path to fall back to. Where the
//! context cannot be made, or GTK refuses its frames ([`refuse`]), live
//! pictures show their app icons, and the journal says why.

use std::ffi::c_void;
use std::fs::File;
use std::os::fd::{AsFd, AsRawFd, OwnedFd};
use std::os::unix::fs::MetadataExt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use glow::HasContext;
use khronos_egl as egl;

/// A buffer format, as DRM fourcc codes.
pub const ARGB8888: u32 = u32::from_le_bytes(*b"AR24");
pub const XRGB8888: u32 = u32::from_le_bytes(*b"XR24");
pub const ABGR8888: u32 = u32::from_le_bytes(*b"AB24");
pub const XBGR8888: u32 = u32::from_le_bytes(*b"XB24");
const MOD_LINEAR: u64 = 0;
const MOD_INVALID: u64 = 0x00ff_ffff_ffff_ffff;

/// Most output buffers one window keeps. One is on screen (and in
/// `card::LAST`), one is being drawn, one covers GTK letting go late; a
/// frame that finds all of them busy is skipped, and the next one comes.
const OUT_SLOTS: usize = 3;

static REFUSED: AtomicBool = AtomicBool::new(false);

/// Turn live frames off for the rest of the process: GTK could not take
/// one of them, and would not take the next.
pub fn refuse(why: &str) {
    if !REFUSED.swap(true, Ordering::Relaxed) {
        log::warn!("jump: gtk refused a live frame ({why}); live pictures stop");
    }
}

/// Whether GTK still takes live frames.
pub fn usable() -> bool {
    !REFUSED.load(Ordering::Relaxed)
}

/// The capture formats the shader reads, best first: alpha over none, so a
/// translucent window keeps its transparency.
pub fn rank(fourcc: u32) -> Option<u8> {
    match fourcc {
        ARGB8888 => Some(0),
        ABGR8888 => Some(1),
        XRGB8888 => Some(2),
        XBGR8888 => Some(3),
        _ => None,
    }
}

fn opaque(fourcc: u32) -> bool {
    matches!(fourcc, XRGB8888 | XBGR8888)
}

// ── A frame for GTK ─────────────────────────────────────────────────────

/// One frame on the GPU: a single-plane linear ARGB8888 dmabuf,
/// premultiplied, which is `gdk::MemoryFormat::B8g8r8a8Premultiplied` laid
/// out the same way. Its buffer goes back to the worker's pool when this
/// and every texture made from it are gone.
pub struct GpuFrame {
    pub fd: OwnedFd,
    pub stride: u32,
    pub offset: u32,
    pub fourcc: u32,
    pub modifier: u64,
    pub width: u32,
    pub height: u32,
    busy: Busy,
}

impl GpuFrame {
    /// The frame as GTK's texture, on the main thread. The buffer goes back
    /// to the pool when GTK lets go of the texture.
    pub fn into_texture(self) -> Result<gtk4::gdk::Texture, String> {
        use gtk4::gdk;
        let display = gdk::Display::default().ok_or("no display")?;
        let builder = gdk::DmabufTextureBuilder::new()
            .set_display(&display)
            .set_width(self.width)
            .set_height(self.height)
            .set_fourcc(self.fourcc)
            .set_modifier(self.modifier)
            .set_premultiplied(true)
            .set_n_planes(1)
            .set_stride(0, self.stride)
            .set_offset(0, self.offset);
        // The fd and the hold on the slot go to the release function, which
        // GTK calls once nothing draws from the buffer.
        let builder = unsafe { builder.set_fd(0, self.fd.as_raw_fd()) };
        let GpuFrame { fd, busy, .. } = self;
        unsafe {
            builder.build_with_release_func(move || {
                drop(fd);
                drop(busy);
            })
        }
        .map_err(|e| e.to_string())
    }
}

/// Marks an output slot in use while alive.
struct Busy(Arc<AtomicBool>);

impl Drop for Busy {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

// ── The context ─────────────────────────────────────────────────────────

/// A GBM device, an EGL display on it, and a surfaceless GLES 3 context
/// with the averaging program: everything the worker needs, made once.
pub struct Gpu {
    /// The device number of the render node, to match the compositor's.
    dev: u64,
    gbm: gbm::Device<File>,
    egl: egl::Instance<egl::Static>,
    display: egl::Display,
    context: egl::Context,
    gl: glow::Context,
    image_target: ImageTargetFn,
    program: glow::Program,
    vao: glow::VertexArray,
    u_origin: Option<glow::UniformLocation>,
    u_ratio: Option<glow::UniformLocation>,
    u_end: Option<glow::UniformLocation>,
    u_opaque: Option<glow::UniformLocation>,
}

type ImageTargetFn = unsafe extern "system" fn(target: u32, image: *const c_void);

const EGL_PLATFORM_GBM_KHR: egl::Enum = 0x31D7;
const EGL_LINUX_DMA_BUF_EXT: egl::Enum = 0x3270;
const EGL_LINUX_DRM_FOURCC_EXT: egl::Attrib = 0x3271;
const EGL_IMAGE_PRESERVED_KHR: egl::Attrib = 0x30D2;
/// Per plane: fd, offset, pitch, modifier low and high.
const EGL_PLANE_ATTRS: [[egl::Attrib; 5]; 4] = [
    [0x3272, 0x3273, 0x3274, 0x3443, 0x3444],
    [0x3275, 0x3276, 0x3277, 0x3445, 0x3446],
    [0x3278, 0x3279, 0x327A, 0x3447, 0x3448],
    [0x3440, 0x3441, 0x3442, 0x3449, 0x344A],
];

impl Gpu {
    /// A context on the render node behind device number `dev`: the node
    /// itself, or the render node of the card it names.
    pub fn open(dev: u64) -> Result<Gpu, String> {
        let path = render_node(dev).ok_or_else(|| format!("no render node for device {dev:#x}"))?;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let dev = file.metadata().map_err(|e| e.to_string())?.rdev();
        let gbm = gbm::Device::new(file).map_err(|e| format!("gbm: {e}"))?;

        let egl = egl::Instance::new(egl::Static);
        let display = unsafe {
            egl.get_platform_display(
                EGL_PLATFORM_GBM_KHR,
                gbm::AsRaw::as_raw(&gbm) as *mut c_void,
                &[egl::ATTRIB_NONE],
            )
        }
        .map_err(|e| format!("egl display: {e}"))?;
        egl.initialize(display)
            .map_err(|e| format!("egl initialize: {e}"))?;
        let extensions = egl
            .query_string(Some(display), egl::EXTENSIONS)
            .map_err(|e| format!("egl extensions: {e}"))?
            .to_string_lossy()
            .into_owned();
        for needed in [
            "EGL_EXT_image_dma_buf_import_modifiers",
            "EGL_KHR_surfaceless_context",
            "EGL_KHR_no_config_context",
        ] {
            if !extensions.split(' ').any(|e| e == needed) {
                return Err(format!("egl lacks {needed}"));
            }
        }
        egl.bind_api(egl::OPENGL_ES_API)
            .map_err(|e| format!("egl bind: {e}"))?;
        let context = egl
            .create_context(
                display,
                unsafe { egl::Config::from_ptr(std::ptr::null_mut()) },
                None,
                &[egl::CONTEXT_MAJOR_VERSION, 3, egl::NONE],
            )
            .map_err(|e| format!("egl context: {e}"))?;
        egl.make_current(display, None, None, Some(context))
            .map_err(|e| format!("egl make current: {e}"))?;

        let gl = unsafe {
            glow::Context::from_loader_function(|name| {
                egl.get_proc_address(name)
                    .map_or(std::ptr::null(), |f| f as *const c_void)
            })
        };
        let image_target: ImageTargetFn = egl
            .get_proc_address("glEGLImageTargetTexture2DOES")
            .map(|f| unsafe { std::mem::transmute::<extern "system" fn(), ImageTargetFn>(f) })
            .ok_or("no glEGLImageTargetTexture2DOES")?;

        let (program, vao) = unsafe { build_program(&gl)? };
        let at = |name: &str| unsafe { gl.get_uniform_location(program, name) };
        let (u_origin, u_ratio, u_end, u_opaque) =
            (at("origin"), at("ratio"), at("end"), at("opaque"));
        unsafe {
            gl.use_program(Some(program));
            gl.uniform_1_i32(at("src").as_ref(), 0);
        }
        log::info!("jump: gpu frames on {}", path.display());
        Ok(Gpu {
            dev,
            gbm,
            egl,
            display,
            context,
            gl,
            image_target,
            program,
            vao,
            u_origin,
            u_ratio,
            u_end,
            u_opaque,
        })
    }

    /// Whether this context is on the device numbered `dev`, or on the
    /// render node of the card it names.
    pub fn serves(&self, dev: u64) -> bool {
        self.dev == dev || render_node(dev).is_some_and(|p| rdev(&p) == Some(self.dev))
    }

    /// A buffer the compositor copies a `width` by `height` window into, in
    /// `fourcc` with one of `modifiers`, imported for the shader to read.
    pub fn capture_buffer(
        &self,
        width: u32,
        height: u32,
        fourcc: u32,
        modifiers: &[u64],
    ) -> Result<CaptureBuffer, String> {
        let format = gbm::Format::try_from(fourcc).map_err(|_| format!("fourcc {fourcc:#x}"))?;
        let usable: Vec<gbm::Modifier> = modifiers
            .iter()
            .copied()
            .filter(|&m| m != MOD_INVALID)
            .map(gbm::Modifier::from)
            .collect();
        let bo = if usable.is_empty() {
            self.gbm.create_buffer_object::<()>(
                width,
                height,
                format,
                gbm::BufferObjectFlags::RENDERING,
            )
        } else {
            self.gbm.create_buffer_object_with_modifiers2::<()>(
                width,
                height,
                format,
                usable.into_iter(),
                gbm::BufferObjectFlags::RENDERING,
            )
        }
        .map_err(|e| format!("gbm buffer {width}x{height}: {e}"))?;
        let planes = planes(&bo)?;
        let modifier = u64::from(bo.modifier());
        let (image, texture) = unsafe { self.import(width, height, fourcc, modifier, &planes)? };
        Ok(CaptureBuffer {
            bo,
            planes,
            fourcc,
            modifier,
            width,
            height,
            image,
            texture,
        })
    }

    /// Average `region` of `src` (x, y, width, height in its pixels) into a
    /// `width` by `height` frame, drawn in one of `pool`'s buffers.
    ///
    /// `None` when every buffer of the pool is still GTK's.
    pub fn downscale(
        &self,
        src: &CaptureBuffer,
        (x0, y0, w, h): (u32, u32, u32, u32),
        (width, height): (u32, u32),
        pool: &mut Pool,
    ) -> Result<Option<GpuFrame>, String> {
        let slot = match pool.free(width, height) {
            Some(i) => i,
            None if pool.slots.len() < OUT_SLOTS => {
                pool.slots.push(unsafe { self.out_slot(width, height)? });
                pool.slots.len() - 1
            }
            None => return Ok(None),
        };
        let slot = &pool.slots[slot];
        let gl = &self.gl;
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(slot.fbo));
            gl.viewport(0, 0, width as i32, height as i32);
            gl.use_program(Some(self.program));
            gl.bind_vertex_array(Some(self.vao));
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(src.texture));
            gl.uniform_2_f32(self.u_origin.as_ref(), x0 as f32, y0 as f32);
            gl.uniform_2_f32(
                self.u_ratio.as_ref(),
                w as f32 / width as f32,
                h as f32 / height as f32,
            );
            gl.uniform_2_f32(self.u_end.as_ref(), (x0 + w) as f32, (y0 + h) as f32);
            gl.uniform_1_i32(self.u_opaque.as_ref(), i32::from(opaque(src.fourcc)));
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
            // The frame crosses to GTK finished. Waiting here costs the
            // worker the draw of a small frame; an implicit fence would
            // cost nothing here but is not honoured by every driver.
            gl.finish();
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        }
        let fd = slot.bo.fd().map_err(|e| format!("out fd: {e}"))?;
        slot.busy.store(true, Ordering::Release);
        Ok(Some(GpuFrame {
            fd,
            stride: slot.bo.stride_for_plane(0),
            offset: slot.bo.offset(0),
            fourcc: ARGB8888,
            modifier: MOD_LINEAR,
            width,
            height,
            busy: Busy(slot.busy.clone()),
        }))
    }

    /// An output buffer: linear ARGB8888, which every GTK renderer imports,
    /// rendered to through a framebuffer.
    unsafe fn out_slot(&self, width: u32, height: u32) -> Result<OutSlot, String> {
        let bo = self
            .gbm
            .create_buffer_object::<()>(
                width,
                height,
                gbm::Format::Argb8888,
                gbm::BufferObjectFlags::RENDERING | gbm::BufferObjectFlags::LINEAR,
            )
            .map_err(|e| format!("gbm out {width}x{height}: {e}"))?;
        let planes = planes(&bo)?;
        let (image, texture) =
            unsafe { self.import(width, height, ARGB8888, MOD_LINEAR, &planes)? };
        let gl = &self.gl;
        let fbo = unsafe { gl.create_framebuffer() }?;
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(texture),
                0,
            );
            let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            if status != glow::FRAMEBUFFER_COMPLETE {
                gl.delete_framebuffer(fbo);
                gl.delete_texture(texture);
                let _ = self.egl.destroy_image(self.display, image);
                return Err(format!("out framebuffer incomplete: {status:#x}"));
            }
        }
        Ok(OutSlot {
            bo,
            image,
            texture,
            fbo,
            width,
            height,
            busy: Arc::new(AtomicBool::new(false)),
        })
    }

    /// A dmabuf as an EGL image and a texture bound to it.
    unsafe fn import(
        &self,
        width: u32,
        height: u32,
        fourcc: u32,
        modifier: u64,
        planes: &[Plane],
    ) -> Result<(egl::Image, glow::Texture), String> {
        let mut attrs: Vec<egl::Attrib> = vec![
            egl::WIDTH as egl::Attrib,
            width as egl::Attrib,
            egl::HEIGHT as egl::Attrib,
            height as egl::Attrib,
            EGL_LINUX_DRM_FOURCC_EXT,
            fourcc as egl::Attrib,
            EGL_IMAGE_PRESERVED_KHR,
            egl::TRUE as egl::Attrib,
        ];
        for (plane, names) in planes.iter().zip(EGL_PLANE_ATTRS) {
            attrs.extend([
                names[0],
                plane.fd.as_raw_fd() as egl::Attrib,
                names[1],
                plane.offset as egl::Attrib,
                names[2],
                plane.stride as egl::Attrib,
            ]);
            if modifier != MOD_INVALID {
                attrs.extend([
                    names[3],
                    (modifier & 0xffff_ffff) as egl::Attrib,
                    names[4],
                    (modifier >> 32) as egl::Attrib,
                ]);
            }
        }
        attrs.push(egl::ATTRIB_NONE);
        let image = self
            .egl
            .create_image(
                self.display,
                unsafe { egl::Context::from_ptr(egl::NO_CONTEXT) },
                EGL_LINUX_DMA_BUF_EXT,
                unsafe { egl::ClientBuffer::from_ptr(std::ptr::null_mut()) },
                &attrs,
            )
            .map_err(|e| format!("egl import {width}x{height} {fourcc:#x}/{modifier:#x}: {e}"))?;
        let gl = &self.gl;
        let texture = unsafe { gl.create_texture() }?;
        unsafe {
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            (self.image_target)(glow::TEXTURE_2D, image.as_ptr());
            // texelFetch only: no filter ever samples between texels.
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::NEAREST as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                glow::NEAREST as i32,
            );
            gl.bind_texture(glow::TEXTURE_2D, None);
        }
        Ok((image, texture))
    }

    /// Free a capture buffer's GL side; its dmabuf goes with the value.
    pub fn release_capture(&self, buffer: CaptureBuffer) {
        unsafe { self.gl.delete_texture(buffer.texture) };
        let _ = self.egl.destroy_image(self.display, buffer.image);
    }

    /// Free a pool's GL side. Buffers GTK still holds stay alive through
    /// their dmabuf fds.
    pub fn release_pool(&self, pool: &mut Pool) {
        for slot in pool.slots.drain(..) {
            unsafe {
                self.gl.delete_framebuffer(slot.fbo);
                self.gl.delete_texture(slot.texture);
            }
            let _ = self.egl.destroy_image(self.display, slot.image);
        }
    }
}

impl Drop for Gpu {
    fn drop(&mut self) {
        unsafe {
            self.gl.delete_program(self.program);
            self.gl.delete_vertex_array(self.vao);
        }
        let _ = self.egl.make_current(self.display, None, None, None);
        let _ = self.egl.destroy_context(self.display, self.context);
        let _ = self.egl.terminate(self.display);
    }
}

// ── Buffers ─────────────────────────────────────────────────────────────

struct Plane {
    fd: OwnedFd,
    offset: u32,
    stride: u32,
}

fn planes(bo: &gbm::BufferObject<()>) -> Result<Vec<Plane>, String> {
    (0..bo.plane_count() as i32)
        .map(|i| {
            Ok(Plane {
                fd: bo
                    .fd_for_plane(i)
                    .map_err(|e| format!("plane {i} fd: {e}"))?,
                offset: bo.offset(i),
                stride: bo.stride_for_plane(i),
            })
        })
        .collect()
}

/// The buffer a window is captured into, and the shader's view of it.
pub struct CaptureBuffer {
    /// Owns the memory the planes' fds and the image refer to.
    #[cfg_attr(not(test), allow(dead_code))]
    bo: gbm::BufferObject<()>,
    planes: Vec<Plane>,
    pub fourcc: u32,
    pub modifier: u64,
    pub width: u32,
    pub height: u32,
    image: egl::Image,
    texture: glow::Texture,
}

impl CaptureBuffer {
    /// Every plane as fd, offset, stride, for `zwp_linux_buffer_params_v1`.
    pub fn planes(&self) -> impl Iterator<Item = (std::os::fd::BorrowedFd<'_>, u32, u32)> {
        self.planes
            .iter()
            .map(|p| (p.fd.as_fd(), p.offset, p.stride))
    }
}

struct OutSlot {
    bo: gbm::BufferObject<()>,
    image: egl::Image,
    texture: glow::Texture,
    fbo: glow::Framebuffer,
    width: u32,
    height: u32,
    busy: Arc<AtomicBool>,
}

/// One window's output buffers.
#[derive(Default)]
pub struct Pool {
    slots: Vec<OutSlot>,
}

impl Pool {
    /// A free slot of this size. Slots of another size, left from before
    /// the window or its picture changed size, go once free.
    fn free(&mut self, width: u32, height: u32) -> Option<usize> {
        self.slots
            .retain(|s| (s.width, s.height) == (width, height) || s.busy.load(Ordering::Acquire));
        self.slots
            .iter()
            .position(|s| (s.width, s.height) == (width, height) && !s.busy.load(Ordering::Acquire))
    }
}

// ── The render node ─────────────────────────────────────────────────────

fn rdev(path: &std::path::Path) -> Option<u64> {
    std::fs::metadata(path).ok().map(|m| m.rdev())
}

/// The render node for device number `dev`: itself when it is one, else
/// the render node of the same card, through sysfs.
fn render_node(dev: u64) -> Option<std::path::PathBuf> {
    let (major, minor) = (libc::major(dev), libc::minor(dev));
    let sys = std::path::PathBuf::from(format!("/sys/dev/char/{major}:{minor}"));
    let name = std::fs::read_link(&sys).ok()?;
    let name = name.file_name()?.to_str()?;
    if name.starts_with("renderD") {
        return Some(std::path::Path::new("/dev/dri").join(name));
    }
    let drm = sys.join("device/drm");
    std::fs::read_dir(drm)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .find(|n| n.starts_with("renderD"))
        .map(|n| std::path::Path::new("/dev/dri").join(n))
}

// ── The shader ──────────────────────────────────────────────────────────

const VERTEX: &str = r#"#version 300 es
// One triangle over the whole target.
void main() {
    vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
    gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}
"#;

/// Each output pixel covers `ratio` source texels each way from
/// `origin + pixel * ratio`. Every texel under it adds its linear light,
/// weighted by the area of it inside, and the sum is divided by the area.
/// Texel rows run from the first row in memory, in the source and in the
/// target alike, so nothing flips.
const FRAGMENT: &str = r#"#version 300 es
precision highp float;
precision highp int;
uniform highp sampler2D src;
uniform vec2 origin;
uniform vec2 ratio;
uniform vec2 end;
uniform bool opaque;
out vec4 color;

vec3 decode(vec3 c) {
    return mix(c / 12.92, pow((c + 0.055) / 1.055, vec3(2.4)), step(vec3(0.04045), c));
}
vec3 encode(vec3 l) {
    return mix(l * 12.92, 1.055 * pow(l, vec3(1.0 / 2.4)) - 0.055, step(vec3(0.0031308), l));
}

void main() {
    vec2 a = origin + floor(gl_FragCoord.xy) * ratio;
    vec2 b = min(a + ratio, end);
    ivec2 i0 = ivec2(floor(a));
    ivec2 i1 = ivec2(ceil(b));
    vec4 sum = vec4(0.0);
    for (int y = i0.y; y < i1.y; y++) {
        float wy = min(b.y, float(y + 1)) - max(a.y, float(y));
        vec4 row = vec4(0.0);
        for (int x = i0.x; x < i1.x; x++) {
            float wx = min(b.x, float(x + 1)) - max(a.x, float(x));
            vec4 p = texelFetch(src, ivec2(x, y), 0);
            float al = opaque ? 1.0 : p.a;
            vec3 lin = al > 0.0 ? decode(clamp(p.rgb / al, 0.0, 1.0)) * al : vec3(0.0);
            row += wx * vec4(lin, al);
        }
        sum += wy * row;
    }
    sum /= (b.x - a.x) * (b.y - a.y);
    float al = clamp(sum.a, 0.0, 1.0);
    vec3 c = al > 0.0 ? encode(clamp(sum.rgb / al, 0.0, 1.0)) * al : vec3(0.0);
    color = vec4(c, al);
}
"#;

unsafe fn build_program(gl: &glow::Context) -> Result<(glow::Program, glow::VertexArray), String> {
    unsafe {
        let program = gl.create_program()?;
        let mut shaders = Vec::new();
        for (kind, source) in [
            (glow::VERTEX_SHADER, VERTEX),
            (glow::FRAGMENT_SHADER, FRAGMENT),
        ] {
            let shader = gl.create_shader(kind)?;
            gl.shader_source(shader, source);
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                return Err(format!("shader: {}", gl.get_shader_info_log(shader)));
            }
            gl.attach_shader(program, shader);
            shaders.push(shader);
        }
        gl.link_program(program);
        for shader in shaders {
            gl.detach_shader(program, shader);
            gl.delete_shader(shader);
        }
        if !gl.get_program_link_status(program) {
            return Err(format!("program: {}", gl.get_program_info_log(program)));
        }
        let vao = gl.create_vertex_array()?;
        Ok((program, vao))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The context on this machine's first render node, or `None` (and a
    /// line saying so) where there is none, as in CI.
    fn gpu() -> Option<Gpu> {
        let dev = rdev(std::path::Path::new("/dev/dri/renderD128"))?;
        match Gpu::open(dev) {
            Ok(g) => Some(g),
            Err(e) => {
                eprintln!("no gpu here, skipped: {e}");
                None
            }
        }
    }

    fn decode(c: f64) -> f64 {
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }

    fn encode(l: f64) -> f64 {
        if l <= 0.003_130_8 {
            l * 12.92
        } else {
            1.055 * l.powf(1.0 / 2.4) - 0.055
        }
    }

    /// What the shader must produce, the slow and obvious way: for every
    /// output pixel, every texel of the region, weighted by the area of it
    /// under the pixel, in linear light, in `f64`. Pixels are four bytes in
    /// memory order, the shader's result in ARGB8888's (B, G, R, A);
    /// `rgba` says the source's order is R, G, B instead.
    fn reference(
        src: &[u8],
        full_w: u32,
        (x0, y0, w, h): (u32, u32, u32, u32),
        (ow, oh): (u32, u32),
        opaque: bool,
        rgba: bool,
    ) -> Vec<u8> {
        let (rx, ry) = (f64::from(w) / f64::from(ow), f64::from(h) / f64::from(oh));
        let mut out = Vec::with_capacity((ow * oh * 4) as usize);
        for oy in 0..oh {
            for ox in 0..ow {
                let (ax, ay) = (f64::from(ox) * rx, f64::from(oy) * ry);
                let (bx, by) = (ax + rx, ay + ry);
                let mut sum = [0f64; 4];
                for y in ay.floor() as u32..(by.ceil() as u32).min(h) {
                    let wy = by.min(f64::from(y + 1)) - ay.max(f64::from(y));
                    for x in ax.floor() as u32..(bx.ceil() as u32).min(w) {
                        let wx = bx.min(f64::from(x + 1)) - ax.max(f64::from(x));
                        let i = (((y0 + y) * full_w + x0 + x) * 4) as usize;
                        let p = &src[i..i + 4];
                        let (r, g, b) = if rgba {
                            (p[0], p[1], p[2])
                        } else {
                            (p[2], p[1], p[0])
                        };
                        let a = if opaque { 1.0 } else { f64::from(p[3]) / 255.0 };
                        let lin = |c: u8| {
                            if a > 0.0 {
                                decode((f64::from(c) / 255.0 / a).min(1.0)) * a
                            } else {
                                0.0
                            }
                        };
                        for (s, v) in sum.iter_mut().zip([lin(r), lin(g), lin(b), a]) {
                            *s += wx * wy * v;
                        }
                    }
                }
                let area = rx * ry;
                let a = (sum[3] / area).clamp(0.0, 1.0);
                let c = |l: f64| {
                    if a > 0.0 {
                        ((encode((l / area / a).clamp(0.0, 1.0)) * a * 255.0).round()) as u8
                    } else {
                        0
                    }
                };
                out.extend([c(sum[2]), c(sum[1]), c(sum[0]), (a * 255.0).round() as u8]);
            }
        }
        out
    }

    #[test]
    fn the_reference_averages_light() {
        // White beside black into one pixel: half the light, code 188. The
        // average of the codes would be 128.
        let src = [255, 255, 255, 255, 0, 0, 0, 255];
        assert_eq!(
            reference(&src, 2, (0, 0, 2, 1), (1, 1), true, false),
            vec![188, 188, 188, 255]
        );
    }

    /// A premultiplied test image: edges, gradients and text-like strokes,
    /// alpha from opaque to clear, so every weight and every branch counts.
    fn image(w: u32, h: u32, alpha: bool) -> Vec<u8> {
        let mut px = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let stroke = (x / 3 + y / 5) % 4 == 0 || (x * 7 + y * 3) % 11 == 0;
                let a = if alpha {
                    ((x * 5 + y * 3) % 256) as u8
                } else {
                    255
                };
                let c = |v: u32| {
                    let v = if stroke { 250 } else { v % 256 };
                    (v * u32::from(a) / 255) as u8
                };
                px.extend([c(x), c(y * 2), c(x + y), a]);
            }
        }
        px
    }

    /// Put `pixels` (tightly packed) into a linear capture buffer.
    fn fill(buffer: &mut CaptureBuffer, pixels: &[u8]) {
        let (w, h) = (buffer.width, buffer.height);
        buffer
            .bo
            .map_mut(0, 0, w, h, |m| {
                let stride = m.stride() as usize;
                let row = (w * 4) as usize;
                let dst = m.buffer_mut();
                for y in 0..h as usize {
                    dst[y * stride..y * stride + row]
                        .copy_from_slice(&pixels[y * row..(y + 1) * row]);
                }
            })
            .expect("map capture");
    }

    /// The frame drawn in `pool`, tightly packed.
    fn read(pool: &Pool, frame: &GpuFrame) -> Vec<u8> {
        let slot = pool
            .slots
            .iter()
            .find(|s| s.busy.load(Ordering::Acquire))
            .expect("the frame's slot");
        let (w, h) = (frame.width, frame.height);
        slot.bo
            .map(0, 0, w, h, |m| {
                let stride = m.stride() as usize;
                let row = (w * 4) as usize;
                (0..h as usize)
                    .flat_map(|y| m.buffer()[y * stride..y * stride + row].to_vec())
                    .collect()
            })
            .expect("map out")
    }

    fn compare(fourcc: u32, (w, h): (u32, u32), cut: (u32, u32, u32, u32), size: (u32, u32)) {
        let Some(gpu) = gpu() else { return };
        let pixels = image(w, h, !opaque(fourcc));
        let mut buffer = gpu
            .capture_buffer(w, h, fourcc, &[MOD_LINEAR])
            .expect("capture buffer");
        fill(&mut buffer, &pixels);
        let (ow, oh) = crate::jump::live::out_size(cut.2, cut.3, size);
        let mut pool = Pool::default();
        let frame = gpu
            .downscale(&buffer, cut, (ow, oh), &mut pool)
            .expect("draw")
            .expect("a free slot");
        let got = read(&pool, &frame);
        let rgba = matches!(fourcc, ABGR8888 | XBGR8888);
        let want = reference(&pixels, w, cut, (ow, oh), opaque(fourcc), rgba);
        let worst = got
            .iter()
            .zip(&want)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap_or(0);
        let off = got.iter().zip(&want).filter(|(a, b)| a != b).count();
        eprintln!(
            "{fourcc:#x} {w}x{h} {cut:?} -> {ow}x{oh}: worst {worst}, {off} of {} bytes differ",
            got.len()
        );
        assert!(worst <= 1, "worst difference {worst}");
        drop(frame);
        gpu.release_pool(&mut pool);
        gpu.release_capture(buffer);
    }

    #[test]
    fn matches_the_reference_opaque() {
        // An odd ratio each way (3.79 and 3.78), as a pin at 2x draws.
        compare(XRGB8888, (997, 613), (0, 0, 997, 613), (263, 200));
    }

    #[test]
    fn matches_the_reference_with_alpha() {
        compare(ARGB8888, (997, 613), (0, 0, 997, 613), (263, 200));
    }

    #[test]
    fn matches_the_reference_for_a_crop_and_rgba_order() {
        compare(XBGR8888, (640, 480), (101, 57, 333, 211), (150, 150));
    }

    #[test]
    fn one_to_one_is_exact() {
        compare(ABGR8888, (64, 48), (0, 0, 64, 48), (64, 48));
    }
}

#[cfg(test)]
mod bench {
    use super::*;

    /// The time for one 2x window frame, 2900x1736 into 800x479, finish
    /// included. `cargo test --release -- --ignored gpu_frame_time`.
    #[test]
    #[ignore]
    fn gpu_frame_time() {
        let Some(dev) = rdev(std::path::Path::new("/dev/dri/renderD128")) else {
            return;
        };
        let gpu = Gpu::open(dev).expect("gpu");
        // Linear, and Intel's Tile4, which sway offers on this GPU.
        for modifier in [MOD_LINEAR, 0x0100_0000_0000_0009] {
            let Ok(buffer) = gpu.capture_buffer(2900, 1736, XRGB8888, &[modifier]) else {
                eprintln!("modifier {modifier:#x}: not here");
                continue;
            };
            let mut pool = Pool::default();
            let mut run = |n: u32| {
                let t = std::time::Instant::now();
                for _ in 0..n {
                    let frame = gpu
                        .downscale(&buffer, (0, 0, 2900, 1736), (800, 479), &mut pool)
                        .expect("draw")
                        .expect("slot");
                    drop(frame);
                }
                t.elapsed().as_secs_f64() * 1000.0 / f64::from(n)
            };
            // The GPU's clock ramps up under load; time it once it has.
            run(100);
            eprintln!("GPU {:#x}: {:.3} ms per frame", buffer.modifier, run(300));
            gpu.release_pool(&mut pool);
            gpu.release_capture(buffer);
        }
    }
}
