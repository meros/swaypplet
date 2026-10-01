//! The Displays tab's arithmetic, apart from GTK so every rule is tested:
//! where an output may be dragged to, how the layout is put back against
//! the origin, which modes and scales to offer, and what to send.
//!
//! Positions and sizes are in the compositor's logical pixels, the space
//! `output … position` is written in: a 3840×2160 panel at scale 1.5 is
//! 2560×1440 there, and turned a quarter it is 1440×2560.
//!
//! A layout the tab will send has every enabled output touching another
//! along an edge, none overlapping, and the top-left corner at 0,0. The
//! drag keeps it that way as it goes (`settle`), so Apply meets a gap or an
//! overlap only when the outputs came from the compositor like that.

use crate::services::displays::{HeadPlan, HeadState, Mode, ModeChoice, naming};
use crate::settings::store::OutputTransform;

/// One output as the tab edits it.
#[derive(Debug, Clone, PartialEq)]
pub struct Draft {
    pub name: String,
    /// "Make Model", empty when the output does not say.
    pub product: String,
    /// The automatic name (`displays::naming::auto_name`).
    pub label: String,
    /// What a name the person gave it is stored under
    /// (`displays::naming::key`).
    pub key: String,
    pub enabled: bool,
    pub mode: Option<Mode>,
    pub modes: Vec<Mode>,
    pub position: (i32, i32),
    pub scale: f64,
    pub transform: OutputTransform,
    /// `None` when the compositor does not report it (protocol below v4).
    pub adaptive_sync: Option<bool>,
    /// The scale its pixel density asks for (`displays::dpi_scale`).
    pub suggested: Option<f64>,
}

impl Draft {
    /// What the person reads: the name they gave this screen, else the
    /// automatic one. Main thread only.
    pub fn shown_name(&self) -> String {
        crate::services::devices::display_name(
            Some(&crate::services::devices::DeviceKey::Display(
                self.key.clone(),
            )),
            &self.label,
        )
    }

    pub fn from_head(h: &HeadState) -> Draft {
        let product = [h.make.as_str(), h.model.as_str()]
            .iter()
            .filter(|s| !s.is_empty() && **s != "Unknown")
            .copied()
            .collect::<Vec<_>>()
            .join(" ");
        Draft {
            label: naming::auto_name(&h.name, &h.make, &h.model),
            key: naming::key(&h.name, &h.make, &h.model, &h.serial),
            name: h.name.clone(),
            product,
            enabled: h.enabled,
            mode: h.mode.or_else(|| best_mode(&h.modes)),
            modes: h.modes.clone(),
            position: h.position,
            scale: h.scale,
            transform: h.transform,
            adaptive_sync: h.adaptive_sync,
            suggested: crate::services::displays::dpi_scale(h),
        }
    }

    /// Its rectangle in the layout.
    pub fn rect(&self) -> Rect {
        let (w, h) = logical_size(self.mode, self.scale, self.transform);
        Rect {
            x: self.position.0,
            y: self.position.1,
            w,
            h,
        }
    }
}

/// Every head as a draft, in the view's order.
pub fn drafts(heads: &[HeadState]) -> Vec<Draft> {
    heads.iter().map(Draft::from_head).collect()
}

/// The size an output takes in the layout. wlroots divides the transformed
/// resolution by the scale and truncates (`wlr_output_effective_resolution`),
/// so this does too: a neighbour placed at `x + w` then meets it exactly.
pub fn logical_size(mode: Option<Mode>, scale: f64, t: OutputTransform) -> (i32, i32) {
    let (w, h, _) = mode.unwrap_or((1920, 1080, 0));
    let (w, h) = if quarter_turn(t) { (h, w) } else { (w, h) };
    let s = if scale > 0.0 { scale } else { 1.0 };
    (
        ((f64::from(w) / s) as i32).max(1),
        ((f64::from(h) / s) as i32).max(1),
    )
}

