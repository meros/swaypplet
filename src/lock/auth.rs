//! Password authentication for the lockscreen.
//!
//! Two pieces: `AttemptGate`, a pure state machine that serializes password
//! attempts (unit-tested), and `pam_verify`, the blocking libpam call that
//! runs on a worker thread via `spawn::spawn_work`.

/// PAM service name. NixOS registers it via
/// `security.pam.services.swaypplet-lock`; other distros drop a file in
/// /etc/pam.d. A missing service falls through to PAM's `other` stack,
/// which denies — fail-closed.
pub const PAM_SERVICE: &str = "swaypplet-lock";

/// Serializes password attempts: one in flight at a time, empty input ignored
/// (mirrors swaylock's `ignore-empty-password` — a stray Enter on the lock
/// screen must not burn a PAM round-trip or flash an error).
#[derive(Default)]
pub struct AttemptGate {
    in_flight: bool,
    failures: u32,
}

impl AttemptGate {
    /// Returns `true` if a verify may start for this input.
    pub fn try_begin(&mut self, password: &str) -> bool {
        if self.in_flight || password.is_empty() {
            return false;
        }
        self.in_flight = true;
        true
    }

    /// Records the attempt outcome; returns the consecutive failure count.
    pub fn finish(&mut self, ok: bool) -> u32 {
        self.in_flight = false;
        if ok {
            self.failures = 0;
        } else {
            self.failures += 1;
        }
        self.failures
    }
}

/// Blocking PAM authenticate + account check. Run on a worker thread only —
/// pam_unix deliberately delays ~2s on failure, which would freeze the UI.
pub fn pam_verify(user: &str, password: &str) -> Result<(), String> {
    let mut client = pam::Client::with_password(PAM_SERVICE)
        .map_err(|e| format!("PAM init failed: {e}"))?;
    // Authenticate only — no session is opened, so nothing to close on drop.
    client.close_on_drop = false;
    client.conversation_mut().set_credentials(user, password);
    client.authenticate().map_err(|e| e.to_string())
}

/// The user we authenticate as: passwd entry for our uid (authoritative),
/// `$USER` as fallback for odd environments.
pub fn current_username() -> Option<String> {
    // SAFETY: getpwuid returns a pointer to static storage (or NULL); we only
    // read pw_name and copy it out before any other libc passwd call.
    unsafe {
        let pw = libc::getpwuid(libc::getuid());
        if !pw.is_null()
            && let Ok(name) = std::ffi::CStr::from_ptr((*pw).pw_name).to_str()
            && !name.is_empty()
        {
            return Some(name.to_string());
        }
    }
    std::env::var("USER").ok().filter(|u| !u.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_password_is_ignored() {
        let mut gate = AttemptGate::default();
        assert!(!gate.try_begin(""));
        // and it must not block a later real attempt
        assert!(gate.try_begin("hunter2"));
    }

    #[test]
    fn second_submit_blocked_while_in_flight() {
        let mut gate = AttemptGate::default();
        assert!(gate.try_begin("first"));
        assert!(!gate.try_begin("second"));
    }

    #[test]
    fn failure_count_increments_and_success_resets() {
        let mut gate = AttemptGate::default();
        assert!(gate.try_begin("a"));
        assert_eq!(gate.finish(false), 1);
        assert!(gate.try_begin("b"));
        assert_eq!(gate.finish(false), 2);
        assert!(gate.try_begin("c"));
        assert_eq!(gate.finish(true), 0);
    }

    #[test]
    fn gate_reopens_after_finish() {
        let mut gate = AttemptGate::default();
        assert!(gate.try_begin("a"));
        gate.finish(false);
        assert!(gate.try_begin("b"));
    }
}
