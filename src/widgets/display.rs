use std::process::Command;

use gtk4::prelude::*;
use gtk4::{Box, Button, Label, Orientation, Revealer, RevealerTransitionType};
use serde::Deserialize;

use crate::icons;
use crate::spawn::spawn_work;

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
    let row = Box::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .hexpand(true)
        .build();
    row.add_css_class("device-row");

    let icon_lbl = Label::builder().label(icons::DISPLAY).build();
    icon_lbl.add_css_class("device-icon");

    let info_box = Box::builder()
        .orientation(Orientation::Vertical)
        .hexpand(true)
        .build();

    let name_lbl = Label::builder().label(&output.name).xalign(0.0).build();
    name_lbl.add_css_class("device-name");

    let mode_text = match &output.current_mode {
        Some(m) if m.width > 0 && m.height > 0 => {
            format!("{}x{} @ {}", m.width, m.height, format_refresh(m.refresh))
        }
        _ => "—".to_string(),
    };
    let mode_lbl = Label::builder().label(&mode_text).xalign(0.0).build();
    mode_lbl.add_css_class("device-status");

    info_box.append(&name_lbl);
    info_box.append(&mode_lbl);

    // Disable button is suppressed when it would turn off the last active display.
    let can_disable = output.active && active_count > 1;
    let btn_label = if output.active { "Disable" } else { "Enable" };
    let toggle_btn = Button::with_label(btn_label);
    toggle_btn.add_css_class("device-action");
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

    row.append(&icon_lbl);
    row.append(&info_box);
    row.append(&toggle_btn);
    row
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
    root: Box,
    summary_btn: Button,
    summary_text: Label,
    summary_arrow: Label,
    detail_revealer: Revealer,
    output_list: Box,
}

impl DisplaySection {
    pub fn new() -> Self {
        // ── Root section box ──────────────────────────────────────────────────
        let root = Box::builder()
            .orientation(Orientation::Vertical)
            .spacing(4)
            .build();
        root.add_css_class("section");

        // ── Summary row (always visible) ──────────────────────────────────────
        let summary_content = Box::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .hexpand(true)
            .build();

        let summary_icon = Label::builder().label(icons::DISPLAY).build();
        summary_icon.add_css_class("section-summary-icon");

        let summary_text = Label::builder()
            .label("Displays")
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .build();
        summary_text.add_css_class("section-summary-label");

        let summary_arrow = Label::builder().label("▸").build();
        summary_arrow.add_css_class("section-expand-arrow");

        summary_content.append(&summary_icon);
        summary_content.append(&summary_text);
        summary_content.append(&summary_arrow);

        let summary_btn = Button::builder().child(&summary_content).build();
        summary_btn.add_css_class("section-summary");
        root.append(&summary_btn);

        // ── Detail revealer ───────────────────────────────────────────────────
        let detail_revealer = Revealer::builder()
            .transition_type(RevealerTransitionType::SlideDown)
            .transition_duration(200)
            .reveal_child(false)
            .build();

        let output_list = Box::builder()
            .orientation(Orientation::Vertical)
            .spacing(2)
            .build();
        output_list.add_css_class("device-list");

        // ── Night light warmth controls in Display Section ───────────────────
        let night_box = Box::builder()
            .orientation(Orientation::Vertical)
            .spacing(4)
            .build();
        night_box.add_css_class("display-night-box");

        let night_hdr = Box::builder()
            .orientation(Orientation::Horizontal)
            .spacing(6)
            .build();
        let night_lbl = Label::builder()
            .label("Night Light Warmth")
            .xalign(0.0)
            .hexpand(true)
            .build();
        night_lbl.add_css_class("display-night-label");
        let night_val_lbl = Label::builder().label("3500K").xalign(1.0).build();
        night_val_lbl.add_css_class("display-night-val");
        night_hdr.append(&night_lbl);
        night_hdr.append(&night_val_lbl);

        let night_scale = gtk4::Scale::builder()
            .orientation(Orientation::Horizontal)
            .adjustment(&gtk4::Adjustment::new(
                3500.0, 2000.0, 6500.0, 100.0, 500.0, 0.0,
            ))
            .draw_value(false)
            .hexpand(true)
            .build();
        night_scale.add_css_class("night-scale");

        {
            let val_lbl = night_val_lbl.clone();
            night_scale.connect_value_changed(move |s| {
                let temp = s.value().round() as u32;
                val_lbl.set_label(&format!("{temp}K"));
            });
        }

        night_box.append(&night_hdr);
        night_box.append(&night_scale);

        let detail_box = Box::builder()
            .orientation(Orientation::Vertical)
            .spacing(8)
            .build();
        detail_box.append(&night_box);
        detail_box.append(&output_list);

        detail_revealer.set_child(Some(&detail_box));
        root.append(&detail_revealer);

        // ── Wire up summary row toggle ────────────────────────────────────────
        {
            let detail_revealer_c = detail_revealer.clone();
            let summary_arrow_c = summary_arrow.clone();
            summary_btn.connect_clicked(move |_| {
                let revealed = !detail_revealer_c.reveals_child();
                detail_revealer_c.set_reveal_child(revealed);
                summary_arrow_c.set_label(if revealed { "▾" } else { "▸" });
            });
        }

        let section = Self {
            root,
            summary_btn,
            summary_text,
            summary_arrow,
            detail_revealer,
            output_list,
        };

        section.refresh();
        section
    }

    /// Re-query swaymsg and rebuild the output list and summary label.
    ///
    /// The blocking `swaymsg` call runs on a background thread; the UI is
    /// updated on the GTK main thread once the result arrives.
    pub fn refresh(&self) {
        let output_list = self.output_list.clone();
        let summary_text = self.summary_text.clone();

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

    /// Switch into page mode: reveal detail immediately, hide the summary
    /// toggle row.
    pub fn expand_for_page(&self) {
        self.summary_btn.set_visible(false);
        self.detail_revealer.set_transition_duration(0);
        self.detail_revealer.set_reveal_child(true);
        self.detail_revealer.set_transition_duration(200);
        self.summary_arrow.set_label("▾");
    }

    /// Return a reference to the root widget for embedding in the panel.
    pub fn widget(&self) -> &Box {
        &self.root
    }
}
