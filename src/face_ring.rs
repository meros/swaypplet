//! The face indicator's visual vocabulary, written down once.
//!
//! Two surfaces render it -- the lock screen's pill and the elevate cue --
//! and they must agree, because the whole point of a fixed indicator is that
//! a user learns to read it once. One builder and one state-swapping loop is
//! one chance for them to drift, and drift here is not cosmetic: a face that
//! means "hold still" in one place and "searching" in another is worse than
//! no face at all.
//!
//! It is a face, not a spinner. A ring said "something is happening"; a face
//! says what: it glances left and right while the camera looks for you,
//! stills and meets your eye when it has found you, smiles when it knows you
//! and frowns when it does not. Built from three CSS-drawn boxes inside the
//! ring so it can move -- eyes translate and blink, the mouth turns -- which a
//! font glyph cannot do. Every state is a paint property on a fixed 22 px
//! box; nothing here changes an allocation.

use gtk4::prelude::*;

/// Every state the face can be in. Enumerated rather than derived, because
/// swapping to a new state means clearing the old ones and there is no way to
/// ask GTK which of them is currently set.
pub const STATES: [&str; 5] = ["looking", "dark", "found", "ok", "fail"];

/// Build the indicator: a ring with a face in it. `size` is the ring's
/// outer size in px; the face scales with it.
///
/// A `Fixed` inside the ring places the eyes and the mouth by hand, which is
/// the one layout that lets them move as paint (transforms) rather than as
/// allocation. The ring is the outer box, so callers style and swap states on
/// the same node they always did.
pub fn build(size: i32) -> gtk4::Box {
    let ring = gtk4::Box::builder()
        .width_request(size)
        .height_request(size)
        .valign(gtk4::Align::Center)
        .build();
    ring.add_css_class("face-ring");

    let inner = gtk4::Fixed::builder()
        .width_request(size)
        .height_request(size)
        .build();
    inner.add_css_class("face-glyph");

    let unit = f64::from(size) / 22.0;
    let px = |v: f64| (v * unit).round();
    let eye_size = px(4.0) as i32;

    let left = gtk4::Box::builder()
        .width_request(eye_size)
        .height_request(eye_size)
        .build();
    left.add_css_class("face-eye");
    left.add_css_class("face-eye-left");
    let right = gtk4::Box::builder()
        .width_request(eye_size)
        .height_request(eye_size)
        .build();
    right.add_css_class("face-eye");
    right.add_css_class("face-eye-right");
    let mouth = gtk4::Box::builder()
        .width_request(px(8.0) as i32)
        .height_request(px(4.0) as i32)
        .build();
    mouth.add_css_class("face-mouth");

    inner.put(&left, px(6.0), px(7.0));
    inner.put(&right, px(12.0), px(7.0));
    inner.put(&mouth, px(7.0), px(12.0));
    ring.append(&inner);
    ring
}

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
    // The pill carries the state as well as the ring, because three of the
    // five states say something the ring cannot: `dark` and `ok` recolour the
    // pill's border and fill, and `looking` breathes its border. The ring
    // keeps what is its own -- the face's motion and the two verdict
    // keyframes -- so the two classes never animate the same property on
    // nested nodes.
    if let Some(pill) = pill {
        pill.add_css_class(&format!("face-pill-{state}"));
    }
}
