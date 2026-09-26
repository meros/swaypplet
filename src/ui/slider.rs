//! Slider, switch and check: controls that hold a value.

use gtk4::prelude::*;
use gtk4::{Align, Orientation};

use super::*;

pub struct SliderRow {
    pub root: gtk4::Box,
    pub icon: gtk4::Label,
    pub scale: gtk4::Scale,
    pub value: gtk4::Label,
}

/// A slider in a full-width row of its own (principle: sliders own a row).
pub fn slider_row(icon: &str, min: f64, max: f64, step: f64) -> SliderRow {
    let root = hbox(4);
    root.add_css_class("ui-slider-row");
    let icon_l = gtk4::Label::new(Some(icon));
    icon_l.add_css_class("ui-slider-icon");
    let scale = gtk4::Scale::with_range(Orientation::Horizontal, min, max, step);
    scale.add_css_class("ui-slider");
    scale.set_hexpand(true);
    scale.set_draw_value(false);
    let value = gtk4::Label::new(None);
    value.add_css_class("ui-slider-value");
    value.set_xalign(1.0);
    root.append(&icon_l);
    root.append(&scale);
    root.append(&value);
    SliderRow {
        root,
        icon: icon_l,
        scale,
        value,
    }
}

pub fn switch() -> gtk4::Switch {
    let s = gtk4::Switch::new();
    s.add_css_class("ui-switch");
    s.set_valign(Align::Center);
    s
}

/// A row with a switch at its end.
pub fn switch_row(title: &str, subtitle: &str) -> (Row, gtk4::Switch) {
    let r = row("", title, subtitle);
    let s = switch();
    r.end.append(&s);
    (r, s)
}

/// Make a scale the component slider (the Helm ribbon's own).
pub fn slider(s: &gtk4::Scale) {
    s.add_css_class("ui-slider");
}

/// The slider for a dense column of them: a smaller knob, less air.
pub fn dense_slider(s: &gtk4::Scale) {
    s.add_css_class("ui-slider");
    s.add_css_class("dense");
}

/// A check box with its label; accent when checked.
pub fn check(label: &str) -> gtk4::CheckButton {
    let c = gtk4::CheckButton::with_label(label);
    c.add_css_class("ui-check");
    c
}
