//! The rows the settings tabs are built from, so the five of them read as
//! one pane: the same label gutter, the same hint placement, the same
//! footer. Built only from the components (`crate::ui`), which carry the
//! colour, type and shape; `data/css/15-settings.css` only places things.
//!
//! Hints live in tooltips per row; only a group carries a visible one. The
//! pane is dense on purpose (see `data/css/15-settings.css`).

use std::path::Path;

use gtk4::prelude::*;

use crate::ui::{self, Kind, Text, Tone};

/// Where a row's control sits: the alternatives zoo's open decision
/// (docs/alternatives-zoo.html, "settings-control-placement"). One switch,
/// read by [`kind_row`]; a slider always fills, and an entry always fills,
/// because both need the width.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Placement {
    /// The control at the row's far end, at its natural width but never
    /// narrower than `.settings-control` (so a column of dropdowns lines up
    /// on both edges): the row reads label, then value, as a sentence does.
    /// macOS System Settings, GNOME 47, iOS.
    Trailing,
    /// The control takes the rest of the row from a fixed label gutter, so
    /// every control in the pane starts at one x: a form's column.
    Fill,
}

pub const CONTROL_PLACEMENT: Placement = Placement::Trailing;

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

/// A label in the gutter and one control taking the rest of the row.
pub fn kind_row(label: &str, control: &impl IsA<gtk4::Widget>) -> gtk4::Box {
    let row = row();
    let name = row_label(label);
    row.append(&name);

    let control = control.as_ref();
    // A text field is the one control that is better wide: what you type
    // into it is read there.
    let fills = CONTROL_PLACEMENT == Placement::Fill || control.is::<gtk4::Entry>();
    if fills {
        control.set_hexpand(true);
    } else {
        name.set_hexpand(true);
        control.set_halign(gtk4::Align::End);
        control.add_css_class("settings-control");
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
    row.append(&row_label(label));

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
            pretty_path(&super::store::path())
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