fn quarter_turn(t: OutputTransform) -> bool {
    use OutputTransform as T;
    matches!(t, T::R90 | T::R270 | T::Flipped90 | T::Flipped270)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    fn at(self, (x, y): (i32, i32)) -> Rect {
        Rect { x, y, ..self }
    }

    fn right(self) -> i32 {
        self.x + self.w
    }

    fn bottom(self) -> i32 {
        self.y + self.h
    }

    /// Whether the two share any area (touching edges do not).
    pub fn overlaps(self, o: Rect) -> bool {
        self.x < o.right() && o.x < self.right() && self.y < o.bottom() && o.y < self.bottom()
    }

    /// Whether the two meet along a length of edge (a corner is not enough:
    /// the pointer cannot cross there).
    pub fn touches(self, o: Rect) -> bool {
        let spans = |a0: i32, a1: i32, b0: i32, b1: i32| a0.max(b0) < a1.min(b1);
        ((self.right() == o.x || o.right() == self.x)
            && spans(self.y, self.bottom(), o.y, o.bottom()))
            || ((self.bottom() == o.y || o.bottom() == self.y)
                && spans(self.x, self.right(), o.x, o.right()))
    }
}

/// Whether every rectangle is reached from the first through edges that
/// touch. One rectangle, or none, is connected.
pub fn connected(rects: &[Rect]) -> bool {
    if rects.len() < 2 {
        return true;
    }
    let mut seen = vec![false; rects.len()];
    let mut stack = vec![0];
    seen[0] = true;
    while let Some(i) = stack.pop() {
        for j in 0..rects.len() {
            if !seen[j] && rects[i].touches(rects[j]) {
                seen[j] = true;
                stack.push(j);
            }
        }
    }
    seen.into_iter().all(|s| s)
}

/// The places `r` can stand flush against one side of `o`, sharing some
/// edge, nearest to `want` along that side. The free coordinate snaps to
/// an edge or the centre of any other output (`others`) when it is within
/// `snap`, and only where it still shares an edge with `o`.
fn flush_against(
    r: Rect,
    o: Rect,
    want: (i32, i32),
    snap: i32,
    others: &[Rect],
) -> [(i32, i32); 4] {
    let free = |want: i32, lo: i32, hi: i32, lines: Vec<i32>| {
        let v = want.clamp(lo, hi);
        lines
            .into_iter()
            .filter(|a| (lo..=hi).contains(a) && (a - v).abs() <= snap)
            .min_by_key(|a| (a - v).abs())
            .unwrap_or(v)
    };
    let ys = others
        .iter()
        .flat_map(|o| [o.y, o.bottom() - r.h, o.y + (o.h - r.h) / 2])
        .collect();
    let xs = others
        .iter()
        .flat_map(|o| [o.x, o.right() - r.w, o.x + (o.w - r.w) / 2])
        .collect();
    let y = free(want.1, o.y - r.h + 1, o.bottom() - 1, ys);
    let x = free(want.0, o.x - r.w + 1, o.right() - 1, xs);
    [
        (o.right(), y),
        (o.x - r.w, y),
        (x, o.bottom()),
        (x, o.y - r.h),
    ]
}

/// The candidates for `rects[i]` near `want`: flush against a side of an
/// other rectangle, overlapping none, and, when the layout was connected
/// before the move, leaving it connected.
fn candidates(rects: &[Rect], i: usize, want: (i32, i32), snap: i32) -> Vec<(i32, i32)> {
    let r = rects[i];
    let was_connected = connected(rects);
    let others: Vec<Rect> = rects
        .iter()
        .enumerate()
        .filter(|(j, _)| *j != i)
        .map(|(_, o)| *o)
        .collect();
    let mut out = Vec::new();
    for o in &others {
        for p in flush_against(r, *o, want, snap, &others) {
            let moved = r.at(p);
            if others.iter().any(|o| moved.overlaps(*o)) {
                continue;
            }
            if was_connected {
                let mut next = rects.to_vec();
                next[i] = moved;
                if !connected(&next) {
                    continue;
                }
            }
            out.push(p);
        }
    }
    out
}

