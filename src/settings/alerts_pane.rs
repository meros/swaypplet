//! The Alerts tab: the popup stack, the hours it keeps quiet, and what a
//! screenshot becomes.
//!
//! Two sections, `alerts` and `capture`, on one tab with one footer. The
//! popup rows are read per card (`notifications/popup.rs`), the schedule by
//! `notifications/quiet.rs` on its tick and on every change here, and the
//! capture rows by `screenshot/` at the moment of the shot.

use std::cell::Cell;
use std::rc::Rc;

use gtk4::prelude::*;

use super::store::{self, After, Alerts, Capture, Corner, Linger};
use super::ui::{self, dropdown_row, scale_row, section_box, switch_row};

fn describe(alerts: &Alerts, capture: &Capture) -> String {
    format!(
        "System default: {} popups, {}, {} at a time, quiet hours {}; shots {} to {}",
        alerts
            .linger
            .label()
            .split(' ')
            .next()
            .unwrap_or("")
            .to_lowercase(),
        alerts.corner.label().to_lowercase(),
        alerts.stack,
        if alerts.quiet {
            format!("{:02}–{:02}", alerts.quiet_from_h, alerts.quiet_to_h)
        } else {
            "off".to_string()
        },
        match capture.after {
            After::Both => "saved and copied",
            After::Save => "saved",
            After::Copy => "copied",
        },
        if capture.folder.is_empty() {
            "~/Pictures/Screenshots"
        } else {
            capture.folder.as_str()
        },
    )
}

struct State {
    linger: gtk4::DropDown,
    corner: gtk4::DropDown,
    stack: gtk4::Scale,
    quiet: gtk4::Switch,
    quiet_from: gtk4::DropDown,
    quiet_to: gtk4::DropDown,
    folder: gtk4::Entry,
    after: gtk4::DropDown,
    annotate: gtk4::Switch,
    status: gtk4::Label,
    updating: Cell<bool>,
}

impl State {
    fn edit_alerts(&self, f: impl FnOnce(&mut Alerts)) {
        if self.updating.get() {
            return;
        }
        store::edit(f);
        self.sync();
    }

    fn edit_capture(&self, f: impl FnOnce(&mut Capture)) {
        if self.updating.get() {
            return;
        }
        store::edit(f);
        self.sync();
    }

    fn sync(&self) {
        self.updating.set(true);
        let settings = store::current();
        let alerts = settings.alerts();
        let capture = settings.capture();
        let pos = |i: Option<usize>| i.unwrap_or(0) as u32;
        self.linger
            .set_selected(pos(Linger::ALL.iter().position(|l| *l == alerts.linger)));
        self.corner
            .set_selected(pos(Corner::ALL.iter().position(|c| *c == alerts.corner)));
        self.stack.set_value(f64::from(alerts.stack));
        self.quiet.set_active(alerts.quiet);
        self.quiet_from
            .set_selected(u32::from(alerts.quiet_from_h.min(23)));
        self.quiet_to
            .set_selected(u32::from(alerts.quiet_to_h.min(23)));
        self.quiet_from.set_sensitive(alerts.quiet);
        self.quiet_to.set_sensitive(alerts.quiet);
        if self.folder.text() != capture.folder {
            self.folder.set_text(&capture.folder);
        }
        self.after
            .set_selected(pos(After::ALL.iter().position(|a| *a == capture.after)));
        self.annotate.set_active(capture.annotate);
        ui::set_source(
            &self.status,
            settings.alerts.is_some() || settings.capture.is_some(),
            &describe(&alerts, &capture),
        );
        self.updating.set(false);
    }
}

pub struct AlertsPane {
    root: gtk4::Box,
    state: Rc<State>,
}

