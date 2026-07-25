//! Fingerprint core: fprintd D-Bus proxies, logind session/sleep gating, and
//! the shared claim/verify state machine (`verify_engine`) that both the lock
//! screen and the greeter's `fp-agent` drive.
//!
//! Everything fingerprint-shaped that isn't UI lives here so the two consumers
//! stay in lock-step:
//!
//! * `verify_engine` owns the whole lifecycle — resolve the device, gate on
//!   the session being active and the machine awake, claim as the target user,
//!   verify, and release on every edge (retarget, deactivate, suspend,
//!   fprintd restart, reader disconnect). It reports progress through a caller
//!   `sink` and defers the terminal side effect (unlock vs. mint-a-token) to
//!   that sink's [`Flow`] result.
//! * `lock::fprint` runs it with a fixed target (own user) and a terminal
//!   match → unlock.
//! * `fp::agent` runs it per greeter connection with a live target channel and
//!   a non-terminal match → mint a token, keep verifying after a retarget.
//!
//! The gating rule is the same for both: at most one holder claims the reader,
//! because each verify loop is armed only while *its* logind session is active
//! (a backgrounded greeter/locker releases the reader for whoever is on the
//! active VT) and the system isn't heading into sleep (a claim held across
//! suspend wedges the synaptics device open inside fprintd — every later Claim
//! fails "already open" until fprintd restarts).

pub mod agent;

use std::time::Duration;

use zbus::export::futures_util::StreamExt;
use zbus::proxy;

// --- fprintd proxies -------------------------------------------------------

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

// --- logind session-active + sleep watching --------------------------------

#[proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
pub(crate) trait LogindManager {
    fn get_session_by_pid(&self, pid: u32) -> zbus::Result<zbus::zvariant::OwnedObjectPath>;
    /// `"auto"` resolves the caller's own session; a real id resolves that
    /// one. Works for user@.service children, where GetSessionByPID fails
    /// because the PID isn't in any session cgroup.
    fn get_session(&self, id: &str) -> zbus::Result<zbus::zvariant::OwnedObjectPath>;
    /// Delay inhibitor: sleep waits (bounded by logind's InhibitDelayMaxSec)
    /// until every holder closes the returned fd.
    fn inhibit(
        &self,
        what: &str,
        who: &str,
        why: &str,
        mode: &str,
    ) -> zbus::Result<zbus::zvariant::OwnedFd>;
    #[zbus(signal)]
    fn prepare_for_sleep(&self, start: bool) -> zbus::Result<()>;
}

#[proxy(
    interface = "org.freedesktop.login1.Session",
    default_service = "org.freedesktop.login1"
)]
pub(crate) trait LogindSession {
    #[zbus(property)]
    fn active(&self) -> zbus::Result<bool>;
}

/// Which logind session a verify loop gates on.
#[derive(Clone, Copy)]
pub enum SessionGate {
    /// The caller's *own* session (GetSession("auto"), then the pid as a
    /// fallback). Used by the lock, which runs inside the user's session.
    Own(u32),
    /// A specific *client's* session by pid — never "auto". Used by the
    /// fp-agent: it's a root daemon with no session of its own, so it must
    /// gate each greeter on that greeter's session, not the daemon's.
    Client(u32),
}

const RESOLVE_ATTEMPTS: u32 = 5;
const RESOLVE_BACKOFF: Duration = Duration::from_millis(500);

/// Push `gate`'s logind-session Active state into `tx`, forever. Never
/// returns: when the session can't be resolved (dev box, session gone) it
/// settles on `true` and parks, so receivers can rely on the channel staying
/// open and select on `changed()` without a busy-loop guard.
pub async fn watch_session_active(
    conn: zbus::Connection,
    gate: SessionGate,
    tx: tokio::sync::watch::Sender<bool>,
) {
    // Resolve once, with bounded retry: session registration can race the
    // locker/agent starting (fast VT handoff, cold boot). Giving up too early
    // is what left an inactive locker holding the fingerprint reader.
    let mut session = None;
    for attempt in 0..RESOLVE_ATTEMPTS {
        match resolve_session(&conn, gate).await {
            Ok(s) => {
                session = Some(s);
                break;
            }
            Err(e) => {
                let last = attempt + 1 == RESOLVE_ATTEMPTS;
                log::log!(
                    if last {
                        log::Level::Warn
                    } else {
                        log::Level::Debug
                    },
                    "session resolve attempt {}/{RESOLVE_ATTEMPTS} failed ({e})",
                    attempt + 1
                );
                if !last {
                    tokio::time::sleep(RESOLVE_BACKOFF).await;
                }
            }
        }
    }
    match session {
        Some(session) => {
            let mut stream = session.receive_active_changed().await;
            let _ = tx.send(session.active().await.unwrap_or(true));
            while let Some(change) = stream.next().await {
                if let Ok(v) = change.get().await {
                    let _ = tx.send(v);
                }
            }
            // Session object vanished (logged out) — hold the last value.
        }
        None => {
            log::warn!("no logind session to gate on after retries; treating as active");
            let _ = tx.send(true);
        }
    }
    std::future::pending::<()>().await;
}

