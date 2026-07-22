//! `swaypplet fp-agent` / `swaypplet fp-check` — out-of-band fingerprint
//! auth for the greeter.
//!
//! greetd's PAM conversation is strictly synchronous: with pam_fprintd in
//! the stack, acking the "place your finger" prompt parks the conversation
//! (and greetd's whole context lock) until the fingerprint resolves —
//! cancels, user switches and parallel password entry all hang. So the
//! greetd stack is password-only and fingerprint runs beside it:
//!
//! * `fp-agent` (root daemon, socket in `/run/swaypplet-fp` reachable by the
//!   `greeter` group) claims the fprintd device *as the target user* and
//!   verifies, exactly like the lock screen does for its own user. On a
//!   match it mints a single-use, short-lived random token into a root-only
//!   file and hands the token to the greeter.
//! * The greeter submits the token as the password answer.
//! * `fp-check` (run by a `sufficient` pam_exec rule ahead of pam_unix in
//!   greetd's stack, as root) compares the submitted authtok against the
//!   token file (constant-time), consumes it, and passes auth on a match.
//!   A real password sails through to pam_unix unharmed.
//!
//! The verify loop is gated on the *client's* logind session being active:
//! a backgrounded greeter (user switched VTs) releases the reader so the
//! now-active session's locker — or another greeter — can claim it. Several
//! greeters may be connected at once; the active-session gate ensures at
//! most one holds the device.
//!
//! Env (both ends): SWAYPPLET_FP_SOCK, SWAYPPLET_FP_TOKEN override the
//! socket and token paths (tests / dev boxes).

use std::io::{Read, Write};
use std::os::unix::io::AsRawFd;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, watch, Semaphore};
use zbus::export::futures_util::StreamExt;
use zbus::proxy;

use crate::lock::fprint::{is_no_enrolled_prints, parse_verify_status, DeviceProxy, ManagerProxy, Verify};

const DEFAULT_SOCK: &str = "/run/swaypplet-fp/agent.sock";
const DEFAULT_TOKEN: &str = "/run/swaypplet-fp/token.json";
const TOKEN_TTL_SECS: u64 = 20;
/// Cap on concurrent client connections. The only legitimate client is the
/// single on-screen greeter; this bounds a misbehaving/hostile peer on the
/// greeter-group socket from spawning unbounded verify loops.
const MAX_CLIENTS: usize = 8;
/// Cap on bytes read from one connection. Commands are tiny JSON lines; this
/// stops an unterminated line from growing this root process's memory.
const MAX_CLIENT_BYTES: u64 = 64 * 1024;

pub(crate) fn sock_path() -> String {
    std::env::var("SWAYPPLET_FP_SOCK")
        .ok()
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| DEFAULT_SOCK.into())
}

fn token_path() -> String {
    std::env::var("SWAYPPLET_FP_TOKEN")
        .ok()
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| DEFAULT_TOKEN.into())
}

/// Client → agent, one JSON object per line.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "kebab-case")]
pub(crate) enum Cmd {
    /// Verify `user`'s fingerprint (replaces any previous target).
    Verify { user: String },
    /// Stop verifying and release the reader.
    Stop,
}

/// Agent → client, one JSON object per line.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "ev", rename_all = "kebab-case")]
pub(crate) enum Ev {
    /// Device claimed and scanning — show the pill.
    Ready,
    /// Transient guidance ("not recognized", "center your finger", …).
    Hint { msg: String },
    /// Fingerprint matched for `user`; submit `token` as the password.
    Match { user: String, token: String },
    /// No usable reader right now — hide the pill. `Ready` may follow later.
    Unavailable { msg: String },
}

// --- logind session-active watching (shared with the lock screen) ---------

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
    fn inhibit(&self, what: &str, who: &str, why: &str, mode: &str)
        -> zbus::Result<zbus::zvariant::OwnedFd>;
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

