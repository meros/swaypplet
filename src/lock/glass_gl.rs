//! GL backend for [`GlassPane`](super::glass::GlassPane).
//!
//! The GSK path in `glass.rs` fakes glass with `push_blur`, because that is
//! the only optical primitive GSK offers. `GskGLShader` was deprecated in GTK
//! 4.16 and does not render under the current renderers, which is usually
//! where the "no custom shaders in GTK4" story stops — but `GtkGLArea` is
//! still public and still hands a client a raw GL context, and
//! `dev/glass-demo/lockprobe.sh` verified that it works on an
//! `ext-session-lock-v1` surface specifically. So this runs real refraction
//! where the blur used to be.
//!
//! Two passes, mirroring the demo. Pass 1 reproduces the wallpaper exactly as
//! the full-screen `Picture` behind the pane draws it, scrim included, into an
//! offscreen buffer with a mip chain. Pass 2 refracts that buffer and writes
//! opaque pixels, resolving the card's rounded corners against pass 1's own
//! output — which is the same pixels the picture behind is showing, so the
//! corner nubs are indistinguishable from what they cover.
//!
//! # Failing safe
//!
//! This is the one surface in swaypplet where a rendering failure means the
//! user cannot get back into their machine. Every path that can fail — no GL
//! context, a shader that will not compile, a driver that will not give a
//! framebuffer — sets [`GlGlass::failed`], and `GlassPane` then hides this
//! widget and draws the GSK blur it always drew. `SWAYPPLET_LOCK_GLASS=gsk`
//! skips the GL path entirely without a rebuild.

use gtk4::prelude::*;
use gtk4::{gdk, glib};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use glow::HasContext;

const VERT: &str = include_str!("shaders/pane.vert");
const BACKDROP_FRAG: &str = include_str!("shaders/pane_backdrop.frag");
const GLASS_FRAG: &str = include_str!("shaders/pane_glass.frag");

/// Optical settings.
///
/// The defaults are the shipped look. Every field can be overridden by an
/// environment variable named after it (`SWAYPPLET_GLASS_FROST=3.0`), which
/// is what `dev/lock-glass-sweep.sh` sweeps to render comparison sheets —
/// tuning a material by rebuilding is a slow way to find out that a number
/// was wrong.
#[derive(Clone, Copy, Debug)]
pub struct Tuning {
    /// Corner radius of the glass body. Defaults to the card's CSS radius,
    /// because the glass and the card's own background have to agree on where
    /// the corner is. Raising it is a real look — a lozenge rather than a
    /// card — but it only works if `.lock-card`'s border-radius moves with it.
    pub radius: f32,
    /// Width of the sloped rim. Never wider than [`CORNER_RADIUS`]: the shader
    /// clamps it anyway, because a band reaching past the radius swallows the
    /// distance field's corner singularity and puts a dark dot in each corner.
    pub bezel: f32,
    /// 0 convex circle, 1 convex squircle, 2 concave, 3 lip.
    pub surface_type: i32,
    /// How tall the slab is. With a narrow bezel the surface rises steeply,
    /// which is what gives a 360 px card an edge that reads as thick glass
    /// rather than as a bevelled sticker.
    pub thickness: f32,
    pub refraction: f32,
    pub dispersion: f32,
    pub samples: i32,
    /// Surface roughness, 0 mirror-smooth to 1 fully diffuse.
    ///
    /// The one knob the physics actually has. A microfacet surface scatters
    /// transmission, widens the specular lobe and blurs the reflection by the
    /// *same* distribution — so frost, shine and reflect_blur below are not
    /// independent settings, they are this one projected three ways, and
    /// `derive()` does the projecting. Setting them apart is what makes a
    /// material look assembled rather than real: a mirror-sharp reflection on
    /// a milky surface is a combination nothing physical can produce.
    ///
    /// Low is wet-looking: a tight bright highlight over a clear image.
    pub roughness: f32,
    /// How much of the backdrop arrives scattered rather than sharp, 0 to 1.
    ///
    /// Named and scaled to match the compositor's `liquid_glass_frost`, so
    /// the lock screen and the bar mean the same thing by the same number.
    /// This backend reaches it with a mip level rather than a kawase pass —
    /// it owns a single texture, not a whole composited output — but that is
    /// an implementation detail on this side of the boundary.
    pub frost: f32,
    /// Roughly how many pixels the scatter spans at `frost` 1.
    pub frost_radius: f32,
    /// Thickness of a surface coating, in nanometres. Real film interference:
    /// light reflected off the top of the film and off its underside differ
    /// in path by twice the thickness, so some wavelengths cancel and others
    /// add, and which ones depends on the viewing angle. That is the
    /// soap-bubble rainbow. 0 disables it; a few hundred nm is where the
    /// visible spectrum lands.
    pub film_nm: f32,
    /// Beer-Lambert absorption per unit of path through the slab, which is
    /// what makes this smoked glass rather than clear glass. Transmission is
    /// `exp(-absorb * path)`, so the thick middle goes dark and the thin rim
    /// stays readable, and the imbalance between channels is the tint: more
    /// blue absorbed than red, so the smoke reads warm-neutral, not cyan.
    pub absorb: f32,
    /// Floor under that path length, so even the thinnest rim is smoked.
    /// Without it the outer millimetre is clear glass and the card looks like
    /// it has a bright wire around it.
    pub absorb_floor: f32,
    /// Forward scattering: how much of the result is turbid haze rather than
    /// a clean image. The "smoke" as distinct from the "dark".
    pub haze: f32,
    /// Fresnel reflection. A client has no environment to reflect, so what
    /// this mixes in is an offset copy of the wallpaper behind the card.
    pub reflection: f32,
    /// How blurred that reflection is, 0 sharp to 1 fully scattered. High
    /// values are why a reflection can be present and still invisible: it is
    /// there, it just has no structure left to recognise.
    pub reflect_blur: f32,
    /// Specular from an invented light. There is no light source on a lock
    /// screen, and a highlight that claims one is the single thing that stops
    /// the material reading as glass and starts it reading as plastic.
    pub specular: f32,
    pub shine: f32,
    /// Light concentration from the height field's Laplacian. Signed: the
    /// same setting that brightens the inner shoulder darkens the outer rim.
    pub lensing: f32,
    /// A drawn line just inside the edge. Not a reflection of anything.
    pub edge_light: f32,
    /// Ambient occlusion just inside the rim: the light a thick edge blocks
    /// from reaching what is under it. What makes glass sit on a surface
    /// instead of hovering over it.
    pub inner_shadow: f32,
    /// Sharpened convergence. A lens does not merely brighten where it
    /// focuses, it throws a hot line; this is the squared convergence term.
    pub caustic: f32,
    /// Shift tone against the backdrop's luminance, for legibility. Apple's
    /// stated mechanism, and the reason a fixed tint fails: the same card has
    /// to read over a white page and a black one.
    pub adaptive: f32,
    /// Scatter more where the backdrop is busy. Detail behind glass is what
    /// makes text on it unreadable, and this removes detail selectively
    /// rather than blurring everything.
    pub adaptive_frost: f32,
    pub noise: f32,
    /// How much of the scrim the glass body takes. The corners take all of
    /// it, because they have to match the picture behind; the body takes
    /// less, because absorption is already darkening it and doing both leaves
    /// an opaque slab with nothing to see through.
    pub scrim_in_glass: f32,
}

