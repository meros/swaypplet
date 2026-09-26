//! The components of the design system (docs/design-system.md §6).
//!
//! One builder per component, each handing back plain GTK widgets with the
//! `ui-*` classes `data/css/00-components.css` styles. A surface is
//! assembled from these; its own classes may place things but not colour,
//! size type or round corners.
//!
//! Spacing between children comes from `tokens::space`, never a literal.

use gtk4::prelude::*;
use gtk4::{Align, Orientation};

use crate::tokens::space;

// ── Text ────────────────────────────────────────────────────────────────

/// The type scale (§3.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Text {
    Display,
    DisplaySm,
    Title,
    TitleSm,
    Body,
    Label,
    Caption,
}

impl Text {
    fn class(self) -> &'static str {
        match self {
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
    for c in [
        "ui-display",
        "ui-display-sm",
        "ui-title",
        "ui-title-sm",
        "ui-body",
        "ui-label",
        "ui-caption",
        "ui-muted",
        "ui-faint",
        "ui-accent",
        "ui-success",
        "ui-warning",
        "ui-danger",
    ] {
        l.remove_css_class(c);
    }
    l.add_css_class(size.class());
    if let Some(c) = tone.class() {
        l.add_css_class(c);
    }
}

// ── Layout helpers ──────────────────────────────────────────────────────

/// A box whose spacing is a step of the space scale (1-based, §3.5).
pub fn stack(orientation: Orientation, step: usize) -> gtk4::Box {
    gtk4::Box::new(orientation, space(step))
}

pub fn vbox(step: usize) -> gtk4::Box {
    stack(Orientation::Vertical, step)
}

pub fn hbox(step: usize) -> gtk4::Box {
    stack(Orientation::Horizontal, step)
}

/// Padding on all four sides from the space scale, as margins.
pub fn pad(w: &impl IsA<gtk4::Widget>, step: usize) {
    let p = space(step);
    w.set_margin_top(p);
    w.set_margin_bottom(p);
    w.set_margin_start(p);
    w.set_margin_end(p);
}

// ── Surface and card ────────────────────────────────────────────────────

/// Mark a window's root as a design-system surface: base type and colour.
pub fn surface(w: &impl IsA<gtk4::Widget>) {
    w.add_css_class("ui-surface");
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Card {
    /// A floating card: panel, launcher, notifications, polkit, lock.
    Floating,
    /// Thin glass: the bar, the OSD, the face cue.
    Thin,
}

/// Make `w` a glass card.
pub fn card(w: &impl IsA<gtk4::Widget>, kind: Card) {
    w.add_css_class("ui-card");
    if kind == Card::Thin {
        w.add_css_class("thin");
    }
}

/// A fill inside a card, for grouping (never glass on glass).
pub fn group(step: usize) -> gtk4::Box {
    let b = vbox(step);
    b.add_css_class("ui-group");
    b
}

pub fn separator() -> gtk4::Box {
    let s = gtk4::Box::new(Orientation::Horizontal, 0);
    s.add_css_class("ui-separator");
    s
}

// ── Button ──────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Primary,
    Secondary,
    Flat,
    Destructive,
}

fn style_button(b: &gtk4::Button, kind: Kind) {
    b.add_css_class("ui-btn");
    match kind {
        Kind::Primary => b.add_css_class("primary"),
        Kind::Flat => b.add_css_class("flat"),
        Kind::Destructive => b.add_css_class("destructive"),
        Kind::Secondary => {}
    }
}

pub fn button(label: &str, kind: Kind) -> gtk4::Button {
    let b = gtk4::Button::with_label(label);
    style_button(&b, kind);
    b
}

/// Restyle an existing button as a component button.
pub fn make_button(b: &gtk4::Button, kind: Kind) {
    style_button(b, kind);
}

pub fn icon_button(icon: &str, tooltip: &str, kind: Kind) -> gtk4::Button {
    let b = gtk4::Button::from_icon_name(icon);
    style_button(&b, kind);
    b.add_css_class("icon");
    b.set_tooltip_text(Some(tooltip));
    b
}

/// A button whose face is a glyph from the icon font.
pub fn glyph_button(glyph: &str, tooltip: &str, kind: Kind) -> gtk4::Button {
    let b = gtk4::Button::with_label(glyph);
    style_button(&b, kind);
    b.add_css_class("icon");
    b.set_tooltip_text(Some(tooltip));
    b
}

// ── Row ─────────────────────────────────────────────────────────────────

/// One list row: an icon, a title over a subtitle, and an end slot.
pub struct Row {
    pub root: gtk4::Box,
    pub icon: gtk4::Label,
    pub title: gtk4::Label,
    pub subtitle: gtk4::Label,
    pub end: gtk4::Box,
}

/// `icon` is a glyph from the icon font; empty for none.
pub fn row(icon: &str, title: &str, subtitle: &str) -> Row {
    let root = hbox(4);
    root.add_css_class("ui-row");
    let icon_l = gtk4::Label::new(Some(icon));
    icon_l.add_css_class("ui-row-icon");
    icon_l.set_visible(!icon.is_empty());
    let texts = vbox(0);
    texts.set_valign(Align::Center);
    texts.set_hexpand(true);
    let title_l = gtk4::Label::new(Some(title));
    title_l.add_css_class("ui-row-title");
    title_l.set_xalign(0.0);
    title_l.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    let sub = gtk4::Label::new(Some(subtitle));
    sub.add_css_class("ui-row-subtitle");
    sub.set_xalign(0.0);
    sub.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    sub.set_visible(!subtitle.is_empty());
    texts.append(&title_l);
    texts.append(&sub);
    let end = hbox(3);
    end.add_css_class("ui-row-end");
    end.set_valign(Align::Center);
    root.append(&icon_l);
    root.append(&texts);
    root.append(&end);
    Row {
        root,
        icon: icon_l,
        title: title_l,
        subtitle: sub,
        end,
    }
}

/// A row you can press: the row inside a flat button.
pub fn row_button(icon: &str, title: &str, subtitle: &str) -> (gtk4::Button, Row) {
    let r = row(icon, title, subtitle);
    r.root.remove_css_class("ui-row");
    let b = gtk4::Button::new();
    b.add_css_class("ui-row");
    b.add_css_class("activatable");
    b.set_child(Some(&r.root));
    (b, r)
}

pub fn set_selected(w: &impl IsA<gtk4::Widget>, selected: bool) {
    if selected {
        w.add_css_class("selected");
    } else {
        w.remove_css_class("selected");
    }
}

// ── Section ─────────────────────────────────────────────────────────────

/// A collapsible group: a header that says what is inside and its state,
/// and a body that opens under it.
pub struct Section {
    pub root: gtk4::Box,
    pub header: gtk4::Button,
    pub icon: gtk4::Label,
    pub title: gtk4::Label,
    pub summary: gtk4::Label,
    pub body: gtk4::Box,
    pub revealer: gtk4::Revealer,
}

pub fn section(icon: &str, title: &str, summary: &str) -> Section {
    let root = vbox(0);
    root.add_css_class("ui-section");
    let header = gtk4::Button::new();
    header.add_css_class("ui-section-header");
    let line = hbox(3);
    let icon_l = gtk4::Label::new(Some(icon));
    icon_l.add_css_class("ui-row-icon");
    let title_l = gtk4::Label::new(Some(title));
    title_l.add_css_class("ui-section-title");
    title_l.set_xalign(0.0);
    let summary_l = gtk4::Label::new(Some(summary));
    summary_l.add_css_class("ui-section-summary");
    summary_l.set_hexpand(true);
    summary_l.set_xalign(1.0);
    summary_l.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    let chevron = gtk4::Image::from_icon_name("pan-end-symbolic");
    chevron.add_css_class("ui-section-chevron");
    line.append(&icon_l);
    line.append(&title_l);
    line.append(&summary_l);
    line.append(&chevron);
    header.set_child(Some(&line));
    let body = vbox(1);
    body.add_css_class("ui-section-body");
    let revealer = gtk4::Revealer::builder()
        .transition_type(gtk4::RevealerTransitionType::SlideDown)
        .transition_duration(crate::anim::duration(crate::tokens::motion::EXPAND.ms) as u32)
        .child(&body)
        .build();
    root.append(&header);
    root.append(&revealer);
    {
        let (root, revealer) = (root.clone(), revealer.clone());
        header.connect_clicked(move |_| {
            let open = !revealer.reveals_child();
            revealer.set_reveal_child(open);
            if open {
                root.add_css_class("open");
            } else {
                root.remove_css_class("open");
            }
        });
    }
    Section {
        root,
        header,
        icon: icon_l,
        title: title_l,
        summary: summary_l,
        body,
        revealer,
    }
}

impl Section {
    pub fn set_open(&self, open: bool) {
        self.revealer.set_reveal_child(open);
        if open {
            self.root.add_css_class("open");
        } else {
            self.root.remove_css_class("open");
        }
    }
}

// ── Slider and switch ───────────────────────────────────────────────────

pub struct SliderRow {
    pub root: gtk4::Box,
    pub icon: gtk4::Label,
    pub scale: gtk4::Scale,
    pub value: gtk4::Label,
}

/// A slider in a full-width row of its own (principle: sliders own a row).
pub fn slider_row(icon: &str, min: f64, max: f64, step: f64) -> SliderRow {
    let root = hbox(4);
    root.add_css_class("ui-slider-row");
    let icon_l = gtk4::Label::new(Some(icon));
    icon_l.add_css_class("ui-slider-icon");
    let scale = gtk4::Scale::with_range(Orientation::Horizontal, min, max, step);
    scale.add_css_class("ui-slider");
    scale.set_hexpand(true);
    scale.set_draw_value(false);
    let value = gtk4::Label::new(None);
    value.add_css_class("ui-slider-value");
    value.set_xalign(1.0);
    root.append(&icon_l);
    root.append(&scale);
    root.append(&value);
    SliderRow {
        root,
        icon: icon_l,
        scale,
        value,
    }
}

pub fn slider(s: &gtk4::Scale) {
    s.add_css_class("ui-slider");
}

pub fn switch() -> gtk4::Switch {
    let s = gtk4::Switch::new();
    s.add_css_class("ui-switch");
    s.set_valign(Align::Center);
    s
}

/// A row with a switch at its end.
pub fn switch_row(title: &str, subtitle: &str) -> (Row, gtk4::Switch) {
    let r = row("", title, subtitle);
    let s = switch();
    r.end.append(&s);
    (r, s)
}

// ── Chip, badge, key, status ────────────────────────────────────────────

pub fn chip(label: &str) -> gtk4::Button {
    let b = gtk4::Button::with_label(label);
    b.add_css_class("ui-chip");
    b
}

pub fn chip_label(label: &str) -> gtk4::Label {
    let l = gtk4::Label::new(Some(label));
    l.add_css_class("ui-chip");
    l
}

pub fn badge(text: &str) -> gtk4::Label {
    let l = gtk4::Label::new(Some(text));
    l.add_css_class("ui-badge");
    l
}

pub fn key(text: &str) -> gtk4::Label {
    let l = gtk4::Label::new(Some(text));
    l.add_css_class("ui-key");
    l
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Ok,
    Warn,
    Bad,
    Neutral,
}

/// A status: a dot and a word, both in the status colour.
pub fn status(kind: Status, label: &str) -> gtk4::Label {
    let l = gtk4::Label::new(Some(&format!("\u{25cf} {label}")));
    l.add_css_class("ui-status");
    set_status(&l, kind);
    l
}

pub fn set_status(l: &gtk4::Label, kind: Status) {
    for c in ["ok", "warn", "bad", "neutral"] {
        l.remove_css_class(c);
    }
    l.add_css_class(match kind {
        Status::Ok => "ok",
        Status::Warn => "warn",
        Status::Bad => "bad",
        Status::Neutral => "neutral",
    });
}

// ── Field ───────────────────────────────────────────────────────────────

pub struct Field {
    pub root: gtk4::Box,
    pub help: gtk4::Label,
}

/// A labelled input with a help line that turns into the error.
pub fn field(label: &str, input: &impl IsA<gtk4::Widget>, help: &str) -> Field {
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
    let h = gtk4::Label::new(Some(help));
    h.add_css_class("ui-field-help");
    h.set_xalign(0.0);
    h.set_wrap(true);
    h.set_visible(!help.is_empty());
    root.append(&h);
    Field { root, help: h }
}

impl Field {
    /// Show `msg` as the error, or clear it.
    pub fn set_error(&self, msg: Option<&str>) {
        match msg {
            Some(m) => {
                self.root.add_css_class("error");
                self.help.set_text(m);
                self.help.set_visible(true);
            }
            None => {
                self.root.remove_css_class("error");
            }
        }
    }
}

pub fn entry(input: &impl IsA<gtk4::Widget>) {
    input.add_css_class("ui-entry");
}

// ── Menu ────────────────────────────────────────────────────────────────

pub fn menu() -> gtk4::Box {
    let b = vbox(0);
    b.add_css_class("ui-menu");
    b
}

pub fn menu_item(label: &str, accel: &str, danger: bool) -> gtk4::Button {
    let b = gtk4::Button::new();
    b.add_css_class("ui-menu-item");
    if danger {
        b.add_css_class("danger");
    }
    let line = hbox(4);
    let l = gtk4::Label::new(Some(label));
    l.set_hexpand(true);
    l.set_xalign(0.0);
    line.append(&l);
    if !accel.is_empty() {
        let a = gtk4::Label::new(Some(accel));
        a.add_css_class("ui-menu-accel");
        line.append(&a);
    }
    b.set_child(Some(&line));
    b
}

// ── Progress ────────────────────────────────────────────────────────────

pub fn progress(fraction: f64) -> gtk4::ProgressBar {
    let p = gtk4::ProgressBar::new();
    p.add_css_class("ui-progress");
    p.set_fraction(fraction);
    p
}

pub fn make_progress(p: &gtk4::ProgressBar) {
    p.add_css_class("ui-progress");
}

// ── Bar components (added by the bar migration) ─────────────────────────

/// Put any widget's text in a tone, not only a label's (a box whose
/// children inherit it).
pub fn set_tone(w: &impl IsA<gtk4::Widget>, tone: Tone) {
    for c in [
        "ui-muted",
        "ui-faint",
        "ui-accent",
        "ui-success",
        "ui-warning",
        "ui-danger",
    ] {
        w.remove_css_class(c);
    }
    if let Some(c) = tone.class() {
        w.add_css_class(c);
    }
}

/// Toggle a component modifier class.
pub fn set_class(w: &impl IsA<gtk4::Widget>, class: &str, on: bool) {
    if on {
        w.add_css_class(class);
    } else {
        w.remove_css_class(class);
    }
}

/// The categorical slots (§3.1): identity only, 1-based.
pub const CATEGORIES: usize = 6;

/// Text in a categorical tone, `n` in 1..=6; anything else clears it.
pub fn set_category(w: &impl IsA<gtk4::Widget>, n: usize) {
    for i in 1..=CATEGORIES {
        w.remove_css_class(&format!("ui-cat-{i}"));
    }
    if (1..=CATEGORIES).contains(&n) {
        w.add_css_class(&format!("ui-cat-{n}"));
    }
}

/// Step a group back as a whole, or bring it forward.
pub fn set_receded(w: &impl IsA<gtk4::Widget>, receded: bool) {
    set_class(w, "ui-receded", receded);
}

/// A Cairo-drawn meter: its `color()` is the accent fill.
pub fn meter(area: &gtk4::DrawingArea) {
    area.add_css_class("ui-meter");
}

// ── Segment ─────────────────────────────────────────────────────────────

/// A track of fused segments; only its ends round. No gap: the segments
/// touch, and a divider separates them.
pub fn segmented() -> gtk4::Box {
    let b = gtk4::Box::new(Orientation::Horizontal, 0);
    b.add_css_class("ui-segmented");
    b
}

/// Make `w` a segment. `quiet` keeps its label muted until it is selected
/// or under the pointer.
pub fn segment(w: &impl IsA<gtk4::Widget>, quiet: bool) {
    w.add_css_class("ui-segment");
    set_class(w, "quiet", quiet);
}

/// Where a segment stands in its control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selection {
    Idle,
    /// What its screen shows.
    Current,
    /// Current, on the screen holding input.
    Focused,
}