/// Push `pid`'s logind-session Active state into `tx`, forever. Never
/// returns: when the session can't be resolved (dev box, session gone) it
/// settles on `true` and parks, so receivers can rely on the channel staying
/// open and select on `changed()` without a busy-loop guard.
pub(crate) async fn watch_session_active(
    conn: zbus::Connection,
    pid: Option<u32>,
    tx: watch::Sender<bool>,
) {
    // Resolve once, with bounded retry: session registration can race the
    // locker/agent starting (fast VT handoff, cold boot). GetSession("auto")
    // resolves our own session even for user@.service children, where
    // GetSessionByPID fails because the PID isn't in a session cgroup — try
    // it first, then fall back to PID. Giving up too early is what left an
    // inactive locker holding the fingerprint reader.
    let mut session = None;
    for attempt in 0..RESOLVE_ATTEMPTS {
        match resolve_session(&conn, pid).await {
            Ok(s) => {
                session = Some(s);
                break;
            }
            Err(e) => {
                let last = attempt + 1 == RESOLVE_ATTEMPTS;
                log::log!(
                    if last { log::Level::Warn } else { log::Level::Debug },
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

const RESOLVE_ATTEMPTS: u32 = 5;
const RESOLVE_BACKOFF: std::time::Duration = std::time::Duration::from_millis(500);

/// Push logind's PrepareForSleep state into `tx`, forever: `true` from
/// PrepareForSleep(start) until the matching resume. Claim holders gate on it
/// and release the reader before sleep — a claim held across suspend leaves
/// the synaptics device wedged open inside fprintd (every later Claim fails
/// with "Device ... is already open" until fprintd restarts). Like
/// `watch_session_active`, never returns: on setup failure it settles on
/// `false` (awake) and parks, so receivers can select on `changed()` freely.
pub(crate) async fn watch_sleep(conn: zbus::Connection, tx: watch::Sender<bool>) {
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
pub(crate) async fn take_sleep_inhibitor(
    conn: &zbus::Connection,
    who: &str,
) -> Option<zbus::zvariant::OwnedFd> {
    let take = async {
        LogindManagerProxy::new(conn)
            .await?
            .inhibit("sleep", who, "release fingerprint reader before sleep", "delay")
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

/// Resolve this process's logind session proxy: GetSession("auto") first
/// (works for user@.service children), then GetSessionByPID as a fallback.
async fn resolve_session(
    conn: &zbus::Connection,
    pid: Option<u32>,
) -> Result<LogindSessionProxy<'static>, String> {
    let manager = LogindManagerProxy::new(conn)
        .await
        .map_err(|e| e.to_string())?;
    let path = match manager.get_session("auto").await {
        Ok(p) => p,
        Err(auto_err) => {
            let pid = pid.ok_or_else(|| format!("GetSession(auto): {auto_err}; no pid fallback"))?;
            manager
                .get_session_by_pid(pid)
                .await
                .map_err(|e| format!("GetSession(auto): {auto_err}; GetSessionByPID: {e}"))?
        }
    };
    LogindSessionProxy::builder(conn)
        .path(path)
        .map_err(|e| e.to_string())?
        .build()
        .await
        .map_err(|e| e.to_string())
}

// --- agent daemon ---------------------------------------------------------

pub fn run_agent() -> ! {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("swaypplet fp-agent: tokio runtime: {e}");
            std::process::exit(1);
        }
    };
    std::process::exit(rt.block_on(serve()));
}

async fn serve() -> i32 {
    let path = sock_path();
    if let Some(dir) = std::path::Path::new(&path).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::remove_file(&path);
    // Socket ownership/mode come from the unit: Group=greeter + UMask=0007.
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("swaypplet fp-agent: bind {path}: {e}");
            return 1;
        }
    };
    let conn = match zbus::Connection::system().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("swaypplet fp-agent: system bus: {e}");
            return 1;
        }
    };
    log::info!("fp-agent listening on {path}");
    // One PrepareForSleep watcher for the daemon; each client's verify loop
    // gates on a clone of the receiver.
    let (sleep_tx, sleep_rx) = watch::channel(false);
    tokio::spawn(watch_sleep(conn.clone(), sleep_tx));
    let slots = Arc::new(Semaphore::new(MAX_CLIENTS));
    loop {
        match listener.accept().await {
            Ok((stream, _)) => match Arc::clone(&slots).try_acquire_owned() {
                Ok(permit) => {
                    let conn = conn.clone();
                    let sleeping = sleep_rx.clone();
                    tokio::spawn(async move {
                        client(stream, conn, sleeping).await;
                        drop(permit);
                    });
                }
                Err(_) => log::warn!("fp-agent: client limit ({MAX_CLIENTS}) reached, dropping"),
            },
            Err(e) => {
                log::warn!("accept: {e}");
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
    }
}

async fn client(stream: UnixStream, conn: zbus::Connection, sleeping: watch::Receiver<bool>) {
    let peer_pid = stream
        .peer_cred()
        .ok()
        .and_then(|c| c.pid())
        .map(|p| p as u32);
    let (r, w) = stream.into_split();

    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Ev>();
    let writer = tokio::spawn(async move {
        let mut w = w;
        while let Some(ev) = out_rx.recv().await {
            let Ok(mut line) = serde_json::to_string(&ev) else {
                continue;
            };
            line.push('\n');
            if w.write_all(line.as_bytes()).await.is_err() {
                break;
            }
        }
    });

    let (active_tx, active_rx) = watch::channel(true);
    let watcher = tokio::spawn(watch_session_active(conn.clone(), peer_pid, active_tx));

    let (target_tx, target_rx) = watch::channel::<Option<String>>(None);
    let verifier = tokio::spawn(verify_loop(conn, target_rx, active_rx, sleeping, out_tx));

    // Bound total input so an unterminated line can't grow this root
    // process's memory; legitimate commands are a handful of bytes each.
    let mut lines = BufReader::new(r.take(MAX_CLIENT_BYTES)).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<Cmd>(line) {
            Ok(Cmd::Verify { user }) => {
                let _ = target_tx.send(Some(user));
            }
            Ok(Cmd::Stop) => {
                let _ = target_tx.send(None);
            }
            Err(e) => log::warn!("bad command line: {e}"),
        }
    }
    // Client gone: dropping the target sender tells the verify loop to stop
    // any running verify, release the claim, and exit.
    drop(target_tx);
    let _ = verifier.await;
    watcher.abort();
    writer.abort();
}

/// One per client connection. Claims/verifies while (a) the client wants a
/// user verified and (b) the client's session is active; releases the device
/// whenever either stops holding.
async fn verify_loop(
    conn: zbus::Connection,
    mut target: watch::Receiver<Option<String>>,
    mut active: watch::Receiver<bool>,
    mut sleeping: watch::Receiver<bool>,
    out: mpsc::UnboundedSender<Ev>,
) {
    macro_rules! send {
        ($ev:expr) => {
            let _ = out.send($ev);
        };
    }

    // Resolve the device lazily and retry per target change, so a machine
    // where fprintd isn't up yet doesn't wedge the connection.
    let device = loop {
        if target.borrow_and_update().is_some() {
            match device_proxy(&conn).await {
                Ok(d) => break d,
                Err(e) => {
                    send!(Ev::Unavailable { msg: format!("no fingerprint device: {e}") });
                }
            }
        }
        if target.changed().await.is_err() {
            return;
        }
    };
    // Subscribe before the first VerifyStart so no status can slip past.
    let mut status_stream = match device.receive_verify_status().await {
        Ok(s) => s,
        Err(e) => {
            send!(Ev::Unavailable { msg: format!("signal subscribe: {e}") });
            return;
        }
    };

    'outer: loop {
        // Park until there is a target, the client's session is active, and
        // the system isn't heading into sleep (a claim held across suspend
        // wedges the reader open inside fprintd — see `watch_sleep`).
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

        // Unenrolled target: report once, wait for a retarget instead of
        // spinning claim/verify against fprintd.
        match device.list_enrolled_fingers(&user).await {
            Ok(fingers) if fingers.is_empty() => {
                send!(Ev::Unavailable { msg: format!("{user}: no enrolled fingerprints") });
                if wait_for_retarget(&mut target, &user).await.is_err() {
                    return;
                }
                continue 'outer;
            }
            Ok(_) => {}
            Err(e) if is_no_enrolled_prints(&e) => {
                send!(Ev::Unavailable { msg: format!("{user}: no enrolled fingerprints") });
                if wait_for_retarget(&mut target, &user).await.is_err() {
                    return;
                }
                continue 'outer;
            }
            // Transient (fprintd restarting) — let the claim loop retry.
            Err(e) => log::warn!("list_enrolled_fingers({user}): {e}"),
        }

        // Claim as the target user; retry while the reader is busy (e.g. a
        // backgrounded locker still releasing it). The sleep delay-inhibitor
        // is held while claimed (taken just before Claim so suspend can't
        // slip between them); `continue 'outer` drops it after the release
        // calls below, so suspend proceeds with the reader idle.
        let mut inhibitor: Option<zbus::zvariant::OwnedFd> = None;
        loop {
            if target.borrow().as_deref() != Some(user.as_str())
                || !*active.borrow()
                || *sleeping.borrow()
            {
                continue 'outer;
            }
            if inhibitor.is_none() {
                inhibitor = take_sleep_inhibitor(&conn, "swaypplet-fp-agent").await;
            }
            match device.claim(&user).await {
                Ok(()) => break,
                Err(e) => {
                    log::debug!("claim({user}) failed, retrying: {e}");
                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_millis(500)) => {}
                        r = target.changed() => if r.is_err() { return },
                        _ = active.changed() => {}
                        _ = sleeping.changed() => {}
                    }
                }
            }
        }
        send!(Ev::Ready);

        // Verify sessions until a match, a retarget, or deactivation.
        'verify: loop {
            if target.borrow().as_deref() != Some(user.as_str())
                || !*active.borrow()
                || *sleeping.borrow()
            {
                let _ = device.release().await;
                continue 'outer;
            }
            if let Err(e) = device.verify_start("any").await {
                log::warn!("VerifyStart({user}) failed, reclaiming: {e}");
                let _ = device.release().await;
                drop(inhibitor.take());
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue 'outer;
            }
            loop {
                tokio::select! {
                    sig = status_stream.next() => {
                        let Some(signal) = sig else {
                            // Stream ended — fprintd restarted; start over.
                            let _ = device.release().await;
                            drop(inhibitor.take());
                            send!(Ev::Unavailable { msg: "fprintd restarted".into() });
                            tokio::time::sleep(Duration::from_secs(1)).await;
                            continue 'outer;
                        };
                        let Ok(args) = signal.args() else { continue };
                        match parse_verify_status(args.result(), *args.done()) {
                            Verify::Match => {
                                let _ = device.verify_stop().await;
                                let _ = device.release().await;
                                drop(inhibitor.take());
                                match mint_token(&user) {
                                    Ok(token) => {
                                        log::info!("fingerprint match for {user}, token minted");
                                        send!(Ev::Match { user: user.clone(), token });
                                    }
                                    Err(e) => {
                                        log::error!("token mint failed: {e}");
                                        send!(Ev::Unavailable { msg: "token mint failed".into() });
                                    }
                                }
                                // Done for this target; wait for a retarget
                                // (the greeter normally disconnects instead).
                                if wait_for_retarget(&mut target, &user).await.is_err() {
                                    return;
                                }
                                continue 'outer;
                            }
                            Verify::NoMatch => {
                                let _ = device.verify_stop().await;
                                send!(Ev::Hint { msg: "Not recognized — try again".into() });
                                continue 'verify;
                            }
                            Verify::Disconnected => {
                                let _ = device.release().await;
                                drop(inhibitor.take());
                                send!(Ev::Unavailable { msg: "reader disconnected".into() });
                                tokio::time::sleep(Duration::from_secs(1)).await;
                                continue 'outer;
                            }
                            Verify::Error => {
                                let _ = device.verify_stop().await;
                                tokio::time::sleep(Duration::from_secs(1)).await;
                                continue 'verify;
                            }
                            Verify::Hint(Some(h)) => { send!(Ev::Hint { msg: h.into() }); }
                            Verify::Hint(None) => {}
                        }
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
                            send!(Ev::Unavailable { msg: "session inactive".into() });
                            continue 'outer;
                        }
                    }
                    _ = sleeping.changed() => {
                        if *sleeping.borrow() {
                            // Release before sleep, then drop the inhibitor
                            // (via `continue 'outer`) so suspend proceeds
                            // with the reader idle; the park loop reclaims
                            // on resume.
                            let _ = device.verify_stop().await;
                            let _ = device.release().await;
                            send!(Ev::Unavailable { msg: "suspending".into() });
                            continue 'outer;
                        }
                    }
                }
            }
        }
    }
}

