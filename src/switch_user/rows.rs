//! Pure picker logic for `swaypplet switch-user`: who is logged in, which
//! greeter to reuse, which VT is free, where the cycle goes next.
//!
//! Everything here is a function of a session snapshot plus static config, so
//! the whole contract is testable without a bus. This replaces the jq filter
//! (`modules/nixos/desktop/switch-user-list.jq`) and the `nix flake check`
//! that guarded it — those cases live on as tests at the bottom of this file.

use super::SwitchUser;

/// One logind session, reduced to what the picker needs.
#[derive(Debug, Clone)]
pub struct Session {
    /// logind session id ("2", "c1").
    pub session: String,
    /// The session's user (logind's `Name`).
    pub user: String,
    /// logind's `TTY`: "tty8", "ttyS0", "pts/2", or "" when there is none.
    pub tty: String,
    /// logind's `Class`: "user", "greeter", "manager"…
    pub class: String,
}

impl Session {
    /// Occupies a terminal of some kind. Sessions without one (D-Bus
    /// activation, `user@` manager scopes) are not switch targets and don't
    /// make anyone count as logged in.
    fn seated(&self) -> bool {
        !self.tty.is_empty()
    }

    /// The VT this session sits on, when it sits on one at all.
    ///
    /// A serial console ("ttyS0") or an ssh pty ("pts/2") has no VT, and must
    /// yield `None` rather than erroring: one stray ssh login must never take
    /// the picker down with it. (This is the `tonumber?` the jq filter needed;
    /// here it falls out of `parse().ok()`.)
    pub fn vt(&self) -> Option<u32> {
        self.tty.strip_prefix("tty")?.parse().ok()
    }
}

/// The session that makes `user` count as logged in: their first seated one,
/// in logind's own listing order.
pub fn session_for<'a>(sessions: &'a [Session], user: &str) -> Option<&'a Session> {
    sessions.iter().find(|s| s.user == user && s.seated())
}

/// Build one picker row.
///
/// `fp` is tri-state on purpose: `Some` is a definite enrollment verdict,
/// `None` means the host couldn't tell (fprintd cold, no default device). A
/// `None` must never collapse to `Some(false)` downstream — a false negative
/// there tears down an already-armed greeter reader.
pub fn row(
    sessions: &[Session],
    user: &str,
    current: &str,
    fp: Option<bool>,
    icon: Option<String>,
) -> SwitchUser {
    let sess = session_for(sessions, user);
    SwitchUser {
        user: user.to_string(),
        current: user == current,
        logged_in: sess.is_some(),
        fingerprint: fp,
        icon,
        vt: sess.and_then(Session::vt),
        session: sess.map(|s| s.session.clone()),
    }
}

/// The greeter to hand a switch off to: the newest seated greeter session.
///
/// logind allocates session ids ascending, so the highest is the freshest.
/// Non-numeric ids ("c1") sort as 0 and so only win when there's nothing else
/// — deliberate, they're the boot-time consoles, not switch greeters.
pub fn idle_greeter(sessions: &[Session]) -> Option<&Session> {
    sessions
        .iter()
        .filter(|s| s.class == "greeter" && s.seated())
        .max_by_key(|s| s.session.parse::<u64>().unwrap_or(0))
}

/// The first spare VT with nothing on it, for starting a fresh greeter.
pub fn free_vt(sessions: &[Session], spare: &[u32]) -> Option<u32> {
    spare.iter().copied().find(|vt| {
        let name = format!("tty{vt}");
        !sessions.iter().any(|s| s.tty == name)
    })
}

/// Whether any configured user still has no session — the cycle offers a
/// greeter slot before it wraps, so a logged-out user is reachable by
/// repeating the switch key instead of only via the picker.
pub fn anyone_logged_out(sessions: &[Session], users: &[String]) -> bool {
    users.iter().any(|u| session_for(sessions, u).is_none())
}

/// Seated sessions that live on a real VT, in VT order — the switch cycle.
fn cycle_order(sessions: &[Session]) -> Vec<(u32, &Session)> {
    let mut rows: Vec<_> = sessions.iter().filter_map(|s| Some((s.vt()?, s))).collect();
    rows.sort_by_key(|(vt, _)| *vt);
    rows
}

/// The next live session above `my_vt`.
pub fn next_above(sessions: &[Session], my_vt: u32) -> Option<&Session> {
    cycle_order(sessions)
        .into_iter()
        .find(|(vt, _)| *vt > my_vt)
        .map(|(_, s)| s)
}

