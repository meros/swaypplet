//! What a workspace looks like, as rectangles: the windows sway would draw if
//! you switched to it, where it would draw them, and in which order.
//!
//! The card's live picture of a workspace is composed from per-window
//! captures (see `live.rs`). A capture is one window's pixels and nothing
//! about where the window sits, so the placement comes from the tree, which
//! already holds it: every view's `rect` is in the output's layout
//! coordinates.
//!
//! The picture is the bounding box of the window content, not the workspace
//! and not the output. That leaves out the bar and the outer gaps, which are
//! the same on every tile, and it keeps every pixel of every window: the tile
//! fits the box whole and never crops (see [`fit`]). A fullscreen view's box is
//! its own rect, the whole output, bar area included.
//!
//! Only what would be on screen is kept. A tabbed or stacked container shows
//! its most recently focused child and none of the others, a fullscreen view
//! is the whole picture, and floating views go on top in the order sway
//! stacks them. Drawing every view would put hidden tabs over the visible one
//! and make the picture wrong in exactly the case where it is hardest to
//! tell.
//!
//! No GTK here, so every rule above is a unit test.

use swayipc::{Node, NodeLayout, NodeType};

/// A workspace's visible windows, relative to the top-left of their bounding
/// box. `width` and `height` are the box's; an empty workspace's box is the
/// workspace itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scene {
    pub width: i32,
    pub height: i32,
    /// Bottom first: tiled views, then floating ones.
    pub windows: Vec<Window>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Window {
    /// The `foreign_toplevel_identifier`, which is how a capture finds this
    /// window. `None` when sway gave none; the window is drawn as its icon.
    pub id: Option<String>,
    /// `app_id`, or the X11 class, for the icon and the fallback tile.
    pub app: String,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// The scene for the workspace called `name`, or `None` when it is gone.
pub fn scene(tree: &Node, name: &str) -> Option<Scene> {
    let ws = find_workspace(tree, name)?;
    let mut windows = Vec::new();

    // A fullscreen view is the whole picture, at its own rect: the output's,
    // which is larger than the workspace's when a bar reserves space.
    if let Some(full) = find_fullscreen(ws) {
        windows.push(content(full));
    } else {
        for child in &ws.nodes {
            tiled(child, &mut windows);
        }
        for child in &ws.floating_nodes {
            // A floating container can hold a whole tiled subtree.
            tiled(child, &mut windows);
        }
    }

    let Some((x0, y0, x1, y1)) = windows
        .iter()
        .map(|w| (w.x, w.y, w.x + w.w, w.y + w.h))
        .reduce(|a, b| (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3)))
    else {
        return Some(Scene {
            width: ws.rect.width,
            height: ws.rect.height,
            windows,
        });
    };
    for w in &mut windows {
        w.x -= x0;
        w.y -= y0;
    }
    Some(Scene {
        width: x1 - x0,
        height: y1 - y0,
        windows,
    })
}

fn find_workspace<'a>(node: &'a Node, name: &str) -> Option<&'a Node> {
    if node.node_type == NodeType::Workspace && node.name.as_deref() == Some(name) {
        return Some(node);
    }
    node.nodes.iter().find_map(|c| find_workspace(c, name))
}

fn find_fullscreen(node: &Node) -> Option<&Node> {
    if is_view(node) && node.fullscreen_mode == Some(1) {
        return Some(node);
    }
    node.nodes
        .iter()
        .chain(node.floating_nodes.iter())
        .find_map(find_fullscreen)
}

/// A view is a node that holds a client. Containers hold nodes.
fn is_view(node: &Node) -> bool {
    node.nodes.is_empty()
        && matches!(node.node_type, NodeType::Con | NodeType::FloatingCon)
        && (node.pid.is_some() || node.app_id.is_some() || node.window_properties.is_some())
}

/// Walk a tiled subtree and push the views that are on screen.
fn tiled(node: &Node, out: &mut Vec<Window>) {
    if is_view(node) {
        out.push(content(node));
        return;
    }
    match node.layout {
        NodeLayout::Tabbed | NodeLayout::Stacked => {
            // One child is shown: the one focused most recently, which is
            // the head of `focus`. A container nothing was ever focused in
            // shows its first child, which is what sway does too.
            let shown = node
                .focus
                .first()
                .and_then(|id| node.nodes.iter().find(|c| c.id == *id))
                .or_else(|| node.nodes.first());
            if let Some(child) = shown {
                tiled(child, out);
            }
        }
        _ => {
            for child in &node.nodes {
                tiled(child, out);
            }
        }
    }
    for child in &node.floating_nodes {
        tiled(child, out);
    }
}

/// A view's content area in layout coordinates: `window_rect` is relative
/// to `rect` and leaves out the border and the title bar.
fn content(node: &Node) -> Window {
    let r = &node.rect;
    let c = &node.window_rect;
    window(node, (r.x + c.x, r.y + c.y, c.width, c.height))
}

