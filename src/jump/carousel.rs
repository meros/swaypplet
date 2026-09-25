//! The row the jump card's places stand in, floating over the desktop.
//!
//! Cover flow: the selected place is at the front, large and square to you;
//! the others stand beside it, turned steeply away, one after another out to
//! the screen's edges. A step slides the row by one place, and the place
//! arriving at the front turns to face you on the way. The places are live,
//! so what slides past is the workspaces as they are right now.
//!
//! There is no card behind them and no ground under them: the surface is a
//! transparent strip across the output. That is also why no two places may
//! overlap at rest. A picture's gaps are transparent, so a place standing
//! behind another would show through it; the spacing is chosen so none does,
//! and how many places fit a side follows from the screen's width. During a
//! step the place leaving the front is still wide while it turns, and for a
//! moment it passes over the next one; no curve for the slide avoids that
//! for one pair without causing it for the pair after, and it lasts a
//! fraction of the step.
//!
//! The row is drawn, not laid out. Each place is allocated at its own size
//! with a 3D transform (`gsk::Transform`, which GTK renders with
//! perspective). Places are drawn back to front, because GTK draws children
//! in order and has no depth test.
//!
//! Where a place goes is [`slot`], which has no GTK in it.

use std::cell::{Cell, RefCell};

use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use gtk4::{gdk, glib, graphene, gsk};

use super::rows;

/// How far a side place is turned away, in degrees. Steep, so a side place
/// takes little width and more of them fit; not so steep that its picture
/// stops reading as one.
const SIDE_DEG: f64 = 56.0;
/// How far behind the front a side place stands, in pixels.
const SIDE_Z: f64 = -150.0;
/// Between the front place and the first side place, on screen.
const FRONT_GAP: f64 = 36.0;
/// Between two side places, on screen.
const SIDE_GAP: f64 = 12.0;
/// Narrower than this on screen, a place is a sliver and not a picture. Far
/// out, perspective shows a turned place ever more edge-on (on a 2560 px
/// stage the eighth would be 39 px), so the row stops here instead.
const MIN_SIDE_W: f64 = 70.0;
/// The camera's distance. Shorter is a stronger perspective.
const DEPTH: f64 = 1600.0;
/// Past the last place that fits, a place fades out over this much of a
/// step instead of vanishing.
const FADE_STEPS: f64 = 0.6;
/// One step, in milliseconds.
const TURN_MS: f64 = 240.0;

/// Where a place stands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slot {
    /// Sideways from the stage's centre, before perspective, in pixels.
    pub x: f64,
    /// Away from the viewer, in pixels: 0 at the front, negative behind.
    pub z: f64,
    /// Turn about the vertical axis, in degrees; positive on the right.
    pub angle: f64,
    pub opacity: f64,
}

/// Where a place's left and right edges land on screen, from the stage's
/// centre: the same projection GTK applies to the transform in
/// `size_allocate`. A turned place is not its width times one shrink: its
/// far edge recedes and its near edge comes forward, so both are projected.
pub fn project(s: &Slot) -> (f64, f64) {
    let half = f64::from(rows::TILE_W) / 2.0;
    let t = s.angle.to_radians();
    let edge = |lx: f64| {
        let x = s.x + lx * t.cos();
        let z = s.z - lx * t.sin();
        x * DEPTH / (DEPTH - z)
    };
    let (a, b) = (edge(-half), edge(half));
    (a.min(b), a.max(b))
}

/// A side place on the right whose centre is at `x` before perspective.
fn side(x: f64) -> Slot {
    Slot {
        x,
        z: SIDE_Z,
        angle: SIDE_DEG,
        opacity: 1.0,
    }
}