pub fn set_selection(w: &impl IsA<gtk4::Widget>, s: Selection) {
    set_class(w, "current", s != Selection::Idle);
    set_class(w, "focused", s == Selection::Focused);
}

/// The one red: act now (an urgent workspace, a dying battery).
pub fn set_danger(w: &impl IsA<gtk4::Widget>, danger: bool) {
    set_class(w, "danger", danger);
}

/// What a segment's ribbon lane says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ribbon {
    Off,
    /// A line: something is live here.
    Working,
    /// The categorical tone `n` (1..=4): this one wants you.
    Category(usize),
}

/// Give a segment its 2 px ribbon lane, transparent until set.
pub fn ribboned(w: &impl IsA<gtk4::Widget>) {
    w.add_css_class("ribboned");
}

pub fn set_ribbon(w: &impl IsA<gtk4::Widget>, r: Ribbon) {
    set_class(w, "ribbon-working", r == Ribbon::Working);
    for n in 1..=4 {
        set_class(w, &format!("ribbon-cat-{n}"), r == Ribbon::Category(n));
    }
}

// ── Mark ────────────────────────────────────────────────────────────────

/// A quiet button straight on thin glass: no fill at rest, muted, ink
/// under the pointer. `quiet` sits it one level lower, at faint.
pub fn mark(child: &impl IsA<gtk4::Widget>, quiet: bool) -> gtk4::Button {
    let b = gtk4::Button::builder().child(child).build();
    make_mark(&b, quiet);
    b
}

