//! The protocol between the idle supervisor (`idle::locker`) and the locker
//! it runs (`swaypplet lock`): one command line in, two lines out, a handful
//! of environment variables and three exit codes. Both ends read them from
//! here, so neither can drift from the other.
//!
//! A pre-warmed locker is spawned with [`ENV_WAIT`], prints [`READY`] once
//! its first window has been paid for, and parks on stdin until the
//! supervisor writes a [`LockCmd`]. A locker spawned cold gets the same
//! facts from [`ENV_REASON`] and [`ENV_FADE`] instead. Either way it prints
//! [`LOCKED`] once the compositor has confirmed the lock, and exits with one
//! of the codes below.

/// Set on a pre-warmed locker: warm up, then wait for a [`LockCmd`] on stdin.
pub const ENV_WAIT: &str = "SWAYPPLET_LOCK_WAIT";
/// Why a cold-spawned locker is locking: `idle`, `manual`, `sleep`, ...
pub const ENV_REASON: &str = "SWAYPPLET_LOCK_REASON";
/// `0` suppresses a cold-spawned locker's cross-fade; anything else keeps it.
pub const ENV_FADE: &str = "SWAYPPLET_LOCK_FADE";

/// The pre-warmed locker has paid for its first window.
pub const READY: &str = "READY";
/// The compositor confirmed the lock. Only this counts as locked.
pub const LOCKED: &str = "LOCKED";

/// Clean unlock: successful auth, or the compositor ended the lock.
pub const EXIT_UNLOCKED: i32 = 0;
/// Anything else, crash-ish. The supervisor relaunches.
pub const EXIT_ERROR: i32 = 1;
/// Lock unavailable: protocol unsupported, or another client holds it.
/// Relaunching would spin.
pub const EXIT_UNAVAILABLE: i32 = 2;

/// Reason given when the command or the environment named none.
pub const UNKNOWN_REASON: &str = "unknown";

/// The `LOCK <reason> [nofade]` line the supervisor writes to a pre-warmed
/// locker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockCmd {
    pub reason: String,
    pub fade: bool,
}

impl LockCmd {
    /// The line, without its newline.
    pub fn format(&self) -> String {
        let suffix = if self.fade { "" } else { " nofade" };
        format!("LOCK {}{suffix}", self.reason)
    }

    /// Read a line the supervisor wrote. Lenient on purpose: whatever
    /// arrives, the locker locks, because a lock screen that refused a
    /// malformed command would leave the session open. The first word is
    /// skipped unread, a missing reason is [`UNKNOWN_REASON`], and `nofade`
    /// counts anywhere after the reason.
    pub fn parse(line: &str) -> Self {
        let mut words = line.split_whitespace();
        let _ = words.next(); // "LOCK"
        let reason = words.next().unwrap_or("");
        let fade = !words.any(|w| w == "nofade");
        Self {
            reason: if reason.is_empty() {
                UNKNOWN_REASON.into()
            } else {
                reason.into()
            },
            fade,
        }
    }
}

/// Whether a line the locker printed is the lock confirmation.
pub fn is_locked(line: &str) -> bool {
    line.trim() == LOCKED
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_survives_the_pipe() {
        for cmd in [
            LockCmd {
                reason: "idle".into(),
                fade: true,
            },
            LockCmd {
                reason: "sleep".into(),
                fade: false,
            },
        ] {
            assert_eq!(LockCmd::parse(&format!("{}\n", cmd.format())), cmd);
        }
    }

    #[test]
    fn the_wire_format_is_what_older_lockers_read() {
        let cmd = LockCmd {
            reason: "switch".into(),
            fade: false,
        };
        assert_eq!(cmd.format(), "LOCK switch nofade");
    }

    #[test]
    fn a_bare_command_still_locks() {
        assert_eq!(
            LockCmd::parse("LOCK\n"),
            LockCmd {
                reason: UNKNOWN_REASON.into(),
                fade: true
            }
        );
        assert_eq!(LockCmd::parse("").reason, UNKNOWN_REASON);
    }

    #[test]
    fn only_the_locked_line_confirms_the_lock() {
        assert!(is_locked("LOCKED"));
        assert!(is_locked("  LOCKED \n"));
        assert!(!is_locked(READY));
        assert!(!is_locked("NOT LOCKED"));
    }
}
