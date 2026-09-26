//! The type scale and the text tones (§3.2, §3.4): labels, glyphs and the
//! small typographic roles (heading, overline, mono, live caption).

use gtk4::prelude::*;

use super::class::swap;

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

/// A label on the type scale, in a tone.
pub fn text(s: &str, size: Text, tone: Tone) -> gtk4::Label {
    let l = gtk4::Label::new(Some(s));
    l.add_css_class(size.class());
    if let Some(c) = tone.class() {
        l.add_css_class(c);
    }
    l.set_xalign(0.0);
    l
}

/// A glyph from the icon font at a size of the scale, regular weight.
pub fn glyph(l: &gtk4::Label, size: Text, tone: Tone) {
    set_text_style(l, size, tone);
    l.add_css_class("ui-glyph");
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

/// A heading over a list inside a section ("Available networks").
pub fn heading(s: &str) -> gtk4::Label {
    let l = text(s, Text::Label, Tone::Muted);
    l.add_css_class("ui-strong");
    l
}

/// Text that stands on bare wallpaper, with no card behind it: gives the
/// glyphs an edge with a shadow of the scrim.
pub fn on_wallpaper(w: &impl IsA<gtk4::Widget>) {
    w.add_css_class("ui-on-wallpaper");
}

/// A caption whose tone changes in place: the colour moves as a state.
pub fn live_caption(l: &gtk4::Label) {
    l.add_css_class("ui-live-caption");
}

/// Mark `label` monospace: a command, a key, raw details.
pub fn mono(label: &gtk4::Label) {
    label.add_css_class("ui-mono");
}

/// A small uppercase label naming what follows: a sender, a task, a group.
/// `s` is uppercased here, so every overline reads the same.
pub fn overline(s: &str, tone: Tone) -> gtk4::Label {
    let l = text(&s.to_uppercase(), Text::Caption, tone);
    l.add_css_class("ui-overline");
    l
}

/// Put an existing label (a badge, a chip) in the overline's tracking.
pub fn make_overline(l: &gtk4::Label) {
    l.add_css_class("ui-overline");
}
