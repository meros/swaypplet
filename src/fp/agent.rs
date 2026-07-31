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
//! The verify loop (the shared [`crate::fp::verify_engine`]) is gated on the
//! *client's* logind session being active: a backgrounded greeter (user
//! switched VTs) releases the reader so the now-active session's locker — or
//! another greeter — can claim it. Several greeters may be connected at once;
//! the active-session gate ensures at most one holds the device.
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
use tokio::sync::{Semaphore, mpsc, watch};

use super::{EngineEvent, Flow, SessionGate, verify_engine, watch_session_active, watch_sleep};

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

    // Gate on the *client's* session, never the daemon's own (it has none).
    // Without a peer pid we can't resolve a session — keep the sender alive in
    // a parked task holding `true`, so the engine's `active.changed()` select
    // arm never spins on a closed channel.
    let (active_tx, active_rx) = watch::channel(true);
    let watcher = match peer_pid {
        Some(pid) => tokio::spawn(watch_session_active(
            conn.clone(),
            SessionGate::Client(pid),
            active_tx,
        )),
        None => {
            log::warn!("fp-agent: no peer pid; cannot gate on client session, treating as active");
            tokio::spawn(async move {
                let _ = active_tx.send(true);
                std::future::pending::<()>().await;
            })
        }
    };

    let (target_tx, target_rx) = watch::channel::<Option<String>>(None);

    // The engine reports progress here; on a match we mint a token and keep
    // going (a greeter may retarget), so the sink returns `Continue`, never
    // `Stop` — the client's disconnect (dropping `target_tx`) is what ends it.
    let out = out_tx.clone();
    let sink = move |ev: EngineEvent| {
        match ev {
            EngineEvent::Ready => {
                let _ = out.send(Ev::Ready);
            }
            EngineEvent::Hint(msg) => {
                let _ = out.send(Ev::Hint { msg });
            }
            EngineEvent::Unavailable(msg) => {
                let _ = out.send(Ev::Unavailable { msg });
            }
            EngineEvent::Match(user) => match mint_token(&user) {
                Ok(token) => {
                    log::info!("fingerprint match for {user}, token minted");
                    let _ = out.send(Ev::Match { user, token });
                }
                Err(e) => {
                    log::error!("token mint failed: {e}");
                    let _ = out.send(Ev::Unavailable {
                        msg: "token mint failed".into(),
                    });
                }
            },
        }
        Flow::Continue
    };
    let mut verifier = tokio::spawn(verify_engine(conn, target_rx, active_rx, sleeping, sink));

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
    // any running verify, release the claim, and exit. Engine teardown runs
    // on timed D-Bus calls, so this resolves in seconds even against a
    // stalled fprintd; the abort is the backstop that keeps a hung teardown
    // from stranding this client slot (one of MAX_CLIENTS) and its claim.
    drop(target_tx);
    if tokio::time::timeout(Duration::from_secs(10), &mut verifier)
        .await
        .is_err()
    {
        log::warn!("fp-agent: verify teardown overran 10s, aborting it");
        verifier.abort();
    }
    watcher.abort();
    writer.abort();
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
        assert!(validate(
            &stored(),
            "meros",
            "aa".repeat(32).as_bytes(),
            999
        ));
        assert!(validate(
            &stored(),
            "meros",
            "aa".repeat(32).as_bytes(),
            1_000
        ));
    }

    #[test]
    fn expired_token_fails() {
        assert!(!validate(
            &stored(),
            "meros",
            "aa".repeat(32).as_bytes(),
            1_001
        ));
    }

    #[test]
    fn wrong_user_fails() {
        assert!(!validate(
            &stored(),
            "melvin",
            "aa".repeat(32).as_bytes(),
            999
        ));
    }

    #[test]
    fn wrong_or_truncated_token_fails() {
        assert!(!validate(
            &stored(),
            "meros",
            "ab".repeat(32).as_bytes(),
            999
        ));
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
