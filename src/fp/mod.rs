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
pub mod verify;

use std::time::Duration;

use zbus::export::futures_util::{FutureExt, StreamExt};
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
    /// Current sleep-transition state — `true` between PrepareForSleep(true)
    /// and the matching resume. Read once after subscribing, because the
    /// signal alone only covers *future* transitions.
    #[zbus(property)]
    fn preparing_for_sleep(&self) -> zbus::Result<bool>;
}

#[proxy(
    interface = "org.freedesktop.systemd1.Manager",
    default_service = "org.freedesktop.systemd1",
    default_path = "/org/freedesktop/systemd1"
)]
pub(crate) trait Systemd {
    fn restart_unit(&self, name: &str, mode: &str)
    -> zbus::Result<zbus::zvariant::OwnedObjectPath>;
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
    let setup = async {
        let manager = LogindManagerProxy::new(&conn).await?;
        let stream = manager.receive_prepare_for_sleep().await?;
        Ok::<_, zbus::Error>((manager, stream))
    }
    .await;
    match setup {
        Ok((manager, mut stream)) => {
            // Seed with the transition already in flight: a locker spawned
            // *by* the sleep transition (the idle daemon locks on
            // PrepareForSleep) starts after the signal fired and would
            // otherwise claim the reader on the way into suspend.
            // Subscribe-then-read is race-free — a transition between the
            // two lands in the stream and overwrites the seed.
            if let Ok(now) = manager.preparing_for_sleep().await {
                let _ = tx.send(now);
            }
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

/// Deadline for any single fprintd D-Bus call. fprintd has documented
/// multi-second stalls (22 s synaptics SSM spins) — an unbounded await in the
/// teardown path holds the sleep delay-inhibitor past logind's
/// InhibitDelayMaxSec, which is exactly the "claim survives into suspend"
/// wedge the inhibitor exists to prevent. A timed-out call may still complete
/// inside fprintd later; the orphaned reply is dropped harmlessly.
const CALL_TIMEOUT: Duration = Duration::from_secs(3);

/// One fprintd call with a deadline. Errors are strings because callers only
/// log or pattern-match them.
async fn timed<T>(
    what: &str,
    call: impl std::future::Future<Output = zbus::Result<T>>,
) -> Result<T, String> {
    match tokio::time::timeout(CALL_TIMEOUT, call).await {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err(format!("{what}: no reply in {CALL_TIMEOUT:?}")),
    }
}

/// fprintd's Claim error when libfprint's device object is still open from a
/// claim that was never released — the holder crashed, or the claim survived
/// a suspend (ours or an external pam_fprintd prompt's). Distinct from
/// ordinary contention ("already claimed", a release is coming): here no
/// release is coming and only an fprintd restart clears the device.
fn is_wedged_claim(err: &str) -> bool {
    err.contains("already open")
}

/// fprintd's Claim error while the reader's USB endpoint is mid-reset — the
/// post-resume re-init window (the s0ix path resets the sensor on wake,
/// see s0ix-suspend.nix). Claims are doomed until it settles, and an Open
/// raced against the reset can leave the device wedged open inside libfprint
/// (observed 2026-07-31 21:28: six stalled Opens on resume, then "already
/// open" for the rest of the boot).
fn is_device_resetting(err: &str) -> bool {
    err.contains("endpoint stalled")
}

/// Restart fprintd to clear a wedged device. Needs a polkit rule allowing
/// active local sessions to restart exactly this unit (the fp-agent is root
/// and passes implicitly); without one this logs and the claim loop keeps
/// retrying — no worse than before.
async fn restart_fprintd(conn: &zbus::Connection) {
    let restart = async {
        SystemdProxy::new(conn)
            .await?
            .restart_unit("fprintd.service", "replace")
            .await
    };
    match tokio::time::timeout(CALL_TIMEOUT, restart).await {
        Ok(Ok(_)) => {
            log::warn!("fprintd device wedged (claim failing \"already open\") — restarted fprintd")
        }
        Ok(Err(e)) => log::warn!("fprintd wedged but restart refused (polkit rule missing?): {e}"),
        Err(_) => log::warn!("fprintd wedged and the restart request timed out"),
    }
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
    /// Face verification progress, streamed while a burst runs. The
    /// fingerprint engine never emits this: fprintd reports nothing between
    /// touch and verdict, whereas faced reports every frame, and the lock
    /// screen would otherwise sit inert for the whole attempt.
    Progress(crate::face::Progress),
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

/// Ping fprintd after this long with no status signal. Silence is what an
/// untouched reader sounds like — every unattended lock screen sits here for
/// its whole life — so silence alone must never restart the verify: the
/// stop/start churn that "recovery" produced is what desynced the synaptics
/// sensor and fed the cancel-echo storms (boots -1/-2, 2026-07-25..30). The
/// ping (a Manager round trip) only detects a daemon that stopped answering;
/// that, and only that, warrants restarting the session.
const VERIFY_SILENCE_PING: Duration = Duration::from_secs(40);

/// Floor between VerifyStart calls within one claim. A human retry is seconds
/// apart, so this is invisible in normal use, and it caps any restart feedback
/// loop at a trickle instead of a spin. Backstop for the cancel echo below.
const VERIFY_RESTART_FLOOR: Duration = Duration::from_millis(400);

/// Warn once when one claim has re-armed this many times — a healthy claim
/// re-arms only on a failed finger or a dead-daemon ping, so anything near
/// this is a loop that wants looking at.
const RESTART_WARN_AT: u32 = 20;

/// Claim retry cadence. The fast one covers the normal handover, where the
/// previous holder is a few hundred milliseconds from releasing. After
/// `CLAIM_QUIET_ATTEMPTS` it steps back: every attempt is a full polkit
/// round trip, and hammering one twice a second for the length of a lock
/// screen buys nothing a two-second poll doesn't.
const CLAIM_RETRY: Duration = Duration::from_millis(500);
const CLAIM_RETRY_SLOW: Duration = Duration::from_secs(2);
const CLAIM_QUIET_ATTEMPTS: u32 = 6;

/// Cooldown between fprintd restarts for a wedged device (see
/// [`is_wedged_claim`]). "Already open" cannot self-resolve — fprintd
/// accepted the claim, so no other holder exists and no release is coming —
/// which is why the restart fires after a single retry rather than a long
/// patience window: the 8-attempts-per-cycle accounting this replaces never
/// fired once in practice (measured 2026-07-31 — every lock session was
/// retargeted or password-unlocked first). The cooldown is engine-lifetime,
/// so claim-cycle resets can't defer the cure, and a restart that doesn't
/// cure (the cause is below fprintd: USB, driver) can't loop.
const UNWEDGE_COOLDOWN: Duration = Duration::from_secs(30);

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
    match tokio::time::timeout(CALL_TIMEOUT, device.list_enrolled_fingers(user)).await {
        Ok(Ok(fingers)) if fingers.is_empty() => Enrollment::None,
        Ok(Ok(_)) => Enrollment::Enrolled,
        Ok(Err(e)) if is_no_enrolled_prints(&e) => Enrollment::None,
        Ok(Err(e)) => {
            log::warn!("list_enrolled_fingers({user}): {e}");
            Enrollment::Unknown
        }
        Err(_) => {
            log::warn!("list_enrolled_fingers({user}): no reply in {CALL_TIMEOUT:?}");
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

    // Terminal statuses fprintd owes us for verify sessions we cancelled
    // ourselves (see `absorb_echo`). Engine-lifetime, like the status stream
    // the echoes arrive on: debt from a cancel just before a release must
    // survive into the next claim, or the stale echo reads as a failed finger
    // there and the restart loop feeds itself.
    let mut echoes: u32 = 0;

    // The device proxy and its status stream live and die together: an
    // fprintd restart (or a reader re-enumeration) invalidates both, so every
    // path that loses one re-resolves both — parking a session on a dead
    // object was one of the ways a lock screen lost fingerprint for good.
    let mut resolve_fails: u32 = 0;

    // Engine-lifetime, deliberately not per claim cycle: retargets and gate
    // flips reset the cycle but must not reset the wedge cure's clock.
    let mut last_unwedge: Option<std::time::Instant> = None;
    'link: loop {
        // Don't touch fprintd until someone wants a verify — the agent sits
        // with no target most of its life.
        loop {
            if target.borrow_and_update().is_some() {
                break;
            }
            if target.changed().await.is_err() {
                return;
            }
        }
        let (device, mut status) = {
            // Resolve + subscribe under one deadline (two quick round trips);
            // subscribe before the first VerifyStart so no status slips past.
            let resolve = async {
                let device = device_proxy(&conn).await?;
                let status = device
                    .receive_verify_status()
                    .await
                    .map_err(|e| e.to_string())?;
                Ok::<_, String>((device, status))
            };
            match tokio::time::timeout(CALL_TIMEOUT, resolve).await {
                Ok(Ok(pair)) => {
                    resolve_fails = 0;
                    pair
                }
                other => {
                    let err = match other {
                        Ok(Err(e)) => e,
                        _ => format!("no reply in {CALL_TIMEOUT:?}"),
                    };
                    // Keep retrying on a timer — fprintd may be cold at boot
                    // or mid-restart, and a lock session must outlive that.
                    // Announce only the first failure of an outage; the pill
                    // is already hidden and the rest is log noise.
                    resolve_fails += 1;
                    if resolve_fails == 1 {
                        emit!(EngineEvent::Unavailable(format!(
                            "no fingerprint device: {err}"
                        )));
                    } else {
                        log::debug!("fprintd resolve retry {resolve_fails}: {err}");
                    }
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    continue 'link;
                }
            }
        };

        'outer: loop {
            // Park until there is a target, the session is active, and the
            // system isn't heading into sleep.
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

            // Cancel-of-a-live-session teardown, shared by every gate-flip
            // path out of the verify loop: the killed session earns fprintd
            // an echo (`absorb_echo`), the claim goes back, and the sink
            // hears about it so no pill outlives its reader.
            macro_rules! stand_down {
                ($reason:expr) => {{
                    let _ = timed("VerifyStop", device.verify_stop()).await;
                    echoes += 1;
                    let _ = timed("Release", device.release()).await;
                    emit!(EngineEvent::Unavailable($reason.into()));
                    continue 'outer;
                }};
            }

            // Enrollment gate: skip the claim/verify spin for a user with no
            // prints, but only on an *authoritative* answer — a failed read
            // falls through and lets the claim loop retry.
            match enrollment(&device, &user).await {
                Enrollment::Enrolled | Enrollment::Unknown => {}
                Enrollment::None => {
                    emit!(EngineEvent::Unavailable(unenrolled_msg(&user)));
                    // Park until the next target command of any kind — the
                    // same user re-sent counts (enrollment may have just
                    // happened), so this is `changed()`, not a value wait.
                    if target.changed().await.is_err() {
                        return;
                    }
                    continue 'outer;
                }
            }

            // Claim as the target user; retry while the reader is busy (e.g.
            // a backgrounded locker still releasing it). The sleep
            // delay-inhibitor lives for this `'outer` iteration — held from
            // just before Claim so suspend can't slip in, and dropped by
            // `continue 'outer`/`'link` (scope exit) after the release calls
            // below, so suspend proceeds with the reader idle.
            let mut inhibitor: Option<zbus::zvariant::OwnedFd> = None;
            let mut attempts: u32 = 0;
            loop {
                if !targeted(&target, &user) || !*active.borrow() || *sleeping.borrow() {
                    continue 'outer;
                }
                if inhibitor.is_none() {
                    inhibitor = take_sleep_inhibitor(&conn, "swaypplet-fp").await;
                }
                match timed("Claim", device.claim(&user)).await {
                    Ok(()) => break,
                    Err(e) => {
                        attempts += 1;
                        // A couple of misses is the normal handover shape (the
                        // other side is still releasing). Past that the reader
                        // is held by something that isn't letting go on its
                        // own — say so once, with fprintd's own words, and
                        // stop promising a pill nobody can touch.
                        if attempts == CLAIM_QUIET_ATTEMPTS {
                            log::warn!("claim({user}) still failing after {attempts}: {e}");
                            emit!(EngineEvent::Unavailable("reader busy".into()));
                        } else {
                            log::debug!("claim({user}) attempt {attempts} failed: {e}");
                        }
                        // A device wedged open never releases on its own; a
                        // fresh fprintd is the only cure. One retry absorbs
                        // the edge where our own timed-out release lands a
                        // moment late; then restart, gated by the
                        // engine-lifetime cooldown (see UNWEDGE_COOLDOWN).
                        // The polkit rule scoping the restart to this one
                        // unit keeps the hammer small.
                        if attempts >= 2
                            && is_wedged_claim(&e)
                            && last_unwedge.is_none_or(|t| t.elapsed() >= UNWEDGE_COOLDOWN)
                        {
                            last_unwedge = Some(std::time::Instant::now());
                            restart_fprintd(&conn).await;
                            tokio::time::sleep(Duration::from_secs(1)).await;
                            continue 'link; // fresh daemon, fresh device + stream
                        }
                        // A resetting endpoint outlasts the fast cadence by
                        // design (2–4 s re-init), and every Open against it
                        // is another chance to wedge the device — go
                        // straight to the gentle poll.
                        let backoff = if attempts < CLAIM_QUIET_ATTEMPTS && !is_device_resetting(&e)
                        {
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

            // The status stream outlives claims, so anything buffered while
            // we were parked — the echo of a cancel, a match that raced a
            // gate flip — belongs to a dead claim. Settle the echo ledger
            // and start this claim with a clean stream, so a stale verdict
            // can never be read as this claim's finger.
            while let Some(Some(sig)) = status.next().now_or_never() {
                if let Ok(args) = sig.args() {
                    let verdict = parse_verify_status(args.result(), *args.done());
                    absorb_echo(&mut echoes, &verdict);
                    log::debug!("discarding pre-claim status: {}", args.result());
                }
            }

            // Verify sessions until a match, a retarget, or a gate flip.
            // `armed` holds back Ready until a verify is actually running: a
            // claim alone is not a reader you can touch, and promising one
            // that isn't scanning is worse than promising nothing.
            let mut armed = false;
            let mut restarts: u32 = 0;
            let mut last_start: Option<std::time::Instant> = None;
            'verify: loop {
                if !targeted(&target, &user) || !*active.borrow() || *sleeping.borrow() {
                    let _ = timed("Release", device.release()).await;
                    emit!(EngineEvent::Unavailable("standing down".into()));
                    continue 'outer;
                }
                // Never re-arm faster than the floor, whatever drove us here.
                if let Some(prev) = last_start {
                    let since = prev.elapsed();
                    if since < VERIFY_RESTART_FLOOR {
                        tokio::time::sleep(VERIFY_RESTART_FLOOR - since).await;
                    }
                }
                if let Err(e) = timed("VerifyStart", device.verify_start("any")).await {
                    log::warn!("VerifyStart({user}) failed, reclaiming: {e}");
                    let _ = timed("Release", device.release()).await;
                    emit!(EngineEvent::Unavailable("reader error, reclaiming".into()));
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
                let watchdog = tokio::time::sleep(VERIFY_SILENCE_PING);
                tokio::pin!(watchdog);
                loop {
                    tokio::select! {
                        sig = status.next() => {
                            let Some(signal) = sig else {
                                // Stream over — fprintd went away and took the
                                // claim and the device object with it.
                                emit!(EngineEvent::Unavailable("fprintd restarted".into()));
                                tokio::time::sleep(Duration::from_secs(1)).await;
                                continue 'link;
                            };
                            watchdog.as_mut().reset(
                                tokio::time::Instant::now() + VERIFY_SILENCE_PING,
                            );
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
                                    let _ = timed("VerifyStop", device.verify_stop()).await;
                                    let _ = timed("Release", device.release()).await;
                                    // Released — drop the inhibitor now so a
                                    // post-match park (agent) can't needlessly
                                    // delay a suspend while unclaimed.
                                    drop(inhibitor.take());
                                    match sink(EngineEvent::Match(user.clone())) {
                                        Flow::Stop => return,
                                        Flow::Continue => {
                                            // Done for this target; park until
                                            // the next command — a retarget, a
                                            // stand-down, or the same user
                                            // re-sent (a greeter re-arming
                                            // after its auth path failed
                                            // downstream of the match).
                                            if target.changed().await.is_err() {
                                                return;
                                            }
                                            continue 'outer;
                                        }
                                    }
                                }
                                Verify::NoMatch => {
                                    let _ = timed("VerifyStop", device.verify_stop()).await;
                                    emit!(EngineEvent::Hint("Not recognized — try again".into()));
                                    continue 'verify;
                                }
                                Verify::Disconnected => {
                                    let _ = timed("Release", device.release()).await;
                                    emit!(EngineEvent::Unavailable("reader disconnected".into()));
                                    // Re-enumeration hands out a fresh object
                                    // path — resolve from scratch.
                                    tokio::time::sleep(Duration::from_secs(1)).await;
                                    continue 'link;
                                }
                                Verify::Error => {
                                    let _ = timed("VerifyStop", device.verify_stop()).await;
                                    tokio::time::sleep(Duration::from_secs(1)).await;
                                    continue 'verify;
                                }
                                Verify::Hint(Some(h)) => emit!(EngineEvent::Hint(h.into())),
                                Verify::Hint(None) => {}
                            }
                        }
                        _ = &mut watchdog => {
                            // Quiet for the whole window. An untouched reader
                            // is silent by nature — every unattended lock
                            // screen lives here — so silence is not failure.
                            // Ping the daemon and only restart the verify if
                            // fprintd itself stopped answering; churning a
                            // healthy claim is what used to desync the sensor.
                            let ping = async {
                                ManagerProxy::new(&conn).await?.get_default_device().await
                            };
                            match tokio::time::timeout(CALL_TIMEOUT, ping).await {
                                Ok(Ok(_)) => watchdog.as_mut().reset(
                                    tokio::time::Instant::now() + VERIFY_SILENCE_PING,
                                ),
                                _ => {
                                    log::info!(
                                        "fprintd quiet for {VERIFY_SILENCE_PING:?} and not \
                                         answering — restarting verify"
                                    );
                                    let _ = timed("VerifyStop", device.verify_stop()).await;
                                    echoes += 1;
                                    continue 'verify;
                                }
                            }
                        }
                        r = target.changed() => {
                            if r.is_err() {
                                let _ = timed("VerifyStop", device.verify_stop()).await;
                                let _ = timed("Release", device.release()).await;
                                return;
                            }
                            if targeted(&target, &user) {
                                // Same target re-sent: a client asking "make
                                // sure you're armed, and say so". The verify
                                // is live — answer with the state, so a client
                                // that hid its pill on its own (greeter list
                                // upgrade) can resync.
                                emit!(EngineEvent::Ready);
                            } else {
                                stand_down!("standing down");
                            }
                        }
                        _ = active.changed() => {
                            if !*active.borrow() {
                                stand_down!("session inactive");
                            }
                        }
                        _ = sleeping.changed() => {
                            if *sleeping.borrow() {
                                // Release before sleep; `continue 'outer`
                                // (inside stand_down) drops the inhibitor so
                                // suspend proceeds with the reader idle, and
                                // the park loop reclaims on resume.
                                stand_down!("suspending");
                            }
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

/// Does the calling user have enrolled fingerprints? Blocking — call it off
/// the main thread, and well before the answer is needed.
///
/// `None` means fprintd would not say (cold, restarting, no default device).
/// Callers reserving layout for a fingerprint pill should treat that as yes:
/// a slot held open for a reader that never arms costs one pill of blank
/// card, while a slot that appears late costs a card that jumps under the
/// user's eyes — and that is the whole reason this question is asked here
/// rather than when the reader reports in.
///
/// Read-only: `ListEnrolledFingers` neither claims the device nor disturbs
/// another client's claim, so this is safe to call while the desktop is in
/// use.
pub fn self_enrolled_blocking() -> Option<bool> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()?;
    rt.block_on(async {
        let conn = tokio::time::timeout(CALL_TIMEOUT, zbus::Connection::system())
            .await
            .ok()?
            .ok()?;
        let device = tokio::time::timeout(CALL_TIMEOUT, device_proxy(&conn))
            .await
            .ok()?
            .ok()?;
        match enrollment(&device, "").await {
            Enrollment::Enrolled => Some(true),
            Enrollment::None => Some(false),
            Enrollment::Unknown => None,
        }
    })
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
    fn wedged_claim_is_distinct_from_ordinary_contention() {
        // libfprint device left open by a claim that never released —
        // restart-worthy.
        assert!(is_wedged_claim(
            "org.freedesktop.DBus.Error.Failed: Device 06cb:019d is already open"
        ));
        // Ordinary contention: a release is coming; never restart for this.
        assert!(!is_wedged_claim("the device is already claimed"));
        assert!(!is_wedged_claim("Claim: no reply in 3s"));
    }

    #[test]
    fn resetting_endpoint_is_neither_wedged_nor_contention() {
        let e = "net.reactivated.Fprint.Error.Internal: Open failed with \
                 error: endpoint stalled or request not supported";
        assert!(is_device_resetting(e));
        assert!(!is_wedged_claim(e));
        // The wedge signature is not a reset — restart cures it, waiting won't.
        assert!(!is_device_resetting(
            "org.freedesktop.DBus.Error.Failed: Device 06cb:019d is already open"
        ));
    }

    #[test]
    fn unenrolled_msg_names_target_but_not_own_user() {
        assert_eq!(unenrolled_msg(""), "no enrolled fingerprints");
        assert_eq!(unenrolled_msg("melvin"), "melvin: no enrolled fingerprints");
    }
}
