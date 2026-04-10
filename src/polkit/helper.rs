//! Subprocess wrapper around `polkit-agent-helper-1`.
//!
//! The helper is a small SUID-root binary shipped by polkit that performs
//! the actual PAM conversation. It's the trusted boundary that lets an
//! unprivileged agent (us) authenticate the user without ever touching
//! libpam ourselves — fingerprint, password, hardware tokens all flow
//! through whatever PAM stack the host has configured.
//!
//! ## Wire protocol
//!
//! Spawn:  `polkit-agent-helper-1 <username>`
//!
//! First thing we write to its stdin: `<cookie>\n`
//!
//! Then it streams stdout lines until exit:
//! - `PAM_PROMPT_ECHO_OFF <prompt>` — expect a hidden response (password)
//! - `PAM_PROMPT_ECHO_ON  <prompt>` — expect a visible response
//! - `PAM_ERROR_MSG       <msg>`    — error to display
//! - `PAM_TEXT_INFO       <msg>`    — info to display ("Place finger on
//!   fingerprint reader" comes through here)
//! - `SUCCESS`                      — auth succeeded; helper has already
//!   called AuthenticationAgentResponse2 on the polkit authority
//! - `FAILURE`                      — auth failed; helper exits
//!
//! We respond to PROMPT lines by writing `<response>\n` to stdin.

use std::io::Write;
use std::os::fd::{AsRawFd, RawFd};
use std::path::Path;
use std::process::{Child, Command, Stdio};

#[derive(Debug, Clone)]
pub enum HelperEvent {
    /// Hidden prompt — typically the password.
    PromptEchoOff(String),
    /// Visible prompt — uncommon but supported (e.g. one-time codes).
    PromptEchoOn(String),
    /// PAM error line, surfaced to the user in red.
    Error(String),
    /// PAM informational line. Fingerprint hints arrive here, e.g.
    /// "Place finger on fingerprint reader".
    Info(String),
    /// Authentication succeeded. The helper has already informed polkit.
    Success,
    /// Authentication failed. The helper is about to exit.
    Failure,
}

/// Locations to look for `polkit-agent-helper-1`. Override with the env
/// var `SWAYPPLET_POLKIT_HELPER` for development.
const HELPER_PATHS: &[&str] = &[
    "/run/wrappers/bin/polkit-agent-helper-1", // NixOS setuid wrapper
    "/usr/lib/polkit-1/polkit-agent-helper-1", // Arch, Debian
    "/usr/libexec/polkit-1/polkit-agent-helper-1", // Fedora, RHEL
    "/usr/lib/polkit-agent/polkit-agent-helper-1", // Older layouts
];

pub fn helper_path() -> Option<String> {
    if let Ok(p) = std::env::var("SWAYPPLET_POLKIT_HELPER") {
        if Path::new(&p).exists() {
            return Some(p);
        }
        log::warn!(
            "SWAYPPLET_POLKIT_HELPER points to non-existent path: {p}; \
             falling back to defaults"
        );
    }
    HELPER_PATHS
        .iter()
        .find(|p| Path::new(p).exists())
        .map(|s| s.to_string())
}

pub struct Helper {
    child: Child,
    stdout_fd: RawFd,
    stdout_buf: Vec<u8>,
}

