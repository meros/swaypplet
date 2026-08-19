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
    make_layer_window(&window, config, monitor);
    window
}

/// Turn an already-built window into a layer surface.
///
/// Split out of [`create_layer_window_on`] for the greeter, which has no
/// `GApplication` to build against (`greet::run` is a bare `gtk4::init()` plus
/// its own main loop, like the locker) and whose window comes out of
/// `lock::ui::SurfaceSet::build_surface` with its content already in it. Same
/// calls in the same order either way, so there is one description of what a
/// swaypplet layer surface is.
///
/// Must run before the window is presented: `gtk_layer_init_for_window` swaps
/// the surface type, and GTK has already asked the compositor for an xdg
/// toplevel by the time a presented window could be converted.
pub fn make_layer_window(
    window: &gtk4::Window,
    config: &LayerShellConfig,
    monitor: Option<&gdk::Monitor>,
) {
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
}

/// The output with the camera above it.
///
/// Everything that reports a face check has to appear on the built-in panel,
/// because that is the only screen with a lens over it. On the focused output
/// it is worse than useless: it invites the user to look at an external
/// monitor, which points their face away from the sensor at exactly the
/// moment it is trying to read one.
///
/// Connector names are the signal. Wayland gives them straight through from
/// DRM, where the internal panel is eDP (laptops), LVDS (older ones) or DSI
/// (tablets and some ARM machines). Nothing else can be internal, so an
/// unmatched name is external and a machine with no match at all has no
/// built-in panel to prefer.
pub fn is_internal(monitor: &gdk::Monitor) -> bool {
    let Some(connector) = monitor.connector() else {
        return false;
    };
    let name = connector.to_uppercase();
    name.starts_with("EDP") || name.starts_with("LVDS") || name.starts_with("DSI")
}

/// Resolve the built-in panel, or `None` when there is not one.
///
/// Looked up per call rather than cached: monitors come and go, and a stale
/// `gdk::Monitor` pins a surface to an output the compositor has forgotten.
pub fn internal_monitor() -> Option<gdk::Monitor> {
    let display = gdk::Display::default()?;
    display
        .monitors()
        .into_iter()
        .flatten()
        .filter_map(|obj| obj.downcast::<gdk::Monitor>().ok())
        .find(is_internal)
}
