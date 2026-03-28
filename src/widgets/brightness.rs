use std::cell::RefCell;
use std::process::Command;
use std::rc::Rc;

use gtk4::prelude::*;

use crate::icons;

// ── brightnessctl helpers ─────────────────────────────────────────────────────

/// Returns current brightness as a percentage (1–100), or `None` on failure.
fn read_brightness() -> Option<u32> {
    // `brightnessctl -m` emits: device,class,current,max,percentage%
    let out = Command::new("brightnessctl")
        .arg("-m")
        .output()
        .ok()
        .filter(|o| o.status.success())?;

    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().next()?;
    // Field 3 (0-indexed) is "NN%"
    let pct_field = line.split(',').nth(3)?;
    let pct_str = pct_field.trim().trim_end_matches('%');
    pct_str.parse::<u32>().ok()
}

fn set_brightness(value: u32) {
    let arg = format!("{}%", value);
    let _ = Command::new("brightnessctl").args(["set", &arg]).spawn();
}

// ── BrightnessSection ─────────────────────────────────────────────────────────

pub struct BrightnessSection {
    root: gtk4::Box,
    scale: gtk4::Scale,
    pct_label: gtk4::Label,
    summary_text: gtk4::Label,
    summary_arrow: gtk4::Label,
    detail_revealer: gtk4::Revealer,
    /// Guard flag: true while `refresh()` is programmatically updating the scale
    /// so the value-changed handler does not call `brightnessctl set` in response.
    updating: Rc<RefCell<bool>>,
}

impl BrightnessSection {
    pub fn new() -> Self {
        let root = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(6)
            .build();
        root.add_css_class("section");

        // ── Summary row (always visible) ──────────────────────────────────────
        let summary_content = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(6)
            .build();

        let summary_icon = gtk4::Label::builder()
            .label(icons::BRIGHTNESS)
            .halign(gtk4::Align::Center)
            .valign(gtk4::Align::Center)
            .build();
        summary_icon.add_css_class("section-summary-icon");

        let summary_text = gtk4::Label::builder()
            .label("0%")
            .halign(gtk4::Align::Start)
            .hexpand(true)
            .xalign(0.0)
            .build();
        summary_text.add_css_class("section-summary-label");

        let summary_arrow = gtk4::Label::builder()
            .label("▸")
            .halign(gtk4::Align::Center)
            .valign(gtk4::Align::Center)
            .build();
        summary_arrow.add_css_class("section-expand-arrow");

        summary_content.append(&summary_icon);
        summary_content.append(&summary_text);
        summary_content.append(&summary_arrow);

        let summary_btn = gtk4::Button::builder()
            .child(&summary_content)
            .build();
        summary_btn.add_css_class("section-summary");

        // ── Detail revealer ───────────────────────────────────────────────────
        let detail_revealer = gtk4::Revealer::builder()
            .transition_type(gtk4::RevealerTransitionType::SlideDown)
            .transition_duration(200)
            .reveal_child(false)
            .build();

        // ── Brightness row (inside revealer) ──────────────────────────────────
        let row = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(6)
            .build();
        row.add_css_class("volume-row");

        let icon = gtk4::Label::builder()
            .label(icons::BRIGHTNESS)
            .halign(gtk4::Align::Center)
            .valign(gtk4::Align::Center)
            .build();
        icon.add_css_class("volume-icon-btn");

        let scale = gtk4::Scale::with_range(gtk4::Orientation::Horizontal, 1.0, 100.0, 1.0);
        scale.set_hexpand(true);
        scale.set_draw_value(false);

        let pct_label = gtk4::Label::new(Some("0%"));
        pct_label.add_css_class("volume-pct");
        pct_label.set_width_chars(5);
        pct_label.set_xalign(1.0);

        row.append(&icon);
        row.append(&scale);
        row.append(&pct_label);

        detail_revealer.set_child(Some(&row));

        // ── Toggle gesture ────────────────────────────────────────────────────
        {
            let revealer = detail_revealer.clone();
            let arrow = summary_arrow.clone();
            summary_btn.connect_clicked(move |_| {
                let expanded = !revealer.reveals_child();
                revealer.set_reveal_child(expanded);
                arrow.set_text(if expanded { "▾" } else { "▸" });
            });
        }

        root.append(&summary_btn);
        root.append(&detail_revealer);

        let updating = Rc::new(RefCell::new(false));

        // ── Scale signal ──────────────────────────────────────────────────────
        {
            let upd = updating.clone();
            let lbl = pct_label.clone();
            scale.connect_value_changed(move |s| {
                if *upd.borrow() {
                    return;
                }
                let value = s.value().round() as u32;
                set_brightness(value);
                lbl.set_text(&format!("{}%", value));
            });
        }

        let section = BrightnessSection {
            root,
            scale,
            pct_label,
            summary_text,
            summary_arrow,
            detail_revealer,
            updating,
        };
        section.refresh();
        section
    }

    /// Re-reads brightness from `brightnessctl` on a background thread and
    /// updates the UI when the result arrives.
    pub fn refresh(&self) {
        let updating = self.updating.clone();
        let scale = self.scale.clone();
        let pct_label = self.pct_label.clone();
        let summary_text = self.summary_text.clone();

        crate::spawn::spawn_work(
            || read_brightness(),
            move |pct_opt| {
                if let Some(pct) = pct_opt {
                    *updating.borrow_mut() = true;
                    scale.set_value(pct as f64);
                    pct_label.set_text(&format!("{}%", pct));
                    summary_text.set_text(&format!("{}%", pct));
                    *updating.borrow_mut() = false;
                }
            },
        );
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }
}
