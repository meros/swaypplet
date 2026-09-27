//! "Keep this layout?": the safety net under the Displays tab's Apply.
//!
//! A layout can turn the only screen dark, and then nobody can press
//! anything. So an applied layout is on trial: it stands only when Keep is
//! pressed within [`SECONDS`]; otherwise, or when the page goes away (the
//! panel closed, another tab), the layout from before is sent back. A page
//! that goes away while the layout is still on its way reverts the moment
//! it lands.
//!
//! Pure: the pane feeds it what happened and does what it returns.

use crate::services::displays::Outcome;

/// How long a new layout has to be kept.
pub const SECONDS: u32 = 15;

#[derive(Debug, Clone, PartialEq)]
enum State<P> {
    Idle,
    /// Sent, no answer yet. `leave` set when the page went away meanwhile.
    Applying {
        previous: P,
        leave: bool,
    },
    Confirming {
        previous: P,
        left: u32,
    },
    /// The layout from before is on its way back.
    Reverting {
        why: Why,
    },
}

/// Why a layout was sent back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    Asked,
    TimedOut,
    Left,
}

/// What the pane does next.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect<P> {
    Nothing,
    /// Show the question with this many seconds left, and tick each second.
    Ask(u32),
    /// Send `P`, the layout from before, and report back with `reverted`.
    Revert(P),
    /// It is over: say so, and let the tab be edited again.
    Done(End),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum End {
    Kept,
    /// The compositor refused the new layout; nothing changed.
    Refused,
    /// The outputs changed while it was on its way; nothing changed.
    Stale,
    Reverted(Why),
    /// The layout from before could not be put back.
    RevertFailed,
    /// The set of outputs changed during the question: the profiles take
    /// over, and there is nothing sensible to put back.
    Replugged,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Keep<P> {
    state: State<P>,
}

impl<P: Clone> Default for Keep<P> {
    fn default() -> Self {
        Keep { state: State::Idle }
    }
}

impl<P: Clone> Keep<P> {
    /// Nothing in flight and nothing asked: the tab can be edited.
    pub fn idle(&self) -> bool {
        matches!(self.state, State::Idle)
    }

    /// A layout was sent; `previous` is what to put back. Refused (false)
    /// unless idle.
    pub fn sent(&mut self, previous: P) -> bool {
        if !self.idle() {
            return false;
        }
        self.state = State::Applying {
            previous,
            leave: false,
        };
        true
    }

    /// The compositor answered the layout being tried, or the revert.
    pub fn answered(&mut self, outcome: Outcome) -> Effect<P> {
        match std::mem::replace(&mut self.state, State::Idle) {
            State::Applying { previous, leave } => match outcome {
                Outcome::Succeeded if leave => {
                    self.state = State::Reverting { why: Why::Left };
                    Effect::Revert(previous)
                }
                Outcome::Succeeded => {
                    self.state = State::Confirming {
                        previous,
                        left: SECONDS,
                    };
                    Effect::Ask(SECONDS)
                }
                Outcome::Failed => Effect::Done(End::Refused),
                Outcome::Cancelled => Effect::Done(End::Stale),
            },
            State::Reverting { why } => match outcome {
                Outcome::Succeeded => Effect::Done(End::Reverted(why)),
                _ => Effect::Done(End::RevertFailed),
            },
            other => {
                self.state = other;
                Effect::Nothing
            }
        }
    }

    /// A second went by.
    pub fn tick(&mut self) -> Effect<P> {
        match &mut self.state {
            State::Confirming { left, .. } if *left > 1 => {
                *left -= 1;
                Effect::Ask(*left)
            }
            State::Confirming { .. } => self.revert(Why::TimedOut),
            _ => Effect::Nothing,
        }
    }

    /// Keep was pressed.
    pub fn keep(&mut self) -> Effect<P> {
        match self.state {
            State::Confirming { .. } => {
                self.state = State::Idle;
                Effect::Done(End::Kept)
            }
            _ => Effect::Nothing,
        }
    }

    /// Revert was pressed.
    pub fn undo(&mut self) -> Effect<P> {
        self.revert(Why::Asked)
    }

    /// The page went away: revert now, or as soon as the layout lands.
    pub fn left(&mut self) -> Effect<P> {
        match &mut self.state {
            State::Applying { leave, .. } => {
                *leave = true;
                Effect::Nothing
            }
            _ => self.revert(Why::Left),
        }
    }

    /// A display was plugged in or out. During the question the profiles
    /// take over; the layout from before names outputs that changed.
    pub fn replugged(&mut self) -> Effect<P> {
        match self.state {
            State::Confirming { .. } => {
                self.state = State::Idle;
                Effect::Done(End::Replugged)
            }
            _ => Effect::Nothing,
        }
    }

    fn revert(&mut self, why: Why) -> Effect<P> {
        match std::mem::replace(&mut self.state, State::Idle) {
            State::Confirming { previous, .. } => {
                self.state = State::Reverting { why };
                Effect::Revert(previous)
            }
            other => {
                self.state = other;
                Effect::Nothing
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trying() -> Keep<&'static str> {
        let mut k = Keep::default();
        assert!(k.sent("before"));
        assert!(!k.sent("again"), "one at a time");
        k
    }

    #[test]
    fn kept_within_the_time_it_stands() {
        let mut k = trying();
        assert_eq!(k.answered(Outcome::Succeeded), Effect::Ask(SECONDS));
        assert_eq!(k.tick(), Effect::Ask(SECONDS - 1));
        assert_eq!(k.keep(), Effect::Done(End::Kept));
        assert!(k.idle());
        assert_eq!(k.tick(), Effect::Nothing);
    }

    #[test]
    fn unanswered_it_goes_back_when_the_time_is_up() {
        let mut k = trying();
        k.answered(Outcome::Succeeded);
        for left in (1..SECONDS).rev() {
            assert_eq!(k.tick(), Effect::Ask(left));
        }
        assert_eq!(k.tick(), Effect::Revert("before"));
        assert_eq!(k.keep(), Effect::Nothing, "too late to keep");
        assert_eq!(
            k.answered(Outcome::Succeeded),
            Effect::Done(End::Reverted(Why::TimedOut))
        );
        assert!(k.idle());
    }

    #[test]
    fn leaving_the_page_reverts_now_or_when_it_lands() {
        let mut k = trying();
        k.answered(Outcome::Succeeded);
        assert_eq!(k.left(), Effect::Revert("before"));
        assert_eq!(
            k.answered(Outcome::Succeeded),
            Effect::Done(End::Reverted(Why::Left))
        );

        let mut k = trying();
        assert_eq!(k.left(), Effect::Nothing, "still on its way");
        assert_eq!(k.answered(Outcome::Succeeded), Effect::Revert("before"));
        assert_eq!(
            k.answered(Outcome::Succeeded),
            Effect::Done(End::Reverted(Why::Left))
        );

        // Leaving with nothing going on does nothing.
        let mut k: Keep<&str> = Keep::default();
        assert_eq!(k.left(), Effect::Nothing);
    }

    #[test]
    fn a_refused_or_stale_layout_ends_it_with_nothing_to_undo() {
        let mut k = trying();
        assert_eq!(k.answered(Outcome::Failed), Effect::Done(End::Refused));
        assert!(k.idle());
        let mut k = trying();
        assert_eq!(k.answered(Outcome::Cancelled), Effect::Done(End::Stale));
        let mut k = trying();
        k.left();
        assert_eq!(k.answered(Outcome::Failed), Effect::Done(End::Refused));
    }

    #[test]
    fn revert_on_request_and_a_failed_revert_says_so() {
        let mut k = trying();
        k.answered(Outcome::Succeeded);
        assert_eq!(k.undo(), Effect::Revert("before"));
        assert_eq!(k.answered(Outcome::Failed), Effect::Done(End::RevertFailed));
    }

    #[test]
    fn a_replug_during_the_question_hands_over_to_the_profiles() {
        let mut k = trying();
        assert_eq!(k.replugged(), Effect::Nothing, "not while applying");
        k.answered(Outcome::Succeeded);
        assert_eq!(k.replugged(), Effect::Done(End::Replugged));
        assert!(k.idle());
    }
}
