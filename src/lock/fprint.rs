//! Fingerprint verification via fprintd, concurrent with password entry.
//!
//! Talks to fprintd directly on the system bus instead of going through PAM:
//! the PAM stack stays password-only (a pam_fprintd module in the stack would
//! double-claim the device), and typed zbus deserialization rules out the
//! null-field crashes the old patched swaylock-effects C path guarded against.
//!
//! Worker model mirrors `polkit::agent`: a std::thread runs a current-thread
//! tokio runtime, pushes `FpEvent`s over a std mpsc channel, and the GTK side
//! drains it from a periodic glib timeout. The worker never gives up while the
//! session is locked — fprintd restarts (e.g. across suspend/resume) surface
//! as claim/verify errors, and the reclaim loop picks the reader back up.

use std::sync::mpsc;
use std::time::Duration;

use zbus::export::futures_util::StreamExt;
use zbus::proxy;

#[proxy(
    interface = "net.reactivated.Fprint.Manager",
    default_service = "net.reactivated.Fprint",
    default_path = "/net/reactivated/Fprint/Manager"
)]
pub(crate) trait Manager {
    fn get_default_device(&self) -> zbus::Result<zbus::zvariant::OwnedObjectPath>;
}

#[proxy(
    interface = "net.reactivated.Fprint.Device",
    default_service = "net.reactivated.Fprint"
)]
pub(crate) trait Device {
    /// Empty username = the calling user (polkit-gated to active local sessions).
    fn claim(&self, username: &str) -> zbus::Result<()>;
    fn release(&self) -> zbus::Result<()>;
    /// Enrolled finger names for `username` (empty = the calling user). Errors
    /// with `net.reactivated.Fprint.Error.NoEnrolledPrints` when none exist.
    fn list_enrolled_fingers(&self, username: &str) -> zbus::Result<Vec<String>>;
    fn verify_start(&self, finger_name: &str) -> zbus::Result<()>;
    fn verify_stop(&self) -> zbus::Result<()>;

    #[zbus(signal)]
    fn verify_status(&self, result: String, done: bool) -> zbus::Result<()>;
}

/// Events pushed to the GTK side.
pub enum FpEvent {
    /// Device claimed and scanning — show the pill.
    Ready,
    /// Transient guidance ("not recognized", "center your finger", …).
    Hint(&'static str),
    /// Fingerprint matched: unlock.
    Match,
    /// No usable reader (right now) — hide the pill. May be followed by
    /// `Ready` again if the reader comes back.
    Unavailable(String),
}

/// Interpretation of one fprintd VerifyStatus signal.
#[derive(Debug, PartialEq, Eq)]
pub enum Verify {
    Match,
    NoMatch,
    Disconnected,
    /// done=true with an unknown/error status: stop and restart the verify.
    Error,
    /// done=false: verify session continues; optionally show a hint.
    Hint(Option<&'static str>),
}

/// True when the D-Bus error is fprintd's "this user has no enrolled prints".
pub(crate) fn is_no_enrolled_prints(e: &zbus::Error) -> bool {
    if let zbus::Error::MethodError(name, _, _) = e {
        name.as_str() == "net.reactivated.Fprint.Error.NoEnrolledPrints"
    } else {
        false
    }
}

/// Pure mapping from fprintd's (result, done) to what we do about it.
/// Statuses per fprintd docs; unknown strings fail toward restart/ignore,
/// never toward unlock.
pub fn parse_verify_status(result: &str, done: bool) -> Verify {
    match result {
        "verify-match" => Verify::Match,
        "verify-no-match" => Verify::NoMatch,
        "verify-disconnected" => Verify::Disconnected,
        "verify-retry-scan" => Verify::Hint(Some("Try again")),
        "verify-swipe-too-short" => Verify::Hint(Some("Swipe longer")),
        "verify-finger-not-centered" => Verify::Hint(Some("Center your finger")),
        "verify-remove-and-retry" => Verify::Hint(Some("Remove and try again")),
        _ if done => Verify::Error,
        _ => Verify::Hint(None),
    }
}

/// Spawn the fingerprint worker. Returns the event channel; the thread is
/// detached and exits after a match, a disconnect, or when the process dies
/// (fprintd auto-releases the claim when our bus connection drops).
pub fn start() -> mpsc::Receiver<FpEvent> {
    let (tx, rx) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("fprint".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = tx.send(FpEvent::Unavailable(format!("tokio runtime: {e}")));
                    return;
                }
            };
            rt.block_on(run(tx));
        });
    if let Err(e) = spawned {
        log::warn!("fprint worker thread failed to spawn: {e}");
    }
    rx
}