fn dist2(a: (i32, i32), b: (i32, i32)) -> i64 {
    let (dx, dy) = (i64::from(a.0 - b.0), i64::from(a.1 - b.1));
    dx * dx + dy * dy
}

/// Where `rects[i]` goes when it is dragged to `want`: the nearest place
/// flush against another output that overlaps nothing and leaves no one
/// stranded, with its free edge snapped to the others' edges within
/// `snap`. Alone, it goes wherever it is put. With nowhere to go, it stays.
pub fn settle(rects: &[Rect], i: usize, want: (i32, i32), snap: i32) -> (i32, i32) {
    if rects.len() < 2 {
        return want;
    }
    candidates(rects, i, want, snap)
        .into_iter()
        .min_by_key(|p| dist2(*p, want))
        .unwrap_or((rects[i].x, rects[i].y))
}

/// Where `rects[i]` goes on an arrow key: `step` along `dir`, or, when that
/// would leave it floating or overlapping, the nearest allowed place that
/// is still a move that way. It walks round a corner instead of stopping.
pub fn nudge(rects: &[Rect], i: usize, dir: (i32, i32), step: i32) -> (i32, i32) {
    let from = (rects[i].x, rects[i].y);
    let want = (from.0 + dir.0 * step, from.1 + dir.1 * step);
    if rects.len() < 2 {
        return want;
    }
    candidates(rects, i, want, 0)
        .into_iter()
        .filter(|p| (p.0 - from.0) * dir.0 + (p.1 - from.1) * dir.1 > 0)
        .min_by_key(|p| dist2(*p, want))
        .unwrap_or(from)
}

/// Move the enabled outputs together so the top-left one is at 0,0, as
/// sway numbers them. Disabled ones keep their numbers; they are not sent.
pub fn normalize(drafts: &mut [Draft]) {
    let on = || drafts.iter().filter(|d| d.enabled);
    let (Some(x), Some(y)) = (
        on().map(|d| d.position.0).min(),
        on().map(|d| d.position.1).min(),
    ) else {
        return;
    };
    for d in drafts.iter_mut().filter(|d| d.enabled) {
        d.position = (d.position.0 - x, d.position.1 - y);
    }
}

/// Keep the neighbours of `drafts[i]` against it after its size changed
/// from `old` (a new mode, scale or rotation): every enabled output wholly
/// to its right moves by the change in width, every one wholly below by the
/// change in height. A row or a column stays a row or a column.
pub fn follow_resize(drafts: &mut [Draft], i: usize, old: Rect) {
    let new = drafts[i].rect();
    let (dw, dh) = (new.w - old.w, new.h - old.h);
    for (j, d) in drafts.iter_mut().enumerate() {
        if j == i || !d.enabled {
            continue;
        }
        if d.position.0 >= old.right() {
            d.position.0 += dw;
        }
        if d.position.1 >= old.bottom() {
            d.position.1 += dh;
        }
    }
    normalize(drafts);
}

/// Close the gaps across the layout: where no enabled output covers a band
/// of x (or y), everything past the band moves back across it. What an
/// output switched off leaves behind in a row or a column.
pub fn close_gaps(drafts: &mut [Draft]) {
    for axis in [0, 1] {
        let span = |d: &Draft| {
            let r = d.rect();
            if axis == 0 {
                (r.x, r.right())
            } else {
                (r.y, r.bottom())
            }
        };
        let mut spans: Vec<(i32, i32)> = drafts.iter().filter(|d| d.enabled).map(span).collect();
        spans.sort_unstable();
        let mut shifts = Vec::new();
        let mut reach = match spans.first() {
            Some(s) => s.1,
            None => return,
        };
        for (a, b) in spans.into_iter().skip(1) {
            if a > reach {
                shifts.push((reach, a - reach));
            }
            reach = reach.max(b);
        }
        for d in drafts.iter_mut().filter(|d| d.enabled) {
            let at = if axis == 0 {
                &mut d.position.0
            } else {
                &mut d.position.1
            };
            let by: i32 = shifts
                .iter()
                .filter(|(edge, _)| *at >= *edge)
                .map(|(_, w)| w)
                .sum();
            *at -= by;
        }
    }
    normalize(drafts);
}