fn window(node: &Node, (x, y, w, h): (i32, i32, i32, i32)) -> Window {
    let app = node
        .app_id
        .clone()
        .or_else(|| {
            node.window_properties
                .as_ref()
                .and_then(|p| p.class.clone())
        })
        .unwrap_or_default();
    Window {
        id: node.foreign_toplevel_identifier.clone(),
        app,
        x,
        y,
        w,
        h,
    }
}

/// Where a scene lands in a box of `box_w` by `box_h`: the scale and the
/// offset that fit it whole, centered.
///
/// Whole, always. Filling the box instead would crop the scene's edges, and
/// the edges are window content: a terminal's first column, a browser's
/// scrollbar. A scene whose aspect differs from the box's gets thin bands.
/// The tile never changes size for it.
pub fn fit(scene_w: i32, scene_h: i32, box_w: i32, box_h: i32) -> (f64, f64, f64) {
    if scene_w <= 0 || scene_h <= 0 || box_w <= 0 || box_h <= 0 {
        return (0.0, 0.0, 0.0);
    }
    let (sw, sh, bw, bh) = (
        f64::from(scene_w),
        f64::from(scene_h),
        f64::from(box_w),
        f64::from(box_h),
    );
    let s = (bw / sw).min(bh / sh);
    (s, (bw - sw * s) / 2.0, (bh - sh * s) / 2.0)
}

#[cfg(test)]
mod tests {
    use super::super::place::fixture::{node, output, tree};
    use super::*;
    use serde_json::{Value, json};

    fn r(x: i32, y: i32, w: i32, h: i32) -> Value {
        json!({"x": x, "y": y, "width": w, "height": h})
    }

    /// A view at `rect`, whose content fills it.
    fn view(id: i64, ident: &str, app: &str, rect: Value) -> Value {
        let (w, h) = (rect["width"].clone(), rect["height"].clone());
        node(json!({
            "id": id, "type": "con", "pid": 100 + id, "app_id": app,
            "foreign_toplevel_identifier": ident,
            "rect": rect, "window_rect": {"x": 0, "y": 0, "width": w, "height": h},
        }))
    }

    fn container(id: i64, layout: &str, focus: Vec<i64>, children: Vec<Value>) -> Value {
        node(json!({
            "id": id, "type": "con", "layout": layout,
            "focus": focus, "nodes": children,
        }))
    }

    /// One output with one workspace `"1"` at y = 30 (below a 30 px bar).
    fn ws(tiled: Vec<Value>, floating: Vec<Value>) -> Node {
        let ws = node(json!({
            "id": 10, "type": "workspace", "num": 1, "name": "1",
            "rect": r(0, 30, 1440, 870),
            "nodes": tiled, "floating_nodes": floating,
        }));
        tree(vec![2], vec![output(2, "eDP-1", vec![10], vec![ws])])
    }

    fn ids(s: &Scene) -> Vec<&str> {
        s.windows.iter().filter_map(|w| w.id.as_deref()).collect()
    }

    #[test]
    fn a_split_shows_every_view_relative_to_the_workspace() {
        let t = ws(
            vec![
                view(20, "a", "foot", r(0, 30, 720, 870)),
                view(21, "b", "firefox", r(720, 30, 720, 870)),
            ],
            vec![],
        );
        let s = scene(&t, "1").unwrap();
        assert_eq!((s.width, s.height), (1440, 870));
        assert_eq!(ids(&s), ["a", "b"]);
        assert_eq!(
            (s.windows[1].x, s.windows[1].y),
            (720, 0),
            "the bar above y = 30 is not in the picture"
        );
    }

    #[test]
    fn the_picture_is_the_content_and_not_the_gaps() {
        // 10 px outer gaps: the views start at (10, 40) and end 10 px short.
        let t = ws(
            vec![
                view(20, "a", "foot", r(10, 40, 705, 850)),
                view(21, "b", "foot", r(725, 40, 705, 850)),
            ],
            vec![],
        );
        let s = scene(&t, "1").unwrap();
        assert_eq!((s.width, s.height), (1420, 850));
        assert_eq!((s.windows[0].x, s.windows[0].y), (0, 0));
        assert_eq!((s.windows[1].x, s.windows[1].y), (715, 0));
    }

    #[test]
    fn a_float_past_the_workspace_edge_is_still_all_in_the_picture() {
        let t = ws(
            vec![view(20, "tiled", "foot", r(0, 30, 1440, 870))],
            vec![view(40, "float", "mpv", r(1200, 700, 400, 300))],
        );
        let s = scene(&t, "1").unwrap();
        assert_eq!((s.width, s.height), (1600, 970));
    }