impl Default for Tuning {
    fn default() -> Self {
        Tuning {
            radius: CORNER_RADIUS,
            // Equal to the radius, which is the widest the shader will use.
            bezel: 18.0,
            surface_type: 1,
            // Steep enough that the outer part of the rim passes 45 degrees,
            // which is where the reflection starts landing back on the
            // wallpaper instead of leaving upward. Below about 40 there is no
            // droplet edge to see.
            thickness: 60.0,
            refraction: 1.5,
            // Barely there. Wider dispersion is the demo's party trick, but
            // this sits under a password field and a prism edge there is a
            // legibility bug.
            dispersion: 0.004,
            samples: 4,
            roughness: 0.30,
            frost: 0.45,
            frost_radius: 10.0,
            film_nm: 0.0,
            absorb: 0.78,
            absorb_floor: 0.14,
            haze: 0.14,
            // 1.0 is the physical value: Schlick's F already *is* the
            // reflectance, and it only passes 0.2 beyond about 72 degrees, so
            // the reflection confines itself to the outer rim without help.
            // It sat at 0.80 only to compensate for two errors that cancelled
            // — the reflection being absorbed by a slab it never entered, and
            // being sampled off unscrimmed wallpaper. Both are fixed.
            reflection: 1.0,
            // Nearly sharp. The reflection is the thing worth seeing, and
            // blurring it was why it read as milk before.
            reflect_blur: 0.15,
            specular: 0.03,
            shine: 46.0,
            lensing: 0.22,
            edge_light: 0.05,
            inner_shadow: 0.0,
            caustic: 0.0,
            adaptive: 0.0,
            adaptive_frost: 0.0,
            noise: 0.012,
            scrim_in_glass: 0.45,
        }
    }
}

