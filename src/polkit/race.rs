//! The pam_race side of the elevation card.
//!
//! `sudo` in a terminal and `pkexec` from anywhere are one question — may
//! this run as root? — and both run the same PAM module, pam_race (nixos
//! repo, pkgs/pam-race), which races the camera, the reader and the password.
//! What the two used to differ in was the surface: `pkexec` had this card
//! because polkit routes the conversation through the agent, and `sudo` had
//! the terminal plus, for a few seconds, a face-only card that vanished the
//! moment the camera gave up.
//!
//! pam_race now reports to this process directly, over a socket in the
//! session user's runtime directory, and that is what lets the card be the
//! same card for both. This module is that socket: it listens, checks that
//! the caller is root (pam_race runs inside setuid `sudo` and the setuid
//! polkit helper, and nothing else may start a card), parses the frames, and
//! hands the orchestrator one [`Message`] per event with a [`Link`] it can
//! answer on.
//!
//! What travels down the link and what does not
//! --------------------------------------------
//! Down: the answer to `begin` — whether a card was drawn and whether the
//! camera may be asked — and a password typed into the card. The password
//! is handled by pam_race exactly as one typed at the terminal: stored for
//! the stack, never checked by pam_race itself.
//!
//! Not down: the face confirm. That still goes through faced (`face.rs`),
//! because a PAM conversation is answerable by a pipe and the press must come
//! from somewhere a pipe cannot reach. A password is something a pipe could
//! already supply, so carrying one here changes nothing about what a pipe
//! can do.
//!
//! Lifetimes
//! ---------
//! One connection per PAM handle, not per prompt. sudo keeps its handle open
//! for the whole command and retries a wrong password on it, so a second
//! `begin` on the same link is a retry, `granted` is the stack accepting the
//! credentials (the only place a password's verdict is known), and the hangup
//! at `pam_end` is the transaction ending by any other route. The card needs
//! no clock of its own.

use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_channel::Sender;
use serde_json::Value;

/// The socket's name under `$XDG_RUNTIME_DIR`. pam_race hardcodes the same
/// path under `/run/user/<uid>`.
const SOCKET_NAME: &str = "pam-race.sock";

/// Frames larger than this are not from pam_race.
const FRAME_MAX: usize = 16 * 1024;

/// Which biometric a channel frame is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Face,
    Fp,
}

/// What ended the race in one authenticate call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Won {
    Face,
    Fp,
    /// A password was handed to the stack. Right or wrong is not known
    /// here: `Granted` or another `Begin` says which.
    Password,
    /// Every channel closed with nothing to hand on. pam_unix prompts on
    /// its own from here, and the card can do nothing more.
    None,
}

/// A transaction starting, or restarting after a wrong password.
#[derive(Clone, Debug)]
pub struct Begin {
    /// The account being authenticated as.
    pub user: String,
    /// The process pam_race runs in: `sudo` itself, or polkit's helper. The
    /// key that joins this to a polkit session and to a faced announce.
    pub pid: u32,
    pub exe: String,
    pub cmdline: String,
    /// Whether pam_race started a fingerprint helper. Not yet "the reader
    /// is armed"; that arrives as a channel frame.
    pub fp: bool,
}

#[derive(Clone, Debug)]
pub enum Event {
    Begin(Begin),
    /// A biometric came up or went away.
    Channel {
        channel: Channel,
        live: bool,
    },
    End {
        won: Won,
    },
    /// The stack accepted the credentials.
    Granted,
    /// pam_end, or the caller died. The transaction is over either way.
    Hangup,
}

/// One event, and the connection it came on.
pub struct Message {
    pub link: Link,
    pub event: Event,
}

/// The writing half of one connection. Cloned into the elevation that owns
/// it; every clone writes to the same socket.
#[derive(Clone)]
pub struct Link {
    id: u64,
    stream: Arc<UnixStream>,
}

impl Link {
    /// Identity across clones, for telling two connections apart.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// The answer to `begin`. `shown` false means no card and pam_race
    /// treats the agent as absent; `face` says whether the camera may be
    /// asked, which is how the setting reaches the module without a rebuild.
    pub fn card(&self, shown: bool, face: bool) {
        self.send(&format!(
            "{{\"event\":\"card\",\"shown\":{shown},\"face\":{face}}}"
        ));
    }