/// Wait until the verify target is anything other than `user` (including
/// `None`). `Err` when the client disconnected.
async fn wait_for_retarget(
    target: &mut watch::Receiver<Option<String>>,
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

// --- token ----------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct StoredToken {
    user: String,
    token: String,
    /// Unix seconds.
    expires: u64,
}

/// Advisory lock over the token file, held for the whole mint/consume
/// critical section so a mint and a `fp-check` consume can't interleave. The
/// lock lives on a sibling `.lock` file (never removed) rather than the token
/// itself, since consume unlinks the token while checking it. Released on
/// drop (the fd close drops the flock).
struct TokenLock(#[allow(dead_code)] std::fs::File);

impl TokenLock {
    fn acquire(token_path: &str) -> Option<Self> {
        use std::os::unix::fs::OpenOptionsExt;
        let f = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .mode(0o600)
            .open(format!("{token_path}.lock"))
            .ok()?;
        // Blocking LOCK_EX: the critical section is a few file ops.
        if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return None;
        }
        Some(TokenLock(f))
    }
}

fn mint_token(user: &str) -> Result<String, String> {
    let mut buf = [0u8; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf))
        .map_err(|e| format!("urandom: {e}"))?;
    let token: String = buf.iter().map(|b| format!("{b:02x}")).collect();
    let expires = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs()
        + TOKEN_TTL_SECS;
    let stored = StoredToken {
        user: user.into(),
        token: token.clone(),
        expires,
    };
    let path = token_path();
    let _lock = TokenLock::acquire(&path);
    let payload = serde_json::to_vec(&stored).map_err(|e| e.to_string())?;
    // Write to a temp sibling then rename, so a concurrent `fp-check` read
    // never sees a torn file — it observes either the old token or the new
    // one, never a partial write.
    let tmp = format!("{path}.tmp.{}", std::process::id());
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&tmp).map_err(|e| format!("{tmp}: {e}"))?;
    f.write_all(&payload).map_err(|e| e.to_string())?;
    f.sync_all().map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("rename {tmp}: {e}")
    })?;
    Ok(token)
}

