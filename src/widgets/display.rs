use std::process::Command;

use gtk4::prelude::*;
use gtk4::{Box, Label};
use serde::Deserialize;

use crate::icons;
use crate::spawn::spawn_work;
use crate::ui;

// ── Data types ────────────────────────────────────────────────────────────────

/// One entry from `swaymsg -t get_outputs`; unknown fields are ignored.
#[derive(Debug, Clone, Deserialize)]
struct OutputInfo {
    name: String,
    active: bool,
    /// Absent for disabled outputs.
    current_mode: Option<Mode>,
}

#[derive(Debug, Clone, Deserialize)]
struct Mode {
    width: u32,
    height: u32,
    /// Refresh rate in millihertz (e.g. 60000 = 60 Hz).
    refresh: u32,
}

// ── Backend helpers ───────────────────────────────────────────────────────────

/// Run `swaymsg -t get_outputs --raw` and parse the JSON response.
fn get_outputs() -> Vec<OutputInfo> {
    let Ok(out) = Command::new("swaymsg")
        .args(["-t", "get_outputs", "--raw"])
        .output()
    else {
        return Vec::new();
    };

    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        log::warn!("failed to parse swaymsg get_outputs JSON: {}", e);
        Vec::new()
    })
}

/// Format refresh rate: millihertz → integer Hz string.
fn format_refresh(mhz: u32) -> String {
    format!("{}Hz", (mhz + 500) / 1000)
}

// ── Toggle action ─────────────────────────────────────────────────────────────

/// Run `swaymsg output <name> enable|disable` (blocking — call from a
/// background thread, e.g. via `spawn_work`).
fn toggle_output_blocking(name: &str, enable: bool) -> bool {
    let cmd = if enable { "enable" } else { "disable" };
    Command::new("swaymsg")
        .args(["output", name, cmd])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

// ── Row builder ───────────────────────────────────────────────────────────────

/// Build a single output row and return it along with the widget that should be
/// refreshed when the toggle completes (`output_list`).
fn make_output_row(output: &OutputInfo, active_count: usize, output_list: &Box) -> Box {
    let mode_text = match &output.current_mode {
        Some(m) if m.width > 0 && m.height > 0 => {
            format!("{}x{} @ {}", m.width, m.height, format_refresh(m.refresh))
        }
        _ => "—".to_string(),
    };
    let r = ui::row(icons::DISPLAY, &output.name, &mode_text);

    // Disable button is suppressed when it would turn off the last active display.
    let can_disable = output.active && active_count > 1;
    let btn_label = if output.active { "Disable" } else { "Enable" };
    let toggle_btn = ui::small_button(btn_label, ui::Kind::Secondary);
    if !can_disable && output.active {
        // Last active display: prevent disabling.
        toggle_btn.set_sensitive(false);
        toggle_btn.set_tooltip_text(Some("Cannot disable the only active display"));
    }

    // ── Toggle handler ────────────────────────────────────────────────────────
    {
        let name = output.name.clone();
        let active = output.active;
        let output_list_c = output_list.clone();

        toggle_btn.connect_clicked(move |btn| {
            // Re-validate against the freshest state before disabling: the
            // row's `can_disable` was computed at last list-populate time,
            // so two rapid Disable clicks on two active displays could both
            // pass the stale check and leave zero active outputs.
            if active && get_outputs().iter().filter(|o| o.active).count() <= 1 {
                btn.set_tooltip_text(Some("Cannot disable the only active display"));
                return;
            }

            btn.set_sensitive(false);

            // Refresh the list after the command completes.
            let name_bg = name.clone();
            let output_list_refresh = output_list_c.clone();
            spawn_work(
                move || toggle_output_blocking(&name_bg, !active),
                move |_ok| {
                    // Re-populate the list to reflect the new state.
                    populate_output_list(&output_list_refresh);
                },
            );
        });
    }

    r.end.append(&toggle_btn);
    r.root
}

// ── List population ───────────────────────────────────────────────────────────

/// Clear `list` and rebuild it from the current `swaymsg` output (synchronous).
fn populate_output_list(list: &Box) {
    populate_output_list_with_data(list, &get_outputs());
}

/// Clear `list` and rebuild it from pre-fetched output data.
fn populate_output_list_with_data(list: &Box, outputs: &[OutputInfo]) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }

    let active_count = outputs.iter().filter(|o| o.active).count();

    for output in outputs {
        list.append(&make_output_row(output, active_count, list));
    }
}

// ── DisplaySection ────────────────────────────────────────────────────────────

pub struct DisplaySection {
    section: ui::Section,
    output_list: Box,
}

impl DisplaySection {
    pub fn new() -> Self {
        let section = ui::section(icons::DISPLAY, "Displays", "");
        ui::glyph(&section.icon, ui::Text::Title, ui::Tone::Fg);
        let output_list = ui::vbox(1);

        // ── Night light warmth, above the outputs ─────────────────────────────
        let night = ui::group(1);
        night.add_css_class("display-night");
        night.append(&ui::heading("Night Light Warmth"));
        let night_row = ui::slider_row("󰖔", 2000.0, 6500.0, 100.0);
        ui::glyph(&night_row.icon, ui::Text::Title, ui::Tone::Fg);
        night_row.scale.adjustment().set_page_increment(500.0);
        night_row.scale.set_value(3500.0);
        night_row.value.set_label("3500K");
        {
            let val_lbl = night_row.value.clone();
            night_row.scale.connect_value_changed(move |s| {
                let temp = s.value().round() as u32;
                val_lbl.set_label(&format!("{temp}K"));
            });
        }
        night.append(&night_row.root);

        let detail_box = ui::vbox(3);
        detail_box.append(&night);
        detail_box.append(&output_list);
        section.body.append(&detail_box);

        let display = Self {
            section,
            output_list,
        };

        display.refresh();
        display
    }

    /// Re-query swaymsg and rebuild the output list and summary label.
    ///
    /// The blocking `swaymsg` call runs on a background thread; the UI is
    /// updated on the GTK main thread once the result arrives.
    pub fn refresh(&self) {
        let output_list = self.output_list.clone();
        let summary_text: Label = self.section.summary.clone();

        spawn_work(get_outputs, move |outputs| {
            populate_output_list_with_data(&output_list, &outputs);

            let active_count = outputs.iter().filter(|o| o.active).count();
            let summary = match active_count {
                0 => "No displays".to_string(),
                1 => outputs
                    .iter()
                    .find(|o| o.active)
                    .map(|o| o.name.clone())
                    .unwrap_or_default(),
                n => format!("{n} displays"),
            };
            summary_text.set_label(&summary);
        });
    }

    /// Switch into page mode: the body alone, open at once.
    pub fn expand_for_page(&self) {
        self.section.show_as_page();
    }

    /// Return a reference to the root widget for embedding in the panel.
    pub fn widget(&self) -> &Box {
        &self.section.root
    }
}
