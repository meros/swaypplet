//! Two-pass GL renderer behind the demo.
//!
//! Pass 1 draws the backdrop (wallpaper plus whatever moving content the test
//! card adds) into an offscreen colour buffer, then builds its mip chain.
//! Pass 2 draws the glass, and the only thing it can read is that buffer.
//!
//! Keeping the backdrop offscreen is not an implementation detail, it is the
//! point: it is the same shape as a compositor sampling its own composited
//! output, and it means the glass pass has no access to the shapes' own
//! colours, only to what is behind them.

use gdk4::prelude::{TextureExt, TextureExtManual};
use glow::HasContext;
use std::collections::HashMap;

use crate::state::{MAX_RIPPLES, MAX_SHAPES, State};

const VERT: &str = include_str!("../shaders/fullscreen.vert");
const BACKDROP_FRAG: &str = include_str!("../shaders/backdrop.frag");
const GLASS_FRAG: &str = include_str!("../shaders/glass.frag");

pub struct Renderer {
    gl: glow::Context,
    backdrop: Program,
    glass: Program,
    vao: glow::VertexArray,
    fbo: glow::Framebuffer,
    color: glow::Texture,
    size: (i32, i32),
    max_lod: f32,
    wallpaper: Option<Wallpaper>,
    /// Rolling average of wall-clock frame intervals. Useful as a liveness
    /// check, but it measures the compositor's pacing as much as the shader.
    pub frame_ms: f64,
    last_frame: i64,
    /// GPU time for the glass pass alone, via GL_TIME_ELAPSED. This is the
    /// number that actually answers "what does this effect cost", because it
    /// excludes vsync, the backdrop pass and the compositor.
    pub glass_ms: f64,
    timers: [Option<glow::Query>; 2],
    timer_slot: usize,
    timer_primed: bool,
}

struct Wallpaper {
    texture: glow::Texture,
    width: f32,
    height: f32,
}

struct Program {
    id: glow::Program,
    locs: HashMap<String, Option<glow::UniformLocation>>,
}

impl Program {
    fn loc(&mut self, gl: &glow::Context, name: &str) -> Option<glow::UniformLocation> {
        if let Some(cached) = self.locs.get(name) {
            return *cached;
        }
        let loc = unsafe { gl.get_uniform_location(self.id, name) };
        self.locs.insert(name.to_string(), loc);
        loc
    }
}

impl Renderer {
    pub fn new(gl: glow::Context) -> Result<Self, String> {
        unsafe {
            let backdrop = link(&gl, VERT, BACKDROP_FRAG)?;
            let glass = link(&gl, VERT, GLASS_FRAG)?;
            let vao = gl
                .create_vertex_array()
                .map_err(|e| format!("vertex array: {e}"))?;
            let fbo = gl
                .create_framebuffer()
                .map_err(|e| format!("framebuffer: {e}"))?;
            let color = gl.create_texture().map_err(|e| format!("texture: {e}"))?;
            let timers = [gl.create_query().ok(), gl.create_query().ok()];
            Ok(Renderer {
                gl,
                backdrop,
                glass,
                vao,
                fbo,
                color,
                size: (0, 0),
                max_lod: 0.0,
                wallpaper: None,
                frame_ms: 0.0,
                last_frame: 0,
                glass_ms: 0.0,
                timers,
                timer_slot: 0,
                timer_primed: false,
            })
        }
    }

