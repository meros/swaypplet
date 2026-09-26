//! Where each workspace is drawn while the switcher is up.
//!
//! The switcher lays the workspaces you came from out in a row across the
//! output, all at [`SCALE`] and [`ALPHA`]: the selected one in the middle,
//! its neighbours peeking in at the edges, the rest off screen. sway draws
//! them (the `workspace_transform` command, nixos
//! patches/swayfx-ws-transform.patch) and animates every change, so a step
//! is one command per workspace and the movement is the compositor's.
//!
//! No GTK here: this is arithmetic on two rectangles, and the part of the
//! switcher that has to be right to the pixel.
//!
//! One rule shapes it. Scene positions in sway are global, and nothing clips
//! a workspace to its output, so a workspace moved past the edge of this
//! output is drawn on the one next to it. Nothing here goes further than
//! the edge: a workspace leaves the row by sliding to the edge and fading
//! out there, and comes in from the same place.

/// A rectangle in layout coordinates: x, y, width, height.
pub type Rect = (f64, f64, f64, f64);

/// Every workspace in the row, the selected one included.
pub const SCALE: f64 = 0.8;
/// The same for all of them, so the position says which one is selected
/// and nothing else has to.
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

    /// Just past the output's right edge (or left): the whole workspace out
    /// of sight, none of it on the next output yet.
    fn edge(&self, right: bool) -> f64 {
        let m = self.middle();
        if right {
            self.output.0 + self.output.2 - m.0
        } else {
            self.output.0 - (m.0 + m.2)
        }
    }

    /// The workspace `offset` places right of the middle (negative: left).
    /// `shown` false fades it out where it is, which is how the row ends.
    pub fn look(&self, offset: i64, shown: bool) -> Look {
        let (dx, alpha) = match offset {
            -1..=1 => (offset as f64 * self.spacing(), ALPHA),
            o => (self.edge(o > 0), 0.0),
        };
        Look {
            scale: SCALE,
            alpha: if shown { alpha } else { 0.0 },
            dx,
            dy: 0.0,
        }
    }
}

/// `names[0]` is the workspace you are on, and `names[1 + cursor]` the
/// selected one. Every workspace's look with that one in the middle.
pub fn layout(row: &Row, names: &[String], cursor: usize) -> Vec<(String, Look)> {
    let middle = cursor as i64 + 1;
    names
        .iter()
        .enumerate()
        .map(|(j, name)| (name.clone(), row.look(j as i64 - middle, true)))
        .collect()
}

/// Opening: every other workspace waits past the right edge (a hidden
/// workspace given alpha 0 remembers where, and moves from there when it is
/// next shown), then the row with the first step selected. The one you are
/// on moves left, the one you came from slides in to the middle.
pub fn open(row: &Row, names: &[String]) -> Vec<(String, Look)> {
    let wait = row.look(2, false);
    names
        .iter()
        .skip(1)
        .map(|n| (n.clone(), wait))
        .chain(layout(row, names, 0))
        .collect()
}

/// Committing to the selected workspace: what runs before the switch (every
/// other workspace fades out where it is) and after it (the selected one
/// grows to its full size from the middle).
pub fn commit(
    row: &Row,
    names: &[String],
    cursor: usize,
) -> (Vec<(String, Look)>, Option<(String, Look)>) {
    let middle = cursor + 1;
    let before = names
        .iter()
        .enumerate()
        .filter(|(j, _)| *j != middle)
        .map(|(j, n)| (n.clone(), row.look(j as i64 - middle as i64, false)))
        .collect();
    let after = names.get(middle).map(|n| (n.clone(), Look::FULL));
    (before, after)
}

/// Cancelling: the one you are on grows back from wherever the row had it,
/// and the others fade out on their way to where they would be with it in
/// the middle.
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
    fn nothing_is_ever_placed_past_the_output() {
        // Past the edge is the next output, and sway would draw it there.
        let r = row();
        let m = r.middle();
        for offset in -5..=5 {
            let look = r.look(offset, true);
            let x = left_edge(&r, look);
            assert!(x <= 1600.0 && x + m.2 >= 0.0, "offset {offset} at {x}");
            if offset.abs() > 1 {
                assert_eq!(look.alpha, 0.0);
            }
        }
    }

    #[test]
    fn every_workspace_in_view_looks_the_same() {
        let r = row();
        for offset in -1..=1 {
            let look = r.look(offset, true);
            assert_eq!((look.scale, look.alpha), (SCALE, ALPHA));
        }
    }

    #[test]
    fn opening_puts_the_origin_left_and_the_first_step_in_the_middle() {
        let r = row();
        let all = open(&r, &names(4));
        // Staged first: the three others, past the right edge.
        assert!(all[..3].iter().all(|(_, l)| l.alpha == 0.0 && l.dx > 0.0));
        let row: Vec<_> = all[3..].to_vec();
        assert_eq!(row[0], ("1".into(), r.look(-1, true)));
        assert_eq!(row[1], ("2".into(), r.look(0, true)));
        assert_eq!(row[2], ("3".into(), r.look(1, true)));
        assert_eq!(row[3].1.alpha, 0.0);
    }

    #[test]
    fn a_step_moves_everything_one_place_left() {
        let r = row();
        let a = layout(&r, &names(4), 0);
        let b = layout(&r, &names(4), 1);
        assert_eq!(b[2].1, a[1].1, "3 takes the middle 2 had");
        assert_eq!(b[1].1, a[0].1, "2 takes the left place 1 had");
    }

    #[test]
    fn commit_fades_the_rest_and_grows_the_selected_one() {
        let r = row();
        let (before, after) = commit(&r, &names(3), 1);
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