pub fn make_mark(b: &gtk4::Button, quiet: bool) {
    b.add_css_class("ui-mark");
    set_class(b, "quiet", quiet);
}

// ── Bay ─────────────────────────────────────────────────────────────────

/// One task's slot on the board; `task` (1..=4) picks its tone.
pub fn bay(child: &impl IsA<gtk4::Widget>, task: usize) -> gtk4::Button {
    let b = gtk4::Button::builder().child(child).build();
    b.add_css_class("ui-bay");
    b.add_css_class(&format!("cat-{task}"));
    b
}

/// A bay's state, as the classes the component styles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BayState {
    Socket,
    Working,
    /// Halted on a prompt; rides the unacked fill.
    Blocked,
    Waiting {
        acked: bool,
        overdue: bool,
    },
    Stopped,
    Stale,
}

impl BayState {
    pub fn classes(self) -> &'static [&'static str] {
        match self {
            BayState::Socket => &["socket"],
            BayState::Working => &["working"],
            BayState::Blocked => &["waiting", "unacked", "blocked"],
            BayState::Waiting { acked: true, .. } => &["waiting"],
            BayState::Waiting {
                acked: false,
                overdue: false,
            } => &["waiting", "unacked"],
            BayState::Waiting {
                acked: false,
                overdue: true,
            } => &["waiting", "unacked", "overdue"],
            BayState::Stopped => &["stopped"],
            BayState::Stale => &["stale"],
        }
    }
}

