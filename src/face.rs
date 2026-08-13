//! The confirm agent: the human half of face authentication for sudo and pkexec.
//!
//! Why this lives in the panel and not in PAM
//! ------------------------------------------
//! The obvious place to ask "press to confirm" is the PAM conversation, and
//! it is the wrong place. A PAM prompt is answerable by a pipe: `yes | sudo
//! -S id` satisfies it, and so does SUDO_ASKPASS pointing at a script. If the
//! confirmation lived there, a face merely being in front of the camera plus
//! a one-line shell pipeline would be a root shell.
//!
//! So faced collects the confirmation here instead, from an agent registered
//! over the session socket. A pipe cannot register one: it would have to be
//! running in the user's compositor session, holding a connection open, and
//! drawing something the user can see.
//!
//! What the daemon sends
//! ---------------------
//! Two stages on one held stream. `announce` when the camera opens, so the
//! user learns immediately that something asked for elevation and can see
//! what asked. `confirm` once the face has matched, which is when a press
//! becomes meaningful. `cancel` if the attempt ends first.
//!
//! The peer fields say which process is asking. They are the whole reason the
//! announce stage exists: a confirmation prompt with no attribution trains
//! the user to press a key whenever one appears, which is exactly the
//! behaviour an attacker wants.

// Not wired into the panel yet: nothing calls start(), so the protocol layer
// below is dead code on purpose.
//
// That is the safe state, not an oversight. faced refuses Verify(elevate)
// outright when no confirm agent is registered, and refuses it *before*
// opening the camera. An agent that registered but had no surface to draw
// would be strictly worse: the daemon would light the emitter, spend a full
// burst, wait out the six-second confirm window and then deny, every time.
// So this stays unregistered until there is a surface that can answer.
#![allow(dead_code)]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::mpsc::Sender;
use std::time::Duration;

const SOCKET: &str = "/run/faced/session.sock";
const INTERFACE: &str = "se.meros.Face1";

/// What the panel is asked to show.
#[derive(Clone, Debug)]
pub struct Request {
    pub id: String,
    pub stage: Stage,
    pub purpose: String,
    pub target_user: String,
    pub peer_exe: String,
    pub peer_cmdline: String,
    pub expires_ms: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stage {
    /// The camera has opened. Tell the user what is asking; no press yet.
    Announce,
    /// The face matched. A press now authorises.
    Confirm,
    /// The attempt ended without a decision. Take the surface down.
    Cancel,
}

impl Stage {
    fn parse(s: &str) -> Option<Stage> {
        match s {
            "announce" => Some(Stage::Announce),
            "confirm" => Some(Stage::Confirm),
            "cancel" => Some(Stage::Cancel),
            _ => None,
        }
    }
}

/// Minimal field extraction. The daemon's replies are flat and machine
/// generated, so this does not need a JSON parser, and pulling one in for a
/// handful of string fields would be the larger risk.
fn field<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("\"{key}\":\"");
    let start = json.find(&needle)? + needle.len();
    let rest = &json[start..];
    let bytes = rest.as_bytes();
    let mut end = 0;
    while end < bytes.len() {
        match bytes[end] {
            b'\\' => end += 2,
            b'"' => return Some(&rest[..end]),
            _ => end += 1,
        }
    }
    None
}

fn number(json: &str, key: &str) -> Option<u64> {
    let needle = format!("\"{key}\":");
    let start = json.find(&needle)? + needle.len();
    let rest = &json[start..];
    let end = rest.find(|c: char| !c.is_ascii_digit())?;
    rest[..end].parse().ok()
}

fn send(stream: &mut UnixStream, msg: &str) -> std::io::Result<()> {
    stream.write_all(msg.as_bytes())?;
    stream.write_all(&[0])?;
    stream.flush()
}

/// Answer a request. Opens its own connection, because the agent's own stream
/// is held open by the daemon for the life of the registration.
pub fn reply(id: &str, allow: bool) {
    let decision = if allow { "allow" } else { "deny" };
    let msg = format!(
        "{{\"method\":\"{INTERFACE}.ConfirmReply\",\"parameters\":\
         {{\"id\":\"{id}\",\"decision\":\"{decision}\"}}}}"
    );
    match UnixStream::connect(SOCKET).and_then(|mut s| send(&mut s, &msg)) {
        Ok(()) => log::info!("face: confirm {decision} for {id}"),
        Err(e) => log::warn!("face: could not answer confirm {id}: {e}"),
    }
}