/// The lowest-VT session that isn't ours — where the cycle wraps to.
pub fn wrap_target(sessions: &[Session], my_vt: u32) -> Option<&Session> {
    cycle_order(sessions)
        .into_iter()
        .find(|(vt, _)| *vt != my_vt)
        .map(|(_, s)| s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sess(session: &str, user: &str, tty: &str, class: &str) -> Session {
        Session {
            session: session.into(),
            user: user.into(),
            tty: tty.into(),
            class: class.into(),
        }
    }

    /// The snapshot the old `nix flake check` contract test used, verbatim:
    /// a serial console, a normal VT, and an ssh pty.
    fn contract() -> Vec<Session> {
        vec![
            sess("c1", "meros", "ttyS0", "user"),
            sess("c2", "melvin", "tty8", "user"),
            sess("c3", "alice", "pts/2", "user"),
        ]
    }

    // --- ported from checks.switch-user-list-contract -----------------------

    #[test]
    fn serial_console_has_no_vt_but_is_logged_in() {
        let r = row(&contract(), "meros", "meros", None, None);
        assert!(r.logged_in);
        assert_eq!(r.vt, None);
        assert!(r.current);
    }

    #[test]
    fn normal_vt_is_parsed_out() {
        let r = row(&contract(), "melvin", "meros", None, None);
        assert!(r.logged_in);
        assert_eq!(r.vt, Some(8));
        assert!(!r.current);
    }

    #[test]
    fn ssh_pty_has_no_vt_but_is_logged_in() {
        let r = row(&contract(), "alice", "meros", None, None);
        assert!(r.logged_in);
        assert_eq!(r.vt, None);
    }

    #[test]
    fn logged_out_user_has_no_session() {
        let r = row(&contract(), "ghost", "meros", None, None);
        assert!(!r.logged_in);
        assert_eq!(r.vt, None);
        assert_eq!(r.session, None);
    }

    #[test]
    fn fingerprint_tri_state_passes_through() {
        for fp in [Some(true), Some(false), None] {
            assert_eq!(row(&contract(), "meros", "meros", fp, None).fingerprint, fp);
        }
    }

    // --- logic the jq filter never covered ----------------------------------

    #[test]
    fn sessions_without_a_tty_dont_count_as_logged_in() {
        let s = vec![sess("5", "meros", "", "user")];
        assert!(!row(&s, "meros", "meros", None, None).logged_in);
    }

    #[test]
    fn newest_greeter_wins_and_non_greeters_are_ignored() {
        let s = vec![
            sess("2", "greeter", "tty8", "greeter"),
            sess("9", "melvin", "tty9", "user"),
            sess("7", "greeter", "tty10", "greeter"),
        ];
        assert_eq!(idle_greeter(&s).unwrap().session, "7");
    }

    #[test]
    fn a_greeter_without_a_tty_is_not_a_target() {
        let s = vec![sess("7", "greeter", "", "greeter")];
        assert!(idle_greeter(&s).is_none());
    }

    #[test]
    fn free_vt_skips_occupied_and_respects_configured_order() {
        let s = vec![sess("2", "meros", "tty8", "user")];
        assert_eq!(free_vt(&s, &[8, 9, 10]), Some(9));
        assert_eq!(free_vt(&s, &[8]), None);
    }

    #[test]
    fn a_serial_session_never_occupies_a_spare_vt() {
        // "ttyS0" must not be read as VT 0 or, worse, match "tty8" loosely.
        let s = vec![sess("c1", "meros", "ttyS0", "user")];
        assert_eq!(free_vt(&s, &[8, 9]), Some(8));
    }

    #[test]
    fn cycle_goes_up_then_wraps_to_the_lowest_other_vt() {
        let s = vec![
            sess("3", "melvin", "tty9", "user"),
            sess("1", "meros", "tty2", "user"),
            sess("5", "greeter", "tty10", "greeter"),
        ];
        assert_eq!(next_above(&s, 2).unwrap().session, "3");
        assert_eq!(next_above(&s, 9).unwrap().session, "5");
        assert!(next_above(&s, 10).is_none());
        assert_eq!(wrap_target(&s, 10).unwrap().session, "1");
    }

    #[test]
    fn cycle_ignores_sessions_that_arent_on_a_vt() {
        let s = vec![
            sess("c1", "meros", "ttyS0", "user"),
            sess("c3", "alice", "pts/2", "user"),
        ];
        assert!(next_above(&s, 0).is_none());
        assert!(wrap_target(&s, 0).is_none());
    }

    #[test]
    fn logged_out_detection_drives_the_greeter_slot() {
        let users = vec!["meros".to_string(), "melvin".to_string()];
        let both = vec![
            sess("1", "meros", "tty2", "user"),
            sess("3", "melvin", "tty9", "user"),
        ];
        assert!(!anyone_logged_out(&both, &users));
        assert!(anyone_logged_out(&both[..1], &users));
    }
}
