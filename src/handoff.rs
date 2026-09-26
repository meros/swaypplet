//! Tell sway where a picture of what is about to appear is on screen, so
//! the window grows out of it (the swayfx `handoff` command,
//! nixos patches/swayfx-handoff.patch).
//!
//! The command takes layout coordinates, and a widget only knows where it
//! is inside its surface. Layer shell does not tell a client where the
//! compositor put its surface either, so [`surface_origin`] works it out the
//! way wlroots places one: inside the output, or inside the output's usable
//! area when the surface does not claim an exclusive zone of its own, then
//! by its anchors and margins.
//!
//! A sway without the patch answers the command with an error, and nothing
//! else happens: the transition is sway's ordinary one. That is why the
//! hand-off never shares a `;` list with the command it precedes: sway stops
//! a list at its first unknown command.

use gtk4::prelude::*;
use gtk4_layer_shell::{Edge, LayerShell};

/// A rectangle in layout coordinates.
pub type Rect = (f64, f64, f64, f64);

/// Where `window`'s surface sits in the output layout, or `None` when that
/// cannot be known (not a layer surface, not on an output yet).
pub fn surface_origin(window: &gtk4::Window) -> Option<(f64, f64)> {
    if !window.is_layer_window() {
        return None;
    }
    let surface = window.surface()?;
    let monitor = surface.display().monitor_at_surface(&surface)?;
    let geo = monitor.geometry();
    let mut bounds = (
        f64::from(geo.x()),
        f64::from(geo.y()),
        f64::from(geo.width()),
        f64::from(geo.height()),
    );
    // A surface with no exclusive zone of its own is placed inside what the
    // others (the bar) leave, which sway reports as the workspace's rect.
    if window.exclusive_zone() == 0
        && let Some(connector) = monitor.connector()
        && let Some(usable) = usable_area(&connector)
    {
        bounds = usable;
    }
    let (w, h) = (f64::from(window.width()), f64::from(window.height()));
    let axis = |start: f64, len: f64, size: f64, a: Edge, b: Edge| -> f64 {
        let (ma, mb) = (f64::from(window.margin(a)), f64::from(window.margin(b)));
        match (window.is_anchor(a), window.is_anchor(b)) {
            (true, false) => start + ma,
            (false, true) => start + len - size - mb,
            // Both or neither: centred in what is left between the margins.
            _ => start + ma + (len - ma - mb - size) / 2.0,
        }
    };
    Some((
        axis(bounds.0, bounds.2, w, Edge::Left, Edge::Right),
        axis(bounds.1, bounds.3, h, Edge::Top, Edge::Bottom),
    ))
}

/// The output's usable area: the rect sway gives the workspace shown there.
fn usable_area(connector: &str) -> Option<Rect> {
    let workspaces = crate::sway_ipc::connect().ok()?.get_workspaces().ok()?;
    let ws = workspaces
        .iter()
        .find(|ws| ws.output == connector && ws.visible)?;
    Some((
        f64::from(ws.rect.x),
        f64::from(ws.rect.y),
        f64::from(ws.rect.width),
        f64::from(ws.rect.height),
    ))
}

/// `widget`'s box in layout coordinates, `inner` (x, y, w, h in the widget's
/// own coordinates) when given, the whole widget otherwise.
pub fn widget_rect(widget: &impl IsA<gtk4::Widget>, inner: Option<Rect>) -> Option<Rect> {
    let window = widget.root()?.downcast::<gtk4::Window>().ok()?;
    let (ox, oy) = surface_origin(&window)?;
    let (x, y, w, h) = inner.unwrap_or((
        0.0,
        0.0,
        f64::from(widget.width()),
        f64::from(widget.height()),
    ));
    let a = widget.compute_point(&window, &gtk4::graphene::Point::new(x as f32, y as f32))?;
    let b = widget.compute_point(
        &window,
        &gtk4::graphene::Point::new((x + w) as f32, (y + h) as f32),
    )?;
    let (w, h) = (f64::from(b.x() - a.x()), f64::from(b.y() - a.y()));
    (w >= 1.0 && h >= 1.0).then(|| (ox + f64::from(a.x()), oy + f64::from(a.y()), w, h))
}

fn fmt(r: Rect) -> String {
    format!(
        "{} {} {} {}",
        r.0.round(),
        r.1.round(),
        r.2.round().max(1.0),
        r.3.round().max(1.0)
    )
}

/// The next new window opens out of `widget`.
pub fn open_from(widget: &impl IsA<gtk4::Widget>) {
    if let Some(r) = widget_rect(widget, None) {
        crate::sway_ipc::run_command(&format!("handoff open {}", fmt(r)));
    }
}
