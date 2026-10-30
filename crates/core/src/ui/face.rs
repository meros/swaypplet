//! The face indicator: a face in a ring, in a thin glass pill.
//!
//! `ui::face_ring(size)`, `ui::face_pill(size)`; `ui::set_face_state`,
//! `ui::set_face_enter` at runtime.

use gtk4::Align;
use gtk4::prelude::*;

use super::class::{swap, toggle};
use super::{Card, Text, Tone, card, hbox, text, vbox};

/// Every state the face can be in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaceState {
    /// The camera is looking: the eyes glance.
    Looking,
    /// No usable frame: the eyes close.
    Dark,
    /// A face is in view: the eyes meet yours.
    Found,
    /// Recognised: it smiles.
    Ok,
    /// Not recognised: a frown and a shake.
    Fail,
}

impl FaceState {
    pub const ALL: [FaceState; 5] = [
        FaceState::Looking,
        FaceState::Dark,
        FaceState::Found,
        FaceState::Ok,
        FaceState::Fail,
    ];

    fn class(self) -> &'static str {
        match self {
            FaceState::Looking => "looking",
            FaceState::Dark => "dark",
            FaceState::Found => "found",
            FaceState::Ok => "ok",
            FaceState::Fail => "fail",
        }
    }
}

/// The face in a ring. It is a face, not a spinner: a ring said "something is
/// happening", a face says what. Three boxes placed by hand in a `Fixed` so
/// the eyes and mouth move as paint (transforms), never as allocation.
/// `size` is the ring's outer size in px; the face scales with it.
pub fn face_ring(size: i32) -> gtk4::Box {
    let ring = gtk4::Box::builder()
        .width_request(size)
        .height_request(size)
        .valign(Align::Center)
        .build();
    ring.add_css_class("ui-face-ring");

    let inner = gtk4::Fixed::builder()
        .width_request(size)
        .height_request(size)
        .build();

    // Drawn on a 22 px grid and scaled from it.
    let unit = f64::from(size) / 22.0;
    let px = |v: f64| (v * unit).round();
    let eye_size = px(4.0) as i32;
    let eye = || {
        let e = gtk4::Box::builder()
            .width_request(eye_size)
            .height_request(eye_size)
            .build();
        e.add_css_class("ui-face-eye");
        e
    };
    let mouth = gtk4::Box::builder()
        .width_request(px(8.0) as i32)
        .height_request(px(4.0) as i32)
        .build();
    mouth.add_css_class("ui-face-mouth");

    inner.put(&eye(), px(6.0), px(7.0));
    inner.put(&eye(), px(12.0), px(7.0));
    inner.put(&mouth, px(7.0), px(12.0));
    ring.append(&inner);
    ring
}

/// The face indicator: a thin glass pill holding the ring and a line of
/// words, inside a wrapper that carries the entrance.
pub struct FacePill {
    /// Carries the entrance ([`set_face_enter`]), so
    /// a state change on the pill cannot replay it.
    pub wrap: gtk4::Box,
    pub pill: gtk4::Box,
    pub ring: gtk4::Box,
    pub label: gtk4::Label,
}

pub fn face_pill(ring_size: i32) -> FacePill {
    let pill = hbox(4);
    pill.set_halign(Align::Center);
    pill.set_valign(Align::Start);
    card::adopt(&pill, Card::Thin);
    pill.add_css_class("ui-face-pill");
    let ring = face_ring(ring_size);
    let label = text("", Text::Body, Tone::Fg);
    label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    label.set_hexpand(true);
    pill.append(&ring);
    pill.append(&label);
    let wrap = vbox(0);
    wrap.set_halign(Align::Center);
    wrap.set_valign(Align::Start);
    wrap.append(&pill);
    FacePill {
        wrap,
        pill,
        ring,
        label,
    }
}

/// Put the ring (and the pill, if given) into `state`. `None` clears
/// without setting anything, which is what a hidden indicator wants: a
/// stale class on a hidden widget makes the next show start mid-animation
/// in the previous state.
///
/// The pill carries the state as well as the ring because three states say
/// something the ring cannot: `Dark` and `Ok` recolour the pill, `Looking`
/// breathes its border. The ring keeps the face's motion and the verdict
/// keyframes, so the two never animate one property on nested nodes.
pub fn set_face_state(ring: &gtk4::Box, pill: Option<&gtk4::Box>, state: Option<FaceState>) {
    let one = state.map(FaceState::class);
    swap(ring, FaceState::ALL.map(FaceState::class), one);
    if let Some(pill) = pill {
        swap(pill, FaceState::ALL.map(FaceState::class), one);
    }
}

/// Play the pill's entrance on the wrapper that carries it. Clear it and
/// set it again on a later frame to replay it.
pub fn set_face_enter(wrap: &impl IsA<gtk4::Widget>, entering: bool) {
    toggle(wrap, "ui-face-enter", entering);
}
