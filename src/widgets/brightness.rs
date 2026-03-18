use std::cell::RefCell;
use std::process::Command;
use std::rc::Rc;

use gtk4::prelude::*;

// ── Nerd Font icon ────────────────────────────────────────────────────────────
const ICON_BRIGHTNESS: &str = "󰃟";

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

        // ── Section title ────────────────────────────────────────────────────
        let title = gtk4::Label::new(Some("DISPLAY"));
        title.add_css_class("section-title");
        title.set_xalign(0.0);
        root.append(&title);

        // ── Brightness row ───────────────────────────────────────────────────
        let row = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(6)
            .build();
        row.add_css_class("volume-row");

        let icon = gtk4::Label::new(Some(ICON_BRIGHTNESS));
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
        root.append(&row);

        let updating = Rc::new(RefCell::new(false));

        // ── Scale signal ─────────────────────────────────────────────────────
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

        let section = BrightnessSection { root, scale, pct_label, updating };
        section.refresh();
        section
    }

    /// Re-reads brightness from `brightnessctl` and updates the UI.
    pub fn refresh(&self) {
        if let Some(pct) = read_brightness() {
            *self.updating.borrow_mut() = true;
            self.scale.set_value(pct as f64);
            self.pct_label.set_text(&format!("{}%", pct));
            *self.updating.borrow_mut() = false;
        }
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }
}