pub fn set_bay_state(b: &gtk4::Button, state: BayState, local: bool) {
    for c in [
        "socket", "working", "waiting", "blocked", "unacked", "overdue", "stopped", "stale",
    ] {
        b.remove_css_class(c);
    }
    for c in state.classes() {
        b.add_css_class(c);
    }
    set_class(b, "local", local);
}

/// The age chip beside a bay's numeral.
pub fn bay_chip() -> gtk4::Label {
    let l = gtk4::Label::new(None);
    l.add_css_class("ui-bay-chip");
    l
}

// ── Popover and list ────────────────────────────────────────────────────

/// A popover that draws nothing itself; its child is the card.
pub fn popover(child: &impl IsA<gtk4::Widget>, position: gtk4::PositionType) -> gtk4::Popover {
    gtk4::Popover::builder()
        .position(position)
        .has_arrow(false)
        .css_classes(["ui-popover"])
        .child(child)
        .build()
}

/// A card with no glass behind it (a popup, outside the compositor's
/// layer effects): the same shape, a solid raised fill.
pub fn solid_card(w: &impl IsA<gtk4::Widget>) {
    card(w, Card::Floating);
    w.add_css_class("solid");
}

/// A ListBox that draws nothing itself, for rows of `ui-row` content that
/// light under the pointer or the keyboard.
pub fn list() -> gtk4::ListBox {
    let l = gtk4::ListBox::builder()
        .selection_mode(gtk4::SelectionMode::None)
        .build();
    l.add_css_class("ui-list");
    l
}

