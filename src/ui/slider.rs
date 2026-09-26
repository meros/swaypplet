//! Slider, switch and check: controls that hold a value.
//!
//! `ui::slider_row(..)`, `ui::slider::adopt(&scale, Density)`,
//! `ui::switch()`, `ui::switch_row(..)`, `ui::check(label)`;
//! `ui::set_over_range` at runtime.

use gtk4::prelude::*;
use gtk4::{Align, Orientation};

use super::class::toggle;
use super::{Face, Kind, Row, Size, button_with, hbox, row};

/// How much room a slider takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Density {
    /// A slider that owns its row: the full knob.
    Normal,
    /// One of a dense column of them: a smaller knob, less air.
    Dense,
}

/// Style a scale the caller built as the component slider.
pub fn adopt(s: &gtk4::Scale, density: Density) {
    s.add_css_class("ui-slider");
    if density == Density::Dense {
        s.add_css_class("dense");
    }
}

/// Past the normal range (volume over 100 %): the fill turns warning.
pub fn set_over_range(s: &gtk4::Scale, over: bool) {
    toggle(s, "over", over);
}

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
    adopt(&scale, Density::Normal);
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

impl SliderRow {
    /// Turn the row's icon into a flat button (mute, say). The glyph stays
    /// the row's icon label, so callers keep setting it there.
    pub fn icon_button(&self, tooltip: &str) -> gtk4::Button {
        self.root.remove(&self.icon);
        let b = button_with(
            Face::Icon {
                child: self.icon.upcast_ref(),
                tooltip,
            },
            Kind::Flat,
            Size::Normal,
        );
        self.root.prepend(&b);
        b
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

/// A check box with its label; accent when checked.
///
/// A component of the system with no surface using it since the Glass
/// tab lost its fill checks; kept, with its stylesheet, for the next one.
#[allow(dead_code)]
pub fn check(label: &str) -> gtk4::CheckButton {
    let c = gtk4::CheckButton::with_label(label);
    c.add_css_class("ui-check");
    c
}
