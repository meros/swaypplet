//! The face indicator: a face in a viewfinder, in a thin glass pill.
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

/// The face in a viewfinder. It is a face, not a spinner: a spinner says
/// "something is happening", a face says what. Four corner brackets frame it
/// (the sign for "a camera is reading a face"), a scan line sweeps it while
/// the camera looks, and the eyes and mouth carry the verdict.
///
/// Every part is a box placed by hand in a `Fixed`, so the states move as
/// paint (transforms), never as allocation. `size` is the outer size in px;
/// the face is drawn on a 36 px grid and scaled from it.
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

    let unit = f64::from(size) / 36.0;
    let px = |v: f64| (v * unit).round();
    let corner = px(11.0) as i32;
    let far = size - corner;

    // The frame: its own node, so the breathing scales the four corners
    // together and the face inside does not move.
    let frame = gtk4::Fixed::builder()
        .width_request(size)
        .height_request(size)
        .build();
    frame.add_css_class("ui-face-frame");
    for (class, x, y) in [
        ("tl", 0, 0),
        ("tr", far, 0),
        ("bl", 0, far),
        ("br", far, far),
    ] {
        let c = gtk4::Box::builder()
            .width_request(corner)
            .height_request(corner)
            .build();
        c.add_css_class("ui-face-corner");
        c.add_css_class(class);
        frame.put(&c, f64::from(x), f64::from(y));
    }
    inner.put(&frame, 0.0, 0.0);

    let eye_size = px(5.0) as i32;
    let eye = || {
        let e = gtk4::Box::builder()
            .width_request(eye_size)
            .height_request(eye_size)
            .build();
        e.add_css_class("ui-face-eye");
        e
    };
    let mouth = gtk4::Box::builder()
        .width_request(px(12.0) as i32)
        .height_request(px(5.0) as i32)
        .build();
    mouth.add_css_class("ui-face-mouth");

    let scan = gtk4::Box::builder()
        .width_request(size - 2 * px(8.0) as i32)
        .height_request(2)
        .build();
    scan.add_css_class("ui-face-scan");

    inner.put(&eye(), px(10.0), px(11.0));
    inner.put(&eye(), px(21.0), px(11.0));
    inner.put(&mouth, px(12.0), px(21.0));
    inner.put(&scan, px(8.0), px(9.0));
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
    let pill = hbox(0);
    pill.set_halign(Align::Center);
    pill.set_valign(Align::Start);
    card::adopt(&pill, Card::Thin);
    pill.add_css_class("ui-face-pill");
    let ring = face_ring(ring_size);
    let label = text("", Text::Title, Tone::Fg);
    label.add_css_class("ui-face-words");
    label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    // The pill is wider than its words (one fixed width for every wording,
    // see 14-elevation-face.css). The glyph stays where it is whatever the
    // words say, and the words start at a fixed gap from it: centring the
    // pair would move the glyph every time the wording changed length.
    ring.set_margin_end(14);
    let row = hbox(0);
    row.set_halign(Align::Start);
    row.set_hexpand(true);
    row.append(&ring);
    row.append(&label);
    pill.append(&row);
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

/// The indicator state a scan's progress renders as. Note `Face` is
/// `Found`: the engine names what it is doing, the indicator what it looks
/// like, and they are not the same word.
impl From<swaypplet_base::face::Progress> for FaceState {
    fn from(p: swaypplet_base::face::Progress) -> FaceState {
        use swaypplet_base::face::Progress;
        match p {
            Progress::Looking => FaceState::Looking,
            Progress::Dark => FaceState::Dark,
            Progress::Face
            | Progress::TurnToScreen
            | Progress::HoldStill
            | Progress::MoveCloser => FaceState::Found,
        }
    }
}