/// A rounded frame for an image, filled while it has none.
pub fn thumb() -> gtk4::Box {
    let b = gtk4::Box::builder()
        .halign(Align::Center)
        .valign(Align::Center)
        .overflow(gtk4::Overflow::Hidden)
        .build();
    b.add_css_class("ui-thumb");
    b
}

/// A button in a dense place (a popover's footer): label size, low.
pub fn small_button(label: &str, kind: Kind) -> gtk4::Button {
    let b = button(label, kind);
    b.add_css_class("small");
    b
}

/// A row of a `ui::list`: `content` becomes the row's padded, lit face.
pub fn list_row(content: &impl IsA<gtk4::Widget>) -> gtk4::ListBoxRow {
    content.add_css_class("ui-row");
    gtk4::ListBoxRow::builder().child(content).build()
}

// ── Auth components (added by the auth migration) ───────────────────────

/// Make `w` the glass card that sits over a [`scrim`]: the lock's and the
/// greeter's. It paints the key pre-compensated for the black under it, so
/// the two layers composite to exactly the key every other card paints and
/// the compositor drops them (`.ui-card.over-scrim` in 00-components.css has
/// the arithmetic). Only ever over a scrim: on its own it lands in the band
/// `glass.nix` reserves for nothing, a flat slab with no bevel.
pub fn card_over_scrim(w: &impl IsA<gtk4::Widget>) {
    card(w, Card::Floating);
    w.add_css_class("over-scrim");
}

