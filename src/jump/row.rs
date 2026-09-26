//! Where each workspace is drawn while the switcher is up.
//!
//! The switcher lays the workspaces you came from out on a strip across the
//! output, all at [`SCALE`]: the selected one in the middle and opaque, its
//! neighbours peeking in at the edges at [`ALPHA`], the rest further along
//! the strip past the edges. sway draws them (the `workspace_transform`
//! command, nixos patches/swayfx-ws-transform.patch) and animates every
//! change, so a step is one command per workspace and the movement is the
//! compositor's.
//!
//! A step moves every workspace by the same distance in the same time, so
//! the strip moves as one piece, the gaps never change and a workspace
//! comes into view from exactly where it was waiting. That needs the ones
//! past the edge to be drawn there, and they would be drawn on the output
//! next to this one: scene positions are global. sway draws a transformed
//! workspace on its own output only, cut at its edge
//! (patches/scenefx-only-output.patch).
//!
//! No GTK here: this is arithmetic on two rectangles.

/// A rectangle in layout coordinates: x, y, width, height.
pub type Rect = (f64, f64, f64, f64);

/// Every workspace on the strip, the selected one included.
pub const SCALE: f64 = 0.8;
/// Every workspace but the selected one, which is opaque.
pub const ALPHA: f64 = 0.85;

/// How sway draws one workspace: scaled about the output's centre, faded,
/// then moved.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub scale: f64,
    pub alpha: f64,
    pub dx: f64,
    pub dy: f64,
}

impl Look {
    /// The workspace as it is when no switcher is up.
    pub const FULL: Look = Look {
        scale: 1.0,
        alpha: 1.0,
        dx: 0.0,
        dy: 0.0,
    };

