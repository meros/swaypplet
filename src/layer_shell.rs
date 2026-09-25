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

/// Destroy a layer window, including one that was never shown.
///
/// Destroying a window takes it off the application, and on Wayland GTK then
/// removes its surface from the session: `window_forget` in
/// gtkapplication-wayland.c passes `gtk_native_get_surface()` straight to
/// `gdk_wayland_toplevel_remove_from_session`, which dereferences it. A
/// window that was never shown has no surface, so that is a NULL
/// dereference, and the process goes down with every surface in it (GTK
/// 4.22.4). Per-monitor cards (the keybind sheet, the OSD) and pins rebuilt
/// on another output are routinely destroyed without ever having been shown,
/// on an unplug or a focus change. Realizing first gives GTK the surface it
/// assumes; nothing is mapped, so nothing appears.
pub fn destroy_window(window: &gtk4::Window) {
    if !window.is_realized() {
        gtk4::prelude::WidgetExt::realize(window);
    }
    window.destroy();
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
    monitors().find(is_internal)
}

/// The monitor sway calls `name`, or `None` when it is not connected.
///
/// Sway's output names are DRM connector names, which is the same string
/// `gdk::Monitor::connector` reports, so the two sides need no table. Looked
/// up per call, for the reason [`internal_monitor`] gives.
pub fn monitor_by_connector(name: &str) -> Option<gdk::Monitor> {
    monitors().find(|m| m.connector().is_some_and(|c| c == name))
}

fn monitors() -> impl Iterator<Item = gdk::Monitor> {
    // `ListModel::into_iter` borrows the model, so the list has to outlive
    // the iterator: collect it here rather than chaining off a temporary.
    let list: Vec<gdk::Monitor> = gdk::Display::default()
        .into_iter()
        .flat_map(|display| {
            display
                .monitors()
                .into_iter()
                .flatten()
                .filter_map(|obj| obj.downcast::<gdk::Monitor>().ok())
                .collect::<Vec<_>>()
        })
        .collect();
    list.into_iter()
}