/// The centres, before perspective, of the side places on the right that
/// fit a stage `stage_w` wide, nearest first. Each stands one gap clear of
/// the one before it, found by bisection on its projected left edge (which
/// grows with the centre).
fn side_centres(stage_w: f64) -> Vec<f64> {
    let mut centres = Vec::new();
    let mut edge = f64::from(rows::TILE_W) / 2.0 + FRONT_GAP;
    for _ in 0..rows::MAX_ROWS {
        let (mut lo, mut hi) = (0.0, 4.0 * stage_w);
        for _ in 0..48 {
            let mid = (lo + hi) / 2.0;
            if project(&side(mid)).0 < edge {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let (left, right) = project(&side(hi));
        if right > stage_w / 2.0 || right - left < MIN_SIDE_W {
            break;
        }
        centres.push(hi);
        edge = right + SIDE_GAP;
    }
    centres
}

/// How many places fit on one side of a stage `stage_w` wide. `slot` reads
/// the centres directly; this is their count for the tests.
#[cfg(test)]
fn per_side(stage_w: f64) -> usize {
    side_centres(stage_w).len()
}

/// Where place `i` stands when the row is at `pos` (a fractional index: 2.0
/// is place 2 at the front, 2.5 halfway to place 3), on a stage `stage_w`
/// wide. `None` when it is out of sight.
pub fn slot(i: usize, pos: f64, stage_w: f64) -> Option<Slot> {
    let d = i as f64 - pos;
    let sign = d.signum();
    let a = d.abs();
    let centres = side_centres(stage_w);
    let n = centres.len() as f64;
    if centres.is_empty() || a > n + FADE_STEPS {
        return None;
    }
    // The spot `a` places out, before perspective: the front at 0, then the
    // side centres, and one more step past the last for a place fading out.
    let at = |k: usize| -> f64 {
        match k {
            0 => 0.0,
            k if k <= centres.len() => centres[k - 1],
            k => {
                let last = centres[centres.len() - 1];
                let prev = if centres.len() > 1 {
                    centres[centres.len() - 2]
                } else {
                    0.0
                };
                last + (k - centres.len()) as f64 * (last - prev)
            }
        }
    };
    let k = a.floor() as usize;
    let f = a - a.floor();
    let x = at(k) + (at(k + 1) - at(k)) * f;
    // Turning happens on the way from the front to the first side spot.
    let turn = a.min(1.0);
    // Full opacity for every place that fits. GTK draws a node with
    // opacity below 1 offscreen first, at a scale it reads from the
    // transform with `graphene_matrix_decompose` (gskgpunodeprocessor.c,
    // `extract_scale_from_transform`). For a perspective matrix that
    // estimate comes out low for places turned one way, and those places
    // were drawn small and stretched: the left side was blurred and the
    // right was not. At full opacity a place goes straight to the screen
    // through its transform, at the screen's own resolution. Only a place
    // fading out mid-step pays for the offscreen, for a fraction of a step.
    let opacity = if a <= n {
        1.0
    } else {
        1.0 - (a - n) / FADE_STEPS
    };
    Some(Slot {
        x: sign * x,
        z: SIDE_Z * turn,
        angle: sign * SIDE_DEG * turn,
        opacity: opacity.max(0.0),
    })
}

/// The transform that puts a place at `s` on a stage `w` by `h`: into the
/// middle, through the perspective, out to its spot, turned, and centred on
/// its own middle.
///
/// Then flattened: the depth every point comes out at is set to 0. The
/// renderer clips by depth, and a turned place spans ±175 px of it, so its
/// far half was cut away along a vertical line. Depth is not needed for the
/// picture: the perspective lives in `w`, which flattening leaves alone, and
/// the places are already drawn back to front by hand. The flattened matrix
/// maps depth to depth (a widget's points are all at 0), so it stays
/// invertible and pointer picking still works.
fn placed(s: &Slot, w: f32, h: f32) -> gsk::Transform {
    let t = gsk::Transform::new()
        .translate(&graphene::Point::new(w / 2.0, h / 2.0))
        .perspective(DEPTH as f32)
        .translate_3d(&graphene::Point3D::new(s.x as f32, 0.0, s.z as f32))
        .rotate_3d(s.angle as f32, &graphene::Vec3::y_axis())
        .translate(&graphene::Point::new(
            -rows::TILE_W as f32 / 2.0,
            -rows::TILE_H as f32 / 2.0,
        ));
    // graphene multiplies row vectors (p * M), so a point's depth is the
    // third column.
    let mut m = t.to_matrix().to_float();
    m[2] = 0.0;
    m[6] = 0.0;
    m[10] = 1.0;
    m[14] = 0.0;
    gsk::Transform::new().matrix(&graphene::Matrix::from_float(m))
}

/// Room around a place's texture for what it draws past its own edges: the
/// window shadows and the selection ring.
const TEXTURE_MARGIN: f32 = 32.0;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Carousel {
        /// Where the row is right now.
        pub pos: Cell<f64>,
        pub tick: RefCell<Option<gtk4::TickCallbackId>>,
        /// A place turned left, rendered to a texture: its content node,
        /// which GTK reuses until something in the place changes, and the
        /// texture made from it (see `snapshot`).
        pub textures: RefCell<Vec<Option<(gsk::RenderNode, gdk::Texture)>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Carousel {
        const NAME: &'static str = "SwayppletCarousel";
        type Type = super::Carousel;
        type ParentType = gtk4::Widget;
    }

    impl ObjectImpl for Carousel {
        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for Carousel {
        fn measure(&self, orientation: gtk4::Orientation, _: i32) -> (i32, i32, i32, i32) {
            // As wide as it is given (the strip spans the output), as tall as
            // the stage.
            match orientation {
                gtk4::Orientation::Horizontal => (0, 0, -1, -1),
                _ => (rows::STAGE_H, rows::STAGE_H, -1, -1),
            }
        }

        fn size_allocate(&self, width: i32, _height: i32, _baseline: i32) {
            let pos = self.pos.get();
            let widget = self.obj();
            let mut child = widget.first_child();
            let mut i = 0;
            while let Some(c) = child {
                match slot(i, pos, f64::from(width)) {
                    Some(s) => {
                        c.set_child_visible(true);
                        c.set_opacity(s.opacity);
                        // At the origin, untransformed: `snapshot` places
                        // it (see `left_group` for why it cannot be the
                        // allocation transform). Nothing picks a place with
                        // the pointer; the card is driven by the keyboard.
                        c.allocate(rows::TILE_W, rows::TILE_H, -1, None);
                    }
                    None => c.set_child_visible(false),
                }
                child = c.next_sibling();
                i += 1;
            }
        }

        fn snapshot(&self, snapshot: &gtk4::Snapshot) {
            // Back to front: the farthest place first, the front one last.
            let pos = self.pos.get();
            let widget = self.obj();
            let width = f64::from(widget.width());
            let mut children = Vec::new();
            let mut child = widget.first_child();
            let mut i = 0;
            while let Some(c) = child {
                if slot(i, pos, width).is_some() {
                    children.push(((i as f64 - pos).abs(), i, c.clone()));
                }
                child = c.next_sibling();
                i += 1;
            }
            children.sort_by(|a, b| b.0.total_cmp(&a.0));
            let (w, h) = (widget.width() as f32, widget.height() as f32);
            let content = |c: &gtk4::Widget| {
                let snap = gtk4::Snapshot::new();
                widget.snapshot_child(c, &snap);
                snap.to_node()
            };

            // Far to near. A place turned left is drawn from a texture of
            // itself, rendered here at the output's scale. GTK draws the
            // content of a 3D-transformed node at a scale it reads from the
            // matrix with `graphene_matrix_decompose`
            // (gskgpunodeprocessor.c, `extract_scale_from_transform`), and
            // for these perspective matrices that reading follows the turn:
            // 0.97 across for a place turned right, 0.29 for one turned left.
            // Text, icons and anything GTK draws offscreen in a left place
            // were rasterized at under a third of their size and stretched.
            // Flipping the matrix, or drawing the left side through an
            // offscreen of its own, still hands GTK a left-turned plane
            // somewhere, so neither helped. A texture sampled with linear
            // filtering is the one thing GTK draws under that transform
            // without consulting the estimate.
            let mut textures = self.textures.borrow_mut();
            for (_, i, c) in &children {
                let Some(s) = slot(*i, pos, width) else {
                    continue;
                };
                let Some(node) = content(c) else { continue };
                if s.angle >= 0.0 {
                    snapshot.append_node(gsk::TransformNode::new(node, Some(&placed(&s, w, h))));
                    continue;
                }
                if textures.len() <= *i {
                    textures.resize(*i + 1, None);
                }
                // GTK hands back the same node until the place changes (a
                // frame, the selection), so an unchanged place is not
                // rendered again, and a slide renders nothing at all.
                let texture = match &textures[*i] {
                    // The cache holds the node, so its address cannot be reused
                    // for another while it is compared here.
                    Some((cached, texture)) if cached.as_ptr() == node.as_ptr() => texture.clone(),
                    _ => {
                        let Some(texture) = render_place(&*widget, &node) else {
                            continue;
                        };
                        textures[*i] = Some((node, texture.clone()));
                        texture
                    }
                };
                let m = TEXTURE_MARGIN;
                snapshot.save();
                snapshot.transform(Some(&placed(&s, w, h)));
                snapshot.append_scaled_texture(
                    &texture,
                    gsk::ScalingFilter::Linear,
                    &graphene::Rect::new(
                        -m,
                        -m,
                        rows::TILE_W as f32 + 2.0 * m,
                        rows::TILE_H as f32 + 2.0 * m,
                    ),
                );
                snapshot.restore();
            }
        }
    }
}

/// A place's content rendered to a texture at the output's scale, with
/// [`TEXTURE_MARGIN`] around it. `None` before the widget has a renderer.
fn render_place(widget: &impl IsA<gtk4::Widget>, node: &gsk::RenderNode) -> Option<gdk::Texture> {
    let renderer = widget.native()?.renderer()?;
    let sf = widget.scale_factor() as f32;
    let m = TEXTURE_MARGIN;
    let scaled = gsk::TransformNode::new(node, Some(&gsk::Transform::new().scale(sf, sf)));
    Some(renderer.render_texture(
        scaled,
        Some(&graphene::Rect::new(
            -m * sf,
            -m * sf,
            (rows::TILE_W as f32 + 2.0 * m) * sf,
            (rows::TILE_H as f32 + 2.0 * m) * sf,
        )),
    ))
}

glib::wrapper! {
    pub struct Carousel(ObjectSubclass<imp::Carousel>)
        @extends gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

impl Carousel {
    pub fn new() -> Self {
        glib::Object::new()
    }

    pub fn append(&self, child: &impl IsA<gtk4::Widget>) {
        child.set_parent(self);
    }

    /// Slide the row so place `index` is at the front. Animated from wherever
    /// the row is, so a quick run of steps moves it smoothly.
    pub fn turn_to(&self, index: usize) {
        let imp = self.imp();
        if let Some(id) = imp.tick.take() {
            id.remove();
        }
        let from = imp.pos.get();
        let target = index as f64;
        if from == target {
            return;
        }
        if !crate::anim::animations_enabled() {
            imp.pos.set(target);
            self.queue_allocate();
            return;
        }
        let ms = crate::anim::duration(TURN_MS);
        let start = glib::monotonic_time();
        let id = self.add_tick_callback(move |row, _| {
            let t = (((glib::monotonic_time() - start) as f64 / 1000.0) / ms).clamp(0.0, 1.0);
            row.imp()
                .pos
                .set(from + (target - from) * crate::anim::standard(t));
            row.queue_allocate();
            if t >= 1.0 {
                row.imp().tick.take();
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
        imp.tick.replace(Some(id));
    }
}

impl Default for Carousel {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The laptop panel (1280 logical), a 1080p screen, a 1440p one.
    const STAGES: [f64; 3] = [1280.0, 1920.0, 2560.0];

    /// Where the flattened transform puts a place's corner, on screen and
    /// in depth.
    fn corner(s: &Slot, lx: f32, ly: f32) -> (f64, f64) {
        let m = placed(s, 0.0, 0.0).to_matrix();
        let p = m.transform_vec4(&graphene::Vec4::new(lx, ly, 0.0, 1.0));
        (f64::from(p.x() / p.w()), f64::from(p.z()))
    }

    #[test]
    fn a_turned_place_is_drawn_flat_with_its_perspective_kept() {
        let s = slot(4, 3.0, 1280.0).unwrap();
        let (l, r) = project(&s);
        let w = rows::TILE_W as f32;
        let (x0, z0) = corner(&s, 0.0, 0.0);
        let (x1, z1) = corner(&s, w, 0.0);
        assert_eq!((z0, z1), (0.0, 0.0), "nothing left to clip by depth");
        assert!(
            (x0.min(x1) - l).abs() < 0.5 && (x0.max(x1) - r).abs() < 0.5,
            "edges {x0}..{x1}, projected {l}..{r}"
        );
    }

    #[test]
    fn the_selected_place_is_at_the_front_square_on() {
        let s = slot(3, 3.0, 1280.0).unwrap();
        assert_eq!((s.x, s.z, s.angle), (0.0, 0.0, 0.0));
        assert_eq!(s.opacity, 1.0);
        let (l, r) = project(&s);
        assert!(
            (r - l - f64::from(rows::TILE_W)).abs() < 1e-9,
            "full size at the front"
        );
    }

    #[test]
    fn a_wider_screen_shows_more_places_a_side() {
        let counts: Vec<usize> = STAGES.iter().map(|w| per_side(*w)).collect();
        assert!(counts[0] >= 2, "the laptop shows {} a side", counts[0]);
        assert!(counts[1] > counts[0], "{counts:?}");
        assert!(counts[2] >= counts[1], "{counts:?}");
    }

    #[test]
    fn no_place_shown_is_a_sliver() {
        for w in STAGES {
            for i in 3..=3 + per_side(w) {
                let (l, r) = project(&slot(i, 3.0, w).unwrap());
                assert!(
                    r - l >= MIN_SIDE_W,
                    "stage {w}: place {i} is {} wide",
                    r - l
                );
            }
        }
    }

    #[test]
    fn a_side_place_is_wide_enough_to_read() {
        let first = project(&slot(4, 3.0, 1280.0).unwrap());
        assert!(
            first.1 - first.0 > 150.0,
            "the first side place is {} wide",
            first.1 - first.0
        );
    }

    #[test]
    fn places_stand_symmetrically_and_turned_away() {
        let right = slot(4, 3.0, 1280.0).unwrap();
        let left = slot(2, 3.0, 1280.0).unwrap();
        assert!((right.x + left.x).abs() < 1e-9);
        assert!(right.angle > 0.0 && left.angle < 0.0);
        assert!(right.z < 0.0);
    }

    #[test]
    fn no_two_places_overlap_at_rest() {
        for w in STAGES {
            for step in 0..=7 {
                let pos = f64::from(step);
                let mut spans: Vec<(f64, f64)> = (0..8)
                    .filter_map(|i| slot(i, pos, w))
                    .filter(|s| s.opacity > 0.0)
                    .map(|s| project(&s))
                    .collect();
                spans.sort_by(|a, b| a.0.total_cmp(&b.0));
                for pair in spans.windows(2) {
                    assert!(
                        pair[1].0 >= pair[0].1 - 0.5,
                        "stage {w} at {pos}: {:?} overlaps {:?}",
                        pair[0],
                        pair[1]
                    );
                }
            }
        }
    }

    #[test]
    fn every_place_that_fits_is_on_the_stage() {
        for w in STAGES {
            for i in 0..8 {
                if let Some(s) = slot(i, 3.0, w)
                    && s.opacity > 0.5
                {
                    let (l, r) = project(&s);
                    assert!(
                        l >= -w / 2.0 && r <= w / 2.0,
                        "stage {w}: place {i} spans {l}..{r}"
                    );
                }
            }
        }
    }

    #[test]
    fn past_the_last_that_fits_a_place_fades_then_goes() {
        let n = per_side(1280.0);
        let last = 3 + n;
        assert_eq!(
            slot(last, 3.0, 1280.0).unwrap().opacity,
            1.0,
            "a place at rest is never drawn offscreen"
        );
        assert!(slot(last + 1, 3.0, 1280.0).is_none());
        let fading = slot(last + 1, 3.5, 1280.0).unwrap();
        assert!(fading.opacity < 0.2);
    }
}
