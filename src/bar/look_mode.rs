//! Look mode segment: the Look tab's Mode setting (Auto, Dark, Light) as one
//! glyph on the instrument track, beside the clock.
//!
//! A click moves the setting to the next of `ThemeMode::ALL`, the order the
//! Look tab lists them in. It writes the setting and nothing else: the
//! stylesheet and the glass follow on `theme::watch`, as they do when the
//! Look tab changes it. The glyph shows the setting, not the mode it
//! resolves to, so Auto stays recognisable by day and by night; the tooltip
//! says which mode Auto shows now, read when it opens.

use gtk4::prelude::*;

use crate::settings::store::{self, Look, ThemeMode};
use crate::tokens::Mode;

// Nerd Font: md-theme_light_dark / md-weather_night / md-white_balance_sunny.
const ICON_AUTO: &str = "\u{f050e}";
const ICON_DARK: &str = "\u{f0594}";
const ICON_LIGHT: &str = "\u{f0599}";

fn icon(mode: ThemeMode) -> &'static str {
    match mode {
        ThemeMode::Auto => ICON_AUTO,
        ThemeMode::Dark => ICON_DARK,
        ThemeMode::Light => ICON_LIGHT,
    }
}

fn name(mode: ThemeMode) -> &'static str {
    match mode {
        ThemeMode::Auto => "Auto",
        ThemeMode::Dark => "Dark",
        ThemeMode::Light => "Light",
    }
}

/// The mode after `mode` in the Look tab's order, wrapping.
fn next(mode: ThemeMode) -> ThemeMode {
    let all = ThemeMode::ALL;
    let i = all.iter().position(|m| *m == mode).unwrap_or(0);
    all[(i + 1) % all.len()]
}

fn tooltip(setting: ThemeMode, shown: Mode) -> String {
    let now = match (setting, shown) {
        (ThemeMode::Auto, Mode::Dark) => "Auto, dark now".to_string(),
        (ThemeMode::Auto, Mode::Light) => "Auto, light now".to_string(),
        (mode, _) => name(mode).to_string(),
    };
    format!("Mode: {now}. Click for {}.", name(next(setting)))
}

fn setting() -> ThemeMode {
    store::with(|s| s.look().mode)
}

pub fn build() -> gtk4::Button {
    let label = gtk4::Label::new(Some(icon(setting())));
    let btn = gtk4::Button::builder()
        .child(&label)
        .css_classes(["bar-seg"])
        .has_tooltip(true)
        .build();
    crate::ui::segment::adopt(&btn, false);

    btn.connect_clicked(|_| {
        let mode = next(setting());
        store::edit::<Look>(|l| l.mode = mode);
    });
    btn.connect_query_tooltip(|_, _, _, _, tip| {
        tip.set_text(Some(&tooltip(setting(), crate::theme::shown().mode)));
        true
    });

    // The Look tab, the CLI and another bar all change the setting too.
    let weak = label.downgrade();
    store::observe(move || {
        if let Some(label) = weak.upgrade() {
            label.set_label(icon(setting()));
        }
    });

    btn
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_click_visits_every_mode_and_comes_back() {
        let mut mode = ThemeMode::Auto;
        let mut seen = Vec::new();
        for _ in ThemeMode::ALL {
            seen.push(mode);
            mode = next(mode);
        }
        assert_eq!(mode, ThemeMode::Auto);
        assert_eq!(seen, ThemeMode::ALL);
    }

    #[test]
    fn the_tooltip_says_what_auto_shows() {
        assert_eq!(
            tooltip(ThemeMode::Auto, Mode::Light),
            "Mode: Auto, light now. Click for Dark."
        );
        assert_eq!(
            tooltip(ThemeMode::Light, Mode::Light),
            "Mode: Light. Click for Auto."
        );
    }
}
