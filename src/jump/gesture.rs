//! When to draw, when to move, and when to do neither.
//!
//! Kept as a state machine with no GTK in it, because the parts of this that
//! can go wrong are all about ordering - a release that beats the surface up,
//! a foreign binding landing mid-gesture, a grab left holding the keyboard
//! after the user walked away - and none of those are reachable from a test if
//! the logic lives inside widget callbacks. `mod.rs` translates GTK events
//! into [`Ev`] and applies [`Action`]s to widgets, and decides nothing.
//!
//! The rule the whole design rests on: **stepping never moves you.** No
//! workspace command is issued until the modifier is released. That is not
//! only about not disturbing the view. `task_state.rs` treats focusing a
//! waiting session's workspace as acknowledging it, permanently for that
//! episode - so a switcher that previewed by focusing would silently clear the
//! very "task 2 is waiting" state the row exists to show, for every place you
//! stepped past on the way. Commit-on-release makes that unreachable rather
//! than merely avoided.

/// What the outside world reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ev {
    /// `Super+Tab`.
    Step,
    /// `Super+Shift+Tab`.
    StepBack,
    /// Escape, through the surface's own grab. Not a sway binding.
    ///
    /// The only way to cancel. Something else moving you mid-gesture - sway
    /// running its own binding while the card is up - is not a cancel here:
    /// the gesture still ends normally and `mod.rs` refuses to run the
    /// command when the workspace it started on is no longer the one you are
    /// on. That guard catches every external move, not only the ones that
    /// came from a binding, and it needs no second event subscription.
    Escape,
    /// The modifier came up: commit.
    SuperReleased,
    /// Nothing at all happened for long enough that the grab is suspect.
    Watchdog,
}

/// What the surface should do about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Map the surface, invisible, and take the keyboard.
    Map,
    Unmap,
    Select(usize),
    /// Run this sway command. At most one per gesture, always last.
    Run(String),
}

#[derive(Debug, Default)]
pub struct Gesture {
    live: bool,
    cursor: usize,
    /// The command per row, captured when the gesture started. Held rather
    /// than re-read, so a workspace appearing mid-gesture cannot shift what
    /// the highlighted row commits to.
    commands: Vec<String>,
}

impl Gesture {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_live(&self) -> bool {
        self.live
    }

    #[cfg(test)]
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Feed one event, get the actions to apply, in order.
    ///
    /// `commands` is only read when a gesture starts; on every later event the
    /// captured list is used and the argument is ignored.
    pub fn on(&mut self, ev: Ev, commands: &[String]) -> Vec<Action> {
        match (self.live, ev) {
            // ── starting ────────────────────────────────────────────────
            (false, Ev::Step) => {
                if commands.is_empty() {
                    // Nowhere to go back to. No surface, so no grab to strand
                    // and no flash of an empty card.
                    return Vec::new();
                }
                self.live = true;
                self.cursor = 0;
                self.commands = commands.to_vec();
                vec![Action::Map, Action::Select(0)]
            }
            // A tap so short the surface never mapped. Nothing is live, so
            // there is nothing to commit and nothing to tidy up.
            (false, _) => Vec::new(),

            // ── walking ─────────────────────────────────────────────────
            (true, Ev::Step) => {
                // Clamped, not wrapped. Wrapping past the end lands you back
                // at the top of a list you were walking away from, which is
                // never what a held key means.
                self.cursor = (self.cursor + 1).min(self.commands.len() - 1);
                vec![Action::Select(self.cursor)]
            }
            (true, Ev::StepBack) => {
                self.cursor = self.cursor.saturating_sub(1);
                vec![Action::Select(self.cursor)]
            }

            // ── ending ──────────────────────────────────────────────────
            (true, Ev::SuperReleased) | (true, Ev::Watchdog) => {
                let command = self.commands.get(self.cursor).cloned();
                self.reset();
                // Unmap first: keyboard focus has to land on the destination,
                // not on a layer surface that is about to stop existing.
                let mut actions = vec![Action::Unmap];
                if let Some(c) = command {
                    actions.push(Action::Run(c));
                }
                actions
            }
            (true, Ev::Escape) => {
                self.reset();
                vec![Action::Unmap]
            }
        }
    }

    fn reset(&mut self) {
        self.live = false;
        self.cursor = 0;
        self.commands.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmds(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("workspace number {i}")).collect()
    }

