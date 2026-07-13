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

    let mut claimed = false;
    let mut reported_down = false;
    loop {
        if !claimed {
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
                    tokio::time::sleep(Duration::from_secs(3)).await;
                    continue;
                }
            }
        }

        if let Err(e) = device.verify_start("any").await {
            log::warn!("VerifyStart failed, reclaiming: {e}");
            claimed = false;
            let _ = device.release().await;
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        }

        // One verify session: wait for statuses until a done=true result.
        loop {
            let Some(signal) = status_stream.next().await else {
                // Stream ended — device object vanished; reclaim from scratch.
                claimed = false;
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