/// Push logind's PrepareForSleep state into `tx`, forever: `true` from
/// PrepareForSleep(start) until the matching resume. Claim holders gate on it
/// and release the reader before sleep — a claim held across suspend leaves
/// the synaptics device wedged open inside fprintd (every later Claim fails
/// with "Device ... is already open" until fprintd restarts). Like
/// `watch_session_active`, never returns: on setup failure it settles on
/// `false` (awake) and parks, so receivers can select on `changed()` freely.
pub async fn watch_sleep(conn: zbus::Connection, tx: tokio::sync::watch::Sender<bool>) {
    let stream = async {
        let manager = LogindManagerProxy::new(&conn).await?;
        manager.receive_prepare_for_sleep().await
    }
    .await;
    match stream {
        Ok(mut stream) => {
            while let Some(sig) = stream.next().await {
                if let Ok(args) = sig.args() {
                    let _ = tx.send(*args.start());
                }
            }
            // Stream ended (system bus gone) — hold the last value.
        }
        Err(e) => {
            log::warn!("sleep watch unavailable, claims will span suspend: {e}");
            let _ = tx.send(false);
        }
    }
    std::future::pending::<()>().await;
}

/// Take a sleep delay-inhibitor; dropping the fd releases it. Held exactly
/// while a fingerprint claim is held, so the PrepareForSleep handler gets to
/// VerifyStop+Release before logind lets the machine suspend. `None` (logind
/// too old, dev container) just means sleep won't wait for us.
pub async fn take_sleep_inhibitor(
    conn: &zbus::Connection,
    who: &str,
) -> Option<zbus::zvariant::OwnedFd> {
    let take = async {
        LogindManagerProxy::new(conn)
            .await?
            .inhibit(
                "sleep",
                who,
                "release fingerprint reader before sleep",
                "delay",
            )
            .await
    };
    match take.await {
        Ok(fd) => Some(fd),
        Err(e) => {
            log::warn!("sleep delay-inhibitor unavailable: {e}");
            None
        }
    }
}

/// Resolve the logind session proxy for `gate` (see [`SessionGate`]).
async fn resolve_session(
    conn: &zbus::Connection,
    gate: SessionGate,
) -> Result<LogindSessionProxy<'static>, String> {
    let manager = LogindManagerProxy::new(conn)
        .await
        .map_err(|e| e.to_string())?;
    let path = match gate {
        SessionGate::Own(pid) => match manager.get_session("auto").await {
            Ok(p) => p,
            Err(auto_err) => manager
                .get_session_by_pid(pid)
                .await
                .map_err(|e| format!("GetSession(auto): {auto_err}; GetSessionByPID: {e}"))?,
        },
        SessionGate::Client(pid) => manager
            .get_session_by_pid(pid)
            .await
            .map_err(|e| e.to_string())?,
    };
    LogindSessionProxy::builder(conn)
        .path(path)
        .map_err(|e| e.to_string())?
        .build()
        .await
        .map_err(|e| e.to_string())
}

async fn device_proxy(conn: &zbus::Connection) -> Result<DeviceProxy<'static>, String> {
    let manager = ManagerProxy::new(conn).await.map_err(|e| e.to_string())?;
    let path = manager
        .get_default_device()
        .await
        .map_err(|e| e.to_string())?;
    DeviceProxy::builder(conn)
        .path(path)
        .map_err(|e| e.to_string())?
        .build()
        .await
        .map_err(|e| e.to_string())
}

// --- shared verify engine --------------------------------------------------