    #[test]
    fn a_tabbed_container_shows_only_its_focused_child() {
        let t = ws(
            vec![container(
                30,
                "tabbed",
                vec![32, 31],
                vec![
                    view(31, "hidden", "foot", r(0, 30, 1440, 870)),
                    view(32, "shown", "foot", r(0, 30, 1440, 870)),
                ],
            )],
            vec![],
        );
        assert_eq!(ids(&scene(&t, "1").unwrap()), ["shown"]);
    }

    #[test]
    fn a_stacked_container_with_no_focus_history_shows_its_first_child() {
        let t = ws(
            vec![container(
                30,
                "stacked",
                vec![],
                vec![
                    view(31, "first", "foot", r(0, 30, 1440, 870)),
                    view(32, "second", "foot", r(0, 30, 1440, 870)),
                ],
            )],
            vec![],
        );
        assert_eq!(ids(&scene(&t, "1").unwrap()), ["first"]);
    }

    #[test]
    fn floating_views_go_on_top_of_tiled_ones() {
        let t = ws(
            vec![view(20, "tiled", "foot", r(0, 30, 1440, 870))],
            vec![view(40, "float", "pavucontrol", r(400, 200, 600, 400))],
        );
        let s = scene(&t, "1").unwrap();
        assert_eq!(ids(&s), ["tiled", "float"], "last is drawn on top");
        assert_eq!((s.windows[1].x, s.windows[1].y), (400, 170));
        assert_eq!((s.width, s.height), (1440, 870), "the float is inside");
    }

    #[test]
    fn a_fullscreen_view_is_the_whole_picture() {
        let mut full = view(21, "full", "mpv", r(0, 0, 1440, 900));
        full["fullscreen_mode"] = json!(1);
        let t = ws(
            vec![view(20, "under", "foot", r(0, 30, 720, 870)), full],
            vec![],
        );
        let s = scene(&t, "1").unwrap();
        assert_eq!(ids(&s), ["full"]);
        assert_eq!(
            (
                s.windows[0].x,
                s.windows[0].y,
                s.windows[0].w,
                s.windows[0].h
            ),
            (0, 0, 1440, 900),
            "its own rect, over the bar's area too, not squeezed"
        );
        assert_eq!((s.width, s.height), (1440, 900));
    }

    #[test]
    fn the_content_rect_leaves_out_the_title_bar() {
        let mut v = view(20, "a", "foot", r(0, 30, 1440, 870));
        v["window_rect"] = r(2, 24, 1436, 844);
        let t = ws(vec![v], vec![]);
        let w = &scene(&t, "1").unwrap().windows[0];
        assert_eq!(
            (w.x, w.y, w.w, w.h),
            (0, 0, 1436, 844),
            "the picture starts at the content"
        );
    }

    #[test]
    fn an_empty_workspace_is_a_scene_with_no_windows() {
        let s = scene(&ws(vec![], vec![]), "1").unwrap();
        assert!(s.windows.is_empty());
        assert_eq!((s.width, s.height), (1440, 870), "the workspace itself");
    }

    #[test]
    fn a_missing_workspace_is_none() {
        assert!(scene(&ws(vec![], vec![]), "7").is_none());
    }

    #[test]
    fn fit_never_crops() {
        // Near the box's aspect, where filling it was once allowed: a
        // 1280x800 panel minus a 30 px bar. Whole, with thin bands.
        let (s, dx, dy) = fit(1280, 770, 240, 150);
        assert!((s - 240.0 / 1280.0).abs() < 1e-9, "width-bound");
        assert!(dx.abs() < 1e-9);
        assert!(dy > 0.0 && dy < 4.0, "bands of {dy} px");
        for (w, h) in [(1280, 770), (1440, 900), (1200, 1920), (3440, 1440), (7, 3)] {
            let (s, dx, dy) = fit(w, h, 240, 150);
            assert!(dx >= 0.0 && dy >= 0.0, "{w}x{h} starts outside the box");
            assert!(f64::from(w) * s <= 240.0 + 1e-9 && f64::from(h) * s <= 150.0 + 1e-9);
        }
    }

    #[test]
    fn a_portrait_workspace_is_letterboxed_and_centered() {
        let (s, dx, dy) = fit(1200, 1920, 240, 150);
        assert!((s - 150.0 / 1920.0).abs() < 1e-9);
        assert!(dx > 0.0, "bands at the sides");
        assert!(dy.abs() < 1e-9);
    }

    #[test]
    fn an_ultrawide_workspace_is_letterboxed() {
        let (_, dx, dy) = fit(3440, 1440, 240, 150);
        assert!(dx.abs() < 1e-9);
        assert!(dy > 0.0, "bands at top and bottom");
    }

    #[test]
    fn fit_of_an_empty_scene_draws_nothing() {
        assert_eq!(fit(0, 0, 320, 200), (0.0, 0.0, 0.0));
    }
}
