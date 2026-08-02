//! Fingerprint verification for the lock screen, concurrent with password
//! entry. A thin adapter over the shared [`crate::fp::verify_engine`].
//!
//! Talks to fprintd directly on the system bus instead of going through PAM:
//! the PAM stack stays password-only (a pam_fprintd module in the stack would
//! double-claim the device), and typed zbus deserialization rules out the
//! null-field crashes the old patched swaylock-effects C path guarded against.
//!
//! Worker model mirrors `polkit::agent`: a std::thread runs a current-thread
//! tokio runtime, pushes [`crate::fp::EngineEvent`]s over a std mpsc channel,
//! and the GTK side drains it from a periodic glib timeout. The engine verifies
//! the lock's *own* user (empty username = the caller) at a constant target and
//! unlocks on the first match; the worker never gives up while the session is
//! locked — fprintd restarts (e.g. across suspend/resume) surface as
//! claim/verify errors, and the engine's reclaim loop picks the reader back up.

use std::sync::mpsc;

use crate::fp::{EngineEvent, Flow, SessionGate, verify_engine, watch_session_active, watch_sleep};

/// Spawn the fingerprint worker. Returns the event channel; the thread is
/// detached and exits after a match (the engine's sink returns `Stop`) or when
/// the process dies (fprintd auto-releases the claim when our bus connection
/// drops).
pub fn start() -> mpsc::Receiver<EngineEvent> {
    let (tx, rx) = mpsc::channel();
    crate::spawn::spawn_tokio_thread("fprint", run(tx));
    rx
}

async fn run(tx: mpsc::Sender<EngineEvent>) {
    let conn = match zbus::Connection::system().await {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(EngineEvent::Unavailable(format!("system bus: {e}")));
            return;
        }
    };

    // Gate the claim on our own session being active: with fast user
    // switching, a backgrounded locker holding the reader would starve the
    // greeter (or the active user's locker). Release while inactive, reclaim
    // on return.
    let (active_tx, active_rx) = tokio::sync::watch::channel(true);
    tokio::spawn(watch_session_active(
        conn.clone(),
        SessionGate::Own(std::process::id()),
        active_tx,
    ));
    // Gate the claim on the system being awake too: a claim held across
    // suspend wedges the synaptics device open inside fprintd. Release before
    // sleep, reclaim on resume (the engine holds a delay-inhibitor while
    // claimed so our release lands first).
    //
    // Seed from the lock reason: a sleep-triggered locker is born *inside*
    // the PrepareForSleep window, before `watch_sleep`'s first report can
    // land, and must not claim the reader on the way into suspend. The
    // watcher's own read of logind's current state overwrites this within
    // a round trip either way.
    let born_sleeping = std::env::var("SWAYPPLET_LOCK_REASON").as_deref() == Ok("sleep");
    let (sleep_tx, sleep_rx) = tokio::sync::watch::channel(born_sleeping);
    tokio::spawn(watch_sleep(conn.clone(), sleep_tx));

    // Constant target: the lock always verifies its own user (empty = the
    // caller). Hold the sender for the engine's lifetime so the target channel
    // never closes (a closed target tells the engine to stop).
    let (_target_tx, target_rx) = tokio::sync::watch::channel(Some(String::new()));

    // Forward every engine event to the GTK side; a match unlocks, so the sink
    // returns `Stop` and the engine tears down after releasing the reader. A
    // dead channel means the process is exiting — also stop.
    let sink = move |ev: EngineEvent| {
        let unlock = matches!(ev, EngineEvent::Match(_));
        if tx.send(ev).is_err() || unlock {
            Flow::Stop
        } else {
            Flow::Continue
        }
    };

    verify_engine(conn, target_rx, active_rx, sleep_rx, sink).await;
}
