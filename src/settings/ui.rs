//! The rows the settings tabs are built from, so the four of them read as one
//! pane: the same label gutter, the same hint placement, the same footer.
//!
//! Hints live in tooltips per row; only a group carries a visible one. The
//! pane is dense on purpose (see `data/style.css`, "Settings pane").

use std::path::Path;

use gtk4::prelude::*;

/// A titled run of rows.
pub fn section_box(title: &str, hint: &str) -> gtk4::Box {
    let container = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(4)
        .build();
    container.add_css_class("settings-group");

    let heading = gtk4::Label::builder().label(title).xalign(0.0).build();
    heading.add_css_class("settings-group-title");
    container.append(&heading);

    let sub = gtk4::Label::builder()
        .label(hint)
        .xalign(0.0)
        .wrap(true)
        .build();
    sub.add_css_class("settings-group-hint");
    container.append(&sub);

    container
}

/// The label in a row's gutter.
fn row_label(label: &str) -> gtk4::Label {
    let name = gtk4::Label::builder().label(label).xalign(0.0).build();
    name.add_css_class("settings-row-label");
    name
}

/// An empty row, for the helpers below to fill.
fn row() -> gtk4::Box {
    let row = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(10)
        .build();
    row.add_css_class("settings-row");
    row
}

/// A label in the gutter and one control taking the rest of the row.
pub fn kind_row(label: &str, control: &impl IsA<gtk4::Widget>) -> gtk4::Box {
    let row = row();
    row.append(&row_label(label));

    let control = control.as_ref();
    control.set_hexpand(true);
    row.append(control);
    row
}

/// A label, the hint as its tooltip, and a switch at the far end.
pub fn switch_row(label: &str, hint: &str, active: bool) -> (gtk4::Box, gtk4::Switch) {
    let row = row();
    row.add_css_class("settings-switch-row");
    row.set_tooltip_text(Some(hint));

    let name = row_label(label);
    name.set_hexpand(true);
    row.append(&name);

    let switch = gtk4::Switch::builder()
        .active(active)
        .valign(gtk4::Align::Center)
        .build();
    row.append(&switch);
    (row, switch)
}

/// A label and a dropdown over `choices`.
pub fn dropdown_row(label: &str, hint: &str, choices: &[&str]) -> (gtk4::Box, gtk4::DropDown) {
    let dropdown = gtk4::DropDown::from_strings(choices);
    dropdown.add_css_class("settings-dropdown");
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

    let scale = gtk4::Scale::with_range(gtk4::Orientation::Horizontal, min, max, step);
    scale.set_draw_value(false);
    scale.set_hexpand(true);
    scale.add_css_class("settings-scale");

    let value = gtk4::Label::builder()
        .label(show(min))
        .xalign(1.0)
        .width_chars(6)
        .build();
    value.add_css_class("settings-row-value");
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

/// The strip under a tab: its action buttons, and a line saying where the
/// values currently come from.
pub fn footer(buttons: &[&gtk4::Button]) -> (gtk4::Box, gtk4::Label) {
    let footer = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(8)
        .build();
    footer.add_css_class("settings-footer");

    let row = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(8)
        .build();
    for button in buttons {
        row.append(*button);
    }
    footer.append(&row);

    let status = gtk4::Label::builder().xalign(0.0).wrap(true).build();
    status.add_css_class("settings-status");
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
    let button = gtk4::Button::with_label(label);
    button.add_css_class("settings-action");
    button.set_tooltip_text(Some(hint));
    button
}

/// Where a tab's values come from: the defaults, or the settings file.
/// `faint` at the default, plain once there is an override — the one piece
/// of state the screen behind the pane does not show.
pub fn set_source(status: &gtk4::Label, overridden: bool, default_text: &str) {
    if overridden {
        status.set_text(&format!(
            "Custom — saved to {}",
            pretty_path(&super::store::path())
        ));
        status.remove_css_class("settings-status-system");
    } else {
        status.set_text(default_text);
        status.add_css_class("settings-status-system");
    }
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