// --- pam_exec check helper ------------------------------------------------

/// `swaypplet fp-check`: pam_exec (expose_authtok) helper. Exit 0 ⇔ the
/// authtok on stdin is the current, unexpired token for $PAM_USER. Consumes
/// the token on success (single use) and on expiry.
pub fn run_check() -> ! {
    std::process::exit(check());
}

fn check() -> i32 {
    if std::env::var("PAM_TYPE").as_deref() != Ok("auth") {
        return 1;
    }
    let Ok(user) = std::env::var("PAM_USER") else {
        return 1;
    };
    let mut authtok = Vec::new();
    if std::io::stdin().read_to_end(&mut authtok).is_err() {
        return 1;
    }
    // pam_exec may append a trailing NUL/newline to the authtok.
    while matches!(authtok.last(), Some(0) | Some(b'\n') | Some(b'\r')) {
        authtok.pop();
    }
    let path = token_path();
    // Hold the token lock across read+validate+consume so a second concurrent
    // check can't validate the same single-use token before we remove it. A
    // wrong guess (non-token authtok) leaves the token in place; only a match
    // or an expired token consumes it.
    let _lock = TokenLock::acquire(&path);
    let Ok(raw) = std::fs::read(&path) else {
        return 1;
    };
    let Ok(stored) = serde_json::from_slice::<StoredToken>(&raw) else {
        let _ = std::fs::remove_file(&path);
        return 1;
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(u64::MAX);
    if !validate(&stored, &user, &authtok, now) {
        if now > stored.expires {
            let _ = std::fs::remove_file(&path);
        }
        return 1;
    }
    let _ = std::fs::remove_file(&path);
    0
}

/// Pure validation: right user, unexpired, constant-time token equality.
fn validate(stored: &StoredToken, user: &str, authtok: &[u8], now: u64) -> bool {
    if now > stored.expires || stored.user != user {
        return false;
    }
    ct_eq(stored.token.as_bytes(), authtok)
}

/// Constant-time byte equality (for equal lengths; length itself is public —
/// minted tokens are always 64 hex chars).
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut d = 0u8;
    for (x, y) in a.iter().zip(b) {
        d |= x ^ y;
    }
    d == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stored() -> StoredToken {
        StoredToken {
            user: "meros".into(),
            token: "aa".repeat(32),
            expires: 1_000,
        }
    }

    #[test]
    fn valid_token_passes() {
        assert!(validate(&stored(), "meros", "aa".repeat(32).as_bytes(), 999));
        assert!(validate(&stored(), "meros", "aa".repeat(32).as_bytes(), 1_000));
    }

    #[test]
    fn expired_token_fails() {
        assert!(!validate(&stored(), "meros", "aa".repeat(32).as_bytes(), 1_001));
    }

    #[test]
    fn wrong_user_fails() {
        assert!(!validate(&stored(), "melvin", "aa".repeat(32).as_bytes(), 999));
    }

    #[test]
    fn wrong_or_truncated_token_fails() {
        assert!(!validate(&stored(), "meros", "ab".repeat(32).as_bytes(), 999));
        assert!(!validate(&stored(), "meros", b"aa", 999));
        assert!(!validate(&stored(), "meros", b"", 999));
    }

    #[test]
    fn ct_eq_basics() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"ab"));
        assert!(ct_eq(b"", b""));
    }
}