/// Put an output being switched on at the right of the others, top-aligned.
pub fn place_new(drafts: &mut [Draft], i: usize) {
    let right = drafts
        .iter()
        .enumerate()
        .filter(|(j, d)| *j != i && d.enabled)
        .map(|(_, d)| d.rect().right())
        .max()
        .unwrap_or(0);
    let top = drafts
        .iter()
        .enumerate()
        .filter(|(j, d)| *j != i && d.enabled)
        .map(|(_, d)| d.position.1)
        .min()
        .unwrap_or(0);
    drafts[i].position = (right, top);
}

/// What is wrong with a layout, if anything, in words for the status line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    NoneEnabled,
    Overlap(String, String),
    Apart(String),
}

impl Problem {
    pub fn say(&self) -> String {
        match self {
            Problem::NoneEnabled => "At least one display must stay on.".into(),
            Problem::Overlap(a, b) => format!("{a} and {b} overlap. Drag one clear of the other."),
            Problem::Apart(a) => format!("{a} does not touch the others. Drag it against one."),
        }
    }
}

pub fn check(drafts: &[Draft]) -> Result<(), Problem> {
    let on: Vec<&Draft> = drafts.iter().filter(|d| d.enabled).collect();
    if on.is_empty() {
        return Err(Problem::NoneEnabled);
    }
    for (a, da) in on.iter().enumerate() {
        for db in &on[a + 1..] {
            if da.rect().overlaps(db.rect()) {
                return Err(Problem::Overlap(da.name.clone(), db.name.clone()));
            }
        }
    }
    let rects: Vec<Rect> = on.iter().map(|d| d.rect()).collect();
    if !connected(&rects) {
        // Name one that the first cannot reach.
        let mut reach = vec![false; rects.len()];
        reach[0] = true;
        let mut grew = true;
        while grew {
            grew = false;
            for i in 0..rects.len() {
                for j in 0..rects.len() {
                    if reach[i] && !reach[j] && rects[i].touches(rects[j]) {
                        reach[j] = true;
                        grew = true;
                    }
                }
            }
        }
        let far = reach.iter().position(|r| !r).unwrap_or(0);
        return Err(Problem::Apart(on[far].name.clone()));
    }
    Ok(())
}

/// The configuration for `drafts`, one plan per draft in order. A mode the
/// output lists goes as that mode; anything else as a custom mode.
pub fn plans(drafts: &[Draft]) -> Vec<HeadPlan> {
    drafts
        .iter()
        .map(|d| {
            if !d.enabled {
                return HeadPlan::Disable;
            }
            HeadPlan::Enable {
                mode: d.mode.map(|m| {
                    if d.modes.contains(&m) {
                        ModeChoice::Listed(m)
                    } else {
                        ModeChoice::Custom(m)
                    }
                }),
                position: Some(d.position),
                scale: Some(d.scale),
                transform: Some(d.transform),
                adaptive_sync: d.adaptive_sync,
            }
        })
        .collect()
}

// ── Modes ────────────────────────────────────────────────────────────────

/// The mode to show when an output is switched on without one: the
/// largest, at its fastest refresh.
pub fn best_mode(modes: &[Mode]) -> Option<Mode> {
    modes
        .iter()
        .copied()
        .max_by_key(|&(w, h, r)| (u64::from(w) * u64::from(h), r))
}

/// The resolutions an output offers, largest first, each once. `current`
/// joins them when the output does not list it (a custom mode, or a
/// headless output that lists none).
pub fn resolutions(modes: &[Mode], current: Option<Mode>) -> Vec<(u32, u32)> {
    let mut out: Vec<(u32, u32)> = modes
        .iter()
        .chain(current.iter())
        .map(|&(w, h, _)| (w, h))
        .collect();
    out.sort_unstable_by_key(|&(w, h)| std::cmp::Reverse((u64::from(w) * u64::from(h), w)));
    out.dedup();
    out
}

