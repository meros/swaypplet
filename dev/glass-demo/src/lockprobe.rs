//! Does a `GtkGLArea` render on an `ext-session-lock-v1` surface?
//!
//! `swaypplet --preview lock` renders the lock UI in a plain toplevel, which
//! is the right way to iterate on styling but proves nothing about the actual
//! lock surface: a session-lock surface is created through a different
//! protocol path, and whether GTK will hand it a GL context is the one thing
//! standing between this demo and `src/lock/glass.rs`. So this takes a real
//! lock, puts a `GtkGLArea` on it, reads the framebuffer back, and unlocks.
//!
//! # Safety
//!
//! Taking a session lock is not a reversible mistake: `ext-session-lock-v1`
//! is designed so that a locker which dies without unlocking leaves the
//! session locked, which is exactly what you want from a screen locker and
//! exactly what you do not want from a probe. Three independent guards, all
//! of which must pass:
//!
//! 1. `GLASS_DEMO_LOCK_PROBE=1` must be set. Only `lockprobe.sh` sets it.
//! 2. `WAYLAND_DISPLAY` must differ from `GLASS_DEMO_HOST_DISPLAY`, which
//!    `lockprobe.sh` fills in with the *outer* session's socket before it
//!    starts the nested compositor. Running against the host is refused.
//! 3. A `glib` timeout that unlocks and quits fires unconditionally, armed
//!    before `lock()` is called rather than after.
//!
//! It never touches PAM, never reads input, and never keeps the lock past its
//! own timeout.

use gtk4::prelude::*;
use gtk4::{gdk, glib};
use std::cell::RefCell;
use std::rc::Rc;

use crate::render::Renderer;
use crate::state::State;

/// Hard ceiling on how long the lock is held, whatever else happens.
const HOLD_MS: u32 = 3500;

pub fn refuse_reason() -> Option<String> {
    if std::env::var("GLASS_DEMO_LOCK_PROBE").as_deref() != Ok("1") {
        return Some("GLASS_DEMO_LOCK_PROBE=1 not set; run dev/glass-demo/lockprobe.sh".into());
    }
    let here = std::env::var("WAYLAND_DISPLAY").unwrap_or_default();
    let host = std::env::var("GLASS_DEMO_HOST_DISPLAY").unwrap_or_default();
    if host.is_empty() {
        return Some("GLASS_DEMO_HOST_DISPLAY unset; refusing to guess".into());
    }
    if here == host {
        return Some(format!(
            "WAYLAND_DISPLAY={here} is the host session; this probe only runs nested"
        ));
    }
    None
}