async fn run(tx: mpsc::Sender<FpEvent>) {
    macro_rules! send {
        ($ev:expr) => {
            if tx.send($ev).is_err() {
                return; // GTK side gone — process is exiting
            }
        };
    }

    let conn = match zbus::Connection::system().await {
        Ok(c) => c,
        Err(e) => {
            send!(FpEvent::Unavailable(format!("system bus: {e}")));
            return;
        }
    };
    // Gate the claim on our own session being active: with fast user
    // switching, a backgrounded locker holding the reader would starve the
    // greeter (or the active user's locker). Release while inactive,
    // reclaim on return. `watch_session_active` never returns, so `active`
    // stays selectable without a closed-channel guard.
    let (active_tx, mut active) = tokio::sync::watch::channel(true);
    tokio::spawn(crate::fp_agent::watch_session_active(
        conn.clone(),
        Some(std::process::id()),
        active_tx,
    ));
    // Gate the claim on the system being awake too: a claim held across
    // suspend wedges the synaptics device open inside fprintd (it can't
    // suspend a busy device; every later Claim fails "already open" until
    // fprintd restarts). Release before sleep, reclaim on resume. A delay
    // inhibitor held while claimed guarantees our release lands first.
    let (sleep_tx, mut sleeping) = tokio::sync::watch::channel(false);
    tokio::spawn(crate::fp_agent::watch_sleep(conn.clone(), sleep_tx));

    let manager = match ManagerProxy::new(&conn).await {
        Ok(m) => m,
        Err(e) => {
            send!(FpEvent::Unavailable(format!("fprintd manager: {e}")));
            return;
        }
    };
    let device_path = match manager.get_default_device().await {
        Ok(p) => p,
        Err(e) => {
            // Typical on machines without a reader — quiet, informative.
            send!(FpEvent::Unavailable(format!("no fingerprint device: {e}")));
            return;
        }
    };
    let device = match DeviceProxy::builder(&conn).path(device_path) {
        Ok(b) => match b.build().await {
            Ok(d) => d,
            Err(e) => {
                send!(FpEvent::Unavailable(format!("fprintd device: {e}")));
                return;
            }
        },
        Err(e) => {
            send!(FpEvent::Unavailable(format!("fprintd device path: {e}")));
            return;
        }
    };

    // Subscribe before the first VerifyStart so no status can slip past.
    let mut status_stream = match device.receive_verify_status().await {
        Ok(s) => s,
        Err(e) => {
            send!(FpEvent::Unavailable(format!("signal subscribe: {e}")));
            return;
        }
    };

    // Skip the reader entirely for an unenrolled user: no pill, no 3s claim
    // spin. fprintd resolves "" to the calling user.
    match device.list_enrolled_fingers("").await {
        Ok(fingers) if fingers.is_empty() => {
            send!(FpEvent::Unavailable("no enrolled fingerprints".into()));
            return;
        }
        Ok(_) => {}
        Err(e) if is_no_enrolled_prints(&e) => {
            send!(FpEvent::Unavailable("no enrolled fingerprints".into()));
            return;
        }
        Err(e) => {
            // Transient (device busy, fprintd restarting) — let the claim loop
            // retry rather than permanently disabling the reader.
            log::warn!("list_enrolled_fingers failed, proceeding: {e}");
        }
    }

    let mut claimed = false;
    // Sleep delay-inhibitor, held exactly while `claimed` (see the sleep
    // gate above). Dropping the fd releases it.
    let mut inhibitor: Option<zbus::zvariant::OwnedFd> = None;
    let mut reported_down = false;
    loop {
        if !claimed {
            // Park while our session is in the background (user switched
            // away — whoever is on the active VT owns the reader) or the
            // system is heading into sleep.
            loop {
                let why = if !*active.borrow_and_update() {
                    "session inactive"
                } else if *sleeping.borrow_and_update() {
                    "suspending"
                } else {
                    break;
                };
                inhibitor = None; // unclaimed and parked — don't delay sleep
                send!(FpEvent::Unavailable(why.into()));
                reported_down = true;
                tokio::select! {
                    r = active.changed() => if r.is_err() { break },
                    r = sleeping.changed() => if r.is_err() { break },
                }
                // Watcher gone on either channel (parks forever, so this
                // can't happen) — fall back to ungated.
            }
            // Inhibitor before Claim so sleep can't slip between them; kept
            // across quick claim retries (it only delays sleep, never blocks).
            if inhibitor.is_none() {
                inhibitor = crate::fp_agent::take_sleep_inhibitor(&conn, "swaypplet-lock").await;
            }
            match device.claim("").await {
                Ok(()) => {
                    claimed = true;
                    reported_down = false;
                    send!(FpEvent::Ready);
                }
                Err(e) => {
                    // Reader busy or fprintd restarting (suspend/resume).
                    // Report once, keep retrying — password stays available.
                    if !reported_down {
                        log::warn!("fprintd claim failed, retrying: {e}");
                        send!(FpEvent::Unavailable(format!("claim: {e}")));
                        reported_down = true;
                    }
                    // Wake early on a gate flip so the park loop can drop
                    // the inhibitor instead of stalling a pending suspend.
                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_secs(3)) => {}
                        _ = active.changed() => {}
                        _ = sleeping.changed() => {}
                    }
                    continue;
                }
            }
        }

        if let Err(e) = device.verify_start("any").await {
            log::warn!("VerifyStart failed, reclaiming: {e}");
            claimed = false;
            let _ = device.release().await;
            inhibitor = None;
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        }

        // One verify session: wait for statuses until a done=true result,
        // or until our session goes inactive (release the reader for the
        // now-active VT; the outer loop reclaims when we're back).
        loop {
            let next = tokio::select! {
                sig = status_stream.next() => sig,
                r = active.changed() => {
                    if r.is_err() {
                        // Watcher gone (parks forever, so this can't happen)
                        // — pin the gate open rather than spinning on a
                        // closed channel.
                        let (tx, rx) = tokio::sync::watch::channel(true);
                        std::mem::forget(tx);
                        active = rx;
                        continue;
                    }
                    if !*active.borrow() {
                        let _ = device.verify_stop().await;
                        let _ = device.release().await;
                        inhibitor = None;
                        claimed = false;
                        send!(FpEvent::Unavailable("session inactive".into()));
                        reported_down = true;
                        break;
                    }
                    continue;
                }
                r = sleeping.changed() => {
                    if r.is_err() {
                        // Same pin-open fallback as `active`, settled awake.
                        let (tx, rx) = tokio::sync::watch::channel(false);
                        std::mem::forget(tx);
                        sleeping = rx;
                        continue;
                    }
                    if *sleeping.borrow() {
                        // Release before sleep, then drop the inhibitor so
                        // suspend proceeds with the reader idle. The outer
                        // loop parks until resume and reclaims.
                        let _ = device.verify_stop().await;
                        let _ = device.release().await;
                        inhibitor = None;
                        claimed = false;
                        send!(FpEvent::Unavailable("suspending".into()));
                        reported_down = true;
                        break;
                    }
                    continue;
                }
            };
            let Some(signal) = next else {
                // Stream ended — device object vanished; reclaim from scratch.
                claimed = false;
                inhibitor = None;
                break;
            };
            let Ok(args) = signal.args() else { continue };
            match parse_verify_status(args.result(), *args.done()) {
                Verify::Match => {
                    let _ = device.verify_stop().await;
                    let _ = device.release().await;
                    send!(FpEvent::Match);
                    return;
                }
                Verify::NoMatch => {
                    let _ = device.verify_stop().await;
                    send!(FpEvent::Hint("Not recognized — try again"));
                    break; // restart verify
                }
                Verify::Disconnected => {
                    send!(FpEvent::Unavailable("reader disconnected".into()));
                    reported_down = true;
                    claimed = false;
                    inhibitor = None;
                    break; // reclaim loop; reader may come back
                }
                Verify::Error => {
                    let _ = device.verify_stop().await;
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    break; // restart verify
                }
                Verify::Hint(Some(h)) => send!(FpEvent::Hint(h)),
                Verify::Hint(None) => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_unlocks_regardless_of_done_flag() {
        assert_eq!(parse_verify_status("verify-match", true), Verify::Match);
        assert_eq!(parse_verify_status("verify-match", false), Verify::Match);
    }

    #[test]
    fn no_match_retries() {
        assert_eq!(parse_verify_status("verify-no-match", true), Verify::NoMatch);
    }

    #[test]
    fn disconnect_reported() {
        assert_eq!(
            parse_verify_status("verify-disconnected", true),
            Verify::Disconnected
        );
    }

    #[test]
    fn retry_statuses_map_to_hints() {
        for s in [
            "verify-retry-scan",
            "verify-swipe-too-short",
            "verify-finger-not-centered",
            "verify-remove-and-retry",
        ] {
            assert!(matches!(parse_verify_status(s, false), Verify::Hint(Some(_))));
        }
    }

    #[test]
    fn unknown_status_never_unlocks() {
        // Unknown terminal status → restart the verify session.
        assert_eq!(parse_verify_status("verify-unknown-error", true), Verify::Error);
        assert_eq!(parse_verify_status("something-new", true), Verify::Error);
        // Unknown non-terminal status → ignore, keep waiting.
        assert_eq!(parse_verify_status("something-new", false), Verify::Hint(None));
        // Empty string (the old null-deref bug class) → safe fallthrough.
        assert_eq!(parse_verify_status("", true), Verify::Error);
        assert_eq!(parse_verify_status("", false), Verify::Hint(None));
    }
}