/// The dimming layer under a modal full-screen surface (`--scrim`).
pub fn scrim() -> gtk4::Box {
    let b = vbox(0);
    b.add_css_class("ui-scrim");
    b.set_hexpand(true);
    b.set_vexpand(true);
    b
}

/// What a surface paints when it only needs *a* commit, never a visible
/// pixel: the lock's commit pixel, drawn at 1–2/255 alpha so GSK sees a
/// changed node and nobody sees anything. Black is the scrim's colour, so
/// even that one 255th adds nothing the scrim under it does not already.
pub const INVISIBLE_INK: crate::tokens::Rgb = crate::tokens::Rgb::BLACK;

/// Text that stands on bare wallpaper, with no card behind it: gives the
/// glyphs an edge with a shadow of the scrim.
pub fn on_wallpaper(w: &impl IsA<gtk4::Widget>) {
    w.add_css_class("ui-on-wallpaper");
}

/// A well: a box sunk below the card, for text that came from outside it
/// (a command line, polkit's raw details).
pub fn well() -> gtk4::Box {
    let b = vbox(0);
    b.add_css_class("ui-well");
    b
}

/// Style a dropdown as a control on the card.
pub fn dropdown(d: &gtk4::DropDown) {
    d.add_css_class("ui-dropdown");
}

/// A chip whose face is a widget rather than a word: an avatar and a name.
pub fn chip_with(child: &impl IsA<gtk4::Widget>) -> gtk4::Button {
    let b = gtk4::Button::new();
    b.add_css_class("ui-chip");
    b.add_css_class("rich");
    b.set_child(Some(child));
    b
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
    set_class(field, state.class(), on);
}

/// A caption whose tone changes in place: the colour moves as a state.
pub fn live_caption(l: &gtk4::Label) {
    l.add_css_class("ui-live-caption");
}

/// Mark `label` monospace: a command, a key, raw details.
pub fn mono(label: &gtk4::Label) {
    label.add_css_class("ui-mono");
}

// ── Avatar ──────────────────────────────────────────────────────────────

/// A round avatar for `name`: the picture at `icon_path` when it loads,
/// otherwise a monogram on a categorical fill hashed from the name. `size` is
/// the diameter in px; `logged_in` adds the presence dot. Add `.active` to
/// ring the current user.
pub fn avatar(name: &str, icon_path: Option<&str>, size: i32, logged_in: bool) -> gtk4::Widget {
    crate::avatar::avatar(name, icon_path, size, logged_in)
}

// ── Face indicator ──────────────────────────────────────────────────────

/// Every state the face can be in. Enumerated rather than derived, because
/// swapping to a new state means clearing the old ones and GTK cannot be
/// asked which is set.
pub const FACE_STATES: [&str; 5] = ["looking", "dark", "found", "ok", "fail"];

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
    /// Carries the entrance (`ui-face-enter`, toggled with `set_class`), so
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
    card(&pill, Card::Thin);
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

/// Put the ring (and the pill, if given) into `state`, one of
/// [`FACE_STATES`]. Empty clears without setting anything, which is what a
/// hidden indicator wants: a stale class on a hidden widget makes the next
/// show start mid-animation in the previous state.
///
/// The pill carries the state as well as the ring because three states say
/// something the ring cannot: `dark` and `ok` recolour the pill, `looking`
/// breathes its border. The ring keeps the face's motion and the verdict
/// keyframes, so the two never animate one property on nested nodes.
pub fn set_face_state(ring: &gtk4::Box, pill: Option<&gtk4::Box>, state: &str) {
    for old in FACE_STATES {
        ring.remove_css_class(old);
        if let Some(pill) = pill {
            pill.remove_css_class(old);
        }
    }
    if state.is_empty() {
        return;
    }
    ring.add_css_class(state);
    if let Some(pill) = pill {
        pill.add_css_class(state);
    }
}
