//! Session inhibitors — the standing "don't do that" switches.
//!
//! Two of them ship, one per thing the machine does on its own that a user
//! sometimes wants stopped:
//!
//! | Inhibitor | Stops | Authority |
//! |-----------|-------|-----------|
//! | No Sleep  | suspending: logind's lid handling and the idle manager's suspend tier | a transient user unit holding a logind `handle-lid-switch` block inhibitor |
//! | No Lock   | locking by itself: the idle lock tier, the walk-away lock, and the dim that leads to them | a transient user unit holding a logind `idle` block inhibitor |
//!
//! Neither one blocks an *explicit* request. `systemctl suspend` from the
//! power menu still sleeps the machine under No Sleep, and
//! `loginctl lock-session`, the lid-close binding, a VT switch and the
//! before-sleep lock still lock the session under No Lock. These switches
//! are about what happens when nobody asked, which is the only part that
//! ever surprises anyone.
//!
//! # Established, not optimistic
//!
//! Both inhibitors keep their real state in a transient systemd user unit
//! rather than in this process, and that is the design rather than an
//! accident. Three processes care — the panel toggles, the bar's hazard lane
//! reports, and the idle manager obeys — and none of them share memory. An
//! authority all three can already reach is cheaper than a third IPC channel,
//! and it is the only version that survives one of the three restarting.
//!
//! So every [`Inhibitor::arm`] confirms by reading the authority back, and
//! the exit status of whatever it ran is a log line rather than the verdict.
//! `systemctl --user stop` exits 5 for a unit that is not loaded — goal
//! already reached, reported as failure — and trusting that put the tile
//! back to the state the user had just left.
//!
//! `systemd-inhibit` also makes both switches visible outside this project:
//! they appear in `systemd-inhibit --list` with a name and a reason, rather
//! than being a flag only swaypplet knows how to read. No Lock's `idle`
//! inhibitor is honest about what it means (this session is not to be treated
//! as idle) and costs nothing on this host, where logind's own `IdleAction`
//! is `ignore`.
//!
//! This module provides the ACTIONS ([`Inhibitor::arm`]) and the readings
//! ([`Inhibitor::read`], [`Inhibitor::armed`], [`idle_inhibited`]). It holds
//! no policy: what to do when one is armed is the idle manager's business
//! (idle/mod.rs), and how to draw an armed inhibitor is the widgets'
//! (widgets/tiles.rs, bar/hazards.rs). The in-process [`Observed`] cells
//! below are a cache of established readings for the GTK side, never a second
//! source of truth.

use std::cell::Cell;
use std::process::Command;

use swayipc::Node;

use crate::service::Observed;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Inhibitor {
    NoSleep,
    NoLock,
}

pub use Inhibitor::{NoLock, NoSleep};

impl Inhibitor {
    /// Display order, and the index into the [`Observed`] registry.
    pub const ALL: [Inhibitor; 2] = [NoSleep, NoLock];

