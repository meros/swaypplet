use gtk4::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

pub struct LayerShellConfig {
    pub namespace: &'static str,
    pub default_width: Option<i32>,
    pub default_height: Option<i32>,
    pub anchors: &'static [(Edge, bool)],
    pub margins: &'static [(Edge, i32)],
    pub keyboard_mode: KeyboardMode,
}

pub fn create_layer_window(app: &gtk4::Application, config: &LayerShellConfig) -> gtk4::Window {
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
    window.set_layer(Layer::Overlay);
    window.set_namespace(config.namespace);
    window.set_keyboard_mode(config.keyboard_mode);

    for &(edge, anchored) in config.anchors {
        window.set_anchor(edge, anchored);
    }
    for &(edge, margin) in config.margins {
        window.set_margin(edge, margin);
    }

    window
}