impl Helper {
    /// Spawn the helper for `username` and write `cookie` to its stdin.
    pub fn spawn(username: &str, cookie: &str) -> std::io::Result<Self> {
        let path = helper_path().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "polkit-agent-helper-1 not found in known locations \
                 (set SWAYPPLET_POLKIT_HELPER to override)",
            )
        })?;

        log::debug!("spawning polkit helper: {path} {username}");

        let mut child = Command::new(&path)
            .arg(username)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        // The helper expects the cookie as the very first line on stdin.
        if let Some(stdin) = child.stdin.as_mut() {
            stdin.write_all(cookie.as_bytes())?;
            stdin.write_all(b"\n")?;
            stdin.flush()?;
        }

        // Make stdout non-blocking so we can drain it from the GTK main
        // loop without ever stalling.
        let stdout_fd = child
            .stdout
            .as_ref()
            .ok_or_else(|| {
                std::io::Error::other("polkit helper stdout not piped")
            })?
            .as_raw_fd();
        unsafe {
            let flags = libc::fcntl(stdout_fd, libc::F_GETFL);
            if flags >= 0 {
                libc::fcntl(stdout_fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
            }
        }

        Ok(Helper {
            child,
            stdout_fd,
            stdout_buf: Vec::with_capacity(256),
        })
    }

    /// Raw fd of the helper's stdout — used to register a glib watch.
    pub fn stdout_raw_fd(&self) -> RawFd {
        self.stdout_fd
    }

    /// Drain stdout into the line buffer and emit complete-line events.
    /// The boolean is `true` when the helper has closed stdout (EOF).
    pub fn read_events(&mut self) -> (Vec<HelperEvent>, bool) {
        let mut events = Vec::new();
        let mut chunk = [0u8; 1024];
        let mut eof = false;

        loop {
            let n = unsafe {
                libc::read(
                    self.stdout_fd,
                    chunk.as_mut_ptr() as *mut _,
                    chunk.len(),
                )
            };
            if n == 0 {
                eof = true;
                break;
            }
            if n < 0 {
                let err = std::io::Error::last_os_error();
                match err.kind() {
                    std::io::ErrorKind::WouldBlock => break,
                    std::io::ErrorKind::Interrupted => continue,
                    _ => {
                        log::warn!("polkit helper stdout read error: {err}");
                        eof = true;
                        break;
                    }
                }
            }
            self.stdout_buf.extend_from_slice(&chunk[..n as usize]);
        }

        while let Some(pos) = self.stdout_buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.stdout_buf.drain(..=pos).collect();
            let trimmed = String::from_utf8_lossy(&line[..line.len() - 1])
                .trim_end_matches('\r')
                .to_string();
            if let Some(event) = parse_line(&trimmed) {
                log::debug!("polkit helper -> {trimmed}");
                events.push(event);
            } else if !trimmed.is_empty() {
                log::debug!("polkit helper (unparsed) -> {trimmed}");
            }
        }

        (events, eof)
    }

    /// Send a response line back to the helper.
    pub fn send_response(&mut self, response: &str) -> std::io::Result<()> {
        if let Some(stdin) = self.child.stdin.as_mut() {
            stdin.write_all(response.as_bytes())?;
            stdin.write_all(b"\n")?;
            stdin.flush()?;
        }
        Ok(())
    }
}

impl Drop for Helper {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn parse_line(line: &str) -> Option<HelperEvent> {
    if let Some(rest) = line.strip_prefix("PAM_PROMPT_ECHO_OFF ") {
        Some(HelperEvent::PromptEchoOff(rest.to_string()))
    } else if let Some(rest) = line.strip_prefix("PAM_PROMPT_ECHO_ON ") {
        Some(HelperEvent::PromptEchoOn(rest.to_string()))
    } else if let Some(rest) = line.strip_prefix("PAM_ERROR_MSG ") {
        Some(HelperEvent::Error(rest.to_string()))
    } else if let Some(rest) = line.strip_prefix("PAM_TEXT_INFO ") {
        Some(HelperEvent::Info(rest.to_string()))
    } else if line == "SUCCESS" {
        Some(HelperEvent::Success)
    } else if line == "FAILURE" {
        Some(HelperEvent::Failure)
    } else {
        None
    }
}

/// Heuristic: does this PAM info/prompt line refer to the fingerprint reader?
pub fn is_fingerprint_hint(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("finger")
        || lower.contains("swipe")
        || lower.contains("touch the sensor")
        || lower.contains("fprint")
}
