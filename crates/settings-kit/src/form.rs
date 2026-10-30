//! The rows the settings tabs are built from, so the five of them read as
//! one pane: the same label gutter, the same hint placement, the same
//! footer. Built only from the components (`crate::ui`), which carry the
//! colour, type and shape; `crates/ui/data/css/15-settings.css` only places things.
//!
//! Each group carries a visible hint; a row carries its long hint in a
//! tooltip, and a short visible line under its name where the name does
//! not say enough ([`subtitle`]). The pane is dense on purpose (see
//! `crates/ui/data/css/15-settings.css`).

use std::path::Path;

use gtk4::prelude::*;

use swaypplet_ui::{
    ui,
    ui::{Kind, Text, Tone},
};

/// What a wrapping label is allowed to ASK for, in characters.
///
/// A GtkLabel with `wrap` set still requests the whole text on one line as
/// its natural width, and it gets it when the parent has room: the hint on
/// the Quiet Hours group is 780 px of text, which made the settings pane ask
/// for 886 px and stretched every row on every tab to match (a GtkStack
/// takes the widest page). This caps the request; the label still wraps to
/// whatever width it is finally given, so at the pane's full column it fills
/// the line as before.
pub const HINT_CHARS: i32 = 64;

/// A subtitle's width beside a rail, in characters: what fits the rail
/// rows' gutter (`.settings-rail-label`).
const RAIL_GUTTER_CHARS: i32 = 22;

/// The label of a row with a rail (a scale), in the rails' gutter: one
/// width for every rail row of every pane, so the rails start at one x
/// with or without a subtitle under the name.
pub fn rail_label(label: &str) -> gtk4::Label {
    let name = row_label(label);
    name.add_css_class("settings-rail-label");
    name.set_xalign(0.0);
    name
}

/// A tab's column of groups.
pub fn pane() -> gtk4::Box {
    let root = ui::vbox(4);
    root.set_hexpand(true);
    root
}

/// A titled run of rows: a group fill, its name, and the one visible hint.
pub fn section_box(title: &str, hint: &str) -> gtk4::Box {
    let container = ui::group(1);
    container.set_hexpand(true);
    container.add_css_class("settings-group");
    container.append(&ui::overline(title, ui::Tone::Muted));

    let sub = hint_label(hint);
    sub.add_css_class("settings-group-hint");
    container.append(&sub);

    container
}

/// Running text under a group's name or in a status line: faint, wrapping,
/// capped at [`HINT_CHARS`].
fn hint_label(text: &str) -> gtk4::Label {
    let l = ui::text(text, Text::Caption, Tone::Faint);
    l.set_wrap(true);
    l.set_max_width_chars(HINT_CHARS);
    l
}

/// The label in a row's gutter.
pub fn row_label(label: &str) -> gtk4::Label {
    let name = ui::text(label, Text::Body, Tone::Fg);
    name.add_css_class("settings-row-label");
    name
}

/// A short line under a row's name, for a row whose name does not say
/// enough by itself ("Screen off on battery": shorter, on battery only).
/// A few words, plain; the row's tooltip keeps the long hint. The name and
/// the line stand in one column at the row's start, so the control keeps
/// its place at the far end.
pub fn subtitle(row: &gtk4::Box, text: &str) {
    let Some(name) = row.first_child() else {
        return;
    };
    if !name.has_css_class("settings-row-label") {
        return;
    }
    let column = ui::vbox(0);
    column.set_valign(gtk4::Align::Center);
    column.set_hexpand(name.hexpands());
    row.add_css_class("settings-row-two-line");
    row.insert_child_after(&column, Some(&name));
    row.remove(&name);
    column.append(&name);
    let line = ui::text(text, Text::Caption, Tone::Muted);
    line.set_xalign(0.0);
    line.set_wrap(true);
    line.set_max_width_chars(HINT_CHARS);
    // Beside a rail the column is the rail's fixed gutter: the line wraps
    // inside it, so every rail of the pane still starts at one x.
    if name.has_css_class("settings-rail-label") {
        name.remove_css_class("settings-rail-label");
        column.add_css_class("settings-rail-label");
        line.set_max_width_chars(RAIL_GUTTER_CHARS);
    }
    column.append(&line);
}

