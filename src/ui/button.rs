//! Buttons: a kind (what pressing it does), a size, and a face (what it
//! shows).
//!
//! `ui::button(label, kind)` for the common case, `ui::button_with(face,
//! kind, size)` for anything else, `ui::toggle_button(..)` for one that
//! stays down, `ui::button::adopt(&b, kind, size)` for a button GTK or the
//! caller built; `ui::set_button_kind`, `ui::set_armed` at runtime.

use gtk4::prelude::*;

use super::class::{swap, toggle};

/// What pressing it does, as the fill says it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Primary,
    Secondary,
    Flat,
    Destructive,
}

impl Kind {
    pub const ALL: [Kind; 4] = [
        Kind::Primary,
        Kind::Secondary,
        Kind::Flat,
        Kind::Destructive,
    ];

    fn class(self) -> Option<&'static str> {
        match self {
            Kind::Primary => Some("primary"),
            Kind::Flat => Some("flat"),
            Kind::Destructive => Some("destructive"),
            Kind::Secondary => None,
        }
    }
}

/// How much room it takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    /// A control's height, body text.
    Normal,
    /// A dense place (a popover's footer, a notification's actions): label
    /// size, lower.
    Small,
}

/// What the button shows.
#[derive(Clone, Copy, Debug)]
pub enum Face<'a> {
    /// Words.
    Label(&'a str),
    /// A glyph from the icon font, on a square button. A glyph says nothing
    /// to someone who does not know it, so the tooltip names it.
    Glyph { glyph: &'a str, tooltip: &'a str },
    /// Any widget (a styled glyph, a picture) on a square button, named by
    /// the tooltip.
    Icon {
        child: &'a gtk4::Widget,
        tooltip: &'a str,
    },
    /// Any widget on the normal shape (a glyph and words on one line).
    Child(&'a gtk4::Widget),
}

impl Face<'_> {
    fn apply(self, b: &gtk4::Button) {
        match self {
            Face::Label(l) => b.set_label(l),
            Face::Glyph { glyph, tooltip } => {
                b.set_label(glyph);
                b.add_css_class("icon");
                b.set_tooltip_text(Some(tooltip));
            }
            Face::Icon { child, tooltip } => {
                b.set_child(Some(child));
                b.add_css_class("icon");
                b.set_tooltip_text(Some(tooltip));
            }
            Face::Child(child) => b.set_child(Some(child)),
        }
    }
}

/// Style a button the caller (or GTK) built as a component button.
pub fn adopt(b: &(impl IsA<gtk4::Button> + IsA<gtk4::Widget>), kind: Kind, size: Size) {
    b.add_css_class("ui-btn");
    if let Some(c) = kind.class() {
        b.add_css_class(c);
    }
    if size == Size::Small {
        b.add_css_class("small");
    }
}

/// A button of words, at the normal size.
pub fn button(label: &str, kind: Kind) -> gtk4::Button {
    button_with(Face::Label(label), kind, Size::Normal)
}

pub fn button_with(face: Face, kind: Kind, size: Size) -> gtk4::Button {
    let b = gtk4::Button::new();
    face.apply(&b);
    adopt(&b, kind, size);
    b
}

/// A button that stays down: a tool in a toolbar. Checked, a flat one takes
/// the accent.
pub fn toggle_button(face: Face, kind: Kind, size: Size) -> gtk4::ToggleButton {
    let b = gtk4::ToggleButton::new();
    face.apply(b.upcast_ref());
    adopt(&b, kind, size);
    b
}

/// Restyle a button as another kind in place ("Forget" turning into its
/// destructive confirmation).
pub fn set_button_kind(b: &(impl IsA<gtk4::Button> + IsA<gtk4::Widget>), kind: Kind) {
    swap(b, Kind::ALL.iter().filter_map(|k| k.class()), kind.class());
}

/// How long an armed button waits for its second press.
pub const ARMED: std::time::Duration = std::time::Duration::from_secs(6);

/// A button that asks before it acts: its first press arms it (its face
/// becomes `question`, pulsing), and a second press within [`ARMED`] runs
/// `act`; otherwise it disarms and shows its own face again. For an action
/// that throws something away (a profile, a learned history, an agent's
/// work), where the first press is the question.
pub fn arm(button: &gtk4::Button, question: &str, act: impl Fn(&gtk4::Button) + 'static) {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    let armed = Rc::new(Cell::new(false));
    // The face it had: its label, or for a glyph its child. GTK reuses a
    // label child for the question, so a label is kept as text.
    #[derive(Default)]
    enum Was {
        #[default]
        Nothing,
        Label(String),
        Child(gtk4::Widget),
    }
    let face: Rc<RefCell<Was>> = Rc::default();
    let question = question.to_string();
    let disarm = {
        let face = face.clone();
        move |b: &gtk4::Button| {
            set_armed(b, false);
            match std::mem::take(&mut *face.borrow_mut()) {
                Was::Label(text) => b.set_label(&text),
                Was::Child(child) => b.set_child(Some(&child)),
                Was::Nothing => {}
            }
        }
    };
    button.connect_clicked(move |b| {
        if !armed.replace(true) {
            *face.borrow_mut() = match (b.label(), b.child()) {
                (Some(text), _) => Was::Label(text.to_string()),
                (None, Some(child)) => Was::Child(child),
                (None, None) => Was::Nothing,
            };
            b.set_label(&question);
            set_armed(b, true);
            let (b, armed, disarm) = (b.clone(), armed.clone(), disarm.clone());
            glib::timeout_add_local_once(ARMED, move || {
                if armed.replace(false) {
                    disarm(&b);
                }
            });
            return;
        }
        armed.set(false);
        disarm(b);
        act(b);
    });
}

/// Arm a destructive button for its confirming second press, or disarm it.
pub fn set_armed(b: &(impl IsA<gtk4::Button> + IsA<gtk4::Widget>), armed: bool) {
    toggle(b, "armed", armed);
}