    /// The transient unit that holds this inhibitor. `systemd-run --collect`,
    /// so a failed run tidies itself up instead of leaving a unit name the
    /// next arm cannot reuse.
    fn unit(self) -> &'static str {
        match self {
            NoSleep => "swaypplet-nosleep-inhibit.service",
            NoLock => "swaypplet-nolock-inhibit.service",
        }
    }

    /// What the held logind inhibitor blocks. For No Sleep this is the whole
    /// mechanism; for No Lock it is a truthful marker, since the tiers it
    /// stops are this project's own and logind knows nothing about them.
    fn what(self) -> &'static str {
        match self {
            NoSleep => "handle-lid-switch",
            NoLock => "idle",
        }
    }

    /// Log scope, so a journal grep finds one switch and not the other.
    fn scope(self) -> &'static str {
        match self {
            NoSleep => "no-sleep",
            NoLock => "no-lock",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            // md-sleep-off, md-lock-open: both switches say the machine is
            // *not* doing its usual thing, which is what the bar's hazard
            // lane has to convey with no label beside it.
            NoSleep => "󰒳",
            NoLock => "󰌿",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            NoSleep => "No Sleep",
            NoLock => "No Lock",
        }
    }

    /// The one wording for an armed inhibitor. Tile and hazard glyph share
    /// it; they used to carry near-identical strings that had already drifted
    /// apart, and one of them was wrong about what its switch did.
    pub fn tooltip_on(self) -> &'static str {
        match self {
            NoSleep => "No Sleep: lid close and idle no longer suspend",
            NoLock => "No Lock: no idle lock, no walk-away lock, no dim",
        }
    }

    pub fn tooltip_off(self) -> &'static str {
        match self {
            NoSleep => "No Sleep: off",
            NoLock => "No Lock: off",
        }
    }

    /// Read the authority. `None` means it could not be reached at all — the
    /// tile shows disabled rather than guessing a state.
    ///
    /// Blocking; call from a background thread (or the idle manager's own
    /// loop, which is not a UI thread).
    pub fn read(self) -> Option<bool> {
        match unit_state(self.unit()) {
            UnitState::Active => Some(true),
            // A transient unit is *supposed* not to exist when it is off, so
            // not-loaded is the off state rather than a fault.
            UnitState::Inactive | UnitState::NotLoaded => Some(false),
            UnitState::NoSystemctl => None,
        }
    }

    /// [`read`](Self::read) as a policy answer: an authority we cannot reach
    /// is not an armed inhibitor.
    ///
    /// That default is deliberate and it is the safe one both times. An
    /// unreadable No Sleep suspends, an unreadable No Lock locks — the
    /// machine falls back to protecting itself, never to sitting unlocked
    /// because a subprocess failed to spawn.
    pub fn armed(self) -> bool {
        self.read() == Some(true)
    }

    /// Flip the inhibitor and report whether the authority now agrees.
    /// Blocking; call from a background thread.
    pub fn arm(self, on: bool) -> bool {
        if on {
            let args: Vec<String> = vec![
                "--user".into(),
                "--quiet".into(),
                "--collect".into(),
                format!("--unit={}", self.unit()),
                format!(
                    "--description=swaypplet {} ({} inhibitor)",
                    self.label(),
                    self.what()
                ),
                "systemd-inhibit".into(),
                format!("--what={}", self.what()),
                "--who=swaypplet".into(),
                format!("--why={} mode", self.label()),
                "--mode=block".into(),
                "sleep".into(),
                "infinity".into(),
            ];
            run(self.scope(), Command::new("systemd-run").args(&args));
        } else {
            run(
                self.scope(),
                Command::new("systemctl").args(["--user", "stop", self.unit()]),
            );
        }

        // The authority decides, not the exit status. See the module docs.
        let established = self.read();
        if established != Some(on) {
            log::warn!(
                "inhibit[{}]: arm({on}) did not take (now {established:?})",
                self.label()
            );
        }
        established == Some(on)
    }
}

// ── Established state, for the GTK side ─────────────────────────────────

thread_local! {
    /// One cell per [`Inhibitor::ALL`] entry, indexed by `as usize`. Fed by
    /// whoever establishes a reading (the panel's tiles, [`prime`]); read by
    /// the bar's hazard lane. Main thread only, like every widget consumer.
    static STATE: [Observed<bool>; 2] = [Observed::new(false), Observed::new(false)];
    /// [`prime`] runs once per process, not once per bar window.
    static PRIMED: Cell<bool> = const { Cell::new(false) };
}

/// Borrow an inhibitor's state cell (to read it or to observe changes).
pub fn observed<R>(which: Inhibitor, f: impl FnOnce(&Observed<bool>) -> R) -> R {
    STATE.with(|cells| f(&cells[which as usize]))
}

/// Publish an established reading. No-op when nothing changed.
pub fn publish(which: Inhibitor, armed: bool) {
    observed(which, |cell| cell.set_if_changed(armed));
}

/// Read every inhibitor once and publish, so a process that shows inhibitor
/// state without ever toggling one (the standalone `swaypplet bar`) starts
/// out correct instead of blind until the first toggle.
pub fn prime() {
    if PRIMED.with(|p| p.replace(true)) {
        return;
    }
    for which in Inhibitor::ALL {
        crate::spawn::spawn_work(
            move || which.read(),
            move |state| {
                if let Some(armed) = state {
                    publish(which, armed);
                }
            },
        );
    }
}

// ── Compositor idle inhibition ──────────────────────────────────────────