/// The value beside a rail: fixed width, tabular, so the number does not
/// shove the rail as it grows a digit.
pub fn value_label() -> gtk4::Label {
    let value = ui::text("", Text::Caption, Tone::Muted);
    ui::set_mono(&value, true);
    ui::set_numeric(&value, true);
    value.set_xalign(1.0);
    value.set_width_chars(6);
    value
}

/// The words beside a control (an offset, a place, today's times): the
/// value label's place and tone, in the text face. Mono is for numbers
/// that change in place; a sentence in it reads as code.
pub fn words_label() -> gtk4::Label {
    let value = ui::text("", Text::Caption, Tone::Muted);
    ui::set_numeric(&value, true);
    value.set_xalign(1.0);
    value
}

/// A rail for a dense column of them.
pub fn scale(min: f64, max: f64, step: f64) -> gtk4::Scale {
    let scale = gtk4::Scale::with_range(gtk4::Orientation::Horizontal, min, max, step);
    scale.set_draw_value(false);
    scale.set_hexpand(true);
    ui::slider::adopt(&scale, ui::Density::Dense);
    scale
}

/// An empty row, for the helpers below to fill.
pub fn row() -> gtk4::Box {
    let row = ui::hbox(3);
    row.add_css_class("settings-row");
    row
}

/// A label, then one control at the row's far end, at its natural width
/// but never narrower than `.settings-control`, so a column of dropdowns
/// lines up on both edges: the row reads label, then value, as a sentence
/// does (macOS System Settings, GNOME 47, iOS). A text field fills the
/// row instead. Filling every row from a fixed gutter was the other answer
/// the 2026-09 alternatives zoo rendered (docs/alternatives-zoo.html); the
/// owner chose this one.
pub fn kind_row(label: &str, control: &impl IsA<gtk4::Widget>) -> gtk4::Box {
    let row = row();
    let name = row_label(label);
    row.append(&name);

    let control = control.as_ref();
    // A text field is the one control that is better wide: what you type
    // into it is read there.
    if control.is::<gtk4::Entry>() {
        control.set_hexpand(true);
    } else {
        name.set_hexpand(true);
        control.set_halign(gtk4::Align::End);
        control.add_css_class("settings-control");
        // A box of small things (the accent swatches, two buttons) is
        // narrower than the control column, and a box packs from its start:
        // its contents ended short of the column's right edge. Its first
        // child takes the slack and stands at the end of it, so the row
        // ends where every dropdown's does. A box whose children already
        // share the width (a clock) is left alone.
        if let Some(group) = control.downcast_ref::<gtk4::Box>() {
            let mut child = group.first_child();
            let mut fills = false;
            while let Some(c) = child {
                fills |= c.hexpands();
                child = c.next_sibling();
            }
            if !fills && let Some(first) = group.first_child() {
                first.set_hexpand(true);
                first.set_halign(gtk4::Align::End);
            }
        }
    }
    row.append(control);
    row
}

/// A label, the hint as its tooltip, and a switch at the far end.
pub fn switch_row(label: &str, hint: &str, active: bool) -> (gtk4::Box, gtk4::Switch) {
    let row = row();
    row.set_tooltip_text(Some(hint));

    let name = row_label(label);
    name.set_hexpand(true);
    row.append(&name);

    let switch = ui::switch();
    switch.set_active(active);
    row.append(&switch);
    (row, switch)
}

/// Rows that only mean something while a switch above them is on: indented
/// one step under it, and insensitive as a whole (the label with the
/// control) while it is off, so a row that cannot change says so the same
/// way on every pane. The values stay on screen and in the file.
pub fn dependent(rows: &[&gtk4::Box]) {
    for row in rows {
        row.add_css_class("settings-dependent");
    }
}

/// Set the rows [`dependent`] marked as live or not.
pub fn set_live(rows: &[&gtk4::Box], on: bool) {
    for row in rows {
        row.set_sensitive(on);
    }
}

/// A dropdown over `choices`, the pane's height.
pub fn dropdown(choices: &[&str]) -> gtk4::DropDown {
    ui::dropdown(choices)
}

/// A label and a dropdown over `choices`.
pub fn dropdown_row(label: &str, hint: &str, choices: &[&str]) -> (gtk4::Box, gtk4::DropDown) {
    let dropdown = dropdown(choices);
    let row = kind_row(label, &dropdown);
    row.set_tooltip_text(Some(hint));
    (row, dropdown)
}

