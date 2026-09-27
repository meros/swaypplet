//! The type scale and the text tones (§3.2, §3.4): labels, glyphs and the
//! small typographic roles (heading, overline, mono, live caption).

use gtk4::prelude::*;

use super::class::{swap, toggle};

/// The type scale (§3.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Text {
    /// The lock screen's clock, and nothing else.
    Hero,
    Display,
    DisplaySm,
    Title,
    TitleSm,
    Body,
    Label,
    Caption,
}

impl Text {
    pub const ALL: [Text; 8] = [
        Text::Hero,
        Text::Display,
        Text::DisplaySm,
        Text::Title,
        Text::TitleSm,
        Text::Body,
        Text::Label,
        Text::Caption,
    ];

    fn class(self) -> &'static str {
        match self {
            Text::Hero => "ui-hero",
            Text::Display => "ui-display",
            Text::DisplaySm => "ui-display-sm",
            Text::Title => "ui-title",
            Text::TitleSm => "ui-title-sm",
            Text::Body => "ui-body",
            Text::Label => "ui-label",
            Text::Caption => "ui-caption",
        }
    }
}

/// The text levels and status tones (§3.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Fg,
    Muted,
    Faint,
    Accent,
    Success,
    Warning,
    Danger,
}

impl Tone {
    pub const ALL: [Tone; 7] = [
        Tone::Fg,
        Tone::Muted,
        Tone::Faint,
        Tone::Accent,
        Tone::Success,
        Tone::Warning,
        Tone::Danger,
    ];

    fn class(self) -> Option<&'static str> {
        match self {
            Tone::Fg => None,
            Tone::Muted => Some("ui-muted"),
            Tone::Faint => Some("ui-faint"),
            Tone::Accent => Some("ui-accent"),
            Tone::Success => Some("ui-success"),
            Tone::Warning => Some("ui-warning"),
            Tone::Danger => Some("ui-danger"),
        }
    }
}

/// A weight off the type scale's own (§3.4): a label that has to stand out
/// from its row without changing size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Weight {
    /// Whatever the label's size gives it.
    Regular,
    Strong,
}

/// A label on the type scale, in a tone.
pub fn text(s: &str, size: Text, tone: Tone) -> gtk4::Label {
    let l = gtk4::Label::new(Some(s));
    set_text_style(&l, size, tone);
    l.set_xalign(0.0);
    l
}

/// Restyle an existing label onto the scale.
pub fn set_text_style(l: &gtk4::Label, size: Text, tone: Tone) {
    swap(l, Text::ALL.map(Text::class), Some(size.class()));
    set_tone(l, tone);
}

/// Put any widget's text in a tone, not only a label's (a box whose
/// children inherit it).
pub fn set_tone(w: &impl IsA<gtk4::Widget>, tone: Tone) {
    swap(w, Tone::ALL.iter().filter_map(|t| t.class()), tone.class());
}

/// Set a label's weight apart from its size.
pub fn set_weight(w: &impl IsA<gtk4::Widget>, weight: Weight) {
    toggle(w, "ui-strong", weight == Weight::Strong);
}

/// Tabular figures, so a number that changes does not shift what is beside
/// it (a clock, a level, a time).
pub fn set_numeric(w: &impl IsA<gtk4::Widget>, numeric: bool) {
    toggle(w, "ui-numeric", numeric);
}

/// Monospace: a command, a key, raw details, a hex value.
pub fn set_mono(w: &impl IsA<gtk4::Widget>, mono: bool) {
    toggle(w, "ui-mono", mono);
}

/// A heading over a list inside a section ("Available networks"). Not an
/// [`overline`]: it names a list at label size, where an overline names a
/// card's sender or a group in caps.
pub fn heading(s: &str) -> gtk4::Label {
    let l = text(s, Text::Label, Tone::Muted);
    set_weight(&l, Weight::Strong);
    l
}

/// A small uppercase label naming what follows: a sender, a task, a group.
/// `s` is uppercased here, so every overline reads the same.
pub fn overline(s: &str, tone: Tone) -> gtk4::Label {
    let l = text(&s.to_uppercase(), Text::Caption, tone);
    overline::adopt(&l);
    l
}

pub mod overline {
    use gtk4::prelude::*;

    /// Put a label the caller built (a badge, a chip) in the overline's
    /// tracking.
    pub fn adopt(l: &gtk4::Label) {
        l.add_css_class("ui-overline");
    }
}

pub mod glyph {
    use gtk4::prelude::*;

    use super::{Text, Tone, set_text_style};

    /// A label holding a glyph from the icon font, at a size of the scale
    /// and regular weight (a bold glyph is a different glyph).
    pub fn adopt(l: &gtk4::Label, size: Text, tone: Tone) {
        set_text_style(l, size, tone);
        l.add_css_class("ui-glyph");
    }
}

pub mod on_wallpaper {
    use gtk4::prelude::*;

    /// Text that stands on bare wallpaper, with no card behind it. Its ink
    /// and the halo that gives the glyphs an edge follow the wallpaper
    /// behind it (`tokens::on_wallpaper`): dark ink on a bright image.
    pub fn adopt(w: &impl IsA<gtk4::Widget>) {
        w.add_css_class("ui-on-wallpaper");
    }
}

pub mod on_backdrop {
    use gtk4::prelude::*;

    /// Text on the lock's backdrop where the compositor blurs and dims the
    /// wallpaper (`settings::glass::lock_backdrop`): light ink in either
    /// mode, with no halo (`tokens::backdrop`).
    pub fn adopt(w: &impl IsA<gtk4::Widget>) {
        w.add_css_class("ui-on-backdrop");
    }
}

pub mod live_caption {
    use gtk4::prelude::*;

    /// A caption whose tone changes in place: the colour moves as a state.
    pub fn adopt(l: &gtk4::Label) {
        l.add_css_class("ui-live-caption");
    }
}
