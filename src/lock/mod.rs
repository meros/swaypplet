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

pub(crate) mod auth;
pub(crate) mod face;
pub(crate) mod fprint;
pub mod glass;
pub mod ui;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gtk4::glib;
use gtk4::prelude::*;

use crate::fp::EngineEvent;
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

    let Some(user) = auth::current_username() else {
        eprintln!("swaypplet lock: cannot determine current user");
        std::process::exit(EXIT_ERROR);
    };

    crate::theme::load_css();

    let main_loop = glib::MainLoop::new(None, false);
    let exit_code = Rc::new(RefCell::new(EXIT_ERROR));
    let surfaces = SurfaceSet::new();
    surfaces.set_current_user(&user);
    let gate = Rc::new(RefCell::new(auth::AttemptGate::default()));

    // The same user picker the greeter shows. Your own chip is inert (the
    // password field below it is already aimed at you); anyone else's hands
    // off to the host switcher, which locks this session and either jumps to
    // theirs or opens a greeter. So a lock screen and a greeter answer the
    // same gesture the same way, which is the whole point of the pair.
    if crate::switch_user::available() {
        let surfaces_cb = surfaces.clone();
        surfaces.enable_user_chips(
            &[],
            Rc::new(move |target: String| {
                if Some(&target) == auth::current_username().as_ref() {
                    surfaces_cb.focus_entry();
                    return;
                }
                surfaces_cb.set_status("Switching…", StatusKind::Info);
                // Let the handoff play, then switch. The D-Bus round trips
                // and the VT change add their own latency on top, so the
                // beat is never cut short by being early.
                let delay = surfaces_cb.begin_handoff(&target);
                glib::timeout_add_local_once(delay, move || {
                    crate::switch_user::switch_to(&target);
                });
            }),
        );
        // Off-thread: logind + fprintd round trips must never delay the
        // lock surface. The row is hidden until this lands.
        let surfaces = surfaces.clone();
        spawn_work(crate::switch_user::list, move |list| {
            let Some(list) = list.filter(|l| l.len() > 1) else {
                return;
            };
            let chips: Vec<ui::UserChip> = list
                .iter()
                .map(|u| ui::UserChip {
                    user: u.user.clone(),
                    logged_in: u.logged_in,
                    icon: u.icon.clone(),
                })
                .collect();
            surfaces.set_user_chips(&chips);
        });
    }

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
        instance.connect_monitor(move |instance, monitor| {
            let window = surfaces.build_surface(on_submit.clone(), Some(monitor));
            instance.assign_window_to_monitor(&window, monitor);
            window.present();
        });
    }

    // Fingerprint + clock start once the compositor confirms the lock.
    {
        let surfaces = surfaces.clone();
        let instance_for_fp = instance.clone();
        let instance_for_face = instance.clone();
        let user_for_face = user.clone();
        instance.connect_locked(move |_| {
            log::info!("session locked");

            // Readiness handshake with the idle supervisor: one line on
            // stdout once the compositor has confirmed the lock. The
            // supervisor gates DPMS-off and the logind sleep inhibitor on
            // this — process-alive is not session-locked (a suspend freeze
            // can land mid-startup, before the lock request is even sent).
            // Errors ignored: stdout may be a closed pipe on relaunch.
            {
                use std::io::Write;
                let mut out = std::io::stdout();
                let _ = writeln!(out, "LOCKED");
                let _ = out.flush();
            }

            let clock_surfaces = surfaces.clone();
            glib::timeout_add_seconds_local(1, move || {
                clock_surfaces.tick();
                glib::ControlFlow::Continue
            });

            // Face attempts share the fingerprint worker's event type but not
            // its pill: the UI has one biometric indicator and the finger owns
            // it, so a face match unlocks and everything else only logs.
            {
                let face_rx = face::start(user_for_face.clone());
                let surfaces = surfaces.clone();
                let instance = instance_for_face.clone();
                glib::timeout_add_local(Duration::from_millis(200), move || {
                    while let Ok(ev) = face_rx.try_recv() {
                        match ev {
                            EngineEvent::Match(_) => {
                                // Show the success state and unlock in the
                                // same pass. The animation plays *as* the
                                // session unlocks, never before it: half a
                                // second of celebration in front of the
                                // unlock is half a second of added latency,
                                // which would undo the work that made the
                                // attempt fast in the first place.
                                surfaces.show_face(true, "ok", "Recognised you");
                                surfaces.flash_success();
                                instance.unlock();
                                return glib::ControlFlow::Break;
                            }
                            EngineEvent::Progress(p) => {
                                let (state, text) = match p {
                                    crate::face::Progress::Looking => {
                                        ("looking", "Looking for you")
                                    }
                                    // Never phrased as the user's fault: the
                                    // emitter or the relay is wrong, and
                                    // telling someone to move their face
                                    // would send them after the wrong thing.
                                    crate::face::Progress::Dark => {
                                        ("dark", "Too dark to see")
                                    }
                                    crate::face::Progress::Face => {
                                        ("found", "Hold still")
                                    }
                                };
                                surfaces.show_face(true, state, text);
                            }
                            EngineEvent::Hint(hint) => {
                                log::info!("face: {hint}");
                                surfaces.show_face(true, "fail", &hint);
                                // Clear it rather than leaving a stale
                                // failure on screen between attempts.
                                let surfaces = surfaces.clone();
                                glib::timeout_add_local_once(
                                    Duration::from_millis(2200),
                                    move || surfaces.show_face(false, "", ""),
                                );
                            }
                            EngineEvent::Unavailable(why) => {
                                log::info!("face: {why}");
                                surfaces.show_face(false, "", "");
                            }
                            EngineEvent::Ready => {}
                        }
                    }
                    glib::ControlFlow::Continue
                });
            }

            let rx = fprint::start();
            let surfaces = surfaces.clone();
            let instance = instance_for_fp.clone();
            glib::timeout_add_local(Duration::from_millis(40), move || {
                while let Ok(ev) = rx.try_recv() {
                    match ev {
                        EngineEvent::Ready => {
                            surfaces.show_fp(true, "Touch fingerprint reader");
                        }
                        EngineEvent::Hint(h) => surfaces.show_fp(true, &h),
                        EngineEvent::Match(_) => {
                            surfaces.flash_success();
                            instance.unlock();
                            return glib::ControlFlow::Break;
                        }
                        EngineEvent::Unavailable(why) => {
                            log::info!("fingerprint unavailable: {why}");
                            surfaces.show_fp(false, "");
                        }
                        // fprintd reports nothing between touch and verdict,
                        // so the fingerprint engine never emits this.
                        EngineEvent::Progress(_) => {}
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
