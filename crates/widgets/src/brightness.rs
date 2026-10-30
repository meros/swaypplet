use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;

use swaypplet_system::brightness;
use swaypplet_ui::ui;
use swaypplet_ui::ui::icons;

// ── BrightnessSection ─────────────────────────────────────────────────────────

pub struct BrightnessSection {
    section: ui::Section,
    scale: gtk4::Scale,
    pct_label: gtk4::Label,
    /// Guard flag: true while `refresh()` is programmatically updating the scale
    /// so the value-changed handler does not call `brightnessctl set` in response.
    updating: Rc<RefCell<bool>>,
}

impl BrightnessSection {
    pub fn new() -> Self {
        let section = ui::section(icons::BRIGHTNESS, "Brightness", "0%");
        ui::glyph::adopt(&section.icon, ui::Text::Title, ui::Tone::Fg);

        // ── Brightness row (inside the section body) ──────────────────────────
        let row = ui::slider_row(icons::BRIGHTNESS, 1.0, 100.0, 1.0);
        ui::glyph::adopt(&row.icon, ui::Text::Title, ui::Tone::Fg);
        let scale = row.scale.clone();
        // Same rail furniture as the volume scale (`audio.rs`): quarter
        // ticks plus one labelled reference. Both scales are hoisted into the
        // start menu's telemetry ribbon side by side, and one rail with ticks
        // beside one without read as two different kinds of control.
        // The label sits at the midpoint rather than the end, where a wide
        // glyph would hang off the rail in a narrow pill.
        for at in [1.0, 25.0, 75.0, 100.0] {
            scale.add_mark(at, gtk4::PositionType::Bottom, None);
        }
        scale.add_mark(50.0, gtk4::PositionType::Bottom, Some("50%"));

        let pct_label = row.value.clone();
        pct_label.set_text("0%");
        pct_label.set_width_chars(5);

        section.body.append(&row.root);

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
                brightness::set(value);
                lbl.set_text(&format!("{}%", value));
            });
        }

        let section = BrightnessSection {
            section,
            scale,
            pct_label,
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
        let summary_text = self.section.summary.clone();

        // A failed read leaves the slider where it was.
        swaypplet_base::spawn::spawn_work(brightness::read, move |pct_opt| {
            if let Some(pct) = pct_opt {
                *updating.borrow_mut() = true;
                scale.set_value(pct as f64);
                pct_label.set_text(&format!("{}%", pct));
                summary_text.set_text(&format!("{}%", pct));
                *updating.borrow_mut() = false;
            }
        });
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.section.root
    }

    /// Clone of the brightness `gtk4::Scale` (range 1–100) so it can be hoisted
    /// to the start-menu top level. The clone shares the same underlying
    /// `GtkAdjustment`, so reads/writes stay in sync with this section.
    pub fn brightness_scale(&self) -> gtk4::Scale {
        self.scale.clone()
    }
}