/// Whether anything is inhibiting idle in the compositor right now — an
/// application holding idle-inhibit-v1 (a video player), or a container the
/// user pointed sway's `inhibit_idle` command at.
///
/// The idle manager's timeout tiers are suppressed by this for free, since
/// `get_idle_notification` honours inhibitors (idle/wayland.rs). Its
/// *absence* policy is not: walking away is not idle, so nothing suppresses
/// it, and it has to ask.
///
/// Blocking (one IPC round trip, single-digit milliseconds). Called on
/// absence edges, not on a tick.
pub fn idle_inhibited() -> bool {
    tree(|root| any_node(root, |n| n.inhibit_idle == Some(true) || user_inhibitor(n)))
        .unwrap_or(false)
}

/// A user-set (`inhibit_idle` command) inhibitor on this node, as opposed to
/// an application's. Nothing in this project sets these any more; a sway
/// keybind or a `swaymsg` by hand still can, and the absence path should
/// honour one exactly like a video player's.
fn user_inhibitor(node: &Node) -> bool {
    node.idle_inhibitors
        .as_ref()
        .is_some_and(|i| i.user != swayipc::UserIdleInhibitType::None)
}

/// Run `f` over a fresh tree snapshot. `None` if sway could not be reached.
fn tree<R>(f: impl FnOnce(&Node) -> R) -> Option<R> {
    match crate::sway_ipc::connect().and_then(|mut c| c.get_tree()) {
        Ok(root) => Some(f(&root)),
        Err(e) => {
            log::warn!("inhibit: sway get_tree failed: {e}");
            None
        }
    }
}

fn any_node(node: &Node, pred: impl Fn(&Node) -> bool + Copy) -> bool {
    pred(node)
        || node
            .nodes
            .iter()
            .chain(node.floating_nodes.iter())
            .any(|child| any_node(child, pred))
}

// ── systemd ─────────────────────────────────────────────────────────────

enum UnitState {
    Active,
    Inactive,
    /// The unit does not exist. For a transient unit that is its off state.
    NotLoaded,
    /// `systemctl` could not be run at all.
    NoSystemctl,
}

fn unit_state(unit: &str) -> UnitState {
    let out = Command::new("systemctl")
        .args([
            "--user",
            "show",
            "-p",
            "LoadState",
            "-p",
            "ActiveState",
            unit,
        ])
        .output();
    let out = match out {
        Ok(out) => out,
        Err(e) => {
            log::warn!("inhibit: systemctl show {unit} failed: {e}");
            return UnitState::NoSystemctl;
        }
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let prop = |key: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(key)?.strip_prefix('='))
            .unwrap_or_default()
            .to_string()
    };
    match (prop("LoadState").as_str(), prop("ActiveState").as_str()) {
        ("loaded", "active") => UnitState::Active,
        ("loaded", _) => UnitState::Inactive,
        _ => UnitState::NotLoaded,
    }
}

/// Run a command to completion, logging what it did. The caller decides
/// success by reading the authority back, so this returns nothing.
fn run(scope: &str, cmd: &mut Command) {
    match cmd.status() {
        Ok(st) if st.success() => log::info!("inhibit[{scope}]: {:?} ok", cmd.get_program()),
        Ok(st) => log::info!("inhibit[{scope}]: {:?} exited {st}", cmd.get_program()),
        Err(e) => log::warn!(
            "inhibit[{scope}]: {:?} failed to spawn: {e}",
            cmd.get_program()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_inhibitor_indexes_its_own_state_cell() {
        for (i, which) in Inhibitor::ALL.iter().enumerate() {
            assert_eq!(*which as usize, i, "{} is out of order", which.label());
        }
    }

    #[test]
    fn labels_and_tooltips_are_distinct() {
        let labels: Vec<_> = Inhibitor::ALL.iter().map(|i| i.label()).collect();
        let mut sorted = labels.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), labels.len());
        for which in Inhibitor::ALL {
            assert_ne!(which.tooltip_on(), which.tooltip_off());
        }
    }

    /// Two switches sharing one transient unit would have either one's disarm
    /// silently turn the other off.
    #[test]
    fn every_inhibitor_owns_its_own_unit() {
        let units: Vec<_> = Inhibitor::ALL.iter().map(|i| i.unit()).collect();
        let mut sorted = units.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), units.len());
    }
}