    /// A password typed into the card. Hex, so neither end needs a JSON
    /// string unescaper for the one value that must arrive intact.
    pub fn password(&self, password: &str) {
        let hex: String = password.bytes().map(|b| format!("{b:02x}")).collect();
        self.send(&format!("{{\"event\":\"password\",\"hex\":\"{hex}\"}}"));
    }

    /// Hang up. pam_race carries on as though there were no agent: the
    /// terminal prompt and the reader stay in the race.
    pub fn close(&self) {
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
    }

    fn send(&self, json: &str) {
        let mut w = &*self.stream;
        if let Err(e) = w
            .write_all(json.as_bytes())
            .and_then(|()| w.write_all(&[0]))
        {
            log::debug!("race: link {} write failed: {e}", self.id);
        }
    }
}

impl std::fmt::Debug for Link {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Link({})", self.id)
    }
}

fn socket_path() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR").map(|d| PathBuf::from(d).join(SOCKET_NAME))
}

/// The uid on the other end, or `None` when the kernel will not say.
fn peer_uid(stream: &UnixStream) -> Option<u32> {
    let mut cred = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let rc = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut cred as *mut libc::ucred).cast(),
            &mut len,
        )
    };
    (rc == 0).then_some(cred.uid)
}

/// Listen for pam_race and forward its frames. Returns false when the socket
/// could not be bound, in which case elevation behaves as it did without
/// this module: the terminal keeps `sudo`, and `pkexec` keeps its card
/// through the helper alone.
pub fn listen(tx: Sender<Message>) -> bool {
    let Some(path) = socket_path() else {
        log::warn!("race: no XDG_RUNTIME_DIR, not listening for pam_race");
        return false;
    };
    // A stale socket from an agent that died is ours to replace; a live one
    // belongs to another agent and the bind below says so.
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            log::warn!("race: cannot listen on {}: {e}", path.display());
            return false;
        }
    };
    log::info!("race: listening on {}", path.display());

    std::thread::Builder::new()
        .name("pam-race-listen".into())
        .spawn(move || {
            let next_id = AtomicU64::new(1);
            for conn in listener.incoming() {
                let stream = match conn {
                    Ok(s) => s,
                    Err(e) => {
                        log::warn!("race: accept: {e}");
                        continue;
                    }
                };
                // Only root starts a card. pam_race runs inside setuid
                // `sudo` and the setuid polkit helper; a same-uid process
                // could otherwise draw a card and be sent the password.
                match peer_uid(&stream) {
                    Some(0) => {}
                    other => {
                        log::warn!("race: refused a connection from uid {other:?}");
                        continue;
                    }
                }
                let link = Link {
                    id: next_id.fetch_add(1, Ordering::Relaxed),
                    stream: Arc::new(stream),
                };
                let tx = tx.clone();
                let name = format!("pam-race-{}", link.id);
                let spawned = std::thread::Builder::new()
                    .name(name)
                    .spawn(move || serve(link, tx));
                if let Err(e) = spawned {
                    log::warn!("race: cannot spawn a reader: {e}");
                }
            }
        })
        .map(|_| true)
        .unwrap_or_else(|e| {
            log::warn!("race: cannot spawn the listener: {e}");
            false
        })
}

/// Read one connection to its end, forwarding each frame.
fn serve(link: Link, tx: Sender<Message>) {
    let mut reader = &*link.stream;
    let mut buf: Vec<u8> = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    loop {
        let n = match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                log::debug!("race: link {} read: {e}", link.id);
                break;
            }
        };
        buf.extend_from_slice(&chunk[..n]);
        while let Some(pos) = buf.iter().position(|&b| b == 0) {
            let frame: Vec<u8> = buf.drain(..=pos).collect();
            match parse(&frame[..frame.len() - 1]) {
                Some(event) => {
                    if tx
                        .send_blocking(Message {
                            link: link.clone(),
                            event,
                        })
                        .is_err()
                    {
                        return;
                    }
                }
                None => log::debug!(
                    "race: link {} sent something that is not an event: {}",
                    link.id,
                    String::from_utf8_lossy(&frame)
                ),
            }
        }
        if buf.len() > FRAME_MAX {
            log::warn!(
                "race: link {} is not speaking the protocol; dropping it",
                link.id
            );
            break;
        }
    }
    link.close();
    let _ = tx.send_blocking(Message {
        link,
        event: Event::Hangup,
    });
}

