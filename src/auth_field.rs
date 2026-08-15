//! The authentication field and its caption — the whole of the auth card.
//!
//! Design and rationale: `docs/AUTH_CARD.md`. The short version is that the
//! lock screen, the greeter and the polkit dialog used to be a stack of rows,
//! one per thing that could ever need saying, each allocated whether or not it
//! had anything to say. Geometry never moved — that rule is stated in
//! docs/AUTH_CARD.md and it stays — but the resting card was a password box
//! with two empty bands under it.
//!
//! **A card's geometry is decided when it is built and never changes again.**
//! Anything that can arrive late is laid out from the first frame and only
//! painted or not; anything that genuinely varies card-to-card is settled
//! before the surface is presented, where a size change costs nothing. This
//! module is where that rule is now kept, because the reserved-row machinery
//! it replaced (`src/slot.rs`) had no callers left once the rows were gone.
//!
//! So the rows collapse into one object. [`AuthField`] is a box that carries
//! all the chrome a text entry used to carry — border, fill, focus, the
//! fingerprint arm pulse, the reject flash — with the entry inside it reduced
//! to a text node, a mark at each edge, and [`Caption`] underneath: two
//! reserved lines that are never empty.
//!
//! Reserved space stops reading as a hole the moment its default tenant is
//! real text. That is the entire idea.
//!
//! ## The honesty rule
//!
//! **The card names only methods that are accepting input at this instant.**
//!
//! The caption says "Enter your password" while fprintd is enumerating, while
//! the device is claimed elsewhere, while the user has no enrolled prints, and
//! forever on a machine with no reader. It names the reader only between
//! `EngineEvent::Ready` and `EngineEvent::Unavailable`. There is no starting-up
//! state, and the fingerprint mark paints nothing until the reader arms — a
//! ghosted whorl was considered and rejected, because it is the card gesturing
//! at a reader that cannot read.

use std::cell::{Cell, RefCell};
use std::time::Duration;

use gtk4::prelude::*;

/// How long a transient caption holds before the resting sentence returns.
///
/// Long enough to read a four-word hint without hurrying, short enough that
/// the card is not still explaining a finger the user has already lifted.
const DWELL: Duration = Duration::from_millis(2000);

/// The words for Caps Lock. There is no mark any more: see [`AuthField`].
const CAPS_TEXT: &str = "Caps Lock is on";

/// What the caption is currently saying, in priority order. The order is
/// total and static: an error outranks the Caps Lock edge, which outranks a
/// fingerprint hint, which outranks the resting sentence.
///
/// No queue and no dwell arithmetic. Two of the four have timers, both
/// [`DWELL`], and neither can preempt an error; an error holds until the next
/// keypress, and a keypress means the user has chosen the password path.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
enum Rank {
    #[default]
    Resting = 0,
    FpHint = 1,
    Caps = 2,
    Status = 3,
}

/// How a caption line is coloured. Not the same axis as [`Rank`]: a status can
/// be an error or an ordinary progress note, and both outrank a hint.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Tone {
    #[default]
    Info,
    Error,
    Success,
}

impl Tone {
    fn class(self) -> &'static str {
        match self {
            Tone::Info => "auth-caption-info",
            Tone::Error => "auth-caption-error",
            Tone::Success => "auth-caption-success",
        }
    }
}

const TONE_CLASSES: [&str; 5] = [
    "auth-caption-info",
    "auth-caption-error",
    "auth-caption-success",
    "auth-caption-fp",
    "auth-caption-caps",
];

/// The field: one bordered box holding the face ring, an entry, and nothing
/// else.
///
/// It used to carry two glyph marks as well — a fingerprint whorl at the
/// leading edge and a Caps Lock arrow at the trailing one. Both are gone. They
/// were never given a CSS rule, so they painted in GTK's default label colour:
/// two dark blobs on a dark card, which is not an indicator, it is a smudge.
/// Neither was carrying much either. What the reader is doing is already in
/// the caption's resting sentence, which names the methods accepting input
/// right now and does so in words; Caps Lock still marks its edge in the
/// caption and, where it actually matters, composes onto the rejection
/// (`Wrong password · Caps Lock is on`) rather than leaving the reader to join
/// a failed attempt to a lit glyph.
///
/// Every state this shows is a paint property. Nothing in here is ever
/// `set_visible`, and the box's height is a constant of its CSS — `min-height`
/// plus padding plus border — so no state can move it.
#[derive(Clone)]
pub struct AuthField {
    root: gtk4::Box,
    /// The leading slot's occupant. The slot itself is always allocated, even
    /// on a row that will never run a face check: it is what puts the
    /// greeter's username and password text on one rail, and that alignment is
    /// visible where a blank row is not.
    face_mark: gtk4::Box,
}