impl AlertsPane {
    pub fn new() -> Self {
        let root = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(14)
            .build();

        let settings = store::current();
        let alerts = settings.alerts();
        let capture = settings.capture();

        let popups = section_box(
            "Notifications",
            "The popup stack. A change takes effect on the next card; the ones on screen keep their place.",
        );
        let linger_labels: Vec<&str> = Linger::ALL.iter().map(|l| l.label()).collect();
        let (row_linger, linger) = dropdown_row(
            "Linger",
            "How long a card with no timeout of its own stays. Longer text stays longer; critical cards stay until dismissed.",
            &linger_labels,
        );
        popups.append(&row_linger);
        let corner_labels: Vec<&str> = Corner::ALL.iter().map(|c| c.label()).collect();
        let (row_corner, corner) =
            dropdown_row("Corner", "Where the stack grows from.", &corner_labels);
        popups.append(&row_corner);
        let (row_stack, stack) = scale_row(
            "Stack",
            "Cards shown at full size before older ones collapse behind them.",
            (1.0, 5.0, 1.0),
            |v| format!("{v:.0}"),
        );
        popups.append(&row_stack);

        let quiet_group = section_box(
            "Quiet hours",
            "Do Not Disturb arms itself at the start of the window and disarms at the end. Toggling it by hand inside the window is respected until then.",
        );
        let (row_quiet, quiet) = switch_row(
            "Quiet hours",
            "On, the schedule below drives Do Not Disturb.",
            alerts.quiet,
        );
        quiet_group.append(&row_quiet);
        let hours: Vec<String> = (0..24).map(|h| format!("{h:02}:00")).collect();
        let hours: Vec<&str> = hours.iter().map(String::as_str).collect();
        let (row_from, quiet_from) = dropdown_row("From", "Start of the window.", &hours);
        quiet_group.append(&row_from);
        let (row_to, quiet_to) = dropdown_row(
            "Until",
            "End of the window. Earlier than the start means it runs past midnight.",
            &hours,
        );
        quiet_group.append(&row_to);

        let capture_group = section_box(
            "Capture",
            "What a screenshot becomes. The colour picker and recordings are not affected.",
        );
        let folder = gtk4::Entry::builder()
            .placeholder_text("~/Pictures/Screenshots")
            .text(&capture.folder)
            .build();
        let folder_row = ui::kind_row("Folder", &folder);
        folder_row.set_tooltip_text(Some(
            "Where shots are saved. Empty is ~/Pictures/Screenshots; a ~ is your home.",
        ));
        capture_group.append(&folder_row);
        let after_labels: Vec<&str> = After::ALL.iter().map(|a| a.label()).collect();
        let (row_after, after) = dropdown_row(
            "After a shot",
            "Save to the folder, copy to the clipboard, or both. The card offers Annotate, Open and Delete only for a saved file.",
            &after_labels,
        );
        capture_group.append(&row_after);
        let (row_annotate, annotate) = switch_row(
            "Annotate every shot",
            "Open the editor on each shot as it is kept. The unedited shot is kept first, so closing the editor loses nothing.",
            capture.annotate,
        );
        capture_group.append(&row_annotate);

        let reset = ui::action_button(
            "Reset to system",
            "Put the system's choices back and drop the alerts and capture sections from the settings file.",
        );
        let (footer, status) = ui::footer(&[&reset]);
        let copy = ui::copy_nix_button(
            &status,
            "The alerts and capture sections as theme/settings.nix holds them.",
            || {
                let s = store::current();
                Some(format!(
                    "{}{}",
                    s.section_as_nix("alerts")?,
                    s.section_as_nix("capture")?
                ))
            },
        );
        if let Some(row) = reset.parent().and_downcast::<gtk4::Box>() {
            row.append(&copy);
        }

        let state = Rc::new(State {
            linger: linger.clone(),
            corner: corner.clone(),
            stack: stack.clone(),
            quiet: quiet.clone(),
            quiet_from: quiet_from.clone(),
            quiet_to: quiet_to.clone(),
            folder: folder.clone(),
            after: after.clone(),
            annotate: annotate.clone(),
            status,
            updating: Cell::new(false),
        });

        {
            let state = state.clone();
            linger.connect_selected_notify(move |d| {
                if let Some(l) = Linger::ALL.get(d.selected() as usize).copied() {
                    state.edit_alerts(|a| a.linger = l);
                }
            });
        }
        {
            let state = state.clone();
            corner.connect_selected_notify(move |d| {
                if let Some(c) = Corner::ALL.get(d.selected() as usize).copied() {
                    state.edit_alerts(|a| a.corner = c);
                }
            });
        }
        {
            let state = state.clone();
            stack.connect_value_changed(move |s| {
                let n = s.value().round() as u8;
                state.edit_alerts(|a| a.stack = n);
            });
        }
        {
            let state = state.clone();
            quiet.connect_active_notify(move |s| {
                let on = s.is_active();
                state.edit_alerts(|a| a.quiet = on);
            });
        }
        {
            let state = state.clone();
            quiet_from.connect_selected_notify(move |d| {
                let h = d.selected().min(23) as u8;
                state.edit_alerts(|a| a.quiet_from_h = h);
            });
        }
        {
            let state = state.clone();
            quiet_to.connect_selected_notify(move |d| {
                let h = d.selected().min(23) as u8;
                state.edit_alerts(|a| a.quiet_to_h = h);
            });
        }
        {
            // On focus-out and Enter rather than per keystroke: a folder is
            // typed as a whole, and half a path is not a folder.
            let state = state.clone();
            let commit = Rc::new(move |e: &gtk4::Entry| {
                let text = e.text().trim().to_string();
                state.edit_capture(|c| c.folder = text.clone());
            });
            {
                let commit = commit.clone();
                folder.connect_activate(move |e| commit(e));
            }
            let focus = gtk4::EventControllerFocus::new();
            let entry = folder.clone();
            focus.connect_leave(move |_| commit(&entry));
            folder.add_controller(focus);
        }
        {
            let state = state.clone();
            after.connect_selected_notify(move |d| {
                if let Some(a) = After::ALL.get(d.selected() as usize).copied() {
                    state.edit_capture(|c| c.after = a);
                }
            });
        }
        {
            let state = state.clone();
            annotate.connect_active_notify(move |s| {
                let on = s.is_active();
                state.edit_capture(|c| c.annotate = on);
            });
        }
        {
            let state = state.clone();
            reset.connect_clicked(move |_| {
                if state.updating.get() {
                    return;
                }
                store::reset::<Alerts>();
                store::reset::<Capture>();
                state.sync();
            });
        }

        root.append(&popups);
        root.append(&quiet_group);
        root.append(&capture_group);
        root.append(&footer);
        state.sync();

        AlertsPane { root, state }
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    pub fn refresh(&self) {
        self.state.sync();
    }
}
