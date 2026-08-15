//! `swaypplet fp-verify <user>` — one fingerprint verification, for pam_race.
//!
//! The `sudo` and `polkit-1` PAM stacks no longer contain pam_fprintd. It
//! blocks: while it waits on the reader the stack has not reached the password
//! prompt, so on a laptop with a finger on the sensor and a password already
//! half-typed exactly one of the two can be making progress. pam_race
//! (nixos repo, pkgs/pam-race) runs face, finger and password as three racing
//! channels instead, and this is the finger one — a child process it starts
//! and kills, rather than a module that owns the stack while it waits.
//!
//! Contract, which pam_race depends on:
//!
//!   * exit 0 — the named user's finger matched, and nothing else happened.
//!   * any other exit — no usable reader for this attempt. pam_race closes
//!     the channel and stops offering a finger in its prompt.
//!   * it never exits by itself on a no-match. The first touch on this
//!     machine's Synaptics sensor reports verify-no-match often enough that
//!     giving up on one would be giving up on the feature, so the loop runs
//!     until it matches or until pam_race kills it.
//!
//! Runs as root, out of the PAM stack, which is what lets it verify a user
//! other than the caller — the same privilege pam_fprintd used to need.
//!
//! Gates: none of the engine's session/sleep gating applies here except sleep.
//! The verification is bounded by the parent, which is bounded by a human
//! standing at the machine; there is no backgrounded claim to release on a VT
//! switch, because the process does not outlive the prompt that started it.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::watch;

use super::{EngineEvent, Flow, verify_engine, watch_sleep};

const EXIT_MATCH: i32 = 0;
const EXIT_NO_READER: i32 = 1;
const EXIT_USAGE: i32 = 2;

pub fn run(mut args: impl Iterator<Item = String>) -> ! {
    let Some(user) = args.next().filter(|u| !u.is_empty()) else {
        eprintln!("usage: swaypplet fp-verify <user>");
        std::process::exit(EXIT_USAGE);
    };
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("swaypplet fp-verify: tokio runtime: {e}");
            std::process::exit(EXIT_NO_READER);
        }
    };
    std::process::exit(rt.block_on(verify(user)));
}

async fn verify(user: String) -> i32 {
    let conn = match zbus::Connection::system().await {
        Ok(c) => c,
        Err(e) => {
            log::warn!("fp-verify: system bus: {e}");
            return EXIT_NO_READER;
        }
    };

    // Release the reader before suspend like every other claim holder does. A
    // claim held across sleep wedges the synaptics device open inside fprintd
    // and costs every later claim on the machine, not just this one.
    let (sleep_tx, sleep_rx) = watch::channel(false);
    tokio::spawn(watch_sleep(conn.clone(), sleep_tx));

    // Constant target, and never inactive: this process exists only for the
    // length of one prompt in the session that raised it.
    let (_target_tx, target_rx) = watch::channel(Some(user));
    let (_active_tx, active_rx) = watch::channel(true);

    let matched = Arc::new(AtomicBool::new(false));
    let seen = matched.clone();
    let sink = move |ev: EngineEvent| match ev {
        EngineEvent::Match(_) => {
            seen.store(true, Ordering::SeqCst);
            Flow::Stop
        }
        // The reader is gone, busy past recovery, or this user has no prints.
        // The engine would keep retrying, which is right for a lock screen
        // that sits for hours and wrong for a prompt somebody is standing in
        // front of: an offer of a finger that will not be read is worse than
        // no offer. Close the channel and let pam_race say so.
        EngineEvent::Unavailable(why) => {
            log::info!("fp-verify: {why}");
            Flow::Stop
        }
        // Hints and readiness have nowhere to go — the prompt is drawn by
        // pam_race, which reads exit statuses and not a progress stream.
        EngineEvent::Ready | EngineEvent::Hint(_) | EngineEvent::Progress(_) => Flow::Continue,
    };

    verify_engine(conn, target_rx, active_rx, sleep_rx, sink).await;

    if matched.load(Ordering::SeqCst) {
        EXIT_MATCH
    } else {
        EXIT_NO_READER
    }
}