    /// Upload a decoded `GdkTexture` once; the demo never changes wallpaper
    /// mid-run, so there is no reupload path.
    pub fn set_wallpaper(&mut self, texture: &gdk4::Texture) {
        let w = texture.width();
        let h = texture.height();
        if w <= 0 || h <= 0 {
            return;
        }
        let stride = (w as usize) * 4;
        let mut data = vec![0u8; stride * h as usize];
        texture.download(&mut data, stride);
        // GdkTexture downloads as BGRA on little-endian; GL_BGRA on upload
        // avoids a swizzle pass over a 4K image.
        unsafe {
            let gl = &self.gl;
            let tex = gl.create_texture().ok();
            let Some(tex) = tex else { return };
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::LINEAR as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                glow::LINEAR as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_S,
                glow::CLAMP_TO_EDGE as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_T,
                glow::CLAMP_TO_EDGE as i32,
            );
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                w,
                h,
                0,
                glow::BGRA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(&data)),
            );
            gl.bind_texture(glow::TEXTURE_2D, None);
            self.wallpaper = Some(Wallpaper {
                texture: tex,
                width: w as f32,
                height: h as f32,
            });
        }
    }

    fn resize(&mut self, w: i32, h: i32) {
        if self.size == (w, h) || w <= 0 || h <= 0 {
            return;
        }
        unsafe {
            let gl = &self.gl;
            gl.bind_texture(glow::TEXTURE_2D, Some(self.color));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                w,
                h,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::LINEAR_MIPMAP_LINEAR as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                glow::LINEAR as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_S,
                glow::CLAMP_TO_EDGE as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_T,
                glow::CLAMP_TO_EDGE as i32,
            );
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.fbo));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(self.color),
                0,
            );
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            gl.bind_texture(glow::TEXTURE_2D, None);
        }
        self.size = (w, h);
        self.max_lod = (w.max(h) as f32).log2().floor().max(0.0);
    }

    pub fn render(&mut self, w: i32, h: i32, state: &State) {
        self.resize(w, h);
        if self.size.0 <= 0 {
            return;
        }

        let now = glib::monotonic_time();
        if self.last_frame != 0 {
            let dt = (now - self.last_frame) as f64 / 1000.0;
            self.frame_ms = if self.frame_ms == 0.0 {
                dt
            } else {
                self.frame_ms * 0.9 + dt * 0.1
            };
        }
        self.last_frame = now;

        // GTK renders the GLArea into its own framebuffer; borrow it back
        // after the offscreen pass instead of assuming it is 0.
        let target = unsafe {
            let id = self.gl.get_parameter_i32(glow::DRAW_FRAMEBUFFER_BINDING);
            if id == 0 {
                None
            } else {
                Some(glow::NativeFramebuffer(
                    std::num::NonZeroU32::new(id as u32).unwrap(),
                ))
            }
        };

        unsafe {
            let gl = &self.gl;
            gl.bind_vertex_array(Some(self.vao));
            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::BLEND);
            gl.viewport(0, 0, self.size.0, self.size.1);

            // ---- pass 1: backdrop -> offscreen
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.fbo));
            gl.use_program(Some(self.backdrop.id));
            let p = &mut self.backdrop;
            gl.uniform_2_f32(
                p.loc(gl, "uResolution").as_ref(),
                self.size.0 as f32,
                self.size.1 as f32,
            );
            gl.uniform_1_f32(p.loc(gl, "uTime").as_ref(), state.time as f32);
            gl.uniform_1_f32(p.loc(gl, "uTestCard").as_ref(), state.test_card);
            gl.active_texture(glow::TEXTURE0);
            match &self.wallpaper {
                Some(wp) => {
                    gl.bind_texture(glow::TEXTURE_2D, Some(wp.texture));
                    gl.uniform_1_f32(p.loc(gl, "uHasWallpaper").as_ref(), 1.0);
                    gl.uniform_2_f32(p.loc(gl, "uWallpaperSize").as_ref(), wp.width, wp.height);
                }
                None => {
                    gl.bind_texture(glow::TEXTURE_2D, None);
                    gl.uniform_1_f32(p.loc(gl, "uHasWallpaper").as_ref(), 0.0);
                    gl.uniform_2_f32(p.loc(gl, "uWallpaperSize").as_ref(), 1.0, 1.0);
                }
            }
            gl.uniform_1_i32(p.loc(gl, "uWallpaper").as_ref(), 0);
            gl.draw_arrays(glow::TRIANGLES, 0, 3);

            // Mip chain doubles as the frost: sampling at a higher LOD is a
            // pre-filtered blur that costs one tap instead of a kawase pair.
            gl.bind_texture(glow::TEXTURE_2D, Some(self.color));
            gl.generate_mipmap(glow::TEXTURE_2D);

            gl.bind_framebuffer(glow::FRAMEBUFFER, target);
            gl.use_program(Some(self.glass.id));
        }

        // ---- pass 2: glass -> GTK's framebuffer
        self.upload_glass_uniforms(state);

        unsafe {
            let gl = &self.gl;
            // Double-buffered timer queries: read the result the *other* slot
            // finished last frame, so nothing ever blocks waiting for the GPU.
            let slot = self.timer_slot;
            let other = 1 - slot;
            if let Some(q) = self.timers[slot] {
                gl.begin_query(glow::TIME_ELAPSED, q);
            }
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.color));
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
            if self.timers[slot].is_some() {
                gl.end_query(glow::TIME_ELAPSED);
            }
            if let Some(q) = self.timers[other].filter(|_| self.timer_primed)
                && gl.get_query_parameter_u32(q, glow::QUERY_RESULT_AVAILABLE) != 0
            {
                let ns = gl.get_query_parameter_u32(q, glow::QUERY_RESULT) as f64;
                self.glass_ms = ns / 1.0e6;
            }
            self.timer_slot = other;
            self.timer_primed = true;
            gl.bind_vertex_array(None);
        }
    }

    fn upload_glass_uniforms(&mut self, state: &State) {
        let gl = &self.gl;
        let p = &mut self.glass;
        let s = &state.params;

        let mut pos = [0f32; MAX_SHAPES * 2];
        let mut half = [0f32; MAX_SHAPES * 2];
        let mut radius = [0f32; MAX_SHAPES];
        let mut rot = [0f32; MAX_SHAPES];
        let count = state.shapes.len().min(MAX_SHAPES);
        for (i, sh) in state.shapes.iter().take(count).enumerate() {
            pos[i * 2] = sh.pos[0];
            pos[i * 2 + 1] = sh.pos[1];
            half[i * 2] = sh.half[0];
            half[i * 2 + 1] = sh.half[1];
            radius[i] = sh.radius;
            rot[i] = sh.rot;
        }

        let mut ripples = [0f32; MAX_RIPPLES * 3];
        let mut rcount = 0usize;
        for r in state.ripples.iter() {
            let age = (state.time - r.born) as f32;
            if !(0.0..=2.6).contains(&age) || rcount >= MAX_RIPPLES {
                continue;
            }
            ripples[rcount * 3] = r.pos[0];
            ripples[rcount * 3 + 1] = r.pos[1];
            ripples[rcount * 3 + 2] = age;
            rcount += 1;
        }

        unsafe {
            gl.uniform_1_i32(p.loc(gl, "uBackdrop").as_ref(), 0);
            gl.uniform_2_f32(
                p.loc(gl, "uResolution").as_ref(),
                self.size.0 as f32,
                self.size.1 as f32,
            );
            gl.uniform_1_f32(p.loc(gl, "uTime").as_ref(), state.time as f32);
            gl.uniform_1_f32(p.loc(gl, "uMaxLod").as_ref(), self.max_lod);

            gl.uniform_1_i32(p.loc(gl, "uShapeCount").as_ref(), count as i32);
            gl.uniform_2_f32_slice(p.loc(gl, "uShapePos").as_ref(), &pos);
            gl.uniform_2_f32_slice(p.loc(gl, "uShapeHalf").as_ref(), &half);
            gl.uniform_1_f32_slice(p.loc(gl, "uShapeRadius").as_ref(), &radius);
            gl.uniform_1_f32_slice(p.loc(gl, "uShapeRot").as_ref(), &rot);

            gl.uniform_1_f32(p.loc(gl, "uMerge").as_ref(), s.merge);
            gl.uniform_1_f32(p.loc(gl, "uBezel").as_ref(), s.bezel);
            gl.uniform_1_f32(p.loc(gl, "uThickness").as_ref(), s.thickness);
            gl.uniform_1_i32(p.loc(gl, "uProfile").as_ref(), s.profile);
            gl.uniform_1_f32(p.loc(gl, "uIor").as_ref(), s.ior);
            gl.uniform_1_f32(p.loc(gl, "uDispersion").as_ref(), s.dispersion);
            gl.uniform_1_i32(p.loc(gl, "uSamples").as_ref(), s.samples);
            gl.uniform_1_f32(p.loc(gl, "uFrost").as_ref(), s.frost);
            gl.uniform_1_f32(p.loc(gl, "uSpecular").as_ref(), s.specular);
            gl.uniform_1_f32(p.loc(gl, "uShine").as_ref(), s.shine);
            gl.uniform_3_f32(
                p.loc(gl, "uLightDir").as_ref(),
                state.light[0],
                state.light[1],
                state.light[2],
            );
            gl.uniform_1_f32(p.loc(gl, "uFresnel").as_ref(), s.fresnel);
            gl.uniform_1_f32(p.loc(gl, "uLensGain").as_ref(), s.lens_gain);
            gl.uniform_4_f32(
                p.loc(gl, "uTint").as_ref(),
                s.tint[0],
                s.tint[1],
                s.tint[2],
                s.tint[3],
            );
            gl.uniform_1_f32(p.loc(gl, "uShadow").as_ref(), s.shadow);
            gl.uniform_1_f32(p.loc(gl, "uRefract").as_ref(), s.refract);
            gl.uniform_1_f32(p.loc(gl, "uEdgeLight").as_ref(), s.edge_light);
            gl.uniform_1_f32(p.loc(gl, "uNoise").as_ref(), s.noise);
            gl.uniform_1_f32(p.loc(gl, "uDebug").as_ref(), state.debug as f32);

            gl.uniform_1_i32(p.loc(gl, "uRippleCount").as_ref(), rcount as i32);
            gl.uniform_3_f32_slice(p.loc(gl, "uRipple").as_ref(), &ripples);
            gl.uniform_1_f32(p.loc(gl, "uRippleAmp").as_ref(), s.ripple_amp);
        }
    }
}