/// A label, a rail and the value beside it, formatted by `show`.
///
/// The value is snapped to `step` in the handler rather than trusted from
/// the adjustment: a Scale's step only governs the keyboard and the wheel,
/// so a drag hands back a continuous value.
pub fn scale_row(
    label: &str,
    hint: &str,
    range: (f64, f64, f64),
    show: fn(f64) -> String,
) -> (gtk4::Box, gtk4::Scale) {
    let (min, max, step) = range;
    let row = row();
    row.set_tooltip_text(Some(hint));
    row.append(&rail_label(label));

    let scale = scale(min, max, step);
    let value = value_label();
    value.set_text(&show(min));
    {
        let value = value.clone();
        scale.connect_value_changed(move |s| {
            let snapped = (s.value() / step).round() * step;
            value.set_text(&show(snapped));
        });
    }

    row.append(&scale);
    row.append(&value);
    (row, scale)
}

/// A button in a group of presets.
pub fn preset_button(label: &str) -> gtk4::Button {
    ui::button(label, Kind::Secondary)
}

/// The line saying where a tab's values currently come from.
pub fn status_label() -> gtk4::Label {
    hint_label("")
}

/// Where the values come from: `system` is faint, an override plain. The one
/// piece of state the screen behind the pane does not show.
pub fn mark_source(status: &gtk4::Label, system: bool) {
    ui::set_text_style(
        status,
        Text::Caption,
        if system { Tone::Faint } else { Tone::Muted },
    );
}

/// The strip under a tab: its action buttons, and a line saying where the
/// values currently come from.
pub fn footer(buttons: &[&gtk4::Button]) -> (gtk4::Box, gtk4::Label) {
    let footer = ui::vbox(3);
    footer.add_css_class("settings-footer");

    let row = ui::hbox(3);
    for button in buttons {
        row.append(*button);
    }
    footer.append(&row);

    let status = status_label();
    footer.append(&status);

    (footer, status)
}

/// "Copy as Nix": `render` on click, into the clipboard, and `done` on the
/// status line to say where to paste it.
pub fn copy_button(
    status: &gtk4::Label,
    hint: &str,
    done: &'static str,
    render: impl Fn() -> Option<String> + 'static,
) -> gtk4::Button {
    let button = action_button("Copy as Nix", hint);
    let status = status.clone();
    button.connect_clicked(move |_| {
        let Some(text) = render() else {
            return;
        };
        match gtk4::gdk::Display::default() {
            Some(display) => {
                display.clipboard().set_text(&text);
                status.set_text(done);
            }
            None => log::warn!("settings: no display, cannot reach the clipboard"),
        }
    });
    button
}

/// [`copy_button`] for a settings section, which pastes into
/// `theme/settings.nix`.
pub fn copy_nix_button(
    status: &gtk4::Label,
    hint: &str,
    render: impl Fn() -> Option<String> + 'static,
) -> gtk4::Button {
    copy_button(
        status,
        hint,
        "Copied — paste into theme/settings.nix",
        render,
    )
}

/// The footer every settings pane ends with: Reset and its Undo, "Copy as
/// Nix" for the pane's `sections` beside them, and the status line under
/// them. Returns the footer and its status line.
pub fn reset_footer(
    reset: &ResetButtons,
    copy_hint: &str,
    sections: &'static [&'static str],
) -> (gtk4::Box, gtk4::Label) {
    let (footer, status) = footer(&[&reset.button, &reset.undo]);
    let copy = copy_nix_button(&status, copy_hint, move || {
        sections_as_nix(&swaypplet_model::store::current(), sections)
    });
    // Beside Reset, in the row `footer` built.
    if let Some(row) = reset.button.parent().and_downcast::<gtk4::Box>() {
        row.append(&copy);
    }
    (footer, status)
}

/// `sections` of `settings` as `theme/settings.nix` holds them, one after
/// the other; `None` if any of them is not a section that has a Nix side.
pub fn sections_as_nix(
    settings: &swaypplet_model::store::Settings,
    sections: &[&str],
) -> Option<String> {
    sections
        .iter()
        .map(|section| settings.section_as_nix(section))
        .collect()
}

