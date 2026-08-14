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
pub(crate) mod fade;
pub(crate) mod fprint;
pub mod glass;
pub mod ui;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gtk4::glib;
use gtk4::prelude::*;

use crate::fp::EngineEvent;
use crate::spawn::spawn_work;
use ui::{StatusKind, SurfaceSet};

/// How long a matched face stays on screen before the session unlocks.
///
/// Sized to the animation rather than picked: `@keyframes face-ring-ok`
/// (data/style.css) runs 550 ms and does its visible work by the overshoot at
/// 60%, so holding to roughly there is the whole gesture as far as the eye is
/// concerned, and the remaining settle is something nobody needs to watch.
/// The face channel is already polled every 200 ms, so this is under two
/// ticks and is not what governs how fast an unlock feels.
const FACE_SETTLE: Duration = Duration::from_millis(350);

/// Startup timing, in milliseconds since `run()` was entered.
///
/// The lock screen takes about a second to appear, and the cross-fade work
/// (docs/LOCK_TRANSITION_WIP.md) is gated on where that second goes. Attribute
/// it rather than guess: every stage below is a point the measurement needed.
/// Debug level, so it costs a branch in normal use.
pub(super) fn stage(what: &str) {
    thread_local! {
        static START: std::cell::OnceCell<std::time::Instant> =
            const { std::cell::OnceCell::new() };
    }
    START.with(|cell| {
        let start = cell.get_or_init(std::time::Instant::now);
        log::debug!("lock startup: {what} at {} ms", start.elapsed().as_millis());
    });
}

/// The chip row's contents, fetched during the warm-up.
///
/// Asked for at lock time this answers mid-entrance — it is two D-Bus round
/// trips, logind and fprintd — and applying it there grows the card while the
/// surface is still half transparent, which is a jump no cross-fade can hide.
/// The warm-up has nothing to wait for, so it asks there instead and the
/// chips are in the first painted frame. What can go stale between warm-up
/// and lock is the `logged_in` dot; the refresh below corrects it once the
/// entrance is over.
static WARM_USERS: std::sync::OnceLock<Vec<crate::switch_user::SwitchUser>> =
    std::sync::OnceLock::new();

/// Why this lock is happening: `idle`, `manual`, `sleep`, `switch`.
///
/// Set from the `LOCK <reason>` line when the locker was pre-warmed, and from
/// the environment when it was spawned the old way. The pre-warmed process is
/// started before anybody knows why the next lock will happen, so the reason
/// cannot ride on its environment.
static REASON: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Set when the supervisor asked for this lock without a cross-fade.
static NO_FADE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether the supervisor suppressed the cross-fade for this lock.
///
/// Some locks must not fade: before-sleep spends the deferral straight out of
/// the logind inhibitor window, and a relaunch after a crash arrives at a
/// compositor already holding an abandoned lock whose backdrop is opaque and
/// pinned, so a ramp would be ignored anyway. Saying so keeps both sides
/// telling the same story.
pub fn fade_suppressed() -> bool {
    NO_FADE.load(std::sync::atomic::Ordering::Relaxed)
        || std::env::var("SWAYPPLET_LOCK_FADE").as_deref() == Ok("0")
}

pub fn reason() -> String {
    REASON
        .get()
        .cloned()
        .or_else(|| std::env::var("SWAYPPLET_LOCK_REASON").ok())
        .unwrap_or_else(|| "unknown".into())
}