    /// Run a script and collect every action, in order.
    fn run(n: usize, script: &[Ev]) -> Vec<Action> {
        let c = cmds(n);
        let mut g = Gesture::new();
        script.iter().flat_map(|e| g.on(*e, &c)).collect()
    }

    fn runs(actions: &[Action]) -> Vec<&str> {
        actions
            .iter()
            .filter_map(|a| match a {
                Action::Run(c) => Some(c.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_tap_goes_back_one_place() {
        let a = run(3, &[Ev::Step, Ev::SuperReleased]);
        assert_eq!(runs(&a), ["workspace number 0"]);
    }

    #[test]
    fn holding_walks_further_back_and_commits_once() {
        let a = run(5, &[Ev::Step, Ev::Step, Ev::Step, Ev::SuperReleased]);
        assert_eq!(
            runs(&a),
            ["workspace number 2"],
            "one command, the last row"
        );
    }

    #[test]
    fn stepping_never_moves_you() {
        // The rule task_state.rs's acking behaviour depends on. Every step
        // before the release must issue nothing at all.
        let a = run(5, &[Ev::Step, Ev::Step, Ev::Step]);
        assert!(runs(&a).is_empty(), "a step issued a command: {a:?}");
    }

    #[test]
    fn escape_leaves_you_where_you_started() {
        let a = run(4, &[Ev::Step, Ev::Step, Ev::Escape, Ev::SuperReleased]);
        assert!(runs(&a).is_empty());
        assert!(a.contains(&Action::Unmap));
    }

    #[test]
    fn a_release_that_outruns_the_surface_still_commits() {
        // The 40ms tap: the release arrives before anything is drawn.
        let a = run(3, &[Ev::Step, Ev::SuperReleased]);
        assert_eq!(runs(&a), ["workspace number 0"]);
    }

    #[test]
    fn a_stranded_grab_commits_and_lets_go() {
        let a = run(3, &[Ev::Step, Ev::Watchdog]);
        assert_eq!(runs(&a), ["workspace number 0"]);
        assert!(a.contains(&Action::Unmap));
    }

    #[test]
    fn nowhere_to_go_draws_nothing() {
        // A session with one workspace. No Map, so no grab exists to strand.
        assert!(run(0, &[Ev::Step]).is_empty());
        assert!(run(0, &[Ev::Step, Ev::SuperReleased]).is_empty());
    }

    #[test]
    fn the_cursor_clamps_at_both_ends() {
        let c = cmds(2);
        let mut g = Gesture::new();
        g.on(Ev::Step, &c);
        for _ in 0..10 {
            g.on(Ev::Step, &c);
        }
        assert_eq!(g.cursor(), 1, "clamped at the last row, not wrapped");
        for _ in 0..10 {
            g.on(Ev::StepBack, &c);
        }
        assert_eq!(g.cursor(), 0, "never steps past the first row");
    }

    #[test]
    fn step_back_while_idle_does_nothing() {
        assert!(run(4, &[Ev::StepBack]).is_empty());
    }

    #[test]
    fn the_gesture_is_reusable_and_starts_clean() {
        let c = cmds(4);
        let mut g = Gesture::new();
        g.on(Ev::Step, &c);
        g.on(Ev::Step, &c);
        g.on(Ev::SuperReleased, &c);
        assert!(!g.is_live());
        let second = g.on(Ev::Step, &c);
        assert_eq!(second, vec![Action::Map, Action::Select(0)]);
        assert_eq!(g.cursor(), 0, "a new gesture starts at the first row");
    }

    #[test]
    fn the_command_list_is_captured_when_the_gesture_starts() {
        // A workspace appearing mid-gesture must not change what the
        // highlighted row commits to.
        let mut g = Gesture::new();
        g.on(Ev::Step, &cmds(3));
        g.on(Ev::Step, &["different".to_string()]);
        let a = g.on(Ev::SuperReleased, &["different".to_string()]);
        assert_eq!(runs(&a), ["workspace number 1"]);
    }

    #[test]
    fn exactly_one_command_per_gesture_however_long_it_runs() {
        let script = [
            Ev::Step,
            Ev::Step,
            Ev::StepBack,
            Ev::Step,
            Ev::Step,
            Ev::StepBack,
            Ev::SuperReleased,
        ];
        assert_eq!(runs(&run(6, &script)).len(), 1);
    }
}