/// Returns the process exit code. Prints a one-line verdict either way.
pub fn run(shot: Option<String>, wallpaper: Option<gdk::Texture>) -> u8 {
    if let Some(why) = refuse_reason() {
        eprintln!("lock probe refused: {why}");
        return 2;
    }
    if let Err(e) = gtk4::init() {
        eprintln!("lock probe: GTK init failed: {e}");
        return 1;
    }
    if !gtk4_session_lock::is_supported() {
        eprintln!("lock probe: compositor lacks ext-session-lock-v1");
        return 3;
    }

    let instance = gtk4_session_lock::Instance::new();
    let main_loop = glib::MainLoop::new(None, false);
    let verdict = Rc::new(RefCell::new(String::from(
        "no render callback ran: GL never started on the lock surface",
    )));
    let ok = Rc::new(std::cell::Cell::new(false));
    // Frames drawn on the lock surface. The verdict is taken after a few:
    // the first allocates the offscreen buffer, the second builds its mip
    // chain, and the GPU timer needs one more before its query is primed.
    let frames = Rc::new(std::cell::Cell::new(0u32));

    // Guard 3, armed before the lock is taken so there is no window in which
    // a failure below can leave the session locked.
    {
        let instance = instance.clone();
        let main_loop = main_loop.clone();
        glib::timeout_add_local_once(
            std::time::Duration::from_millis(HOLD_MS as u64),
            move || {
                instance.unlock();
                main_loop.quit();
            },
        );
    }

    let state = Rc::new(RefCell::new(State::new()));
    let renderer: Rc<RefCell<Option<Renderer>>> = Rc::new(RefCell::new(None));

    instance.connect_monitor({
        let state = state.clone();
        let renderer = renderer.clone();
        let verdict = verdict.clone();
        let ok = ok.clone();
        let frames = frames.clone();
        let wallpaper = wallpaper.clone();
        let shot = shot.clone();
        move |instance, monitor| {
            let window = gtk4::Window::new();

            let area = gtk4::GLArea::new();
            area.set_has_depth_buffer(false);
            area.set_has_stencil_buffer(false);
            area.set_hexpand(true);
            area.set_vexpand(true);
            area.set_allowed_apis(gdk::GLAPI::GL);

            // A stand-in for the lock card: a real GTK widget over the glass,
            // because that is the arrangement the port would use and it is
            // worth knowing that it composites.
            let card = gtk4::Label::new(Some("swaypplet"));
            card.add_css_class("title-1");
            card.set_halign(gtk4::Align::Center);
            card.set_valign(gtk4::Align::Center);

            let overlay = gtk4::Overlay::new();
            overlay.set_child(Some(&area));
            overlay.add_overlay(&card);
            window.set_child(Some(&overlay));

            area.connect_render({
                let state = state.clone();
                let renderer = renderer.clone();
                let verdict = verdict.clone();
                let ok = ok.clone();
                let frames = frames.clone();
                let wallpaper = wallpaper.clone();
                let shot = shot.clone();
                move |area, _ctx| {
                    let scale = area.scale_factor();
                    let (w, h) = (area.width() * scale, area.height() * scale);
                    let mut slot = renderer.borrow_mut();
                    if slot.is_none() {
                        let loader = crate::gl_loader();
                        let gl = unsafe { glow::Context::from_loader_function(|s| loader(s)) };
                        match Renderer::new(gl) {
                            Ok(mut r) => {
                                if let Some(t) = wallpaper.as_ref() {
                                    r.set_wallpaper(t);
                                }
                                *slot = Some(r);
                            }
                            Err(e) => {
                                *verdict.borrow_mut() =
                                    format!("GL context came up but the shader failed: {e}");
                                return glib::Propagation::Stop;
                            }
                        }
                        let mut st = state.borrow_mut();
                        st.scale = scale as f32;
                        st.apply_preset(0, w as f32, h as f32);
                    }
                    if let Some(r) = slot.as_mut() {
                        {
                            let st = state.borrow();
                            r.render(w, h, &st);
                        }
                        let n = frames.get() + 1;
                        frames.set(n);
                        if n >= 4 && !ok.replace(true) {
                            *verdict.borrow_mut() = format!(
                                "GtkGLArea renders on an ext-session-lock-v1 surface: \
                                 {w}x{h}, glass pass {:.3} ms GPU, {n} frames drawn",
                                r.glass_ms
                            );
                            if let Some(dir) = shot.as_ref()
                                && let Some((pixels, pw, ph)) = r.grab()
                            {
                                crate::save_png(dir, "lock-surface", &pixels, pw, ph);
                            }
                        }
                    }
                    glib::Propagation::Stop
                }
            });

            // The lock surface is static, so redraw on a timer rather than a
            // tick callback: a lock screen that spins the GPU at 60 Hz is a
            // battery bug, and this probe should not model one.
            glib::timeout_add_local(std::time::Duration::from_millis(120), {
                let area = area.clone();
                move || {
                    area.queue_render();
                    glib::ControlFlow::Continue
                }
            });

            instance.assign_window_to_monitor(&window, monitor);
            window.present();
        }
    });

    instance.connect_failed({
        let verdict = verdict.clone();
        let main_loop = main_loop.clone();
        move |_| {
            *verdict.borrow_mut() = "compositor refused the lock".into();
            main_loop.quit();
        }
    });

    if !instance.lock() {
        eprintln!("lock probe: lock() refused");
        return 4;
    }
    main_loop.run();
    instance.unlock();

    println!("{}", verdict.borrow());
    if ok.get() { 0 } else { 1 }
}