/// Pay the first-window cost now, then wait to be told to lock.
///
/// The first GTK window a process presents costs ~880 ms (measured;
/// docs/LOCK_TRANSITION_WIP.md), and it lands on whichever window is first.
/// For a locker spawned at lock time that is the lock screen itself, which is
/// why the lock screen takes about a second to appear. Presenting a throwaway
/// 1x1 transparent layer surface absorbs it, and doing that while nothing is
/// waiting makes it free.
///
/// The process then blocks on stdin until the supervisor sends `LOCK
/// <reason>`. Blocking is correct here: there is no main loop yet, nothing to
/// service, and the supervisor may take hours to ask.
fn warm_and_wait() {
    stage("warm: start");
    {
        let app = gtk4::Application::builder()
            .application_id("dev.swaypplet.lockwarm")
            .flags(gtk4::gio::ApplicationFlags::NON_UNIQUE)
            .build();
        let _ = app.register(None::<&gtk4::gio::Cancellable>);
        let config = crate::layer_shell::LayerShellConfig {
            layer: gtk4_layer_shell::Layer::Background,
            namespace: "swaypplet-lock-warm",
            keyboard_mode: gtk4_layer_shell::KeyboardMode::None,
            anchors: &[],
            margins: &[],
            exclusive: false,
            default_width: Some(1),
            default_height: Some(1),
        };
        let window = crate::layer_shell::create_layer_window(&app, &config);
        // Invisible on every axis that matters: one pixel, fully transparent,
        // on the background layer, and gone before the main loop ever runs.
        window.set_opacity(0.0);
        window.present();
        window.destroy();
    }
    stage("warm: done");

    if crate::switch_user::available()
        && let Some(list) = crate::switch_user::list()
    {
        let _ = WARM_USERS.set(list);
    }
    stage("warm: users");

    // Tell the supervisor the expensive part is behind us. It does not wait
    // for this -- a LOCK written early simply sits in the pipe -- but it makes
    // the readiness visible in a log or a manual run.
    {
        use std::io::Write;
        let mut out = std::io::stdout();
        let _ = writeln!(out, "READY");
        let _ = out.flush();
    }

    let mut line = String::new();
    match std::io::BufRead::read_line(&mut std::io::stdin().lock(), &mut line) {
        // EOF: the supervisor is gone, and a lock screen nobody is supervising
        // is worse than none. Exiting here is the same outcome as never having
        // been spawned.
        Ok(0) => {
            log::info!("lock: supervisor closed stdin before asking to lock");
            std::process::exit(EXIT_UNLOCKED);
        }
        Ok(_) => {}
        Err(e) => {
            eprintln!("swaypplet lock: reading lock command failed: {e}");
            std::process::exit(EXIT_ERROR);
        }
    }
    let mut words = line.split_whitespace();
    let _ = words.next(); // "LOCK"
    let reason = words.next().unwrap_or("").to_string();
    if words.any(|w| w == "nofade") {
        NO_FADE.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    let _ = REASON.set(if reason.is_empty() {
        "unknown".into()
    } else {
        reason
    });
    stage("lock commanded");
}

/// The warm-up's answer, in the shape the surfaces want. Empty when the
/// warm-up never ran (a cold `swaypplet lock`) or found nobody to switch to,
/// which leaves the old behaviour: no row, generic button, async refill.
fn warm_chips() -> Vec<ui::UserChip> {
    match WARM_USERS.get() {
        Some(list) if list.len() > 1 => chips_from(list),
        _ => Vec::new(),
    }
}

fn chips_from(list: &[crate::switch_user::SwitchUser]) -> Vec<ui::UserChip> {
    list.iter()
        .map(|u| ui::UserChip {
            user: u.user.clone(),
            logged_in: u.logged_in,
            icon: u.icon.clone(),
        })
        .collect()
}

const EXIT_UNLOCKED: i32 = 0;
const EXIT_ERROR: i32 = 1;
const EXIT_UNAVAILABLE: i32 = 2;

pub fn run() -> ! {
    stage("entry");
    if let Err(e) = gtk4::init() {
        eprintln!("swaypplet lock: GTK init failed: {e}");
        std::process::exit(EXIT_ERROR);
    }
    if !gtk4_session_lock::is_supported() {
        eprintln!("swaypplet lock: compositor lacks ext-session-lock-v1");
        std::process::exit(EXIT_UNAVAILABLE);
    }

    stage("gtk init");
    let instance = gtk4_session_lock::Instance::new();

    let Some(user) = auth::current_username() else {
        eprintln!("swaypplet lock: cannot determine current user");
        std::process::exit(EXIT_ERROR);
    };

    crate::theme::load_css();
    // Decoded here rather than per monitor inside the lock: that burst is
    // exactly the interval the compositor holds the live desktop on screen
    // for, and it is bounded by the fade's first-frame deadline.
    ui::preload_wallpaper();
    stage("css");

    // Pre-warmed mode: absorb the first-window cost and park until told to
    // lock. Spawned the old way (no supervisor), fall straight through.
    if std::env::var_os("SWAYPPLET_LOCK_WAIT").is_some() {
        warm_and_wait();
    }

    // After the LOCK command, never before it: creating this arms the
    // compositor's backdrop for one lock, and that arming expires.
    let fade = fade::LockFade::new();

    let main_loop = glib::MainLoop::new(None, false);
    let exit_code = Rc::new(RefCell::new(EXIT_ERROR));
    let surfaces = SurfaceSet::new();
    surfaces.set_crossfade(fade.enabled());
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
            &warm_chips(),
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
        // lock surface. Usually this only confirms what the warm-up already
        // put on screen, in which case `set_user_chips` does nothing at all.
        let surfaces = surfaces.clone();
        let fade_for_chips = fade.clone();
        spawn_work(crate::switch_user::list, move |list| {
            let Some(list) = list.filter(|l| l.len() > 1) else {
                return;
            };
            let chips = chips_from(&list);
            // Never mid-entrance: a row appearing under a half-transparent
            // surface is the one layout change the fade cannot absorb.
            fade_for_chips.on_settled(move || surfaces.set_user_chips(&chips));
        });
    }

    // One unlock, whoever gets there first. `unlock()` consumes the
    // session-lock object, so calling it twice is a protocol error that takes
    // the locker down with it. Password, fingerprint and face all reach it,
    // and the face path now waits FACE_SETTLE before it calls, which is long
    // enough for a fingerprint touch or an Enter already queued in the entry
    // to arrive and unlock the same session again.
    let unlocking = Rc::new(Cell::new(false));

    // `locked` is deferred until the entrance completes, which opens a window
    // where the password entry is live but the session-lock object is not yet
    // locked. gtk_session_lock_instance_unlock() is a silent no-op while
    // is_locked is false, so an Enter landing in that window would wedge the
    // session locked behind a UI that believes it is done.
    let locked_yet = Rc::new(Cell::new(false));
    let unlock_pending = Rc::new(Cell::new(false));

    // The one way out of the lock: ramp the surfaces down to transparent,
    // then hand the compositor unlock_and_destroy.
    let end_lock: Rc<dyn Fn()> = {
        let fade = fade.clone();
        let instance = instance.clone();
        let locked_yet = locked_yet.clone();
        let unlock_pending = unlock_pending.clone();
        Rc::new(move || {
            if !locked_yet.get() {
                // Finish arriving so the compositor can confirm the lock,
                // rather than playing out an entrance nobody wants any more.
                unlock_pending.set(true);
                fade.rush_to_opaque();
                return;
            }
            let instance = instance.clone();
            fade.exit(move || instance.unlock());
        })
    };

    // ── Password submission ───────────────────────────────────────────
    let on_submit: Rc<dyn Fn(String)> = {
        let surfaces = surfaces.clone();
        let gate = gate.clone();
        let end_lock = end_lock.clone();
        let unlocking = unlocking.clone();
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
            let end_lock = end_lock.clone();
            let unlocking = unlocking.clone();
            spawn_work(
                move || auth::pam_verify(&user, &password),
                move |result| match result {
                    Ok(()) => {
                        gate.borrow_mut().finish(true);
                        if unlocking.replace(true) {
                            return;
                        }
                        surfaces.flash_success();
                        end_lock();
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
        let first_frame_hooked = std::cell::Cell::new(false);
        let fade_cb = fade.clone();
        instance.connect_monitor(move |instance, monitor| {
            stage("monitor: build start");
            let window = surfaces.build_surface(on_submit.clone(), Some(monitor));
            stage("monitor: built");
            // Before assign_window_to_monitor: that call presents the window
            // itself, and ::realize is the only hook early enough to set the
            // multiplier before the first buffer is committed.
            fade_cb.bind(&window);
            instance.assign_window_to_monitor(&window, monitor);
            stage("monitor: assigned");
            window.present();
            stage("monitor: presented");
            // T0, the number the cross-fade is gated on: the first frame GTK
            // actually paints. Hooked on the first monitor only, and once.
            if !first_frame_hooked.replace(true)
                && let Some(clock) = window.frame_clock()
            {
                    let id = Rc::new(RefCell::new(None));
                    let id_c = id.clone();
                    *id.borrow_mut() = Some(clock.connect_after_paint(move |clock| {
                        stage("FIRST FRAME painted");
                    if let Some(id) = id_c.borrow_mut().take() {
                        clock.disconnect(id);
                    }
                }));
            }
        });
    }

    // Fingerprint + clock start once the compositor confirms the lock.
    {
        let surfaces = surfaces.clone();
        let locked_yet = locked_yet.clone();
        let unlock_pending = unlock_pending.clone();
        let end_lock_on_locked = end_lock.clone();
        let unlocking = unlocking.clone();
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

            locked_yet.set(true);
            // Authentication landed during the entrance and parked here
            // waiting for the lock to become real. Now it is.
            if unlock_pending.replace(false) {
                end_lock_on_locked();
                return;
            }
            // Same, but the unlock is already in flight: do not spin up the
            // IR camera and grab the fingerprint device milliseconds before
            // this process exits.
            if unlocking.get() {
                return;
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
                let end_lock = end_lock.clone();
                let unlocking = unlocking.clone();
                glib::timeout_add_local(Duration::from_millis(200), move || {
                    while let Ok(ev) = face_rx.try_recv() {
                        match ev {
                            EngineEvent::Match(_) => {
                                // Show the success state, then unlock a beat
                                // later. Unlocking on the same pass drew the
                                // feedback onto a surface that was already
                                // going away, so the one moment the indicator
                                // had something to say was the one moment it
                                // could not be read. The hold is the shortest
                                // thing that fixes that (see FACE_SETTLE);
                                // long enough to see, short enough that the
                                // attempt still reads as instant.
                                if unlocking.replace(true) {
                                    return glib::ControlFlow::Break;
                                }
                                surfaces.show_face(true, "ok", "Recognised you");
                                surfaces.flash_success();
                                let end_lock = end_lock.clone();
                                glib::timeout_add_local_once(FACE_SETTLE, move || {
                                    end_lock();
                                });
                                return glib::ControlFlow::Break;
                            }
                            EngineEvent::Progress(p) => {
                                surfaces.show_face(true, p.ring(), p.text());
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
            let end_lock = end_lock.clone();
            let unlocking = unlocking.clone();
            glib::timeout_add_local(Duration::from_millis(40), move || {
                while let Ok(ev) = rx.try_recv() {
                    match ev {
                        EngineEvent::Ready => {
                            surfaces.show_fp(true, "Touch fingerprint reader");
                        }
                        EngineEvent::Hint(h) => surfaces.show_fp(true, &h),
                        EngineEvent::Match(_) => {
                            if unlocking.replace(true) {
                                return glib::ControlFlow::Break;
                            }
                            surfaces.flash_success();
                            end_lock();
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

    stage("before lock()");
    if !instance.lock() {
        // ::failed also fires (and would quit the loop), but when lock()
        // reports synchronous failure the loop may not be running yet.
        eprintln!("swaypplet lock: lock request failed");
        std::process::exit(EXIT_UNAVAILABLE);
    }
    // If a window never reports a first paint (an output powered off has no
    // frame clock), start the entrance anyway rather than sitting on a
    // see-through backdrop until the compositor's own deadline fires.
    fade.arm_fallback();

    main_loop.run();

    // Drain remaining main-context work so the unlock request is on the
    // wire before the process (and its Wayland socket) goes away.
    let ctx = glib::MainContext::default();
    while ctx.iteration(false) {}

    std::process::exit(*exit_code.borrow());
}
