use gtk4::gdk;
use gtk4::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

pub struct LayerShellConfig {
    pub namespace: &'static str,
    pub layer: Layer,
    pub default_width: Option<i32>,
    pub default_height: Option<i32>,
    pub anchors: &'static [(Edge, bool)],
    pub margins: &'static [(Edge, i32)],
    pub keyboard_mode: KeyboardMode,
    /// Reserve screen space (auto exclusive zone) — bars, not overlays.
    pub exclusive: bool,
}

pub fn create_layer_window(app: &gtk4::Application, config: &LayerShellConfig) -> gtk4::Window {
    create_layer_window_on(app, config, None)
}

/// `monitor: None` lets the compositor pick the output (usually the
/// focused one); per-output surfaces like the bar pin one explicitly.
pub fn create_layer_window_on(
    app: &gtk4::Application,
    config: &LayerShellConfig,
    monitor: Option<&gdk::Monitor>,
) -> gtk4::Window {
    let mut builder = gtk4::Window::builder().application(app);

    if let Some(w) = config.default_width {
        builder = builder.default_width(w);
    }
    if let Some(h) = config.default_height {
        builder = builder.default_height(h);
    }

    let window = builder.build();

    // Near-unity opacity forces compositor alpha blending so the
    // transparent window background composites correctly (Sway #8904).
    window.set_opacity(0.999);

    window.init_layer_shell();
    window.set_layer(config.layer);
    window.set_namespace(Some(config.namespace));
    window.set_keyboard_mode(config.keyboard_mode);

    if let Some(monitor) = monitor {
        window.set_monitor(Some(monitor));
    }

    for &(edge, anchored) in config.anchors {
        window.set_anchor(edge, anchored);
    }
    for &(edge, margin) in config.margins {
        window.set_margin(edge, margin);
    }

    if config.exclusive {
        window.auto_exclusive_zone_enable();
    }

    window
}