    pub fn command(&self, workspace: &str) -> String {
        format!(
            "workspace_transform \"{}\" {} {} {} {}",
            workspace.replace('\\', "\\\\").replace('"', "\\\""),
            self.scale,
            self.alpha,
            self.dx.round(),
            self.dy.round()
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Row {
    /// The output the row is on.
    output: Rect,
    /// The part of it a workspace fills: sway's workspace rect.
    area: Rect,
}

impl Row {
    pub fn new(output: Rect, area: Rect) -> Row {
        Row { output, area }
    }

    fn centre(&self) -> (f64, f64) {
        (
            self.output.0 + self.output.2 / 2.0,
            self.output.1 + self.output.3 / 2.0,
        )
    }

    /// The selected workspace's box: its area, scaled about the output's
    /// centre as sway scales it.
    pub fn middle(&self) -> Rect {
        let (cx, cy) = self.centre();
        (
            cx + (self.area.0 - cx) * SCALE,
            cy + (self.area.1 - cy) * SCALE,
            self.area.2 * SCALE,
            self.area.3 * SCALE,
        )
    }

    /// From one workspace's middle to the next. The gap between two of them
    /// is a third of the room beside the middle one, so what shows of a
    /// neighbour is the other two thirds.
    fn spacing(&self) -> f64 {
        let width = self.area.2 * SCALE;
        let side = ((self.output.2 - width) / 2.0).max(0.0);
        width + (side / 3.0).max(12.0)
    }

    /// The workspace `offset` places right of the middle (negative: left).
    /// `shown` false fades it out where it is, which is how the strip ends.
    pub fn look(&self, offset: i64, shown: bool) -> Look {
        let alpha = if offset == 0 { 1.0 } else { ALPHA };
        Look {
            scale: SCALE,
            alpha: if shown { alpha } else { 0.0 },
            dx: offset as f64 * self.spacing(),
            dy: 0.0,
        }
    }
}

/// `names[0]` is the workspace you are on, and `names[selected]` the one
/// in the middle. Every workspace's look.
pub fn layout(row: &Row, names: &[String], selected: usize) -> Vec<(String, Look)> {
    names
        .iter()
        .enumerate()
        .map(|(j, name)| (name.clone(), row.look(j as i64 - selected as i64, true)))
        .collect()
}

/// Opening: every other workspace waits where the strip would have it with
/// the one you are on in the middle (a hidden workspace given alpha 0
/// remembers where, and moves from there when it is next shown), then the
/// whole strip moves one place left.
pub fn open(row: &Row, names: &[String]) -> Vec<(String, Look)> {
    names
        .iter()
        .enumerate()
        .skip(1)
        .map(|(j, n)| (n.clone(), row.look(j as i64, false)))
        .chain(layout(row, names, 1))
        .collect()
}

/// Committing to `names[selected]`: what runs before the switch (every
/// other workspace fades out where it is) and after it (the selected one
/// grows to its full size from the middle).
pub fn commit(
    row: &Row,
    names: &[String],
    selected: usize,
) -> (Vec<(String, Look)>, Option<(String, Look)>) {
    let before = names
        .iter()
        .enumerate()
        .filter(|(j, _)| *j != selected)
        .map(|(j, n)| (n.clone(), row.look(j as i64 - selected as i64, false)))
        .collect();
    let after = names.get(selected).map(|n| (n.clone(), Look::FULL));
    (before, after)
}

/// Cancelling: the one you are on grows back from wherever the strip had
/// it, and the others move with the strip as it brings it to the middle,
/// fading out on the way.
pub fn cancel(row: &Row, names: &[String]) -> Vec<(String, Look)> {
    names
        .iter()
        .enumerate()
        .map(|(j, n)| {
            let look = if j == 0 {
                Look::FULL
            } else {
                row.look(j as i64, false)
            };
            (n.clone(), look)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 1600x1000 output with a 30 px bar on top.
    fn row() -> Row {
        Row::new((0.0, 0.0, 1600.0, 1000.0), (0.0, 30.0, 1600.0, 970.0))
    }

    fn names(n: usize) -> Vec<String> {
        (1..=n).map(|i| i.to_string()).collect()
    }

    fn left_edge(row: &Row, look: Look) -> f64 {
        row.middle().0 + look.dx
    }

    #[test]
    fn the_middle_is_the_area_scaled_about_the_output_centre() {
        let m = row().middle();
        assert_eq!(m, (160.0, 124.0, 1280.0, 776.0));
    }

    #[test]
    fn neighbours_peek_in_and_do_not_overlap_the_middle() {
        let r = row();
        let m = r.middle();
        let right = r.look(1, true);
        let left = r.look(-1, true);
        // Two thirds of the 160 px beside the middle one show.
        assert!((left_edge(&r, right) - 1493.33).abs() < 0.01);
        assert!(left_edge(&r, right) > m.0 + m.2);
        assert!(left_edge(&r, left) + m.2 < m.0);
        assert!(left_edge(&r, left) + m.2 > 0.0, "the left one shows");
    }

    #[test]
    fn the_strip_has_one_spacing_everywhere() {
        // A step moves every workspace by the same distance, so the gaps
        // stay the same all the way through the animation.
        let r = row();
        let step = r.look(1, true).dx - r.look(0, true).dx;
        for offset in -6..6 {
            let d = r.look(offset + 1, true).dx - r.look(offset, true).dx;
            assert!(
                (d - step).abs() < 1e-9,
                "offset {offset}: {d} against {step}"
            );
        }
    }

    #[test]
    fn the_middle_is_opaque_and_the_rest_the_same() {
        let r = row();
        assert_eq!(r.look(0, true).alpha, 1.0);
        for offset in [-3, -1, 1, 4] {
            let look = r.look(offset, true);
            assert_eq!((look.scale, look.alpha), (SCALE, ALPHA));
        }
        assert_eq!(r.look(0, true).scale, SCALE);
    }

    #[test]
    fn opening_moves_the_whole_strip_one_place_left() {
        let r = row();
        let all = open(&r, &names(4));
        // Staged first: the three others, where the strip has them with the
        // origin in the middle.
        for (j, (name, look)) in all[..3].iter().enumerate() {
            assert_eq!(name, &(j + 2).to_string());
            assert_eq!(look.alpha, 0.0);
            assert_eq!(look.dx, r.look(j as i64 + 1, true).dx);
        }
        let row: Vec<_> = all[3..].to_vec();
        assert_eq!(row[0], ("1".into(), r.look(-1, true)));
        assert_eq!(row[1], ("2".into(), r.look(0, true)));
        assert_eq!(row[2], ("3".into(), r.look(1, true)));
        assert_eq!(row[3], ("4".into(), r.look(2, true)));
    }

    #[test]
    fn a_step_moves_everything_one_place_left() {
        let r = row();
        let a = layout(&r, &names(4), 1);
        let b = layout(&r, &names(4), 2);
        assert_eq!(b[2].1, a[1].1, "3 takes the middle 2 had");
        assert_eq!(b[1].1, a[0].1, "2 takes the left place 1 had");
    }

    #[test]
    fn the_origin_can_be_the_middle_again() {
        let r = row();
        let all = layout(&r, &names(3), 0);
        assert_eq!(all[0].1, r.look(0, true));
        assert_eq!(all[0].1.alpha, 1.0);
    }

    #[test]
    fn commit_fades_the_rest_and_grows_the_selected_one() {
        let r = row();
        let (before, after) = commit(&r, &names(3), 2);
        assert_eq!(after, Some(("3".into(), Look::FULL)));
        assert_eq!(before.len(), 2);
        assert!(before.iter().all(|(_, l)| l.alpha == 0.0));
        // Fading where they are: 2 stays in its left place.
        assert_eq!(before[1].1.dx, r.look(-1, true).dx);
    }

    #[test]
    fn cancel_brings_the_origin_back_and_nothing_else() {
        let all = cancel(&row(), &names(3));
        assert_eq!(all[0], ("1".into(), Look::FULL));
        assert!(all[1..].iter().all(|(_, l)| l.alpha == 0.0));
    }

    #[test]
    fn names_are_quoted_for_sway() {
        let cmd = Look::FULL.command("9:t\"3");
        assert_eq!(cmd, "workspace_transform \"9:t\\\"3\" 1 1 0 0");
    }
}