/// The guard a pane keeps while it writes its own controls from the
/// store: those writes fire the controls' change signals, and an edit from
/// a signal the pane fired itself must not go back to the store.
#[derive(Debug, Default)]
pub struct Sync {
    writing: std::cell::Cell<bool>,
}

impl Sync {
    /// Whether the pane is writing its controls now, so a change signal is
    /// its own and not an edit.
    pub fn busy(&self) -> bool {
        self.writing.get()
    }

    /// Write the controls with the guard up. Nests: an inner write leaves
    /// the guard as the outer one had it.
    pub fn writing<R>(&self, write: impl FnOnce() -> R) -> R {
        let was = self.writing.replace(true);
        let out = write();
        self.writing.set(was);
        out
    }

    /// An edit from a control: into the store, then `sync` to bring the
    /// other controls along. Nothing while the pane is writing.
    pub fn edit<S: swaypplet_model::store::Section>(
        &self,
        f: impl FnOnce(&mut S),
        sync: impl FnOnce(),
    ) {
        if self.busy() {
            return;
        }
        swaypplet_model::store::edit(f);
        sync();
    }
}

/// "Reset to system", and the "Undo reset" that stands beside it after a
/// reset until anything in the settings changes again. Every settings pane
/// has the pair at the head of its footer. [`ResetButtons::connect`] says
/// what the reset does; the undo puts back exactly the sections it changed,
/// then runs `after_undo` for what the store does not reach by itself (the
/// wallpaper on screen, the controls). A reset offers its way back rather
/// than asking first: the question would be asked every time and read
/// never; the undo is there for when it was a mistake.
pub struct ResetButtons {
    pub button: gtk4::Button,
    pub undo: gtk4::Button,
    /// The file before the reset and right after it, while the undo stands.
    pending: std::rc::Rc<
        std::cell::RefCell<
            Option<(
                swaypplet_model::store::Settings,
                swaypplet_model::store::Settings,
            )>,
        >,
    >,
    /// Ask first ([`swaypplet_ui::ui::arm`]): for a reset that throws away
    /// something made here, the undo aside.
    ask: Option<&'static str>,
}

impl ResetButtons {
    pub fn new(hint: &str) -> ResetButtons {
        let button = action_button("Reset to system", hint);
        let undo = action_button(
            "Undo reset",
            "Put back what the reset took away. Gone once anything changes again.",
        );
        undo.set_visible(false);
        ResetButtons {
            button,
            undo,
            pending: Default::default(),
            ask: None,
        }
    }

    /// The reset asks `question` on its first press (`ui::arm`).
    pub fn asking(mut self, question: &'static str) -> ResetButtons {
        self.ask = Some(question);
        self
    }

    /// What a press of Reset does, and what follows an undo.
    pub fn connect(&self, reset: impl Fn() + 'static, after_undo: impl Fn() + 'static) {
        {
            let (pending, undo) = (self.pending.clone(), self.undo.clone());
            let press = move |_: &gtk4::Button| {
                pending.borrow_mut().take();
                let before = swaypplet_model::store::current();
                reset();
                let after = swaypplet_model::store::current();
                if before != after {
                    *pending.borrow_mut() = Some((before, after));
                    undo.set_visible(true);
                }
            };
            match self.ask {
                Some(question) => swaypplet_ui::ui::arm(&self.button, question, press),
                None => {
                    self.button.connect_clicked(press);
                }
            }
        }
        {
            let (pending, undo) = (self.pending.clone(), self.undo.clone());
            swaypplet_model::store::observe(move || {
                let stale = pending
                    .borrow()
                    .as_ref()
                    .is_some_and(|(_, after)| *after != swaypplet_model::store::current());
                if stale {
                    pending.borrow_mut().take();
                    undo.set_visible(false);
                }
            });
        }
        let pending = self.pending.clone();
        self.undo.connect_clicked(move |b| {
            b.set_visible(false);
            let Some((before, after)) = pending.borrow_mut().take() else {
                return;
            };
            swaypplet_model::store::update(|s| *s = undo_sections(s, &before, &after));
            after_undo();
        });
    }
}