fn parse(frame: &[u8]) -> Option<Event> {
    let v: Value = serde_json::from_slice(frame).ok()?;
    let str_of = |key: &str| v.get(key).and_then(Value::as_str).unwrap_or("").to_string();
    match v.get("event")?.as_str()? {
        "begin" => Some(Event::Begin(Begin {
            user: str_of("user"),
            pid: v.get("pid")?.as_u64()? as u32,
            exe: str_of("exe"),
            cmdline: str_of("cmdline"),
            fp: v.get("fp").and_then(Value::as_bool).unwrap_or(false),
        })),
        "channel" => {
            let channel = match v.get("channel")?.as_str()? {
                "face" => Channel::Face,
                "fp" => Channel::Fp,
                _ => return None,
            };
            let live = match v.get("state")?.as_str()? {
                "live" => true,
                "closed" => false,
                _ => return None,
            };
            Some(Event::Channel { channel, live })
        }
        "end" => {
            let won = match v.get("won").and_then(Value::as_str).unwrap_or("none") {
                "face" => Won::Face,
                "fp" => Won::Fp,
                "password" => Won::Password,
                _ => Won::None,
            };
            Some(Event::End { won })
        }
        "granted" => Some(Event::Granted),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_begin_frame_carries_the_caller() {
        let e = parse(
            br#"{"event":"begin","user":"meros","pid":4242,"exe":"/nix/store/x-sudo/bin/sudo","cmdline":"sudo systemctl restart foo","fp":true}"#,
        );
        let Some(Event::Begin(b)) = e else {
            panic!("not a begin: {e:?}");
        };
        assert_eq!(b.user, "meros");
        assert_eq!(b.pid, 4242);
        assert!(b.fp);
        assert_eq!(b.cmdline, "sudo systemctl restart foo");
    }

    #[test]
    fn channel_and_end_frames_are_typed() {
        assert!(matches!(
            parse(br#"{"event":"channel","channel":"fp","state":"live"}"#),
            Some(Event::Channel {
                channel: Channel::Fp,
                live: true
            })
        ));
        assert!(matches!(
            parse(br#"{"event":"channel","channel":"face","state":"closed"}"#),
            Some(Event::Channel {
                channel: Channel::Face,
                live: false
            })
        ));
        assert!(matches!(
            parse(br#"{"event":"end","won":"password"}"#),
            Some(Event::End { won: Won::Password })
        ));
        assert!(matches!(
            parse(br#"{"event":"granted"}"#),
            Some(Event::Granted)
        ));
    }

    #[test]
    fn an_unknown_frame_is_ignored_rather_than_guessed() {
        assert!(parse(br#"{"event":"card","shown":true}"#).is_none());
        assert!(parse(br#"{"event":"channel","channel":"voice","state":"live"}"#).is_none());
        assert!(parse(b"not json").is_none());
    }

    #[test]
    fn a_password_goes_out_as_hex() {
        let (a, b) = UnixStream::pair().unwrap();
        let link = Link {
            id: 7,
            stream: Arc::new(a),
        };
        link.password("pä\"ss");
        link.card(true, false);
        let mut got = Vec::new();
        drop(link);
        let mut r = &b;
        r.read_to_end(&mut got).unwrap();
        let text = String::from_utf8(got).unwrap();
        let frames: Vec<&str> = text.split('\0').filter(|f| !f.is_empty()).collect();
        assert_eq!(
            frames,
            [
                r#"{"event":"password","hex":"70c3a4227373"}"#,
                r#"{"event":"card","shown":true,"face":false}"#
            ]
        );
    }
}