/// Hold the confirm registration and forward requests to the panel.
fn run(tx: &Sender<Request>) -> std::io::Result<()> {
    let mut stream = UnixStream::connect(SOCKET)?;
    send(
        &mut stream,
        &format!(
            "{{\"method\":\"{INTERFACE}.RegisterAgent\",\"more\":true,\
             \"parameters\":{{\"role\":\"confirm\"}}}}"
        ),
    )?;

    let mut reader = BufReader::new(stream.try_clone()?);
    let mut frame = Vec::new();
    let mut registered = false;

    loop {
        frame.clear();
        // Frames are NUL separated; NUL cannot occur inside JSON text.
        if reader.read_until(0, &mut frame)? == 0 {
            return Ok(());
        }
        if frame.last() == Some(&0) {
            frame.pop();
        }
        let Ok(text) = std::str::from_utf8(&frame) else {
            continue;
        };
        if text.trim().is_empty() {
            continue;
        }
        if text.contains("\"error\"") {
            log::warn!("face: agent registration refused: {text}");
            return Ok(());
        }
        if !registered {
            registered = true;
            log::info!("face: confirm agent registered");
            continue;
        }

        let Some(stage) = field(text, "stage").and_then(Stage::parse) else {
            continue;
        };
        let Some(id) = field(text, "id") else { continue };
        let request = Request {
            id: id.to_string(),
            stage,
            purpose: field(text, "purpose").unwrap_or("elevate").to_string(),
            target_user: field(text, "target_user").unwrap_or("").to_string(),
            peer_exe: field(text, "exe").unwrap_or("").to_string(),
            peer_cmdline: field(text, "cmdline").unwrap_or("").to_string(),
            expires_ms: number(text, "expires_ms").unwrap_or(0),
        };
        if tx.send(request).is_err() {
            return Ok(());
        }
    }
}

/// Start the agent, reconnecting when the daemon restarts.
///
/// Reconnection matters more than it looks. faced is socket activated and may
/// legitimately come and go, and an agent that gave up after the first
/// disconnect would leave elevation permanently refused with nothing in the
/// journal to explain why.
pub fn start(tx: Sender<Request>) {
    std::thread::spawn(move || {
        let mut backoff = Duration::from_millis(500);
        loop {
            match run(&tx) {
                Ok(()) => backoff = Duration::from_millis(500),
                Err(e) => {
                    log::debug!("face: agent connection ended: {e}");
                    backoff = (backoff * 2).min(Duration::from_secs(30));
                }
            }
            std::thread::sleep(backoff);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"parameters":{"id":"q7","stage":"confirm","purpose":"elevate","target_user":"meros","peer":{"uid":0,"pid":31844,"exe":"/run/wrappers/bin/sudo","cmdline":"sudo systemctl restart foo"},"expires_ms":6000},"continues":true}"#;

    #[test]
    fn a_confirm_request_is_read_off_the_stream() {
        assert_eq!(field(SAMPLE, "id"), Some("q7"));
        assert_eq!(field(SAMPLE, "stage"), Some("confirm"));
        assert_eq!(field(SAMPLE, "target_user"), Some("meros"));
        assert_eq!(field(SAMPLE, "exe"), Some("/run/wrappers/bin/sudo"));
        assert_eq!(number(SAMPLE, "expires_ms"), Some(6000));
    }

    #[test]
    fn the_asking_process_survives_escaping() {
        // A cmdline can contain quotes, and the attribution shown to the user
        // is the thing an attacker most wants to corrupt.
        let msg = r#"{"cmdline":"sudo sh -c \"echo hi\"","id":"z1"}"#;
        assert_eq!(field(msg, "cmdline"), Some(r#"sudo sh -c \"echo hi\""#));
        assert_eq!(field(msg, "id"), Some("z1"));
    }

    #[test]
    fn an_unknown_stage_is_ignored_rather_than_guessed() {
        assert_eq!(Stage::parse("allow"), None);
        assert_eq!(Stage::parse("confirm"), Some(Stage::Confirm));
    }
}