/// `now` with every section that differs between `before` and `after` put
/// back as `before` had it: what a reset changed, and nothing else.
pub fn undo_sections(
    now: &swaypplet_model::store::Settings,
    before: &swaypplet_model::store::Settings,
    after: &swaypplet_model::store::Settings,
) -> swaypplet_model::store::Settings {
    let (Ok(mut now_v), Ok(before_v), Ok(after_v)) = (
        serde_json::to_value(now),
        serde_json::to_value(before),
        serde_json::to_value(after),
    ) else {
        return now.clone();
    };
    let (Some(now_o), Some(before_o), Some(after_o)) = (
        now_v.as_object_mut(),
        before_v.as_object(),
        after_v.as_object(),
    ) else {
        return now.clone();
    };
    let keys: std::collections::BTreeSet<&String> = before_o.keys().chain(after_o.keys()).collect();
    for key in keys {
        if before_o.get(key) == after_o.get(key) {
            continue;
        }
        match before_o.get(key) {
            Some(v) => {
                now_o.insert(key.clone(), v.clone());
            }
            None => {
                now_o.remove(key);
            }
        }
    }
    serde_json::from_value(now_v).unwrap_or_else(|_| now.clone())
}

pub fn action_button(label: &str, hint: &str) -> gtk4::Button {
    let button = ui::button(label, Kind::Secondary);
    button.set_tooltip_text(Some(hint));
    button
}

/// Where a tab's values come from: the defaults, or the settings file.
/// Faint at the default, plain once there is an override (`mark_source`).
pub fn set_source(status: &gtk4::Label, overridden: bool, default_text: &str) {
    if overridden {
        status.set_text(&format!(
            "Custom — saved to {}",
            pretty_path(&swaypplet_model::store::path())
        ));
    } else {
        status.set_text(default_text);
    }
    mark_source(status, !overridden);
}

/// `~/.config/…` rather than the whole home path, which is noise in a label.
pub fn pretty_path(path: &Path) -> String {
    let shown = path.display().to_string();
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() => shown.replace(&home, "~"),
        _ => shown,
    }
}

// ── Durations ───────────────────────────────────────────────────────────

/// A dropdown's worth of durations, in seconds, zero meaning never.
///
/// Built from a fixed ladder plus whatever the file currently says, so a
/// value typed by hand (`"lock_after_s": 420`) shows up as "7 min" rather
/// than being snapped to the nearest rung the moment the tab is opened.
pub struct Durations {
    seconds: Vec<u32>,
}

impl Durations {
    pub fn new(ladder: &[u32], current: u32) -> Durations {
        let mut seconds: Vec<u32> = ladder.to_vec();
        if !seconds.contains(&current) {
            seconds.push(current);
        }
        seconds.sort_unstable();
        seconds.dedup();
        Durations { seconds }
    }

    pub fn labels(&self) -> Vec<String> {
        self.seconds.iter().map(|s| duration_label(*s)).collect()
    }

    pub fn index_of(&self, secs: u32) -> Option<usize> {
        self.seconds.iter().position(|s| *s == secs)
    }

    pub fn at(&self, index: usize) -> Option<u32> {
        self.seconds.get(index).copied()
    }
}

pub fn duration_label(secs: u32) -> String {
    match secs {
        0 => "Never".to_string(),
        s if s % 3600 == 0 && s >= 3600 => {
            let h = s / 3600;
            format!("{h} hour{}", if h == 1 { "" } else { "s" })
        }
        s if s % 60 == 0 => format!("{} min", s / 60),
        s => format!("{s} s"),
    }
}

/// The minute rungs a clock dropdown offers, five apart, plus whatever the
/// file says. Same reasoning as `form::Durations`: a minute typed by hand
/// (`"night_from_m": 37`) shows itself rather than being snapped away.
pub fn minute_rungs(current: u8) -> Vec<u8> {
    let mut rungs: Vec<u8> = (0..60).step_by(5).map(|m| m as u8).collect();
    if !rungs.contains(&current) {
        rungs.push(current);
        rungs.sort_unstable();
    }
    rungs
}