/// Progress reported by [`verify_engine`] to its caller's sink.
pub enum EngineEvent {
    /// Device claimed and scanning — show the pill.
    Ready,
    /// Transient guidance ("not recognized", "center your finger", …).
    Hint(String),
    /// No usable reader right now — hide the pill. `Ready` may follow later.
    Unavailable(String),
    /// `user`'s finger matched. The sink performs the terminal action (unlock
    /// / mint a token) and returns [`Flow`] to say whether the engine keeps
    /// running.
    Match(String),
}

/// A sink's answer to the engine after handling an event.
#[derive(PartialEq, Eq)]
pub enum Flow {
    /// Keep the engine running (verify again after a retarget on a match).
    Continue,
    /// Tear the engine down and return (unlock done, or the caller's channel
    /// closed).
    Stop,
}

/// Restart a silent verify session after this long with no status signal —
/// covers a driver that accepted VerifyStart but wedged without ever
/// reporting. A healthy idle reader just re-arms; a match still lands.
const VERIFY_SILENCE_TIMEOUT: Duration = Duration::from_secs(40);

/// Floor between VerifyStart calls within one claim. A human retry is seconds
/// apart, so this is invisible in normal use, and it caps any restart feedback
/// loop at a trickle instead of a spin. Backstop for the cancel echo below.
const VERIFY_RESTART_FLOOR: Duration = Duration::from_millis(400);

/// Warn once when one claim has re-armed this many times — a healthy lock
/// screen re-arms once per [`VERIFY_SILENCE_TIMEOUT`], so anything near this
/// is a loop that wants looking at.
const RESTART_WARN_AT: u32 = 20;

/// Claim retry cadence. The fast one covers the normal handover, where the
/// previous holder is a few hundred milliseconds from releasing. After
/// `CLAIM_QUIET_ATTEMPTS` it steps back: every attempt is a full polkit
/// round trip, and hammering one twice a second for the length of a lock
/// screen buys nothing a two-second poll doesn't.
const CLAIM_RETRY: Duration = Duration::from_millis(500);
const CLAIM_RETRY_SLOW: Duration = Duration::from_secs(2);
const CLAIM_QUIET_ATTEMPTS: u32 = 6;

/// Whether a terminal status is the echo of a verify session we cancelled
/// ourselves, and consume the debt if so.
///
/// fprintd completes a *running* verify that we stop (VerifyStop on an
/// in-flight session) with a terminal `verify-no-match`: the synaptics SSM
/// fails with "Device reported cancellation of operation" and
/// `report_verify_status` fires anyway. Reading that echo as a failed finger
/// restarts the verify, and the restart's own cancellation emits the next
/// echo — a feedback loop that ran ~100 stop/start pairs a second, desynced
/// the sensor's sequence numbers and left fprintd wedged (2026-07-25 18:23:
/// 22 s of CPU in two minutes, then "Device 06cb:019d is already open" on
/// every later claim).
///
/// So the engine counts the terminal statuses it is owed by sessions it
/// killed and absorbs exactly that many. A `Match` is never an echo — a
/// cancellation can only complete as no-match or error — so a finger that
/// landed just before the stop still unlocks.
fn absorb_echo(owed: &mut u32, verdict: &Verify) -> bool {
    if *owed > 0 && matches!(verdict, Verify::NoMatch | Verify::Error) {
        *owed -= 1;
        true
    } else {
        false
    }
}

/// Whether `user` has usable enrolled prints, distinguishing an authoritative
/// "none" from a transient read failure.
enum Enrollment {
    Enrolled,
    None,
    /// The query itself failed (device busy, fprintd restarting) — don't
    /// conclude anything; let the claim loop retry.
    Unknown,
}

async fn enrollment(device: &DeviceProxy<'_>, user: &str) -> Enrollment {
    match device.list_enrolled_fingers(user).await {
        Ok(fingers) if fingers.is_empty() => Enrollment::None,
        Ok(_) => Enrollment::Enrolled,
        Err(e) if is_no_enrolled_prints(&e) => Enrollment::None,
        Err(e) => {
            log::warn!("list_enrolled_fingers({user}): {e}");
            Enrollment::Unknown
        }
    }
}

