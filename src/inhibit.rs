//! Session inhibitors — the standing "don't do that" switches.
//!
//! Three of them ship: **Awake** (the idle manager itself), **Stay Lit**
//! (the compositor's idle inhibitor) and **Clamshell** (logind's lid-switch
//! handling). They look alike in the UI and are nothing alike underneath,
//! which is exactly why they belong in one module: each one's real state
//! lives in a *different* authority outside this process, and every consumer
//! would otherwise have to learn which.
//!
//! | Inhibitor | Authority | Survives a panel restart |
//! |-----------|-----------|--------------------------|
//! | Awake     | the `swaypplet-idle.service` user unit | yes |
//! | Stay Lit  | sway's per-view idle inhibitors | yes |
//! | Clamshell | a transient user unit holding a logind block inhibitor | yes |
//!
//! Keeping the state outside is the point, not an accident. Three processes
//! care — the panel toggles, the bar's hazard lane reports, and the idle
//! manager's absence policy obeys — and none of them share memory. An
//! authority every one of them can already reach is cheaper than a fourth
//! IPC channel, and it is the only version that survives one of the three
//! restarting.
//!
//! This module provides the ACTIONS ([`Inhibitor::arm`]) and the readings
//! ([`Inhibitor::read`], [`idle_inhibited`]). It holds no policy: what to do
//! when idle is inhibited is the idle manager's business (idle/mod.rs), and
//! how to draw an armed inhibitor is the widgets' (widgets/tiles.rs,
//! bar/hazards.rs). The in-process [`Observed`] cells below are a cache of
//! established readings for the GTK side, never a second source of truth.
//!
//! # Established, not optimistic
//!
//! Every [`Inhibitor::arm`] confirms by reading the authority back, and the
//! exit status of whatever it ran is a log line rather than the verdict.
//! Both tools lie in the direction that matters: `systemctl --user stop`
//! exits 5 for a unit that is not loaded (goal already reached, reported as
//! failure) and `swaymsg` exits 2 when criteria match no window (nothing to
//! inhibit, likewise). Trusting either put the tile back to the state the
//! user had just left.
//!
//! # A note on Awake
//!
//! Awake stops the idle manager outright, and the idle manager is also what
//! locks the session before sleep. Armed, a lid close therefore suspends an
//! *unlocked* machine — the sleep path has nobody holding logind's delay
//! inhibitor. That is what the tile has always done (it was "Caffeine"); the
//! tooltip now says so.

use std::cell::Cell;
use std::process::Command;

use swayipc::{Connection, Node};

use crate::service::Observed;

/// The idle manager's user unit. Awake stops it.
const IDLE_UNIT: &str = "swaypplet-idle.service";

/// Transient user unit holding the logind lid-switch block inhibitor.
/// `systemd-run --collect` so a failed run tidies itself up instead of
/// leaving a unit name that the next arm cannot reuse.
const CLAMSHELL_UNIT: &str = "swaypplet-clamshell-inhibit.service";

/// Criteria matching every view in the tree.
///
/// Stay Lit arms and disarms through the same criteria on purpose. sway has
/// no session-wide idle inhibitor — `inhibit_idle` applies to a container —
/// and targeting the *focused* one made the switch asymmetric: arming over
/// one window and disarming over another left the first one inhibiting
/// forever, whereupon the tile read itself back on. Which container is
/// focused when the click lands is also not the one the user is looking at,
/// since the panel is a layer surface and sway falls back to the last
/// focused view.
const EVERY_VIEW: &str = r#"[title=".*"]"#;

/// `open` rather than `focus`: Stay Lit means "hold the screen", not "hold
/// it while this window happens to be focused".
const STAY_LIT_ON: &str = "open";
const STAY_LIT_OFF: &str = "none";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Inhibitor {
    Awake,
    StayLit,
    Clamshell,
}

pub use Inhibitor::{Awake, Clamshell, StayLit};

impl Inhibitor {
    /// Display order, and the index into the [`Observed`] registry.
    pub const ALL: [Inhibitor; 3] = [Awake, StayLit, Clamshell];