/// A label and one clock time: an hour dropdown, a colon, a minute
/// dropdown. Returns the rungs the minute dropdown was built from, which
/// `sync` needs to turn a saved minute back into an index.
pub fn time_row(
    label: &str,
    hint: &str,
    minute: u8,
) -> (gtk4::Box, gtk4::DropDown, gtk4::DropDown, Vec<u8>) {
    let hours: Vec<String> = (0..24).map(|h| format!("{h:02}")).collect();
    let hours: Vec<&str> = hours.iter().map(String::as_str).collect();
    let hour = dropdown(&hours);

    let rungs = minute_rungs(minute);
    let labels: Vec<String> = rungs.iter().map(|m| format!("{m:02}")).collect();
    let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
    let minutes = dropdown(&labels);

    // The two dropdowns share the control column's width, so a clock row
    // lines up on both edges with the dropdowns above and below it.
    hour.set_hexpand(true);
    minutes.set_hexpand(true);
    let clock = swaypplet_ui::ui::hbox(2);
    clock.append(&hour);
    clock.append(&swaypplet_ui::ui::text(
        ":",
        swaypplet_ui::ui::Text::Body,
        swaypplet_ui::ui::Tone::Fg,
    ));
    clock.append(&minutes);

    let row = kind_row(label, &clock);
    row.set_tooltip_text(Some(hint));
    (row, hour, minutes, rungs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_copy_holds_every_section_in_order_or_nothing() {
        let s = swaypplet_model::store::Settings::default();
        let bar = s.section_as_nix("bar").unwrap();
        let pins = s.section_as_nix("pins").unwrap();
        assert_eq!(sections_as_nix(&s, &["bar"]), Some(bar.clone()));
        assert_eq!(sections_as_nix(&s, &["bar", "pins"]), Some(bar + &pins));
        assert_eq!(sections_as_nix(&s, &["bar", "no-such-section"]), None);
    }

    #[test]
    fn the_guard_is_up_only_while_writing_and_nests() {
        let sync = Sync::default();
        assert!(!sync.busy());
        sync.writing(|| {
            assert!(sync.busy());
            sync.writing(|| assert!(sync.busy()));
            // The inner write left the outer one's guard up.
            assert!(sync.busy());
        });
        assert!(!sync.busy());
        assert_eq!(sync.writing(|| 7), 7);
    }

    #[test]
    fn an_undone_reset_puts_back_only_what_it_changed() {
        use swaypplet_model::store::Settings;
        let mut before = Settings::default();
        before
            .set("bar.clock_24h", serde_json::json!(false))
            .unwrap();
        before
            .set("idle.lock_after_s", serde_json::json!(600))
            .unwrap();
        // The Bar pane's reset dropped its section.
        let mut after = before.clone();
        after.bar = None;
        // Something else moved since: it stays as it is now.
        let mut now = after.clone();
        now.set("idle.lock_after_s", serde_json::json!(900))
            .unwrap();
        let undone = undo_sections(&now, &before, &after);
        assert_eq!(undone.get("bar.clock_24h"), Some(serde_json::json!(false)));
        assert_eq!(
            undone.get("idle.lock_after_s"),
            Some(serde_json::json!(900))
        );
    }

    #[test]
    fn durations_keep_the_ladder_and_admit_the_current_value() {
        let d = Durations::new(&[0, 60, 300], 420);
        assert_eq!(d.labels(), vec!["Never", "1 min", "5 min", "7 min"]);
        assert_eq!(d.index_of(420), Some(3));
        assert_eq!(d.at(0), Some(0));
        // A current value already on the ladder is not doubled.
        assert_eq!(Durations::new(&[0, 60], 60).labels().len(), 2);
    }

    #[test]
    fn duration_labels_pick_the_largest_whole_unit() {
        assert_eq!(duration_label(0), "Never");
        assert_eq!(duration_label(30), "30 s");
        assert_eq!(duration_label(90), "90 s");
        assert_eq!(duration_label(60), "1 min");
        assert_eq!(duration_label(900), "15 min");
        assert_eq!(duration_label(3600), "1 hour");
        assert_eq!(duration_label(7200), "2 hours");
    }

    #[test]
    fn minute_rungs_are_five_apart_and_admit_a_typed_value() {
        let five = minute_rungs(0);
        assert_eq!(five.len(), 12);
        assert_eq!(five[0], 0);
        assert_eq!(*five.last().unwrap(), 55);
        assert!(five.windows(2).all(|w| w[1] - w[0] == 5));

        let typed = minute_rungs(37);
        assert_eq!(typed.len(), 13);
        assert_eq!(typed.iter().position(|m| *m == 37), Some(8));
        assert!(typed.windows(2).all(|w| w[0] < w[1]));
        // A value already on the ladder is not doubled.
        assert_eq!(minute_rungs(30).len(), 12);
    }
}