/// The refresh rates at `size`, fastest first, each once.
pub fn refreshes(modes: &[Mode], current: Option<Mode>, size: (u32, u32)) -> Vec<u32> {
    let mut out: Vec<u32> = modes
        .iter()
        .chain(current.iter())
        .filter(|&&(w, h, _)| (w, h) == size)
        .map(|&(_, _, r)| r)
        .collect();
    out.sort_unstable_by(|a, b| b.cmp(a));
    out.dedup();
    out
}

/// The mode for a new resolution: the refresh it had when that size offers
/// it (within 1.5 Hz), else the fastest there is.
pub fn mode_at(modes: &[Mode], current: Option<Mode>, size: (u32, u32), had: u32) -> Mode {
    let rates = refreshes(modes, current, size);
    let near = rates
        .iter()
        .copied()
        .min_by_key(|r| r.abs_diff(had))
        .filter(|r| r.abs_diff(had) <= 1500);
    (
        size.0,
        size.1,
        near.or(rates.first().copied()).unwrap_or(had),
    )
}

pub fn resolution_label((w, h): (u32, u32)) -> String {
    format!("{w} × {h}")
}

/// "60 Hz", "59.94 Hz", "Unknown" for a headless output's 0.
pub fn refresh_label(mhz: u32) -> String {
    if mhz == 0 {
        return "Unknown".into();
    }
    if mhz.is_multiple_of(1000) || mhz.abs_diff((mhz + 500) / 1000 * 1000) <= 10 {
        format!("{} Hz", (mhz + 500) / 1000)
    } else {
        format!("{:.2} Hz", f64::from(mhz) / 1000.0)
    }
}

// ── Scales ───────────────────────────────────────────────────────────────

/// The scales to offer: quarter steps from 1 to 3, plus the current one and
/// the suggested one when they are off that ladder, in order.
pub fn scales(current: f64, suggested: Option<f64>) -> Vec<f64> {
    let mut out: Vec<f64> = (4..=12).map(|q| f64::from(q) / 4.0).collect();
    for extra in [Some(current), suggested].into_iter().flatten() {
        if extra > 0.0 && !out.iter().any(|s| (s - extra).abs() < 1e-3) {
            out.push(extra);
        }
    }
    out.sort_by(f64::total_cmp);
    out
}

/// "1.5×", and "1.5× (suggested)" for the one the density asks for.
pub fn scale_label(scale: f64, suggested: Option<f64>) -> String {
    let s = format!("{scale:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if suggested.is_some_and(|g| (g - scale).abs() < 1e-3) {
        format!("{s}× (suggested)")
    } else {
        format!("{s}×")
    }
}

// ── Rotation ─────────────────────────────────────────────────────────────

/// Every transform, in sway's spelling, with its name in the dropdown.
pub const TRANSFORMS: [(OutputTransform, &str); 8] = [
    (OutputTransform::Normal, "Normal"),
    (OutputTransform::R90, "90° clockwise"),
    (OutputTransform::R180, "180°"),
    (OutputTransform::R270, "90° anticlockwise"),
    (OutputTransform::Flipped, "Flipped"),
    (OutputTransform::Flipped90, "Flipped, 90°"),
    (OutputTransform::Flipped180, "Flipped, 180°"),
    (OutputTransform::Flipped270, "Flipped, 270°"),
];

// ── The canvas ───────────────────────────────────────────────────────────

/// How the layout maps onto the canvas: logical pixel `p` is drawn at
/// `offset + (p - origin) * scale`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fit {
    pub scale: f64,
    pub origin: (i32, i32),
    pub offset: (f64, f64),
}

