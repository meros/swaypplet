//! The canned nx run the tab plays under `SWAYPPLET_SYSTEM_FIXTURE`, which
//! never starts the real one (`swaypplet_system::nx`).

use swaypplet_system::nx::{Action, EXIT, START};

/// The canned run, one line per tick.
pub const FIXTURE_LINES: &[&str] = &[
    "── apply",
    "building the system configuration...",
    "activating the configuration...",
    "setting up /etc...",
    "reloading user units for meros...",
    "restarting the following units: home-manager-meros.service",
    "<<< /nix/var/nix/profiles/system-1149-link",
    ">>> /nix/var/nix/profiles/system-1150-link",
    "Version changes:",
    "[U*]  #1  swaypplet  0.1.0 (3a815c4) -> 0.1.0 (7374675)",
];

/// The fixture's log after `ticks` ticks of `action`; it ends with exit 0.
pub fn fixture_log(action: Action, ticks: usize) -> String {
    let mut text = format!("{START}{}\n", action.arg());
    for line in FIXTURE_LINES.iter().take(ticks) {
        text.push_str(line);
        text.push('\n');
    }
    if ticks > FIXTURE_LINES.len() {
        text.push_str(EXIT);
        text.push_str("0\n");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use swaypplet_system::nx::parse_log;

    #[test]
    fn the_fixture_plays_to_a_success() {
        let mid = parse_log(&fixture_log(Action::Apply, 3));
        assert_eq!(mid.exit, None);
        assert_eq!(mid.tail.len(), 3);
        let end = parse_log(&fixture_log(Action::Apply, FIXTURE_LINES.len() + 1));
        assert_eq!(end.exit, Some(0));
        assert_eq!(end.action, Some(Action::Apply));
    }
}
