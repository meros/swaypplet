//! Motion, by meaning (docs/design-system.md §3.8).
//!
//! Every animation in swaypplet is one of these seven. The name says what
//! is happening on screen, and the duration and curve follow from that, so
//! two things that mean the same move the same way on every surface. The
//! stylesheet gets them as `--motion-*` (duration and curve in one token,
//! for `transition` and `animation`); `src/anim.rs` reads the same values.

/// A CSS `cubic-bezier(x1, y1, x2, y2)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Curve(pub f64, pub f64, pub f64, pub f64);

impl Curve {
    pub fn css(self) -> String {
        format!(
            "cubic-bezier({}, {}, {}, {})",
            self.0, self.1, self.2, self.3
        )
    }
}

/// Between two on-screen states: colour, a value settling, a reflow.
pub const STANDARD: Curve = Curve(0.2, 0.0, 0.0, 1.0);
/// Arriving: starts at speed and settles.
pub const DECELERATE: Curve = Curve(0.0, 0.0, 0.0, 1.0);
/// Leaving: gathers speed and goes.
pub const ACCELERATE: Curve = Curve(0.3, 0.0, 1.0, 1.0);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    pub name: &'static str,
    pub ms: f64,
    pub curve: Curve,
}

/// A control changing state: hover, press, colour, a check. Fast enough to
/// feel like the pointer did it.
pub const STATE: Motion = Motion {
    name: "state",
    ms: 150.0,
    curve: STANDARD,
};
/// Something opening or closing in place: a section, a chevron, a switch's
/// knob, a revealer.
pub const EXPAND: Motion = Motion {
    name: "expand",
    ms: 200.0,
    curve: STANDARD,
};
/// Something arriving: a card, a notification, a row that was not there.
/// The thing being waited for, so it takes its time and settles.
pub const ENTER: Motion = Motion {
    name: "enter",
    ms: 300.0,
    curve: DECELERATE,
};
/// Something leaving. Shorter than its entrance on purpose: waiting for a
/// thing to go is dead time.
pub const EXIT: Motion = Motion {
    name: "exit",
    ms: 200.0,
    curve: ACCELERATE,
};
/// Something on screen moving to where it now belongs: a reflow, a reorder,
/// a card resizing.
pub const MOVE: Motion = Motion {
    name: "move",
    ms: 300.0,
    curve: STANDARD,
};
/// A whole surface crossing a distance: the switcher's strip, a workspace
/// growing from its picture, a panel sliding in.
pub const TRAVEL: Motion = Motion {
    name: "travel",
    ms: 400.0,
    curve: STANDARD,
};
/// The whole screen changing: the lock and the greeter crossfading.
pub const PAGE: Motion = Motion {
    name: "page",
    ms: 500.0,
    curve: STANDARD,
};

pub const ALL: [Motion; 7] = [STATE, EXPAND, ENTER, EXIT, MOVE, TRAVEL, PAGE];

/// Attention loops (a pulse, a shake, breathing) are outside the scale on
/// purpose: they repeat, and their period is the design. They are the only
/// animations that may name their own duration, and only in `@keyframes`
/// users with `infinite` or a count.
pub const ATTENTION_LOOPS_ARE_EXEMPT: () = ();