/// The one claim/verify state machine. Runs until the caller's sink returns
/// [`Flow::Stop`] or the `target` sender is dropped.
///
/// * `target` — the user to verify right now, or `None` to stand down. The
///   lock holds it at a constant own-user; the agent drives it live.
/// * `active` / `sleeping` — the logind gates; the loop only claims while
///   active and awake, releasing on every edge.
/// * `sink` — receives [`EngineEvent`]s. For a non-match event it returns
///   `Stop` only if its own channel died; for a `Match` it does the terminal
///   work and returns `Stop` (lock) or `Continue` (agent).
pub async fn verify_engine(
    conn: zbus::Connection,
    mut target: tokio::sync::watch::Receiver<Option<String>>,
    mut active: tokio::sync::watch::Receiver<bool>,
    mut sleeping: tokio::sync::watch::Receiver<bool>,
    mut sink: impl FnMut(EngineEvent) -> Flow,
) {
    macro_rules! emit {
        ($ev:expr) => {
            if sink($ev) == Flow::Stop {
                return;
            }
        };
    }

    // Resolve the device lazily and retry per target change, so a machine
    // where fprintd isn't up yet doesn't wedge the connection.
    let device = loop {
        if target.borrow_and_update().is_some() {
            match device_proxy(&conn).await {
                Ok(d) => break d,
                Err(e) => emit!(EngineEvent::Unavailable(format!(
                    "no fingerprint device: {e}"
                ))),
            }
        }
        if target.changed().await.is_err() {
            return;
        }
    };
    // Subscribe before the first VerifyStart so no status can slip past.
    let mut status = match device.receive_verify_status().await {
        Ok(s) => s,
        Err(e) => {
            emit!(EngineEvent::Unavailable(format!("signal subscribe: {e}")));
            return;
        }
    };

    'outer: loop {
        // Park until there is a target, the session is active, and the system
        // isn't heading into sleep.
        loop {
            let armed = target.borrow_and_update().is_some()
                && *active.borrow_and_update()
                && !*sleeping.borrow_and_update();
            if armed {
                break;
            }
            tokio::select! {
                r = target.changed() => if r.is_err() { return },
                _ = active.changed() => {}
                _ = sleeping.changed() => {}
            }
        }
        let Some(user) = target.borrow().clone() else {
            continue 'outer;
        };

        // Enrollment gate: skip the claim/verify spin for a user with no
        // prints, but only on an *authoritative* answer — a failed read falls
        // through and lets the claim loop retry.
        match enrollment(&device, &user).await {
            Enrollment::Enrolled | Enrollment::Unknown => {}
            Enrollment::None => {
                emit!(EngineEvent::Unavailable(unenrolled_msg(&user)));
                if wait_for_retarget(&mut target, &user).await.is_err() {
                    return;
                }
                continue 'outer;
            }
        }

        // Claim as the target user; retry while the reader is busy (e.g. a
        // backgrounded locker still releasing it). The sleep delay-inhibitor
        // lives for this `'outer` iteration — held from just before Claim so
        // suspend can't slip in, and dropped by `continue 'outer` (scope exit)
        // after the release calls below, so suspend proceeds with the reader
        // idle.
        let mut inhibitor: Option<zbus::zvariant::OwnedFd> = None;
        let mut attempts: u32 = 0;
        loop {
            if !targeted(&target, &user) || !*active.borrow() || *sleeping.borrow() {
                continue 'outer;
            }
            if inhibitor.is_none() {
                inhibitor = take_sleep_inhibitor(&conn, "swaypplet-fp").await;
            }
            match device.claim(&user).await {
                Ok(()) => break,
                Err(e) => {
                    attempts += 1;
                    // A couple of misses is the normal handover shape (the
                    // other side is still releasing). Past that the reader is
                    // held by something that isn't going to let go on its own,
                    // and the only way anyone finds out is this line — so say
                    // it at warn, once, with fprintd's own words.
                    if attempts == CLAIM_QUIET_ATTEMPTS {
                        log::warn!("claim({user}) still failing after {attempts}: {e}");
                    } else {
                        log::debug!("claim({user}) attempt {attempts} failed: {e}");
                    }
                    let backoff = if attempts < CLAIM_QUIET_ATTEMPTS {
                        CLAIM_RETRY
                    } else {
                        CLAIM_RETRY_SLOW
                    };
                    tokio::select! {
                        _ = tokio::time::sleep(backoff) => {}
                        r = target.changed() => if r.is_err() { return },
                        _ = active.changed() => {}
                        _ = sleeping.changed() => {}
                    }
                }
            }
        }

        // Verify sessions until a match, a retarget, or a gate flip.
        // `armed` holds back Ready until a verify is actually running: a claim
        // alone is not a reader you can touch, and promising one that isn't
        // scanning is worse than promising nothing.
        //
        // `echoes` counts terminal statuses fprintd owes us for verify
        // sessions we cancelled ourselves (see `absorb_echo`); it resets with
        // the claim, since a fresh claim inherits nothing from the old one.
        let mut armed = false;
        let mut echoes: u32 = 0;
        let mut restarts: u32 = 0;
        let mut last_start: Option<std::time::Instant> = None;
        'verify: loop {
            if !targeted(&target, &user) || !*active.borrow() || *sleeping.borrow() {
                let _ = device.release().await;
                continue 'outer;
            }
            // Never re-arm faster than the floor, whatever drove us here.
            if let Some(prev) = last_start {
                let since = prev.elapsed();
                if since < VERIFY_RESTART_FLOOR {
                    tokio::time::sleep(VERIFY_RESTART_FLOOR - since).await;
                }
            }
            if let Err(e) = device.verify_start("any").await {
                log::warn!("VerifyStart({user}) failed, reclaiming: {e}");
                let _ = device.release().await;
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue 'outer;
            }
            last_start = Some(std::time::Instant::now());
            restarts += 1;
            if restarts == RESTART_WARN_AT {
                log::warn!("verify re-armed {restarts}× on one claim — cancel-echo loop?");
            }
            if !armed {
                armed = true;
                emit!(EngineEvent::Ready);
            }
            let watchdog = tokio::time::sleep(VERIFY_SILENCE_TIMEOUT);
            tokio::pin!(watchdog);
            loop {
                tokio::select! {
                    sig = status.next() => {
                        let Some(signal) = sig else {
                            // Stream ended — fprintd restarted; start over.
                            let _ = device.release().await;
                            emit!(EngineEvent::Unavailable("fprintd restarted".into()));
                            tokio::time::sleep(Duration::from_secs(1)).await;
                            continue 'outer;
                        };
                        let Ok(args) = signal.args() else { continue };
                        let verdict = parse_verify_status(args.result(), *args.done());
                        // A session we killed reporting in — not a finger.
                        // Absorb it and stay on the session now running.
                        if absorb_echo(&mut echoes, &verdict) {
                            log::debug!("absorbed cancel echo: {}", args.result());
                            continue;
                        }
                        match verdict {
                            Verify::Match => {
                                let _ = device.verify_stop().await;
                                let _ = device.release().await;
                                // Released — drop the inhibitor now so a
                                // post-match `wait_for_retarget` (agent) can't
                                // needlessly delay a suspend while unclaimed.
                                drop(inhibitor.take());
                                match sink(EngineEvent::Match(user.clone())) {
                                    Flow::Stop => return,
                                    Flow::Continue => {
                                        // Done for this target; wait for a
                                        // retarget (the greeter normally
                                        // disconnects instead).
                                        if wait_for_retarget(&mut target, &user).await.is_err() {
                                            return;
                                        }
                                        continue 'outer;
                                    }
                                }
                            }
                            Verify::NoMatch => {
                                let _ = device.verify_stop().await;
                                emit!(EngineEvent::Hint("Not recognized — try again".into()));
                                continue 'verify;
                            }
                            Verify::Disconnected => {
                                let _ = device.release().await;
                                emit!(EngineEvent::Unavailable("reader disconnected".into()));
                                tokio::time::sleep(Duration::from_secs(1)).await;
                                continue 'outer;
                            }
                            Verify::Error => {
                                let _ = device.verify_stop().await;
                                tokio::time::sleep(Duration::from_secs(1)).await;
                                continue 'verify;
                            }
                            Verify::Hint(Some(h)) => emit!(EngineEvent::Hint(h.into())),
                            Verify::Hint(None) => {}
                        }
                    }
                    _ = &mut watchdog => {
                        // No status for the whole window — the verify may be
                        // wedged. Re-arm it without dropping the claim (no UI
                        // flicker); the reader keeps scanning.
                        //
                        // Getting here means a session is still running (every
                        // terminal status leaves this loop), so the stop below
                        // cancels a live verify and fprintd owes us one more
                        // terminal status for it. Count it, or the restart
                        // reads it as a failed finger and the loop feeds
                        // itself.
                        log::info!("verify silent for {VERIFY_SILENCE_TIMEOUT:?}, restarting");
                        let _ = device.verify_stop().await;
                        echoes += 1;
                        continue 'verify;
                    }
                    r = target.changed() => {
                        if r.is_err() || target.borrow().as_deref() != Some(user.as_str()) {
                            let _ = device.verify_stop().await;
                            let _ = device.release().await;
                            if r.is_err() { return; }
                            continue 'outer;
                        }
                    }
                    _ = active.changed() => {
                        if !*active.borrow() {
                            let _ = device.verify_stop().await;
                            let _ = device.release().await;
                            emit!(EngineEvent::Unavailable("session inactive".into()));
                            continue 'outer;
                        }
                    }
                    _ = sleeping.changed() => {
                        if *sleeping.borrow() {
                            // Release before sleep, then drop the inhibitor
                            // (via `continue 'outer`) so suspend proceeds with
                            // the reader idle; the park loop reclaims on resume.
                            let _ = device.verify_stop().await;
                            let _ = device.release().await;
                            emit!(EngineEvent::Unavailable("suspending".into()));
                            continue 'outer;
                        }
                    }
                }
            }
        }
    }
}