    pub fn icon(self) -> &'static str {
        match self {
            Awake => "󰅶",
            StayLit => "󰍹",
            Clamshell => "󰌢",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Awake => "Awake",
            StayLit => "Stay Lit",
            Clamshell => "Clamshell",
        }
    }

    /// The one wording for an armed inhibitor. Tile and hazard glyph share
    /// it; they used to carry near-identical strings that had already drifted
    /// apart, and one of them was wrong about what Awake does.
    pub fn tooltip_on(self) -> &'static str {
        match self {
            Awake => "Awake: idle manager stopped — no idle lock, blank or suspend",
            StayLit => "Stay Lit: idle timers held off, screen stays lit",
            Clamshell => "Clamshell: lid close no longer suspends",
        }
    }

    pub fn tooltip_off(self) -> &'static str {
        match self {
            Awake => "Awake: off",
            StayLit => "Stay Lit: off",
            Clamshell => "Clamshell: off",
        }
    }

    /// Read the authority. `None` means it could not be reached at all — the
    /// tile shows disabled rather than guessing a state.
    ///
    /// Blocking; call from a background thread (or the idle manager's own
    /// loop, which is not a UI thread).
    pub fn read(self) -> Option<bool> {
        match self {
            // Inverted: the tile is armed when the idle manager is NOT
            // running. LoadState guards the unit-does-not-exist case, where
            // ActiveState would also say "inactive" and light the tile up.
            Awake => match unit_state(IDLE_UNIT) {
                UnitState::Active => Some(false),
                UnitState::Inactive => Some(true),
                // The unit not existing is not "the idle manager is off" —
                // it is a machine this tile cannot speak for.
                UnitState::NotLoaded | UnitState::NoSystemctl => None,
            },
            StayLit => tree(|root| any_node(root, user_inhibitor)),
            Clamshell => match unit_state(CLAMSHELL_UNIT) {
                UnitState::Active => Some(true),
                // A transient unit is *supposed* not to exist when it is
                // off, so not-loaded is the off state rather than a fault.
                UnitState::Inactive | UnitState::NotLoaded => Some(false),
                UnitState::NoSystemctl => None,
            },
        }
    }

    /// Flip the inhibitor and report whether the authority now agrees.
    /// Blocking; call from a background thread.
    pub fn arm(self, on: bool) -> bool {
        match self {
            Awake => run(
                "awake",
                Command::new("systemctl").args([
                    "--user",
                    if on { "stop" } else { "start" },
                    IDLE_UNIT,
                ]),
            ),
            StayLit => run(
                "stay-lit",
                Command::new("swaymsg").args([
                    "-q",
                    EVERY_VIEW,
                    "inhibit_idle",
                    if on { STAY_LIT_ON } else { STAY_LIT_OFF },
                ]),
            ),
            Clamshell if on => run(
                "clamshell",
                Command::new("systemd-run").args([
                    "--user",
                    "--quiet",
                    "--collect",
                    &format!("--unit={CLAMSHELL_UNIT}"),
                    "--description=swaypplet clamshell mode (lid-switch inhibitor)",
                    "systemd-inhibit",
                    "--what=handle-lid-switch",
                    "--who=swaypplet",
                    "--why=Clamshell mode",
                    "--mode=block",
                    "sleep",
                    "infinity",
                ]),
            ),
            Clamshell => run(
                "clamshell",
                Command::new("systemctl").args(["--user", "stop", CLAMSHELL_UNIT]),
            ),
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
    static STATE: [Observed<bool>; 3] = [
        Observed::new(false),
        Observed::new(false),
        Observed::new(false),
    ];
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

/// Whether anything is inhibiting idle in the compositor right now —
/// Stay Lit, or an application holding idle-inhibit-v1 (a video player).
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
/// an application's. Stay Lit sets these; video players set the other kind.
fn user_inhibitor(node: &Node) -> bool {
    node.idle_inhibitors
        .as_ref()
        .is_some_and(|i| i.user != swayipc::UserIdleInhibitType::None)
}

/// Run `f` over a fresh tree snapshot. `None` if sway could not be reached.
fn tree<R>(f: impl FnOnce(&Node) -> R) -> Option<R> {
    match Connection::new().and_then(|mut c| c.get_tree()) {
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
    /// The unit does not exist. Meaningful for a transient unit (that is
    /// its off state) and a fault for a packaged one.
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
}