impl Fit {
    /// The enabled rectangles, fitted into `w`×`h` with `margin` all round
    /// and centred. The layout's box is padded by half an output each way,
    /// so a drag has room to show where it is going.
    pub fn new(rects: &[Rect], w: f64, h: f64, margin: f64) -> Fit {
        let min_x = rects.iter().map(|r| r.x).min().unwrap_or(0);
        let min_y = rects.iter().map(|r| r.y).min().unwrap_or(0);
        let max_x = rects.iter().map(|r| r.right()).max().unwrap_or(1920);
        let max_y = rects.iter().map(|r| r.bottom()).max().unwrap_or(1080);
        let (bw, bh) = (
            f64::from(max_x - min_x).max(1.0),
            f64::from(max_y - min_y).max(1.0),
        );
        let room_w = (w - 2.0 * margin).max(1.0);
        let room_h = (h - 2.0 * margin).max(1.0);
        let scale = (room_w / (bw * 1.25)).min(room_h / (bh * 1.25));
        Fit {
            scale,
            origin: (min_x, min_y),
            offset: ((w - bw * scale) / 2.0, (h - bh * scale) / 2.0),
        }
    }

    pub fn to_canvas(self, (x, y): (i32, i32)) -> (f64, f64) {
        (
            self.offset.0 + f64::from(x - self.origin.0) * self.scale,
            self.offset.1 + f64::from(y - self.origin.1) * self.scale,
        )
    }

    /// A canvas distance in logical pixels.
    pub fn to_layout(self, d: f64) -> i32 {
        (d / self.scale).round() as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }

    fn draft(name: &str, mode: Mode, scale: f64, pos: (i32, i32)) -> Draft {
        Draft {
            name: name.into(),
            product: String::new(),
            label: name.into(),
            key: name.into(),
            enabled: true,
            mode: Some(mode),
            modes: vec![mode],
            position: pos,
            scale,
            transform: OutputTransform::Normal,
            adaptive_sync: None,
            suggested: None,
        }
    }

    #[test]
    fn the_logical_size_divides_by_the_scale_and_turns_with_the_output() {
        let m = Some((3840, 2160, 60000));
        assert_eq!(logical_size(m, 1.5, OutputTransform::Normal), (2560, 1440));
        assert_eq!(logical_size(m, 1.5, OutputTransform::R90), (1440, 2560));
        assert_eq!(
            logical_size(m, 1.0, OutputTransform::Flipped270),
            (2160, 3840)
        );
        // Truncated, as wlroots does: 1000 / 1.5 is 666.67.
        assert_eq!(
            logical_size(Some((1000, 900, 0)), 1.5, OutputTransform::Normal),
            (666, 600)
        );
    }

    #[test]
    fn touching_needs_a_length_of_edge_and_overlap_needs_area() {
        let a = r(0, 0, 100, 100);
        assert!(a.touches(r(100, 50, 10, 10)));
        assert!(a.touches(r(20, -10, 10, 10)));
        assert!(!a.touches(r(100, 100, 10, 10)), "a corner is not an edge");
        assert!(!a.touches(r(101, 0, 10, 10)));
        assert!(!a.overlaps(r(100, 0, 10, 10)));
        assert!(a.overlaps(r(99, 0, 10, 10)));
        assert!(connected(&[a, r(100, 0, 50, 50), r(150, 0, 5, 5)]));
        assert!(!connected(&[a, r(100, 0, 50, 50), r(151, 0, 5, 5)]));
    }

    #[test]
    fn a_drag_lands_flush_against_the_nearest_side() {
        let rects = [r(0, 0, 1920, 1080), r(1920, 0, 1280, 800)];
        // Dropped a little into the first: it goes back to its edge.
        assert_eq!(settle(&rects, 1, (1800, 300), 0), (1920, 300));
        // Dropped far off to the left: flush with the first's left side.
        assert_eq!(settle(&rects, 1, (-3000, 100), 0), (-1280, 100));
        // Dropped below: flush with the bottom.
        assert_eq!(settle(&rects, 1, (400, 1500), 0), (400, 1080));
        // Never so far along an edge that only a corner meets.
        let p = settle(&rects, 1, (1920, 5000), 0);
        assert!(rects[1].at(p).touches(rects[0]), "{p:?}");
    }