impl AuthField {
    /// Wrap `input` in the field. The caller owns the entry because the
    /// greeter's username row is a plain `Entry` and everything else is a
    /// `PasswordEntry`; the chrome around them is identical either way.
    pub fn new(input: &impl IsA<gtk4::Widget>) -> Self {
        let root = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(12)
            .hexpand(true)
            .build();
        root.add_css_class("auth-field");

        // Leading slot. Sized and allocated whether or not anything ever draws
        // in it, because it is the rail the rows above and below line up on;
        // the face ring is the only thing that does draw here now.
        let mark = gtk4::Box::builder()
            .width_request(22)
            .height_request(27)
            .halign(gtk4::Align::Center)
            .valign(gtk4::Align::Center)
            .build();
        mark.add_css_class("auth-mark");
        // Anything inside a field looks like a button, and on a touchscreen
        // someone will tap it. Untargetable, so the tap falls through and
        // focuses the field, which is the right outcome.
        mark.set_can_target(false);
        mark.set_can_focus(false);

        let face_mark = gtk4::Box::builder()
            .width_request(22)
            .height_request(22)
            .valign(gtk4::Align::Center)
            .build();
        face_mark.add_css_class("auth-mark-face");
        face_mark.add_css_class("face-ring");
        face_mark.set_opacity(0.0);
        mark.append(&face_mark);

        // A counterweight, and nothing else: the leading rail is reserved
        // whether or not anything draws in it, so without this the entry sits
        // 34 px off the card's left edge and flush against its right. The
        // Caps Lock glyph used to be what balanced it, and it is not coming
        // back. Same width, no class that paints, untargetable.
        let tail = gtk4::Box::builder().width_request(22).build();
        tail.set_can_target(false);
        tail.set_can_focus(false);

        let input = input.as_ref();
        input.add_css_class("auth-input");
        input.set_hexpand(true);

        root.append(&mark);
        root.append(input);
        root.append(&tail);

        Self { root, face_mark }
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    /// The reader has armed, or stood down. Starts or stops the field's border
    /// pulse; never touches the allocation. What the reader is *for* is said
    /// in the caption, which can use words.
    pub fn set_fp_armed(&self, armed: bool) {
        toggle(&self.root, "auth-fp-armed", armed);
    }

    /// A face check is running, or has stopped. The ring holds the leading
    /// slot on its own now, so this is opacity and state and nothing else.
    pub fn set_face(&self, active: bool, state: &str) {
        self.face_mark.set_opacity(if active { 1.0 } else { 0.0 });
        crate::face_ring::apply(&self.face_mark, None, if active { state } else { "" });
    }

    /// PAM is working. A border and a word, not a card-wide dimming: greying
    /// the whole card for a field-scale event reads as a fault on a card this
    /// small.
    pub fn set_busy(&self, busy: bool) {
        toggle(&self.root, "auth-busy", busy);
    }

    /// Rejected. A paint-only flash on the field, alongside the card's shake.
    pub fn flash_reject(&self) {
        let root = self.root.clone();
        root.remove_css_class("auth-reject");
        glib::idle_add_local_once(move || {
            root.add_css_class("auth-reject");
        });
    }
}

/// The caption: two reserved lines under the field that are never empty.
///
/// Top-aligned on purpose. A one-line message centred in a two-line band
/// leaves blank space bounded above and below, which is a hole; top-aligned,
/// the same slack falls against the card's bottom padding, where it reads as
/// margin.
#[derive(Clone)]
pub struct Caption {
    label: gtk4::Label,
    /// What the card says when it has nothing else to say. Recomputed from the
    /// surface and whether a reader is armed, and it is the floor under every
    /// transient rather than a competitor to them.
    resting: std::rc::Rc<RefCell<String>>,
    shown: std::rc::Rc<Cell<Rank>>,
    /// Bumped whenever something new is shown, so a dwell timer armed for a
    /// message that has since been replaced expires without doing anything.
    epoch: std::rc::Rc<Cell<u64>>,
}

impl Caption {
    pub fn new(max_width_chars: i32) -> Self {
        let label = gtk4::Label::builder()
            .wrap(true)
            .wrap_mode(gtk4::pango::WrapMode::WordChar)
            // `lines` only takes effect with wrapping AND ellipsizing on, and
            // together they are what stops any string claiming a third line.
            // `min-height` in the stylesheet is a floor, not a cap; this is
            // the cap.
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .lines(2)
            .max_width_chars(max_width_chars)
            .xalign(0.0)
            .yalign(0.0)
            .valign(gtk4::Align::Start)
            .build();
        label.add_css_class("auth-caption");
        Self {
            label,
            resting: Default::default(),
            shown: Default::default(),
            epoch: Default::default(),
        }
    }