impl Renderer {
    /// Read the frame back out of whatever framebuffer is currently bound,
    /// already flipped into top-down row order for `GdkMemoryTexture`.
    pub fn grab(&self) -> Option<(Vec<u8>, i32, i32)> {
        let (w, h) = self.size;
        if w <= 0 || h <= 0 {
            return None;
        }
        let stride = w as usize * 4;
        let mut buf = vec![0u8; stride * h as usize];
        unsafe {
            self.gl.pixel_store_i32(glow::PACK_ALIGNMENT, 1);
            self.gl.read_pixels(
                0,
                0,
                w,
                h,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut buf)),
            );
        }
        let mut flipped = vec![0u8; buf.len()];
        for row in 0..h as usize {
            let src = (h as usize - 1 - row) * stride;
            flipped[row * stride..row * stride + stride].copy_from_slice(&buf[src..src + stride]);
        }
        Some((flipped, w, h))
    }
}

unsafe fn link(gl: &glow::Context, vert: &str, frag: &str) -> Result<Program, String> {
    unsafe {
        let program = gl.create_program().map_err(|e| format!("program: {e}"))?;
        let mut stages = Vec::new();
        for (kind, src) in [(glow::VERTEX_SHADER, vert), (glow::FRAGMENT_SHADER, frag)] {
            let shader = gl.create_shader(kind).map_err(|e| format!("shader: {e}"))?;
            gl.shader_source(shader, src);
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                return Err(format!(
                    "{} shader:\n{}",
                    if kind == glow::VERTEX_SHADER {
                        "vertex"
                    } else {
                        "fragment"
                    },
                    gl.get_shader_info_log(shader)
                ));
            }
            gl.attach_shader(program, shader);
            stages.push(shader);
        }
        gl.link_program(program);
        if !gl.get_program_link_status(program) {
            return Err(format!("link: {}", gl.get_program_info_log(program)));
        }
        for shader in stages {
            gl.detach_shader(program, shader);
            gl.delete_shader(shader);
        }
        Ok(Program {
            id: program,
            locs: HashMap::new(),
        })
    }
}