/// The user prefix on an "unenrolled" message: the agent names the target
/// user; the lock (own user, empty string) just states the fact.
fn unenrolled_msg(user: &str) -> String {
    if user.is_empty() {
        "no enrolled fingerprints".into()
    } else {
        format!("{user}: no enrolled fingerprints")
    }
}

fn targeted(target: &tokio::sync::watch::Receiver<Option<String>>, user: &str) -> bool {
    target.borrow().as_deref() == Some(user)
}

/// Wait until the verify target is anything other than `user` (including
/// `None`). `Err` when the target sender was dropped (client disconnected).
async fn wait_for_retarget(
    target: &mut tokio::sync::watch::Receiver<Option<String>>,
    user: &str,
) -> Result<(), ()> {
    loop {
        if target.borrow_and_update().as_deref() != Some(user) {
            return Ok(());
        }
        if target.changed().await.is_err() {
            return Err(());
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
        assert_eq!(
            parse_verify_status("verify-no-match", true),
            Verify::NoMatch
        );
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
            assert!(matches!(
                parse_verify_status(s, false),
                Verify::Hint(Some(_))
            ));
        }
    }

    #[test]
    fn unknown_status_never_unlocks() {
        // Unknown terminal status → restart the verify session.
        assert_eq!(
            parse_verify_status("verify-unknown-error", true),
            Verify::Error
        );
        assert_eq!(parse_verify_status("something-new", true), Verify::Error);
        // Unknown non-terminal status → ignore, keep waiting.
        assert_eq!(
            parse_verify_status("something-new", false),
            Verify::Hint(None)
        );
        // Empty string (the old null-deref bug class) → safe fallthrough.
        assert_eq!(parse_verify_status("", true), Verify::Error);
        assert_eq!(parse_verify_status("", false), Verify::Hint(None));
    }

    #[test]
    fn cancel_echo_absorbs_one_terminal_status_per_stop() {
        let mut owed = 1;
        // The cancelled session's no-match is ours, not the user's.
        assert!(absorb_echo(&mut owed, &Verify::NoMatch));
        assert_eq!(owed, 0);
        // The next one is a real finger.
        assert!(!absorb_echo(&mut owed, &Verify::NoMatch));
    }

    #[test]
    fn cancel_echo_never_swallows_a_match() {
        // A cancellation completes as no-match or error, never as a match —
        // a finger that landed just before the stop still unlocks.
        let mut owed = 1;
        assert!(!absorb_echo(&mut owed, &Verify::Match));
        assert_eq!(owed, 1);
    }

    #[test]
    fn cancel_echo_covers_error_and_leaves_disconnect_alone() {
        let mut owed = 1;
        assert!(absorb_echo(&mut owed, &Verify::Error));
        let mut owed = 1;
        // A reader that vanished needs the reclaim path, echo or not.
        assert!(!absorb_echo(&mut owed, &Verify::Disconnected));
    }

    #[test]
    fn unenrolled_msg_names_target_but_not_own_user() {
        assert_eq!(unenrolled_msg(""), "no enrolled fingerprints");
        assert_eq!(unenrolled_msg("melvin"), "melvin: no enrolled fingerprints");
    }
}
