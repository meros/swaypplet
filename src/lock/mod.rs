//! `swaypplet lock` — ext-session-lock-v1 screen locker.
//!
//! Standalone process: plain `gtk4::init()` + a GLib main loop. Deliberately
//! no GApplication and no session D-Bus in the lock path, so locking works
//! even with a sick session bus. The compositor arbitrates concurrent lockers
//! at the protocol level (second locker gets `::failed`).
//!
//! Exit codes (the swayidle supervisor branches on these):
//!   0 — clean unlock: successful auth, or the compositor ended the lock
//!   2 — lock unavailable: protocol unsupported, or another client holds it
//!   1 — anything else (crash-ish). While locked, a dead client keeps the
//!       session locked (compositor renders a fallback); the supervisor
//!       relaunches us.
//!
//! Auth: PAM password (worker thread, service `swaypplet-lock`) concurrent
//! with fprintd fingerprint (see `fprint.rs`). Unlock only ever follows a
//! PAM success or a fingerprint match.

mod auth;
mod fprint;
pub mod glass;
pub mod ui;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gtk4::glib;
use gtk4::prelude::*;

use crate::spawn::spawn_work;
use ui::{StatusKind, SurfaceSet};

const EXIT_UNLOCKED: i32 = 0;
const EXIT_ERROR: i32 = 1;
const EXIT_UNAVAILABLE: i32 = 2;

pub fn run() -> ! {
    if let Err(e) = gtk4::init() {
        eprintln!("swaypplet lock: GTK init failed: {e}");
        std::process::exit(EXIT_ERROR);
    }
    if !gtk4_session_lock::is_supported() {
        eprintln!("swaypplet lock: compositor lacks ext-session-lock-v1");
        std::process::exit(EXIT_UNAVAILABLE);
    }

    let instance = gtk4_session_lock::Instance::new();

    // The per-monitor `monitor` signal shipped in gtk4-layer-shell 1.1; the
    // 0.1.2 crate predates the binding so we connect by name. Verify the
    // runtime library has it before locking, or we'd lock with no surfaces.
    if glib::subclass::SignalId::lookup("monitor", instance.type_()).is_none() {
        eprintln!("swaypplet lock: gtk4-layer-shell too old (need >=1.1 for ::monitor)");
        std::process::exit(EXIT_UNAVAILABLE);
    }

    let Some(user) = auth::current_username() else {
        eprintln!("swaypplet lock: cannot determine current user");
        std::process::exit(EXIT_ERROR);
    };

    crate::theme::load_css();

    let main_loop = glib::MainLoop::new(None, false);
    let exit_code = Rc::new(RefCell::new(EXIT_ERROR));
    let surfaces = SurfaceSet::new();
    let gate = Rc::new(RefCell::new(auth::AttemptGate::default()));

    // ── Password submission ───────────────────────────────────────────
    let on_submit: Rc<dyn Fn(String)> = {
        let surfaces = surfaces.clone();
        let gate = gate.clone();
        let instance = instance.clone();
        let user = user.clone();
        Rc::new(move |password: String| {
            if !gate.borrow_mut().try_begin(&password) {
                return;
            }
            surfaces.set_status("", StatusKind::Info);
            surfaces.set_verifying(true);
            let user = user.clone();
            let surfaces = surfaces.clone();
            let gate = gate.clone();
            let instance = instance.clone();
            spawn_work(
                move || auth::pam_verify(&user, &password),
                move |result| match result {
                    Ok(()) => {
                        gate.borrow_mut().finish(true);
                        surfaces.flash_success();
                        instance.unlock();
                    }
                    Err(e) => {
                        let failures = gate.borrow_mut().finish(false);
                        log::info!("password attempt {failures} failed: {e}");
                        surfaces.set_verifying(false);
                        surfaces.set_status(
                            &if failures == 1 {
                                "Wrong password".to_string()
                            } else {
                                format!("Wrong password ({failures} attempts)")
                            },
                            StatusKind::Error,
                        );
                        surfaces.shake();
                    }
                },
            );
        })
    };

    // ── Lock lifecycle ────────────────────────────────────────────────
    {
        let code = exit_code.clone();
        let ml = main_loop.clone();
        instance.connect_unlocked(move |_| {
            // Fires on our unlock() *and* on compositor-initiated unlock
            // (e.g. sway shutting down) — both are clean ends of the lock.
            *code.borrow_mut() = EXIT_UNLOCKED;
            let ml = ml.clone();
            // Quit from idle so this loop iteration finishes and the
            // unlock request is flushed to the compositor first.
            glib::idle_add_local_once(move || ml.quit());
        });
    }
    {
        let code = exit_code.clone();
        let ml = main_loop.clone();
        instance.connect_failed(move |_| {
            eprintln!("swaypplet lock: could not acquire session lock (already locked?)");
            *code.borrow_mut() = EXIT_UNAVAILABLE;
            let ml = ml.clone();
            glib::idle_add_local_once(move || ml.quit());
        });
    }

    // One window per monitor; fires for existing monitors at lock time and
    // for hotplugged ones while locked. The compositor destroys windows of
    // removed monitors (SurfaceSet deregisters them on ::destroy).
    {
        let surfaces = surfaces.clone();
        let on_submit = on_submit.clone();
        instance.connect_local("monitor", false, move |values| {
            let instance = values[0]
                .get::<gtk4_session_lock::Instance>()
                .expect("monitor signal: sender is the Instance");
            let monitor = values[1]
                .get::<gdk4::Monitor>()
                .expect("monitor signal: argument is a GdkMonitor");
            let window = surfaces.build_surface(on_submit.clone());
            instance.assign_window_to_monitor(&window, &monitor);
            window.present();
            None
        });
    }

    // Fingerprint + clock start once the compositor confirms the lock.
    {
        let surfaces = surfaces.clone();
        let instance_for_fp = instance.clone();
        instance.connect_locked(move |_| {
            log::info!("session locked");

            let clock_surfaces = surfaces.clone();
            glib::timeout_add_seconds_local(1, move || {
                clock_surfaces.tick();
                glib::ControlFlow::Continue
            });

            let rx = fprint::start();
            let surfaces = surfaces.clone();
            let instance = instance_for_fp.clone();
            glib::timeout_add_local(Duration::from_millis(40), move || {
                while let Ok(ev) = rx.try_recv() {
                    match ev {
                        fprint::FpEvent::Ready => {
                            surfaces.show_fp(true, "Touch fingerprint reader");
                        }
                        fprint::FpEvent::Hint(h) => surfaces.show_fp(true, h),
                        fprint::FpEvent::Match => {
                            surfaces.flash_success();
                            instance.unlock();
                            return glib::ControlFlow::Break;
                        }
                        fprint::FpEvent::Unavailable(why) => {
                            log::info!("fingerprint unavailable: {why}");
                            surfaces.show_fp(false, "");
                        }
                    }
                }
                glib::ControlFlow::Continue
            });
        });
    }

    if !instance.lock() {
        // ::failed also fires (and would quit the loop), but when lock()
        // reports synchronous failure the loop may not be running yet.
        eprintln!("swaypplet lock: lock request failed");
        std::process::exit(EXIT_UNAVAILABLE);
    }

    main_loop.run();

    // Drain remaining main-context work so the unlock request is on the
    // wire before the process (and its Wayland socket) goes away.
    let ctx = glib::MainContext::default();
    while ctx.iteration(false) {}

    std::process::exit(*exit_code.borrow());
}
