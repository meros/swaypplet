//! swaypplet liquid-glass demo.
//!
//! Why this exists, in one paragraph. `GskGLShader` was deprecated in GTK 4.16
//! and does not render under the current renderers, which is where the
//! "custom shaders are dead in GTK4" story usually stops. It is not the whole
//! story: `GtkGLArea` is public, supported, and hands you a raw GL context, so
//! a client can still run arbitrary GLSL over content it owns. This demo does
//! exactly that, and shows real screen-space refraction with Snell's law,
//! spectral dispersion and smooth-union merging, all live at native
//! resolution.
//!
//! The honest boundary is unchanged: on Wayland a client cannot read the
//! desktop behind its own surface, so this technique applies wherever
//! swaypplet already owns the pixels underneath. That is precisely the lock
//! screen, which paints the wallpaper itself (see `src/lock/glass.rs`). For
//! the bar and notifications the backdrop belongs to the compositor and the
//! effect has to live there instead.
//!
//! Controls are printed at startup and listed in README.md.

mod lockprobe;
mod render;
mod state;

use gtk4::gdk;
use gtk4::glib;
use gtk4::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use render::Renderer;
use state::{PRESETS, Params, State};

const CSS: &str = "
window { background: #06070b; }
.panel {
  background: alpha(#0d1017, 0.86);
  border-left: 1px solid alpha(#ffffff, 0.10);
  padding: 14px 16px 18px 16px;
}
.panel label { color: #c9d2e4; font-size: 11px; }
.panel .title { color: #ffffff; font-weight: 700; font-size: 13px; }
.panel .group { color: #7f8ca6; font-size: 10px; letter-spacing: 1px; margin-top: 10px; }
.panel .readout { color: #6ee7ff; font-family: monospace; font-size: 10px; }
.panel scale { min-height: 18px; margin: 0; }
.hud {
  background: alpha(#0d1017, 0.78);
  border: 1px solid alpha(#ffffff, 0.10);
  border-radius: 10px;
  padding: 8px 12px;
  margin: 14px;
  color: #c9d2e4;
  font-family: monospace;
  font-size: 11px;
}
";

const HELP: &str = "\
swaypplet liquid-glass demo

  drag              move a shape
  click empty       ripple (a travelling lens, not a colour effect)
  scroll on shape   resize
  1 - 6             presets: Regular, Clear, Prism, Liquid, Concave, Bar chips
  d                 cycle debug view: off / normals / height / SDF / lens gain
  t                 cycle backdrop: test card, mixed, wallpaper only
  n                 add a shape under the pointer
  r                 reset layout
  p                 pause the backdrop animation
  Tab               show/hide the parameter panel
  q / Escape        quit
";

fn main() -> glib::ExitCode {
    // --lock takes a session lock, so it runs its own main loop and never
    // reaches the GtkApplication below. It is guarded three ways; see
    // src/lockprobe.rs.
    let argv: Vec<String> = std::env::args().collect();
    if argv.iter().any(|a| a == "--lock") {
        let shot = argv
            .iter()
            .position(|a| a == "--shot")
            .and_then(|i| argv.get(i + 1).cloned());
        let wallpaper = argv
            .iter()
            .skip(1)
            .find(|a| !a.starts_with("--") && Some(*a) != shot.as_ref())
            .cloned()
            .or_else(|| std::env::var("SWAYPPLET_LOCK_WALLPAPER").ok())
            .filter(|p| !p.is_empty())
            .and_then(|p| gdk::Texture::from_filename(&p).ok());
        return glib::ExitCode::from(lockprobe::run(shot, wallpaper));
    }

    print!("{HELP}");

    let app = gtk4::Application::builder()
        .application_id("dev.swaypplet.glassdemo")
        .flags(gtk4::gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();

    // HANDLES_COMMAND_LINE so an image path can be passed without GTK trying
    // to interpret it as a file to open.
    let wallpaper: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let shot_dir: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    app.connect_command_line({
        let wallpaper = wallpaper.clone();
        let shot_dir = shot_dir.clone();
        move |app, cmd| {
            let args: Vec<String> = cmd
                .arguments()
                .iter()
                .skip(1)
                .filter_map(|s| s.to_str().map(String::from))
                .collect();
            let mut i = 0;
            while i < args.len() {
                if args[i] == "--lock" {
                    // Handled before GTK is up, in main(); see below.
                    i += 1;
                } else if args[i] == "--shot" && i + 1 < args.len() {
                    *shot_dir.borrow_mut() = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    *wallpaper.borrow_mut() = Some(args[i].clone());
                    i += 1;
                }
            }
            app.activate();
            glib::ExitCode::SUCCESS
        }
    });
    app.connect_activate({
        let wallpaper = wallpaper.clone();
        let shot_dir = shot_dir.clone();
        move |app| {
            if app.windows().is_empty() {
                build(app, wallpaper.borrow().clone(), shot_dir.borrow().clone());
            }
        }
    });

    app.run()
}

/// GL entry points, resolved out of libepoxy.
///
/// libepoxy is the dispatch layer GTK itself calls GL through. It does not
/// export `glDrawArrays`; it exports `epoxy_glDrawArrays`, a *data* symbol
/// holding a self-resolving stub's address (`nm -D` shows 3406 of them and
/// zero plain `gl*`), and the public `glDrawArrays` is a macro over it. So
/// the loader prefixes the name and dereferences what it finds. Getting that
/// deref wrong calls into `.data` and segfaults inside libepoxy with a
/// backtrace that blames the GL call.
///
/// The handle is the process's own global scope, not a fresh `dlopen`:
/// libepoxy is already loaded as a `DT_NEEDED` of libgtk-4, so its symbols
/// are there to be found, and asking for them this way means nothing has to
/// put `libepoxy.so.0` on a search path at runtime. Falls back to opening it
/// by name for the unusual case where GTK got GL some other way.
fn gl_loader() -> impl Fn(&str) -> *const std::ffi::c_void {
    use libloading::os::unix::Library;
    let lib = Library::this();
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
        if let Some(p) = deref(&lib) {
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

fn build(app: &gtk4::Application, wallpaper: Option<String>, shot_dir: Option<String>) {
    let provider = gtk4::CssProvider::new();
    provider.load_from_string(CSS);
    if let Some(display) = gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }

    let state = Rc::new(RefCell::new(State::new()));
    let renderer: Rc<RefCell<Option<Renderer>>> = Rc::new(RefCell::new(None));
    let frame_ms = Rc::new(Cell::new(0.0f64));
    let glass_ms = Rc::new(Cell::new(0.0f64));

    let area = gtk4::GLArea::new();
    area.set_has_depth_buffer(false);
    area.set_has_stencil_buffer(false);
    area.set_auto_render(true);
    area.set_hexpand(true);
    area.set_vexpand(true);
    // `#version 330 core` is desktop GL; asking for it up front turns a
    // confusing link failure into a clear "no GL context" one.
    area.set_allowed_apis(gdk::GLAPI::GL);

    let texture = wallpaper
        .or_else(|| std::env::var("SWAYPPLET_LOCK_WALLPAPER").ok())
        .filter(|p| !p.is_empty())
        .and_then(|p| match gdk::Texture::from_filename(&p) {
            Ok(t) => Some(t),
            Err(e) => {
                eprintln!("wallpaper {p}: {e}");
                None
            }
        });
    if texture.is_none() {
        eprintln!(
            "no wallpaper: pass a path as the first argument or set \
             SWAYPPLET_LOCK_WALLPAPER; the test card still works without one"
        );
    }

    // GL initialisation belongs in ::render, not ::realize. During ::realize
    // the GLArea's context exists but is not necessarily current, and epoxy's
    // dispatch stubs resolve themselves against whatever context *is* current
    // when first called — so the very first `glGetString` segfaults inside
    // libepoxy rather than returning an error. ::render is called with the
    // context current by construction.
    struct Shot {
        dir: Option<String>,
        frame: Cell<u32>,
    }
    let shot = Rc::new(Shot {
        dir: shot_dir.clone(),
        frame: Cell::new(0),
    });

    area.connect_render({
        let renderer = renderer.clone();
        let shot = shot.clone();
        let state = state.clone();
        let frame_ms = frame_ms.clone();
        let glass_ms = glass_ms.clone();
        let texture = texture.clone();
        move |area, _ctx| {
            let scale = area.scale_factor();
            let (w, h) = (area.width() * scale, area.height() * scale);

            let mut slot = renderer.borrow_mut();
            if slot.is_none() {
                let loader = gl_loader();
                let gl = unsafe { glow::Context::from_loader_function(|s| loader(s)) };
                match Renderer::new(gl) {
                    Ok(mut r) => {
                        if let Some(t) = texture.as_ref() {
                            r.set_wallpaper(t);
                        }
                        *slot = Some(r);
                    }
                    Err(e) => {
                        eprintln!("shader: {e}");
                        return glib::Propagation::Stop;
                    }
                }
                let mut st = state.borrow_mut();
                st.scale = scale as f32;
                let preset = st.preset;
                st.apply_preset(preset, w as f32, h as f32);
            }

            if let Some(r) = slot.as_mut() {
                {
                    let st = state.borrow();
                    r.render(w, h, &st);
                }
                frame_ms.set(r.frame_ms);
                glass_ms.set(r.glass_ms);

                // Capture mode: one PNG per preset, advanced on a fixed frame
                // count so the pass is deterministic and needs no compositor
                // screenshot tool.
                if let Some(dir) = shot.dir.as_ref() {
                    let n = shot.frame.get();
                    shot.frame.set(n + 1);
                    // Two warm-up frames per preset: the first allocates the
                    // offscreen buffer, the second has its mip chain.
                    if n >= 2 && (n - 2) % 3 == 2 {
                        let idx = ((n - 2) / 3) as usize;
                        if idx < PRESETS.len() {
                            if let Some((pixels, pw, ph)) = r.grab() {
                                let st = state.borrow();
                                eprintln!(
                                    "{:<10} glass pass {:>6.3} ms GPU  {}x{}  \
                                     {} spectral taps",
                                    PRESETS[idx].0, r.glass_ms, pw, ph, st.params.samples
                                );
                                drop(st);
                                save_png(dir, PRESETS[idx].0, &pixels, pw, ph);
                            }
                            let next = idx + 1;
                            if next < PRESETS.len() {
                                let mut st = state.borrow_mut();
                                st.apply_preset(next, w as f32, h as f32);
                            } else {
                                area.activate_action("window.close", None).ok();
                            }
                        }
                    }
                }
            }
            glib::Propagation::Stop
        }
    });

    // The surface can be realized before it has a size, and it can be resized
    // later; relayout on both so a preset's shapes always fit.
    area.connect_resize({
        let state = state.clone();
        move |area, w, h| {
            let scale = area.scale_factor() as f32;
            let mut st = state.borrow_mut();
            st.scale = scale;
            if st.shapes.is_empty() {
                st.relayout(w as f32 * scale, h as f32 * scale);
            }
        }
    });

    let hud = gtk4::Label::new(None);
    hud.set_css_classes(&["hud"]);
    hud.set_halign(gtk4::Align::Start);
    hud.set_valign(gtk4::Align::Start);
    hud.set_xalign(0.0);

    let (panel, sync) = build_panel(&state, &area);
    let revealer = gtk4::Revealer::new();
    revealer.set_child(Some(&panel));
    revealer.set_transition_type(gtk4::RevealerTransitionType::SlideLeft);
    revealer.set_transition_duration(180);
    revealer.set_reveal_child(true);
    revealer.set_halign(gtk4::Align::End);
    revealer.set_valign(gtk4::Align::Fill);

    let overlay = gtk4::Overlay::new();
    overlay.set_child(Some(&area));
    overlay.add_overlay(&hud);
    overlay.add_overlay(&revealer);

    let window = gtk4::ApplicationWindow::builder()
        .application(app)
        .title("swaypplet — liquid glass")
        .default_width(1500)
        .default_height(940)
        .child(&overlay)
        .build();

    wire_input(&area, &window, &state, &revealer, &sync);

    // One tick callback drives both the clock and the redraw. Time is
    // accumulated rather than read off a start instant so pausing does not
    // make the backdrop jump when it resumes.
    let last_wall = Cell::new(glib::monotonic_time());
    area.add_tick_callback({
        let state = state.clone();
        let hud = hud.clone();
        let frame_ms = frame_ms.clone();
        let glass_ms = glass_ms.clone();
        move |area, _clock| {
            let now = glib::monotonic_time();
            let dt = (now - last_wall.get()) as f64 / 1_000_000.0;
            last_wall.set(now);

            {
                let mut st = state.borrow_mut();
                if !st.paused {
                    st.time += dt.clamp(0.0, 0.25);
                }
                let t = st.time;
                st.ripples.retain(|r| t - r.born < 2.6);
            }

            let st = state.borrow();
            let ms = frame_ms.get();
            hud.set_label(&format!(
                "{}   glass pass {:.3} ms GPU   {:.1} ms/frame ({:.0} fps){}\n\
                 shapes {}   spectral taps {}   ior {:.3}   dispersion {:.4}\n\
                 {}",
                PRESETS[st.preset].0,
                glass_ms.get(),
                ms,
                if ms > 0.0 { 1000.0 / ms } else { 0.0 },
                if st.paused { "   [paused]" } else { "" },
                st.shapes.len(),
                st.params.samples,
                st.params.ior,
                st.params.dispersion,
                match st.debug {
                    1 => "debug: surface normals",
                    2 => "debug: height field",
                    3 => "debug: merged SDF",
                    4 => "debug: light concentration (red gain, blue loss)",
                    _ => "Tab for parameters, 1-6 for presets, d for debug views",
                }
            ));
            drop(st);
            area.queue_render();
            glib::ControlFlow::Continue
        }
    });

    window.present();
}

/// Write one captured frame. `GdkMemoryTexture` is the shortest path from a
/// `glReadPixels` buffer to a PNG without pulling in an image crate.
pub(crate) fn save_png(dir: &str, name: &str, pixels: &[u8], w: i32, h: i32) {
    if let Err(e) = std::fs::create_dir_all(dir) {
        eprintln!("{dir}: {e}");
        return;
    }
    let slug: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let path = format!("{dir}/{slug}.png");
    let bytes = glib::Bytes::from(pixels);
    let texture =
        gdk::MemoryTexture::new(w, h, gdk::MemoryFormat::R8g8b8a8, &bytes, w as usize * 4);
    match texture.save_to_png(&path) {
        Ok(()) => println!("wrote {path}"),
        Err(e) => eprintln!("{path}: {e}"),
    }
}

/// Pointer position in the same device-pixel space the shapes live in.
fn device(area: &gtk4::GLArea, x: f64, y: f64) -> [f32; 2] {
    let s = area.scale_factor() as f32;
    [x as f32 * s, y as f32 * s]
}

fn wire_input(
    area: &gtk4::GLArea,
    window: &gtk4::ApplicationWindow,
    state: &Rc<RefCell<State>>,
    revealer: &gtk4::Revealer,
    sync: &Rc<dyn Fn(&Params)>,
) {
    // Drag: grab a shape, or send a ripple if the press missed everything.
    let drag = gtk4::GestureDrag::new();
    drag.connect_drag_begin({
        let state = state.clone();
        let area = area.clone();
        move |_, x, y| {
            let p = device(&area, x, y);
            let mut st = state.borrow_mut();
            st.pointer = p;
            match st.pick(p) {
                Some(i) => {
                    let grab = [p[0] - st.shapes[i].pos[0], p[1] - st.shapes[i].pos[1]];
                    st.drag = Some(state::Drag { shape: i, grab });
                }
                None => {
                    st.drag = None;
                    st.push_ripple(p);
                }
            }
        }
    });
    drag.connect_drag_update({
        let state = state.clone();
        let area = area.clone();
        move |g, dx, dy| {
            let Some((sx, sy)) = g.start_point() else {
                return;
            };
            let p = device(&area, sx + dx, sy + dy);
            let mut st = state.borrow_mut();
            if let Some(d) = st.drag.as_ref() {
                let (i, grab) = (d.shape, d.grab);
                if let Some(s) = st.shapes.get_mut(i) {
                    s.pos = [p[0] - grab[0], p[1] - grab[1]];
                }
            }
        }
    });
    drag.connect_drag_end({
        let state = state.clone();
        move |_, _, _| state.borrow_mut().drag = None
    });
    area.add_controller(drag);

    // Light direction follows the pointer, which is this demo's stand-in for
    // the gyroscope Apple drives the highlight with.
    let motion = gtk4::EventControllerMotion::new();
    motion.connect_motion({
        let state = state.clone();
        let area = area.clone();
        move |_, x, y| {
            let p = device(&area, x, y);
            let s = area.scale_factor() as f32;
            let (w, h) = (area.width() as f32 * s, area.height() as f32 * s);
            if w < 1.0 || h < 1.0 {
                return;
            }
            let mut st = state.borrow_mut();
            st.pointer = p;
            st.light = [(p[0] / w - 0.5) * 1.6, (p[1] / h - 0.5) * 1.6, 0.72];
        }
    });
    area.add_controller(motion);

    // Scroll resizes the shape under the pointer. The scroll controller
    // carries no coordinates, hence `State::pointer`.
    let scroll = gtk4::EventControllerScroll::new(gtk4::EventControllerScrollFlags::BOTH_AXES);
    scroll.connect_scroll({
        let state = state.clone();
        move |_, _dx, dy| {
            let mut st = state.borrow_mut();
            let k = 1.0 - dy as f32 * 0.08;
            let p = st.pointer;
            let Some(i) = st.pick(p) else {
                return glib::Propagation::Proceed;
            };
            if let Some(s) = st.shapes.get_mut(i) {
                s.half = [
                    (s.half[0] * k).clamp(18.0, 1200.0),
                    (s.half[1] * k).clamp(18.0, 1200.0),
                ];
                s.radius = (s.radius * k).clamp(0.0, 1200.0);
            }
            glib::Propagation::Stop
        }
    });
    area.add_controller(scroll);

    let keys = gtk4::EventControllerKey::new();
    keys.connect_key_pressed({
        let state = state.clone();
        let area = area.clone();
        let revealer = revealer.clone();
        let window = window.clone();
        let sync = sync.clone();
        move |_, key, _, _| {
            let s = area.scale_factor() as f32;
            let (w, h) = (area.width() as f32 * s, area.height() as f32 * s);
            let mut st = state.borrow_mut();
            match key {
                gdk::Key::_1
                | gdk::Key::_2
                | gdk::Key::_3
                | gdk::Key::_4
                | gdk::Key::_5
                | gdk::Key::_6 => {
                    let i = key.to_unicode().and_then(|c| c.to_digit(10)).unwrap_or(1) as usize;
                    st.apply_preset(i.saturating_sub(1), w, h);
                    let params = st.params;
                    drop(st);
                    sync(&params);
                }
                gdk::Key::d => st.debug = (st.debug + 1) % 5,
                gdk::Key::t => {
                    st.test_card = match st.test_card {
                        x if x > 0.9 => 0.45,
                        x if x > 0.2 => 0.0,
                        _ => 1.0,
                    }
                }
                gdk::Key::p => st.paused = !st.paused,
                gdk::Key::n => {
                    let p = st.pointer;
                    st.add_shape(if p[0] > 0.0 { p } else { [w * 0.5, h * 0.5] });
                }
                gdk::Key::r => {
                    let i = st.preset;
                    st.apply_preset(i, w, h);
                }
                gdk::Key::Tab => revealer.set_reveal_child(!revealer.reveals_child()),
                gdk::Key::q | gdk::Key::Escape => window.close(),
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        }
    });
    window.add_controller(keys);
}

type Sync = Rc<dyn Fn(&Params)>;
/// Pushes one parameter's value back into its widget when a preset changes.
type Setter = Box<dyn Fn(&Params)>;

/// The parameter panel. Every slider writes straight into `State::params`, and
/// `sync` pushes a preset's values back out to the widgets without the
/// value-changed handlers echoing them back.
fn build_panel(state: &Rc<RefCell<State>>, area: &gtk4::GLArea) -> (gtk4::Widget, Sync) {
    let column = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    column.set_css_classes(&["panel"]);
    column.set_size_request(268, -1);

    let title = gtk4::Label::new(Some("Liquid glass"));
    title.set_css_classes(&["title"]);
    title.set_xalign(0.0);
    column.append(&title);

    let presets = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
    presets.set_homogeneous(false);
    presets.set_margin_top(8);
    // Handlers are attached after `sync` exists, below: clicking a preset has
    // to move the sliders too, or the panel lies about what is on screen.
    let preset_buttons: Vec<gtk4::Button> = PRESETS
        .iter()
        .map(|(name, _, _)| gtk4::Button::with_label(name))
        .collect();
    let grid = gtk4::FlowBox::new();
    grid.set_selection_mode(gtk4::SelectionMode::None);
    grid.set_max_children_per_line(3);
    grid.set_column_spacing(4);
    grid.set_row_spacing(4);
    for b in &preset_buttons {
        grid.append(b);
    }
    presets.append(&grid);
    column.append(&presets);

    let mut setters: Vec<Setter> = Vec::new();
    let guard = Rc::new(Cell::new(false));

    macro_rules! group {
        ($text:expr) => {{
            let l = gtk4::Label::new(Some($text));
            l.set_css_classes(&["group"]);
            l.set_xalign(0.0);
            column.append(&l);
        }};
    }

    macro_rules! slider {
        ($label:expr, $min:expr, $max:expr, $step:expr, $get:expr, $set:expr) => {{
            let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
            let name = gtk4::Label::new(Some($label));
            name.set_xalign(0.0);
            name.set_size_request(92, -1);
            let readout = gtk4::Label::new(None);
            readout.set_css_classes(&["readout"]);
            readout.set_xalign(1.0);
            readout.set_size_request(46, -1);
            let scale = gtk4::Scale::with_range(gtk4::Orientation::Horizontal, $min, $max, $step);
            scale.set_draw_value(false);
            scale.set_hexpand(true);
            row.append(&name);
            row.append(&scale);
            row.append(&readout);
            column.append(&row);

            let getter: fn(&Params) -> f64 = $get;
            scale.set_value(getter(&state.borrow().params));
            readout.set_label(&format!("{:.3}", getter(&state.borrow().params)));

            scale.connect_value_changed({
                let state = state.clone();
                let guard = guard.clone();
                let readout = readout.clone();
                move |s| {
                    let v = s.value();
                    readout.set_label(&format!("{v:.3}"));
                    if guard.get() {
                        return;
                    }
                    let setter: fn(&mut Params, f64) = $set;
                    setter(&mut state.borrow_mut().params, v);
                }
            });

            setters.push(Box::new({
                let scale = scale.clone();
                move |p: &Params| scale.set_value(getter(p))
            }));
        }};
    }

    group!("SHAPE");
    slider!("merge", 0.0, 220.0, 1.0, |p| p.merge as f64, |p, v| p
        .merge =
        v as f32);
    slider!("bezel", 4.0, 140.0, 1.0, |p| p.bezel as f64, |p, v| p
        .bezel =
        v as f32);
    slider!(
        "thickness",
        4.0,
        280.0,
        1.0,
        |p| p.thickness as f64,
        |p, v| p.thickness = v as f32
    );
    slider!("profile", 0.0, 3.0, 1.0, |p| p.profile as f64, |p, v| p
        .profile =
        v as i32);

    group!("OPTICS");
    slider!("ior", 1.0, 2.4, 0.005, |p| p.ior as f64, |p, v| p.ior =
        v as f32);
    slider!(
        "dispersion",
        0.0,
        0.09,
        0.001,
        |p| p.dispersion as f64,
        |p, v| p.dispersion = v as f32
    );
    slider!(
        "spectral taps",
        1.0,
        24.0,
        1.0,
        |p| p.samples as f64,
        |p, v| p.samples = v as i32
    );
    slider!(
        "refract scale",
        0.0,
        3.0,
        0.01,
        |p| p.refract as f64,
        |p, v| p.refract = v as f32
    );
    slider!(
        "lens gain",
        0.0,
        3.0,
        0.01,
        |p| p.lens_gain as f64,
        |p, v| p.lens_gain = v as f32
    );
    slider!("fresnel", 0.0, 1.0, 0.01, |p| p.fresnel as f64, |p, v| p
        .fresnel =
        v as f32);

    group!("MATERIAL");
    slider!("frost", 0.0, 7.0, 0.05, |p| p.frost as f64, |p, v| p
        .frost =
        v as f32);
    slider!("tint", 0.0, 0.45, 0.005, |p| p.tint[3] as f64, |p, v| p
        .tint[3] =
        v as f32);
    slider!("specular", 0.0, 1.2, 0.01, |p| p.specular as f64, |p, v| {
        p.specular = v as f32
    });
    slider!("shininess", 2.0, 160.0, 1.0, |p| p.shine as f64, |p, v| p
        .shine =
        v as f32);
    slider!(
        "edge light",
        0.0,
        1.6,
        0.01,
        |p| p.edge_light as f64,
        |p, v| p.edge_light = v as f32
    );
    slider!("shadow", 0.0, 1.0, 0.01, |p| p.shadow as f64, |p, v| p
        .shadow =
        v as f32);
    slider!("grain", 0.0, 0.08, 0.001, |p| p.noise as f64, |p, v| p
        .noise =
        v as f32);
    slider!("ripple", 0.0, 90.0, 1.0, |p| p.ripple_amp as f64, |p, v| {
        p.ripple_amp = v as f32
    });

    let sync: Sync = Rc::new({
        let guard = guard.clone();
        move |p: &Params| {
            guard.set(true);
            for s in &setters {
                s(p);
            }
            guard.set(false);
        }
    });

    for (i, b) in preset_buttons.iter().enumerate() {
        b.connect_clicked({
            let state = state.clone();
            let area = area.clone();
            let sync = sync.clone();
            move |_| {
                let s = area.scale_factor() as f32;
                let params = {
                    let mut st = state.borrow_mut();
                    st.apply_preset(i, area.width() as f32 * s, area.height() as f32 * s);
                    st.params
                };
                sync(&params);
            }
        });
    }

    let scroller = gtk4::ScrolledWindow::new();
    scroller.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    scroller.set_child(Some(&column));
    scroller.set_vexpand(true);
    (scroller.upcast(), sync)
}
