//! Fields: the entry, the search entry, the dropdown, and the auth
//! field's states.

use gtk4::prelude::*;

use super::class::toggle;
use super::*;

pub struct Field {
    pub root: gtk4::Box,
}

/// A labelled input: the label (empty for none) over the entry.
pub fn field(label: &str, input: &impl IsA<gtk4::Widget>) -> Field {
    let root = vbox(2);
    root.add_css_class("ui-field");
    if !label.is_empty() {
        let l = gtk4::Label::new(Some(label));
        l.add_css_class("ui-field-label");
        l.set_xalign(0.0);
        root.append(&l);
    }
    input.add_css_class("ui-entry");
    root.append(input);
    Field { root }
}

pub fn entry(input: &impl IsA<gtk4::Widget>) {
    input.add_css_class("ui-entry");
}

/// Style a dropdown as a control on the card.
pub fn dropdown(d: &gtk4::DropDown) {
    d.add_css_class("ui-dropdown");
}

/// What an auth field says about the methods behind it (a component state
/// of `ui::field`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldState {
    /// A reader is accepting a finger: the field breathes.
    Armed,
    /// The stack is checking: a steady accent hairline.
    Busy,
    /// Rejected: one danger flash. Re-adding it restarts the flash.
    Reject,
}

impl FieldState {
    fn class(self) -> &'static str {
        match self {
            FieldState::Armed => "armed",
            FieldState::Busy => "busy",
            FieldState::Reject => "reject",
        }
    }
}

/// Turn a state of a `ui::field` (its `root`) on or off.
pub fn set_field_state(field: &impl IsA<gtk4::Widget>, state: FieldState, on: bool) {
    toggle(field, state.class(), on);
}

/// The field a surface is for: a row tall, its text a step up.
pub fn search_entry(input: &impl IsA<gtk4::Widget>) {
    entry(input);
    input.add_css_class("large");
}

/// A dropdown over `choices`, control-sized.
pub fn dropdown_of(choices: &[&str]) -> gtk4::DropDown {
    let d = gtk4::DropDown::from_strings(choices);
    dropdown(&d);
    d
}
