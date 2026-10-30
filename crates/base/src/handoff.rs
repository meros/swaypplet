//! Tell sway where a picture of what is about to appear is on screen, so
//! the window grows out of it (the swayfx `handoff` command,
//! nixos patches/swayfx-handoff.patch).
//!
//! The command takes layout coordinates, and a widget only knows where it
//! is inside its surface. Layer shell does not tell a client where the
//! compositor put its surface either, so [`Placement::origin`] works it out the
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

/// What placing a layer surface needs from its window, read on the GTK
/// thread; the sway half of the answer ([`usable_area`]) is a round trip and
/// is asked on a worker.
struct Placement {
    /// The output's geometry.
    monitor: Rect,
    /// The output to ask sway about, when the surface claims no exclusive
    /// zone of its own and so sits inside what the others (the bar) leave.
    connector: Option<String>,
    size: (f64, f64),
    /// Anchored, and the margin, per edge: left, right, top, bottom.
    edges: [(bool, f64); 4],
}

impl Placement {
    /// `None` when that cannot be known (not a layer surface, not on an
    /// output yet).
    fn of(window: &gtk4::Window) -> Option<Self> {
        if !window.is_layer_window() {
            return None;
        }
        let surface = window.surface()?;
        let monitor = surface.display().monitor_at_surface(&surface)?;
        let geo = monitor.geometry();
        let edge = |e: Edge| (window.is_anchor(e), f64::from(window.margin(e)));
        Some(Placement {
            monitor: (
                f64::from(geo.x()),
                f64::from(geo.y()),
                f64::from(geo.width()),
                f64::from(geo.height()),
            ),
            connector: (window.exclusive_zone() == 0)
                .then(|| monitor.connector())
                .flatten()
                .map(|c| c.to_string()),
            size: (f64::from(window.width()), f64::from(window.height())),
            edges: [
                edge(Edge::Left),
                edge(Edge::Right),
                edge(Edge::Top),
                edge(Edge::Bottom),
            ],
        })
    }

    /// Where the surface sits in the output layout, the way wlroots places
    /// one: inside the output, or inside `usable` (the output's usable area)
    /// when the surface has no exclusive zone, then by its anchors and
    /// margins.
    fn origin(&self, usable: Option<Rect>) -> (f64, f64) {
        let bounds = usable.unwrap_or(self.monitor);
        let axis = |start: f64, len: f64, size: f64, (a, ma): (bool, f64), (b, mb): (bool, f64)| {
            match (a, b) {
                (true, false) => start + ma,
                (false, true) => start + len - size - mb,
                // Both or neither: centred in what is left between the margins.
                _ => start + ma + (len - ma - mb - size) / 2.0,
            }
        };
        let [left, right, top, bottom] = self.edges;
        (
            axis(bounds.0, bounds.2, self.size.0, left, right),
            axis(bounds.1, bounds.3, self.size.1, top, bottom),
        )
    }
}

/// The output's usable area: the rect sway gives the workspace shown there.
/// Blocking; runs on a worker.
fn usable_area(connector: &str) -> Option<Rect> {
    let workspaces = crate::sway::ipc::connect().ok()?.get_workspaces().ok()?;
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

/// `widget`'s box relative to its window, and the window's placement: the
/// GTK half of the widget's box in layout coordinates.
fn widget_box(widget: &impl IsA<gtk4::Widget>) -> Option<(Placement, Rect)> {
    let window = widget.root()?.downcast::<gtk4::Window>().ok()?;
    let placement = Placement::of(&window)?;
    let a = widget.compute_point(&window, &gtk4::graphene::Point::new(0.0, 0.0))?;
    let b = widget.compute_point(
        &window,
        &gtk4::graphene::Point::new(widget.width() as f32, widget.height() as f32),
    )?;
    let (w, h) = (f64::from(b.x() - a.x()), f64::from(b.y() - a.y()));
    (w >= 1.0 && h >= 1.0).then(|| (placement, (f64::from(a.x()), f64::from(a.y()), w, h)))
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
///
/// The widget is measured here, on the GTK thread; the usable-area query and
/// the command go to a worker, one after the other, as they went before.
pub fn open_from(widget: &impl IsA<gtk4::Widget>) {
    let Some((placement, (x, y, w, h))) = widget_box(widget) else {
        return;
    };
    crate::spawn::spawn_work(
        move || {
            let usable = placement.connector.as_deref().and_then(usable_area);
            let (ox, oy) = placement.origin(usable);
            crate::sway::ipc::run_command_blocking(&format!(
                "handoff open {}",
                fmt((ox + x, oy + y, w, h))
            ))
        },
        |result| {
            if let Err(msg) = result {
                log::warn!("{msg}");
            }
        },
    );
}
