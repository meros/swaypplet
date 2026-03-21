use std::process::Command;
use std::sync::mpsc;

use gtk4::prelude::*;
use gtk4::{Box, Button, Label, Orientation, Revealer, RevealerTransitionType};

// ── Nerd Font icons ───────────────────────────────────────────────────────────
const ICON_DISPLAY: &str = "󰍹";

// ── Data types ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
struct OutputInfo {
    name: String,
    active: bool,
    width: u32,
    height: u32,
    /// Refresh rate in millihertz (e.g. 60000 = 60 Hz).
    refresh_mhz: u32,
    #[allow(dead_code)]
    make: String,
    #[allow(dead_code)]
    model: String,
}

// ── Backend helpers ───────────────────────────────────────────────────────────

/// Extract the string value from a JSON line like `"key": "value",`.
fn extract_string_value(line: &str) -> String {
    // Split on `"` — the value is the 4th token (index 3).
    line.split('"').nth(3).unwrap_or("").to_string()
}

/// Extract the numeric value from a JSON line like `"key": 1234,`.
fn extract_number(line: &str) -> u32 {
    line.split(':')
        .nth(1)
        .and_then(|s| s.trim().trim_end_matches(',').parse().ok())
        .unwrap_or(0)
}

/// Run `swaymsg -t get_outputs --raw` and parse the JSON response.
fn get_outputs() -> Vec<OutputInfo> {
    let Ok(out) = Command::new("swaymsg")
        .args(["-t", "get_outputs", "--raw"])
        .output()
    else {
        return Vec::new();
    };

    let json = String::from_utf8_lossy(&out.stdout);
    parse_outputs(&json)
}

fn parse_outputs(json: &str) -> Vec<OutputInfo> {
    let mut outputs: Vec<OutputInfo> = Vec::new();
    let mut current: Option<OutputInfo> = None;
    let mut in_current_mode = false;

    for line in json.lines() {
        let trimmed = line.trim();

        // A new output object begins whenever we see a top-level "name" key.
        // swaymsg outputs the name field first in each object, so this acts as
        // a reliable object boundary.
        if trimmed.starts_with("\"name\":") {
            // Push the previous output before starting a new one.
            if let Some(o) = current.take() {
                outputs.push(o);
            }
            let name = extract_string_value(trimmed);
            current = Some(OutputInfo {
                name,
                ..Default::default()
            });
            in_current_mode = false;
        }

        let Some(ref mut o) = current else { continue };

        if trimmed.starts_with("\"active\":") {
            o.active = trimmed.contains("true");
        } else if trimmed.contains("\"current_mode\"") {
            in_current_mode = true;
        } else if trimmed.starts_with("\"make\":") {
            o.make = extract_string_value(trimmed);
        } else if trimmed.starts_with("\"model\":") {
            o.model = extract_string_value(trimmed);
        }

        if in_current_mode {
            if trimmed.starts_with("\"width\":") {
                o.width = extract_number(trimmed);
            } else if trimmed.starts_with("\"height\":") {
                o.height = extract_number(trimmed);
            } else if trimmed.starts_with("\"refresh\":") {
                o.refresh_mhz = extract_number(trimmed);
                // current_mode block is complete after refresh.
                in_current_mode = false;
            }
        }
    }

    // Push the final output.
    if let Some(o) = current {
        outputs.push(o);
    }

    outputs
}

/// Format refresh rate: millihertz → integer Hz string.
fn format_refresh(mhz: u32) -> String {
    format!("{}Hz", (mhz + 500) / 1000)
}

// ── Toggle action ─────────────────────────────────────────────────────────────

/// Send `swaymsg output <name> enable|disable` on a background thread.
/// Returns a channel receiver that yields `true` on success, `false` on failure.
fn toggle_output_async(name: String, enable: bool) -> mpsc::Receiver<bool> {
    let (tx, rx) = mpsc::channel::<bool>();
    std::thread::spawn(move || {
        let cmd = if enable { "enable" } else { "disable" };
        let ok = Command::new("swaymsg")
            .args(["output", &name, cmd])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        let _ = tx.send(ok);
    });
    rx
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

    let icon_lbl = Label::builder().label(ICON_DISPLAY).build();
    icon_lbl.add_css_class("device-icon");

    let info_box = Box::builder()
        .orientation(Orientation::Vertical)
        .hexpand(true)
        .build();

    let name_lbl = Label::builder()
        .label(&output.name)
        .xalign(0.0)
        .build();
    name_lbl.add_css_class("device-name");

    let mode_text = if output.width > 0 && output.height > 0 {
        format!(
            "{}x{} @ {}",
            output.width,
            output.height,
            format_refresh(output.refresh_mhz)
        )
    } else {
        "—".to_string()
    };
    let mode_lbl = Label::builder()
        .label(&mode_text)
        .xalign(0.0)
        .build();
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
        let toggle_btn_c = toggle_btn.clone();

        toggle_btn.connect_clicked(move |btn| {
            btn.set_sensitive(false);

            let rx = toggle_output_async(name.clone(), !active);

            // Refresh the list after the command completes.
            let output_list_refresh = output_list_c.clone();
            let btn_refresh = toggle_btn_c.clone();
            glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
                match rx.try_recv() {
                    Ok(_) => {
                        // Re-populate the list to reflect the new state.
                        populate_output_list(&output_list_refresh);
                        glib::ControlFlow::Break
                    }
                    Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        // Command thread dropped without sending — restore button.
                        btn_refresh.set_sensitive(true);
                        glib::ControlFlow::Break
                    }
                }
            });
        });
    }

    row.append(&icon_lbl);
    row.append(&info_box);
    row.append(&toggle_btn);
    row
}

// ── List population ───────────────────────────────────────────────────────────

/// Clear `list` and rebuild it from the current `swaymsg` output.
fn populate_output_list(list: &Box) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }

    let outputs = get_outputs();
    let active_count = outputs.iter().filter(|o| o.active).count();

    for output in &outputs {
        list.append(&make_output_row(output, active_count, list));
    }
}

// ── DisplaySection ────────────────────────────────────────────────────────────

#[allow(dead_code)]
pub struct DisplaySection {
    root: Box,
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

        let summary_icon = Label::builder().label(ICON_DISPLAY).build();
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

        detail_revealer.set_child(Some(&output_list));
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
            summary_text,
            summary_arrow,
            detail_revealer,
            output_list,
        };

        section.refresh();
        section
    }

    /// Re-query swaymsg and rebuild the output list and summary label.
    pub fn refresh(&self) {
        populate_output_list(&self.output_list);

        let outputs = get_outputs();
        let active: Vec<&OutputInfo> = outputs.iter().filter(|o| o.active).collect();

        let summary = match active.len() {
            0 => "No displays".to_string(),
            1 => active[0].name.clone(),
            n => format!("{n} displays"),
        };
        self.summary_text.set_label(&summary);
    }

    /// Return a reference to the root widget for embedding in the panel.
    pub fn widget(&self) -> &Box {
        &self.root
    }
}
