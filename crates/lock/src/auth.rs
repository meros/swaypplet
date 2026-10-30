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

/// One unlock, whoever gets there first, and never before the lock is real.
///
/// Password, fingerprint, face and logind's Unlock all end the lock. The
/// first to succeed wins: `unlock()` consumes the session-lock object, so a
/// second call is a protocol error that takes the locker down with it, and
/// the face path waits `face_settle` before it ends the lock, long enough
/// for a fingerprint touch or a queued Enter to arrive too.
///
/// The compositor confirms the lock (`locked`) only after the entrance, and
/// until then the session-lock object ignores an unlock: an Enter in that
/// window would wedge the session locked behind a UI that believes it is
/// done. Such an unlock waits for the confirmation instead.
#[derive(Default)]
pub struct UnlockGate {
    locked: bool,
    pending: bool,
    unlocking: bool,
}

/// What [`UnlockGate::end`] asks of the lock.
#[derive(Debug, PartialEq, Eq)]
pub enum End {
    /// Fade out and unlock now.
    Now,
    /// Hurry the entrance; [`UnlockGate::on_locked`] unlocks.
    WhenLocked,
}

/// What [`UnlockGate::on_locked`] asks of the lock.
#[derive(Debug, PartialEq, Eq)]
pub enum Locked {
    /// An unlock waited for this: end the lock now.
    Unlock,
    /// An unlock is under way: arm nothing (no camera, no reader).
    Quiet,
    /// Locked and waiting: arm the authenticators.
    Arm,
}

impl UnlockGate {
    /// An authenticator succeeded. `true` for the first one only; every
    /// later one must do nothing.
    pub fn claim(&mut self) -> bool {
        !std::mem::replace(&mut self.unlocking, true)
    }

    /// The winner ends the lock.
    pub fn end(&mut self) -> End {
        if self.locked {
            End::Now
        } else {
            self.pending = true;
            End::WhenLocked
        }
    }

    /// The compositor confirmed the lock.
    pub fn on_locked(&mut self) -> Locked {
        self.locked = true;
        if std::mem::take(&mut self.pending) {
            Locked::Unlock
        } else if self.unlocking {
            Locked::Quiet
        } else {
            Locked::Arm
        }
    }
}

/// Blocking PAM authenticate + account check. Run on a worker thread only —
/// pam_unix deliberately delays ~2s on failure, which would freeze the UI.
pub fn pam_verify(user: &str, password: &str) -> Result<(), String> {
    let mut client =
        pam::Client::with_password(PAM_SERVICE).map_err(|e| format!("PAM init failed: {e}"))?;
    // Authenticate only — no session is opened, so nothing to close on drop.
    client.close_on_drop = false;
    client.conversation_mut().set_credentials(user, password);
    client.authenticate().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_first_authenticator_unlocks() {
        let mut gate = UnlockGate::default();
        assert_eq!(gate.on_locked(), Locked::Arm);
        assert!(gate.claim());
        assert!(!gate.claim(), "a fingerprint after the password");
        assert_eq!(gate.end(), End::Now);
    }

    #[test]
    fn an_unlock_before_the_lock_is_real_waits_for_it() {
        let mut gate = UnlockGate::default();
        assert!(gate.claim());
        assert_eq!(gate.end(), End::WhenLocked);
        assert_eq!(gate.on_locked(), Locked::Unlock);
        assert_eq!(gate.end(), End::Now);
    }

    #[test]
    fn a_lock_confirmed_mid_unlock_arms_nothing() {
        let mut gate = UnlockGate::default();
        assert!(gate.claim());
        // The winner has not called end() yet (the face settle).
        assert_eq!(gate.on_locked(), Locked::Quiet);
        assert_eq!(gate.end(), End::Now);
    }

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