    #[test]
    fn a_drag_snaps_its_free_edge_to_the_others() {
        let rects = [r(0, 0, 1920, 1080), r(1920, 0, 1280, 800)];
        // Tops within the snap: aligned.
        assert_eq!(settle(&rects, 1, (1920, 30), 40), (1920, 0));
        // Bottoms within it: aligned (1080 - 800).
        assert_eq!(settle(&rects, 1, (1920, 260), 40), (1920, 280));
        // Centres: (1080 - 800) / 2.
        assert_eq!(settle(&rects, 1, (1920, 150), 40), (1920, 140));
        // Outside the snap: where it was put.
        assert_eq!(settle(&rects, 1, (1920, 70), 40), (1920, 70));
    }

    #[test]
    fn a_drag_never_overlaps_and_never_strands_a_third() {
        // A row: a | b | c. Dragging b away would leave c alone.
        let rects = [r(0, 0, 100, 100), r(100, 0, 100, 100), r(200, 0, 100, 100)];
        let p = settle(&rects, 1, (100, 400), 0);
        let mut next = rects;
        next[1] = next[1].at(p);
        assert!(connected(&next), "{p:?}");
        assert!(!next[1].overlaps(next[0]) && !next[1].overlaps(next[2]));
        // c can move: round to below b.
        assert_eq!(settle(&rects, 2, (150, 300), 0), (150, 100));
    }

    #[test]
    fn a_lone_output_goes_where_it_is_put() {
        assert_eq!(settle(&[r(0, 0, 10, 10)], 0, (50, 60), 5), (50, 60));
    }

    #[test]
    fn arrow_keys_slide_along_an_edge_and_round_a_corner() {
        let rects = [r(0, 0, 1920, 1080), r(1920, 0, 1280, 800)];
        assert_eq!(nudge(&rects, 1, (0, 1), 10), (1920, 10));
        assert_eq!(nudge(&rects, 1, (0, -1), 10), (1920, -10));
        // Right would float it off the first: nowhere to go that way.
        assert_eq!(nudge(&rects, 1, (1, 0), 10), (1920, 0));
        // Left would overlap: it goes over the top instead, still moving left.
        let p = nudge(&rects, 1, (-1, 0), 10);
        assert!(p.0 < 1920, "{p:?}");
        let mut next = rects;
        next[1] = next[1].at(p);
        assert!(!next[1].overlaps(next[0]) && next[1].touches(next[0]));
    }

    #[test]
    fn the_layout_is_put_back_against_the_origin() {
        let mut d = vec![
            draft("a", (1920, 1080, 60000), 1.0, (-1920, 200)),
            draft("b", (1920, 1080, 60000), 1.0, (0, 100)),
        ];
        d.push(Draft {
            enabled: false,
            position: (-9999, -9999),
            ..draft("c", (10, 10, 0), 1.0, (0, 0))
        });
        normalize(&mut d);
        assert_eq!(d[0].position, (0, 100));
        assert_eq!(d[1].position, (1920, 0));
        assert_eq!(
            d[2].position,
            (-9999, -9999),
            "a disabled output is not moved"
        );
    }

    #[test]
    fn a_resize_carries_the_outputs_to_its_right_and_below() {
        let mut d = vec![
            draft("a", (3840, 2160, 60000), 1.0, (0, 0)),
            draft("b", (1920, 1080, 60000), 1.0, (3840, 0)),
            draft("c", (1920, 1080, 60000), 1.0, (0, 2160)),
        ];
        let old = d[0].rect();
        d[0].scale = 2.0;
        follow_resize(&mut d, 0, old);
        assert_eq!(d[1].position, (1920, 0));
        assert_eq!(d[2].position, (0, 1080));
        assert_eq!(check(&d), Ok(()));
    }

    #[test]
    fn switching_one_off_closes_the_gap_it_leaves() {
        let mut d = vec![
            draft("a", (100, 100, 0), 1.0, (0, 0)),
            draft("b", (100, 100, 0), 1.0, (100, 0)),
            draft("c", (100, 100, 0), 1.0, (200, 0)),
        ];
        d[1].enabled = false;
        assert_eq!(check(&d), Err(Problem::Apart("c".into())));
        close_gaps(&mut d);
        assert_eq!(d[2].position, (100, 0));
        assert_eq!(check(&d), Ok(()));
        d[1].enabled = true;
        place_new(&mut d, 1);
        assert_eq!(d[1].position, (200, 0));
        assert_eq!(check(&d), Ok(()));
    }

