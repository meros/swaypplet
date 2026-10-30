//! The session rail's actions: lock, suspend, log out, restart, shut down.
//! The power menu and the Helm's typed rows (`panel`) both run these, one
//! command each, whichever door it came through.

/// What the session rail does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Session {
    Lock,
    Suspend,
    Logout,
    Reboot,
    Poweroff,
}

impl Session {
    /// The command and its arguments.
    fn argv(self) -> (&'static str, &'static [&'static str]) {
        match self {
            Session::Lock => ("loginctl", &["lock-session"]),
            Session::Suspend => ("systemctl", &["suspend"]),
            Session::Logout => ("swaymsg", &["exit"]),
            Session::Reboot => ("systemctl", &["reboot"]),
            Session::Poweroff => ("systemctl", &["poweroff"]),
        }
    }

    /// Start the command and return; nothing waits on it.
    pub fn run(self) {
        // The render harness (dev/render.sh) runs the real binary as the
        // real user, and a typed "restart" there must not restart the
        // machine it runs on.
        if crate::systemd::dry_session() {
            log::info!("session: {self:?} (dry run)");
            return;
        }
        let (cmd, args) = self.argv();
        if let Err(e) = std::process::Command::new(cmd).args(args).spawn() {
            log::error!("Failed to spawn {} {:?}: {}", cmd, args, e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_action_runs_its_own_command() {
        assert_eq!(Session::Lock.argv(), ("loginctl", &["lock-session"][..]));
        assert_eq!(Session::Suspend.argv(), ("systemctl", &["suspend"][..]));
        assert_eq!(Session::Logout.argv(), ("swaymsg", &["exit"][..]));
        assert_eq!(Session::Reboot.argv(), ("systemctl", &["reboot"][..]));
        assert_eq!(Session::Poweroff.argv(), ("systemctl", &["poweroff"][..]));
    }
}
