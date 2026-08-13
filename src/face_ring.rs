//! The face indicator's visual vocabulary, written down once.
//!
//! Three surfaces render it -- the lock screen, the elevate card, and the
//! look-at-the-camera cue -- and they must agree, because the whole point of a
//! fixed indicator is that a user learns to read it once. Three copies of the
//! same class-swapping loop is three chances for them to drift, and drift here
//! is not cosmetic: a ring that means "hold still" in one place and "searching"
//! in another is worse than no ring at all.

use gtk4::prelude::*;

/// Every state the ring can be in. Enumerated rather than derived, because
/// swapping to a new state means clearing the old ones and there is no way to
/// ask GTK which of them is currently set.
pub const STATES: [&str; 5] = ["looking", "dark", "found", "ok", "fail"];

/// Put `ring` (and optionally its `pill`) into `state`.
///
/// An empty `state` clears without setting anything, which is what a hidden
/// indicator wants: leaving a stale class on a hidden widget means the next
/// show starts mid-animation in the previous state.
pub fn apply(ring: &gtk4::Box, pill: Option<&gtk4::Box>, state: &str) {
    for old in STATES {
        ring.remove_css_class(&format!("face-ring-{old}"));
        if let Some(pill) = pill {
            pill.remove_css_class(&format!("face-pill-{old}"));
        }
    }
    if state.is_empty() {
        return;
    }
    ring.add_css_class(&format!("face-ring-{state}"));
    // The pill carries the state as well as the ring, so the attention glow
    // can live on the whole pill. It is deliberately not on the ring, which is
    // also used on the polkit card -- and that layer is compositor-blurred,
    // where a glow frosts into a flat halo.
    if let Some(pill) = pill {
        pill.add_css_class(&format!("face-pill-{state}"));
    }
}