fn env_f32(name: &str, default: f32) -> f32 {
    std::env::var(format!("SWAYPPLET_GLASS_{name}"))
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

impl Tuning {
    pub fn from_env() -> Self {
        let d = Tuning::default();
        Tuning {
            radius: env_f32("RADIUS", d.radius),
            bezel: env_f32("BEZEL", d.bezel),
            surface_type: env_f32("SURFACE_TYPE", d.surface_type as f32) as i32,
            thickness: env_f32("THICKNESS", d.thickness),
            refraction: env_f32("REFRACTION", d.refraction),
            dispersion: env_f32("DISPERSION", d.dispersion),
            samples: env_f32("SAMPLES", d.samples as f32) as i32,
            roughness: env_f32("ROUGHNESS", d.roughness),
            frost: env_f32("FROST", d.frost),
            frost_radius: env_f32("FROST_RADIUS", d.frost_radius),
            film_nm: env_f32("FILM_NM", d.film_nm),
            absorb: env_f32("ABSORB", d.absorb),
            absorb_floor: env_f32("ABSORB_FLOOR", d.absorb_floor),
            haze: env_f32("HAZE", d.haze),
            reflection: env_f32("REFLECTION", d.reflection),
            reflect_blur: env_f32("REFLECT_BLUR", d.reflect_blur),
            specular: env_f32("SPECULAR", d.specular),
            shine: env_f32("SHINE", d.shine),
            lensing: env_f32("LENSING", d.lensing),
            edge_light: env_f32("EDGE_LIGHT", d.edge_light),
            inner_shadow: env_f32("INNER_SHADOW", d.inner_shadow),
            caustic: env_f32("CAUSTIC", d.caustic),
            adaptive: env_f32("ADAPTIVE", d.adaptive),
            adaptive_frost: env_f32("ADAPTIVE_FROST", d.adaptive_frost),
            noise: env_f32("NOISE", d.noise),
            scrim_in_glass: env_f32("SCRIM_IN_GLASS", d.scrim_in_glass),
        }
    }

    /// How much wallpaper to capture outside the pane, in logical pixels.
    ///
    /// Reflections off the steep part of the rim land outside the shape, and
    /// refraction near the edge can too, so the backdrop pass covers the pane
    /// plus this on every side. Without it those samples clamp to the pane's
    /// edge texel and smear. Scaled off the bezel because that is what sets
    /// how far a ray can travel before it hits the plane.
    pub fn margin(&self) -> f32 {
        (self.bezel * 1.4 + 20.0).clamp(24.0, 128.0)
    }

    /// Per-channel absorption. One scalar drives all three so a sweep has one
    /// axis to move; the ratios are what make the smoke warm rather than grey.
    pub fn absorb_rgb(&self) -> [f32; 3] {
        [self.absorb * 0.90, self.absorb, self.absorb * 1.18]
    }

    /// Project roughness onto the three things it physically controls.
    ///
    /// Returns `(frost, shine, reflect_blur)`. The specular exponent comes
    /// from the standard Blinn-Phong fit to a Beckmann lobe, `2/a^2 - 2`,
    /// which is what keeps a smooth surface's highlight tight and a rough
    /// one's broad. Scatter and reflection blur rise with the same alpha, so
    /// a surface cannot be milky and mirror-sharp at once.
    ///
    /// A zero roughness is a mirror: no scatter, a pinpoint highlight, a
    /// perfectly sharp reflection. That is the wet end.
    pub fn derive(&self) -> (f32, f32, f32) {
        let a = self.roughness.clamp(0.0, 1.0);
        let alpha = (a * a).max(1.0e-4);
        let shine = (2.0 / alpha - 2.0).clamp(2.0, 4096.0);
        (a.powf(0.75), shine, a.powf(0.9))
    }
}

/// Matches `.lock-card` border-radius in style.css, and the GSK path's
/// `CORNER_RADIUS`.
const CORNER_RADIUS: f32 = 18.0;
/// Light from the upper left, in pane-local coordinates (y down). Fixed
/// rather than pointer-driven: the lock screen has no pointer to speak of and
/// a highlight that chased one would be a distraction over a password field.
const LIGHT: [f32; 3] = [-0.45, -0.65, 0.62];
/// Matches `.lock-scrim` (alpha(black, 0.45)), so the glass shows the dimmed
/// wallpaper rather than a brighter raw patch.
const SCRIM: [f32; 4] = [0.0, 0.0, 0.0, 0.45];

/// Log the glass pass's GPU time every 60 frames, and allocate the timer
/// queries that make that possible. Off by default: a lock screen should not
/// be issuing queries nobody reads.
fn stats() -> bool {
    matches!(
        std::env::var("SWAYPPLET_LOCK_GLASS_STATS").as_deref(),
        Ok("1")
    )
}

/// Is the GL backend wanted at all? `SWAYPPLET_LOCK_GLASS=gsk` forces the
/// old path; anything else (including unset) allows GL, which still has to
/// come up successfully before it is used.
pub fn enabled() -> bool {
    !matches!(
        std::env::var("SWAYPPLET_LOCK_GLASS").as_deref(),
        Ok("gsk") | Ok("blur") | Ok("0")
    )
}

pub struct GlGlass {
    area: gtk4::GLArea,
    shared: Rc<Shared>,
}

struct Shared {
    texture: RefCell<Option<gdk::Texture>>,
    /// Texture placement in pane-local device pixels, from `cover_rect`.
    cover: Cell<[f32; 4]>,
    /// Materialize ramp, 0 crisp to 1 full glass.
    t: Cell<f32>,
    /// Device pixels per logical pixel, so the bezel is the same physical
    /// size on a HiDPI panel as on a 1x one.
    scale: Cell<f32>,
    failed: Cell<bool>,
    renderer: RefCell<Option<Renderer>>,
    tuning: Tuning,
    /// Has a frame of actual glass been painted yet?
    ///
    /// The enter ramp is keyed off this rather than off construction. GL
    /// setup, the first wallpaper upload and the pane's first allocation all
    /// have to land before anything can be drawn, and a ramp started before
    /// them is simply over by the time it could be seen — so the glass
    /// arrives at full strength in one frame, which reads as a pop. Deferring
    /// the ramp to here means the first frame the user sees is the first
    /// frame of the animation.
    drawn: Cell<bool>,
    on_ready: RefCell<Option<Box<dyn Fn()>>>,
}

impl GlGlass {
    pub fn new() -> Self {
        let area = gtk4::GLArea::new();
        area.set_has_depth_buffer(false);
        area.set_has_stencil_buffer(false);
        area.set_auto_render(true);
        area.set_can_target(false);
        area.set_can_focus(false);
        // `#version 330 core` is desktop GL. Asking up front turns a confusing
        // link failure into a plain "no context", which the fallback handles.
        area.set_allowed_apis(gdk::GLAPI::GL);

        let shared = Rc::new(Shared {
            texture: RefCell::new(None),
            cover: Cell::new([0.0; 4]),
            t: Cell::new(0.0),
            scale: Cell::new(1.0),
            failed: Cell::new(false),
            renderer: RefCell::new(None),
            tuning: Tuning::from_env(),
            drawn: Cell::new(false),
            on_ready: RefCell::new(None),
        });

        area.connect_render({
            let shared = shared.clone();
            move |area, _ctx| {
                // GL setup belongs here, not in ::realize: during ::realize
                // the context exists but is not necessarily current, and
                // epoxy's dispatch stubs resolve against whatever context is
                // current when first called, so the first GL call segfaults
                // inside libepoxy instead of returning an error.
                if shared.failed.get() {
                    return glib::Propagation::Stop;
                }
                let scale = area.scale_factor();
                let (w, h) = (area.width() * scale, area.height() * scale);
                if w <= 0 || h <= 0 {
                    return glib::Propagation::Stop;
                }
                shared.scale.set(scale as f32);

                let mut slot = shared.renderer.borrow_mut();
                if slot.is_none() {
                    if let Some(err) = area.error() {
                        log::warn!("lock glass: no GL context ({err}); using GSK blur");
                        shared.fail(area);
                        return glib::Propagation::Stop;
                    }
                    let loader = gl_loader();
                    let gl = unsafe { glow::Context::from_loader_function(|s| loader(s)) };
                    match Renderer::new(gl) {
                        Ok(mut r) => {
                            if let Some(t) = shared.texture.borrow().as_ref() {
                                r.set_wallpaper(t);
                            }
                            *slot = Some(r);
                        }
                        Err(e) => {
                            log::warn!("lock glass: {e}; using GSK blur");
                            drop(slot);
                            shared.fail(area);
                            return glib::Propagation::Stop;
                        }
                    }
                }

                if let Some(r) = slot.as_mut()
                    && r.render(w, h, &shared)
                    && !shared.drawn.replace(true)
                {
                    // Drop the borrow before the callback: it starts the
                    // ramp, which calls back in to set the ramp value.
                    drop(slot);
                    if let Some(cb) = shared.on_ready.borrow().as_ref() {
                        cb();
                    }
                }
                glib::Propagation::Stop
            }
        });

        GlGlass { area, shared }
    }

    pub fn widget(&self) -> &gtk4::GLArea {
        &self.area
    }

    pub fn failed(&self) -> bool {
        self.shared.failed.get()
    }

    /// Has real glass been painted yet? Until it has, this widget is
    /// deliberately transparent and the pane shows the picture behind it.
    pub fn drawn(&self) -> bool {
        self.shared.drawn.get()
    }

    /// Called once, on the first frame of actual glass. The enter ramp hangs
    /// off this so the animation starts when there is something to animate.
    pub fn connect_ready(&self, f: impl Fn() + 'static) {
        if self.shared.drawn.get() {
            f();
            return;
        }
        *self.shared.on_ready.borrow_mut() = Some(Box::new(f));
    }

    /// How much wallpaper the backdrop pass needs outside the pane, in device
    /// pixels. A live source has to render that much extra, or the reflection
    /// at the rim has nothing outside the edge to find.
    pub fn margin(&self, scale: f32) -> f32 {
        (self.shared.tuning.margin() * scale).round()
    }

    /// The pixels behind this pane, plus where they sit in pane-local device
    /// coordinates.
    ///
    /// Two callers, one path. A still wallpaper hands over the whole decoded
    /// image once, with `cover` describing the ContentFit::Cover placement the
    /// picture behind uses. A live source hands over a pane-sized crop it has
    /// already rendered, every time it changes, with `cover` spanning the
    /// whole pane. Neither is privileged here: whatever arrives is what gets
    /// refracted, so nothing downstream assumes the backdrop holds still.
    pub fn set_frame(&self, texture: Option<gdk::Texture>, cover: [f32; 4]) {
        *self.shared.texture.borrow_mut() = texture;
        self.shared.cover.set(cover);
        // Uploading needs a current GL context and there is none outside
        // ::render, so flag it and let the next frame do the upload.
        if let Some(r) = self.shared.renderer.borrow_mut().as_mut() {
            r.wallpaper_dirty = true;
        }
        self.area.queue_render();
    }

    /// Materialize ramp, 0 crisp to 1 full glass. This is what the GSK path's
    /// blur sigma maps onto, so one animation drives both backends.
    pub fn set_ramp(&self, t: f32) {
        let t = t.clamp(0.0, 1.0);
        if (self.shared.t.get() - t).abs() > 1.0e-4 {
            self.shared.t.set(t);
            self.area.queue_render();
        }
    }
}

impl Shared {
    fn fail(&self, area: &gtk4::GLArea) {
        self.failed.set(true);
        area.set_visible(false);
        // Whatever contains this pane has to repaint with the GSK glass now.
        if let Some(parent) = area.parent() {
            parent.queue_draw();
        }
    }
}

/// GL entry points, resolved out of libepoxy.
///
/// libepoxy is the dispatch layer GTK itself calls GL through. It does not
/// export `glDrawArrays`; it exports `epoxy_glDrawArrays`, a *data* symbol
/// holding a self-resolving stub's address, and the public name is a macro
/// over it — so the loader prefixes the name and dereferences what it finds.
/// Dereferencing wrongly calls into `.data` and segfaults inside libepoxy with
/// a backtrace that blames the GL call.
///
/// The handle is the process's own global scope rather than a fresh `dlopen`,
/// because libepoxy is already loaded as a `DT_NEEDED` of libgtk-4. That means
/// nothing has to put `libepoxy.so.0` on a search path at runtime, which keeps
/// this out of the package's closure entirely.
fn gl_loader() -> impl Fn(&str) -> *const std::ffi::c_void {
    use libloading::os::unix::Library;
    let this = Library::this();
    let named = std::cell::OnceCell::new();
    move |name: &str| unsafe {
        let mut sym = Vec::with_capacity(name.len() + 8);
        sym.extend_from_slice(b"epoxy_");
        sym.extend_from_slice(name.as_bytes());
        sym.push(0);
        let deref = |l: &Library| {
            l.get::<*const std::ffi::c_void>(&sym)
                .ok()
                .map(|f| f.into_raw() as *const *const std::ffi::c_void)
                .filter(|p| !p.is_null())
                .map(|p| *p)
        };
        if let Some(p) = deref(&this) {
            return p;
        }
        let fallback = named.get_or_init(|| {
            Library::new("libepoxy.so.0")
                .or_else(|_| Library::new("libepoxy.so"))
                .ok()
        });
        fallback
            .as_ref()
            .and_then(deref)
            .unwrap_or(std::ptr::null())
    }
}

// ───────────────────────────────────────────────────────────── renderer

struct Renderer {
    gl: glow::Context,
    backdrop: Program,
    glass: Program,
    vao: glow::VertexArray,
    fbo: glow::Framebuffer,
    color: glow::Texture,
    /// Pane size in device pixels.
    size: (i32, i32),
    /// Backdrop buffer size: the pane plus `margin` on every side.
    buffer: (i32, i32),
    margin: i32,
    max_lod: f32,
    wallpaper: Option<(glow::Texture, f32, f32)>,
    wallpaper_dirty: bool,
    /// Pass 1 and its mip chain only depend on the wallpaper, the pane's
    /// placement and the pane's size — none of which move during the enter
    /// animation. Redoing them per frame would spend a full-pane draw plus a
    /// mip chain on a result that cannot have changed, so they are rebuilt on
    /// demand and the ramp costs the glass pass alone.
    backdrop_dirty: bool,
    backdrop_cover: [f32; 4],
    /// GPU time for the glass pass, from GL_TIME_ELAPSED. Double-buffered so
    /// reading last frame's result never waits on this one.
    timers: [Option<glow::Query>; 2],
    timer_slot: usize,
    timer_primed: bool,
    frames: u32,
    /// How many times pass 1 has actually run. Against `frames` this is the
    /// whole claim the cache makes: a still wallpaper should hold at a
    /// handful for the life of the lock, a live one should track frames.
    backdrop_draws: u32,
    glass_ms: f64,
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
    fn new(gl: glow::Context) -> Result<Self, String> {
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
            let timers = if stats() {
                [gl.create_query().ok(), gl.create_query().ok()]
            } else {
                [None, None]
            };
            Ok(Renderer {
                gl,
                backdrop,
                glass,
                vao,
                fbo,
                color,
                size: (0, 0),
                buffer: (0, 0),
                margin: 0,
                max_lod: 0.0,
                wallpaper: None,
                wallpaper_dirty: false,
                backdrop_dirty: true,
                backdrop_cover: [0.0; 4],
                timers,
                timer_slot: 0,
                timer_primed: false,
                frames: 0,
                backdrop_draws: 0,
                glass_ms: 0.0,
            })
        }
    }

    fn set_wallpaper(&mut self, texture: &gdk::Texture) {
        let (w, h) = (texture.width(), texture.height());
        if w <= 0 || h <= 0 {
            return;
        }
        let stride = w as usize * 4;
        let mut data = vec![0u8; stride * h as usize];
        texture.download(&mut data, stride);
        unsafe {
            let gl = &self.gl;
            if let Some((old, _, _)) = self.wallpaper.take() {
                gl.delete_texture(old);
            }
            let Ok(tex) = gl.create_texture() else { return };
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            for (k, v) in [
                (glow::TEXTURE_MIN_FILTER, glow::LINEAR),
                (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
                (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
                (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
            ] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, k, v as i32);
            }
            // GdkTexture downloads BGRA on little-endian; uploading as GL_BGRA
            // saves a swizzle pass over a 4K image on the lock path, where
            // every millisecond is one the live desktop is still on screen.
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
            self.wallpaper = Some((tex, w as f32, h as f32));
        }
        self.wallpaper_dirty = false;
        self.backdrop_dirty = true;
    }

    fn resize(&mut self, w: i32, h: i32, margin: i32) {
        if self.size == (w, h) && self.margin == margin {
            return;
        }
        self.size = (w, h);
        self.margin = margin;
        self.buffer = (w + 2 * margin, h + 2 * margin);
        let (bw, bh) = self.buffer;
        unsafe {
            let gl = &self.gl;
            gl.bind_texture(glow::TEXTURE_2D, Some(self.color));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                bw,
                bh,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            for (k, v) in [
                (glow::TEXTURE_MIN_FILTER, glow::LINEAR_MIPMAP_LINEAR),
                (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
                (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
                (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
            ] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, k, v as i32);
            }
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
        self.max_lod = (bw.max(bh) as f32).log2().floor().max(0.0);
        self.backdrop_dirty = true;
    }

    fn draw_backdrop(&mut self, cover: [f32; 4]) {
        let (bw, bh) = self.buffer;
        let margin = self.margin as f32;
        unsafe {
            let gl = &self.gl;
            gl.viewport(0, 0, bw, bh);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.fbo));
            gl.use_program(Some(self.backdrop.id));
            let p = &mut self.backdrop;
            gl.uniform_2_f32(p.loc(gl, "uResolution").as_ref(), bw as f32, bh as f32);
            gl.uniform_1_f32(p.loc(gl, "uMargin").as_ref(), margin);
            gl.uniform_4_f32(
                p.loc(gl, "uCover").as_ref(),
                cover[0],
                cover[1],
                cover[2],
                cover[3],
            );
            gl.uniform_1_i32(p.loc(gl, "uWallpaper").as_ref(), 0);
            gl.active_texture(glow::TEXTURE0);
            match self.wallpaper {
                Some((tex, _, _)) => {
                    gl.bind_texture(glow::TEXTURE_2D, Some(tex));
                    gl.uniform_1_f32(p.loc(gl, "uHasWallpaper").as_ref(), 1.0);
                }
                None => {
                    gl.bind_texture(glow::TEXTURE_2D, None);
                    gl.uniform_1_f32(p.loc(gl, "uHasWallpaper").as_ref(), 0.0);
                }
            }
            gl.draw_arrays(glow::TRIANGLES, 0, 3);

            // The mip chain doubles as the frost: sampling at a higher LOD is
            // a pre-filtered blur for one tap instead of a kawase pair.
            gl.bind_texture(glow::TEXTURE_2D, Some(self.color));
            gl.generate_mipmap(glow::TEXTURE_2D);
        }
    }

    /// Returns whether it painted glass. `false` means it deliberately left
    /// the framebuffer clear, so the picture behind the pane shows through.
    fn render(&mut self, w: i32, h: i32, shared: &Shared) -> bool {
        // Nothing to refract yet, so draw nothing at all.
        //
        // A pane is constructed and handed its texture before it has an
        // allocation, and `cover_rect` needs one — it maps the wallpaper
        // through the widget's position in the root. So there is a window,
        // one or two frames wide, where the texture is present but its
        // placement is not, and pass 1 would fill the buffer with black
        // (`uCover.z < 0.5` is "no wallpaper"). Painting that is the flash of
        // a flat dark card before the glass appears.
        //
        // Leaving the framebuffer clear instead shows the crisp picture the
        // pane sits on, which is exactly what this pane looks like at the
        // start of its own ramp. So the first painted frame and every frame
        // before it agree, and there is no step.
        let cover = shared.cover.get();
        let ready = cover[2] >= 1.0 && cover[3] >= 1.0 && shared.texture.borrow().is_some();
        if !ready {
            if stats() {
                log::info!(
                    "lock glass: skip frame, cover={:?} texture={}",
                    cover,
                    shared.texture.borrow().is_some()
                );
            }
            unsafe {
                let gl = &self.gl;
                gl.clear_color(0.0, 0.0, 0.0, 0.0);
                gl.clear(glow::COLOR_BUFFER_BIT);
            }
            return false;
        }

        let margin = (shared.tuning.margin() * shared.scale.get()).round() as i32;
        self.resize(w, h, margin);
        if self.wallpaper_dirty
            && let Some(t) = shared.texture.borrow().as_ref()
        {
            self.set_wallpaper(t);
        }

        // GTK renders the GLArea into its own framebuffer; borrow it back
        // after the offscreen pass rather than assuming it is 0.
        let target = unsafe {
            let id = self.gl.get_parameter_i32(glow::DRAW_FRAMEBUFFER_BINDING);
            std::num::NonZeroU32::new(id as u32).map(glow::NativeFramebuffer)
        };

        if cover != self.backdrop_cover {
            self.backdrop_cover = cover;
            self.backdrop_dirty = true;
        }

        unsafe {
            let gl = &self.gl;
            gl.bind_vertex_array(Some(self.vao));
            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::BLEND);
        }

        // ---- pass 1: wallpaper + scrim -> offscreen, only when it changed
        if self.backdrop_dirty {
            self.draw_backdrop(cover);
            self.backdrop_dirty = false;
            self.backdrop_draws += 1;
        }

        unsafe {
            let gl = &self.gl;
            gl.viewport(0, 0, w, h);
            gl.bind_framebuffer(glow::FRAMEBUFFER, target);
            gl.use_program(Some(self.glass.id));
        }
        // ---- pass 2: glass -> GTK's framebuffer
        let s = shared.scale.get();
        unsafe {
            let gl = &self.gl;
            let p = &mut self.glass;
            gl.uniform_1_i32(p.loc(gl, "uBackdrop").as_ref(), 0);
            gl.uniform_2_f32(p.loc(gl, "uResolution").as_ref(), w as f32, h as f32);
            gl.uniform_2_f32(
                p.loc(gl, "uBuffer").as_ref(),
                self.buffer.0 as f32,
                self.buffer.1 as f32,
            );
            gl.uniform_1_f32(p.loc(gl, "uMargin").as_ref(), self.margin as f32);
            gl.uniform_1_f32(p.loc(gl, "uMaxLod").as_ref(), self.max_lod);
            gl.uniform_1_f32(p.loc(gl, "uT").as_ref(), shared.t.get());
            gl.uniform_2_f32(p.loc(gl, "uHalf").as_ref(), w as f32 * 0.5, h as f32 * 0.5);

            let t = &shared.tuning;
            let ab = t.absorb_rgb();
            gl.uniform_1_f32(p.loc(gl, "uRadius").as_ref(), t.radius * s);
            gl.uniform_1_f32(p.loc(gl, "uBezel").as_ref(), t.bezel * s);
            gl.uniform_1_f32(p.loc(gl, "uThickness").as_ref(), t.thickness * s);
            gl.uniform_1_f32(p.loc(gl, "uIor").as_ref(), t.refraction);
            gl.uniform_1_f32(p.loc(gl, "uDispersion").as_ref(), t.dispersion);
            gl.uniform_1_i32(p.loc(gl, "uSamples").as_ref(), t.samples);
            gl.uniform_1_i32(p.loc(gl, "uProfile").as_ref(), t.surface_type);
            // This backend scatters with a mip level rather than a blur pass, so
            // the shared 0..1 becomes the level whose footprint is about
            // `frost_radius` pixels. Same number in the config, same look.
            // Roughness decides all three; `frost` remains as an explicit
            // override for the cases where the physics is not what is wanted.
            let (rough_frost, rough_shine, rough_reflect) = t.derive();
            let frost = t.frost.max(rough_frost);
            let frost_lod = if frost > 0.001 {
                (frost * t.frost_radius).max(1.0).log2()
            } else {
                0.0
            };
            gl.uniform_1_f32(p.loc(gl, "uFrost").as_ref(), frost_lod);
            gl.uniform_1_f32(p.loc(gl, "uSpecular").as_ref(), t.specular);
            gl.uniform_1_f32(p.loc(gl, "uShine").as_ref(), rough_shine);
            gl.uniform_3_f32(
                p.loc(gl, "uLightDir").as_ref(),
                LIGHT[0],
                LIGHT[1],
                LIGHT[2],
            );
            gl.uniform_1_f32(p.loc(gl, "uFresnel").as_ref(), t.reflection);
            // 0..1 of "how scattered" becomes mip levels above the frost.
            gl.uniform_1_f32(p.loc(gl, "uReflectLod").as_ref(), rough_reflect * 4.0);
            gl.uniform_1_f32(p.loc(gl, "uLensGain").as_ref(), t.lensing);
            gl.uniform_1_f32(p.loc(gl, "uEdgeLight").as_ref(), t.edge_light);
            gl.uniform_1_f32(p.loc(gl, "uNoise").as_ref(), t.noise);
            gl.uniform_1_f32(p.loc(gl, "uInnerShadow").as_ref(), t.inner_shadow);
            gl.uniform_1_f32(p.loc(gl, "uCaustic").as_ref(), t.caustic);
            gl.uniform_1_f32(p.loc(gl, "uAdaptive").as_ref(), t.adaptive);
            gl.uniform_1_f32(p.loc(gl, "uAdaptiveFrost").as_ref(), t.adaptive_frost);
            // Film interference. The shader wants a phase scale rather than
            // nanometres: one full hue cycle per half-wavelength of path
            // difference, and visible light is ~550 nm.
            gl.uniform_1_f32(
                p.loc(gl, "uIridescence").as_ref(),
                if t.film_nm > 1.0 { 1.0 } else { 0.0 },
            );
            gl.uniform_1_f32(p.loc(gl, "uFilm").as_ref(), t.film_nm / 275.0);
            // Glints are microfacets catching the light, so they belong to
            // roughness rather than to a knob of their own.
            gl.uniform_1_f32(p.loc(gl, "uSparkle").as_ref(), rough_frost * 0.35);
            gl.uniform_1_f32(p.loc(gl, "uBump").as_ref(), 0.0);
            gl.uniform_3_f32(p.loc(gl, "uAbsorb").as_ref(), ab[0], ab[1], ab[2]);
            gl.uniform_1_f32(p.loc(gl, "uAbsorbFloor").as_ref(), t.absorb_floor);
            gl.uniform_1_f32(p.loc(gl, "uHaze").as_ref(), t.haze);
            gl.uniform_1_f32(p.loc(gl, "uScrimInGlass").as_ref(), t.scrim_in_glass);
            gl.uniform_4_f32(
                p.loc(gl, "uScrim").as_ref(),
                SCRIM[0],
                SCRIM[1],
                SCRIM[2],
                SCRIM[3],
            );

            // No blending: the shader resolves its own edges against its own
            // copy of the backdrop and writes opaque pixels, so nothing here
            // depends on how GTK interprets the framebuffer's alpha.
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
            gl.bind_vertex_array(None);

            // Read the result the other slot finished last frame, so nothing
            // ever blocks waiting on the GPU.
            self.frames = self.frames.wrapping_add(1);
            if let Some(q) = self.timers[other].filter(|_| self.timer_primed)
                && gl.get_query_parameter_u32(q, glow::QUERY_RESULT_AVAILABLE) != 0
            {
                self.glass_ms = gl.get_query_parameter_u32(q, glow::QUERY_RESULT) as f64 / 1.0e6;
            }
            // Every 20 frames, not every 60: a still wallpaper stops rendering
            // once the enter ramp lands, and a cadence coarser than the ramp
            // would report nothing at all for the common case.
            if stats() && (self.frames <= 3 || self.frames.is_multiple_of(20)) {
                log::info!(
                    "lock glass: {w}x{h}, glass pass {:.3} ms GPU, \
                     {} backdrop draws in {} frames, uT={:.3}",
                    self.glass_ms,
                    self.backdrop_draws,
                    self.frames,
                    shared.t.get()
                );
            }
            self.timer_slot = other;
            self.timer_primed = true;
        }
        true
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
                let stage = if kind == glow::VERTEX_SHADER {
                    "vertex"
                } else {
                    "fragment"
                };
                return Err(format!(
                    "{stage} shader: {}",
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