    pub fn widget(&self) -> &gtk4::Label {
        &self.label
    }

    /// Set the sentence the caption falls back to, and show it if nothing
    /// louder is up. This is where the honesty rule is enforced: callers pass
    /// the wording for the methods that are accepting input right now.
    pub fn set_resting(&self, text: &str) {
        if *self.resting.borrow() == text {
            return;
        }
        *self.resting.borrow_mut() = text.to_string();
        if self.shown.get() == Rank::Resting {
            self.paint(text, Tone::Info, "");
        }
    }

    /// A fingerprint hint. Holds for [`DWELL`], then the resting sentence
    /// returns — unless something louder has taken the line meanwhile.
    pub fn fp_hint(&self, text: &str) {
        self.transient(Rank::FpHint, text, Tone::Info, "auth-caption-fp");
    }

    /// Caps Lock just changed. The words hold for [`DWELL`] and then give the
    /// line back. They used to be the short half of a pair — a mark in the
    /// field stayed lit for as long as the key was latched — but the mark is
    /// gone, and a permanent sentence about a shift key would sit on top of
    /// everything else the caption has to say. What catches the case that
    /// matters is [`Caption::status`], which composes the warning onto a
    /// rejection.
    pub fn caps_edge(&self) {
        self.transient(Rank::Caps, CAPS_TEXT, Tone::Info, "auth-caption-caps");
    }

    /// A status from PAM, greetd or the surface itself. Holds until cleared —
    /// an error is cleared by the next keypress, because a keypress means the
    /// user has chosen the password path and no longer needs telling.
    ///
    /// `caps` composes the Caps Lock warning onto an error rather than
    /// evicting it: a rejected password with Caps Lock on is one fact, and
    /// printing it as two rows left the reader to join them up.
    pub fn status(&self, text: &str, tone: Tone, caps: bool) {
        if text.is_empty() {
            self.clear(Rank::Status);
            return;
        }
        let escaped = glib::markup_escape_text(text);
        let markup = if caps && tone == Tone::Error {
            format!("{escaped}  \u{b7}  <span alpha=\"75%\">{CAPS_TEXT}</span>")
        } else {
            escaped.to_string()
        };
        self.epoch.set(self.epoch.get().wrapping_add(1));
        self.shown.set(Rank::Status);
        self.paint_markup(&markup, tone, "");
        // The full text stays reachable when two lines were not enough.
        self.label
            .set_tooltip_text((text.len() > 80).then_some(text));
    }

    /// Drop back to the resting sentence if `rank` is what is currently up.
    fn clear(&self, rank: Rank) {
        if self.shown.get() != rank {
            return;
        }
        self.epoch.set(self.epoch.get().wrapping_add(1));
        self.shown.set(Rank::Resting);
        self.label.set_tooltip_text(None);
        let resting = self.resting.borrow().clone();
        self.paint(&resting, Tone::Info, "");
    }

    /// Clear a status. Named for the caller's benefit: this is the keypress
    /// path, and `Rank` is private.
    pub fn clear_status(&self) {
        self.clear(Rank::Status);
    }

    fn transient(&self, rank: Rank, text: &str, tone: Tone, extra: &str) {
        if self.shown.get() > rank {
            return;
        }
        self.epoch.set(self.epoch.get().wrapping_add(1));
        let epoch = self.epoch.get();
        self.shown.set(rank);
        self.paint(text, tone, extra);

        let this = self.clone();
        glib::timeout_add_local_once(DWELL, move || {
            // Someone else has spoken since; their timer owns the line now.
            if this.epoch.get() == epoch {
                this.clear(rank);
            }
        });
    }

    fn paint(&self, text: &str, tone: Tone, extra: &str) {
        self.paint_markup(&glib::markup_escape_text(text), tone, extra);
    }

    fn paint_markup(&self, markup: &str, tone: Tone, extra: &str) {
        let want = if extra.is_empty() {
            tone.class()
        } else {
            extra
        };
        for c in TONE_CLASSES {
            if c != want {
                self.label.remove_css_class(c);
            }
        }
        self.label.add_css_class(want);
        self.label.set_markup(markup);
    }
}

fn toggle(w: &impl IsA<gtk4::Widget>, class: &str, on: bool) {
    let w = w.as_ref();
    if on {
        w.add_css_class(class);
    } else {
        w.remove_css_class(class);
    }
}