    #[test]
    fn check_names_what_is_wrong() {
        let d = vec![
            draft("a", (100, 100, 0), 1.0, (0, 0)),
            draft("b", (100, 100, 0), 1.0, (50, 0)),
        ];
        assert_eq!(check(&d), Err(Problem::Overlap("a".into(), "b".into())));
        let mut off = d.clone();
        off.iter_mut().for_each(|d| d.enabled = false);
        assert_eq!(check(&off), Err(Problem::NoneEnabled));
    }

    #[test]
    fn plans_send_listed_modes_as_listed_and_the_rest_as_custom() {
        let mut d = vec![draft("a", (1920, 1080, 60000), 1.0, (0, 0))];
        d[0].adaptive_sync = Some(true);
        let HeadPlan::Enable {
            mode,
            adaptive_sync,
            ..
        } = plans(&d)[0].clone()
        else {
            panic!()
        };
        assert_eq!(mode, Some(ModeChoice::Listed((1920, 1080, 60000))));
        assert_eq!(adaptive_sync, Some(true));
        d[0].modes.clear();
        let HeadPlan::Enable { mode, .. } = plans(&d)[0].clone() else {
            panic!()
        };
        assert_eq!(mode, Some(ModeChoice::Custom((1920, 1080, 60000))));
        d[0].enabled = false;
        assert_eq!(plans(&d)[0], HeadPlan::Disable);
    }

    #[test]
    fn modes_are_offered_largest_and_fastest_first() {
        let modes = [
            (1920, 1080, 60000),
            (3840, 2160, 30000),
            (3840, 2160, 59997),
            (1920, 1080, 144000),
        ];
        assert_eq!(resolutions(&modes, None), vec![(3840, 2160), (1920, 1080)]);
        assert_eq!(refreshes(&modes, None, (1920, 1080)), vec![144000, 60000]);
        assert_eq!(best_mode(&modes), Some((3840, 2160, 59997)));
        // A new size keeps the refresh it had when it can.
        assert_eq!(
            mode_at(&modes, None, (3840, 2160), 60000),
            (3840, 2160, 59997)
        );
        assert_eq!(
            mode_at(&modes, None, (1920, 1080), 30000),
            (1920, 1080, 144000)
        );
        // A headless output lists nothing: its own mode is the one choice.
        assert_eq!(resolutions(&[], Some((1280, 720, 0))), vec![(1280, 720)]);
        assert_eq!(refresh_label(59997), "60 Hz");
        assert_eq!(refresh_label(59940), "59.94 Hz");
        assert_eq!(refresh_label(0), "Unknown");
    }

    #[test]
    fn scales_are_quarter_steps_plus_what_is_in_use_and_suggested() {
        let s = scales(1.6, Some(1.75));
        assert_eq!(s.len(), 10);
        assert!(s.contains(&1.6) && s.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(scale_label(1.5, None), "1.5×");
        assert_eq!(scale_label(2.0, Some(2.0)), "2× (suggested)");
        assert_eq!(scale_label(1.25, Some(2.0)), "1.25×");
    }

    #[test]
    fn the_canvas_fits_the_layout_and_centres_it() {
        let rects = [r(0, 0, 1920, 1080), r(1920, 0, 1920, 1080)];
        let f = Fit::new(&rects, 500.0, 200.0, 10.0);
        let (x0, _) = f.to_canvas((0, 0));
        let (x1, _) = f.to_canvas((3840, 1080));
        assert!((x0 + x1 - 500.0).abs() < 1e-6, "centred");
        assert!(x1 - x0 <= 480.0);
        assert_eq!(f.to_layout(f.scale * 100.0), 100);
    }
}
