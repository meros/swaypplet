//! The Day and night group's map: the land as dots, the place the sun is
//! computed for as a ring, and a click to pick another.
//!
//! The land is `data/land-mask.txt`, Natural Earth's 110 m land (public
//! domain) in 2° cells, made by `dev/land-mask.py`: no image, no network.
//! A place is only as precise as a click, which is plenty: a degree of
//! longitude moves a sunrise by four minutes.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::OnceLock;

use gtk4::prelude::*;

/// The mask's rows, north to south, `#` for land.
const MASK: &str = include_str!("../../data/land-mask.txt");

/// The latitudes the mask spans, and its cell: the inhabited band, so the
/// width is spent on land people live on rather than on the poles.
const NORTH: f64 = 75.0;
const SOUTH: f64 = -57.0;
const CELL: f64 = 2.0;

/// The centre of every land cell, as (lat, lon), read once.
fn land() -> &'static [(f64, f64)] {
    static LAND: OnceLock<Vec<(f64, f64)>> = OnceLock::new();
    LAND.get_or_init(|| {
        MASK.lines()
            .enumerate()
            .flat_map(|(row, line)| {
                let lat = NORTH - CELL / 2.0 - row as f64 * CELL;
                line.char_indices()
                    .filter(|(_, c)| *c == '#')
                    .map(move |(col, _)| (lat, -180.0 + CELL / 2.0 + col as f64 * CELL))
            })
            .collect()
    })
}

pub struct LocationMap {
    /// Holds the map at the mask's own proportions, so a dot is round and
    /// a click lands where it looks.
    frame: gtk4::AspectFrame,
    area: gtk4::DrawingArea,
    /// The place the sun is computed for, drawn as the ring.
    here: Rc<Cell<Option<(f64, f64)>>>,
    /// The place under the pointer, for the caption.
    hover: Rc<Cell<Option<(f64, f64)>>>,
    caption: gtk4::Label,
}

/// Where `lat`/`lon` lands in a `w`×`h` map.
fn project(lat: f64, lon: f64, w: f64, h: f64) -> (f64, f64) {
    let x = (lon + 180.0) / 360.0 * w;
    let y = (NORTH - lat) / (NORTH - SOUTH) * h;
    (x, y)
}

/// The place at `x`/`y` in a `w`×`h` map.
fn unproject(x: f64, y: f64, w: f64, h: f64) -> (f64, f64) {
    let lon = x / w * 360.0 - 180.0;
    let lat = NORTH - y / h * (NORTH - SOUTH);
    (lat.clamp(-90.0, 90.0), lon.clamp(-180.0, 180.0))
}

/// A picked place, to a tenth of a degree: what the file keeps.
fn rounded(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

/// A place as a person reads it: `59.3° N, 18.1° E`.
pub fn describe(lat: f64, lon: f64) -> String {
    let ns = if lat < 0.0 { 'S' } else { 'N' };
    let ew = if lon < 0.0 { 'W' } else { 'E' };
    format!("{:.1}° {ns}, {:.1}° {ew}", lat.abs(), lon.abs())
}

impl LocationMap {
    /// The map; `on_pick` gets the place a click chose.
    pub fn new(on_pick: impl Fn(f64, f64) + 'static) -> Rc<LocationMap> {
        let area = gtk4::DrawingArea::new();
        area.set_hexpand(true);
        area.set_content_height(200);
        let ratio = (360.0 / (NORTH - SOUTH)) as f32;
        let frame = gtk4::AspectFrame::new(0.5, 0.5, ratio, false);
        frame.set_hexpand(true);
        frame.set_child(Some(&area));
        area.set_cursor_from_name(Some("crosshair"));
        area.set_tooltip_text(Some(
            "Click where you are. Within a degree or two is plenty.",
        ));
        let caption = super::form::value_label();
        caption.set_xalign(0.0);

        let map = Rc::new(LocationMap {
            frame,
            area: area.clone(),
            here: Rc::new(Cell::new(None)),
            hover: Rc::new(Cell::new(None)),
            caption,
        });

        {
            let here = map.here.clone();
            area.set_draw_func(move |_, cr, w, h| {
                let (w, h) = (f64::from(w), f64::from(h));
                let paint = crate::theme::paint();
                let fg = paint.fg;
                // The equator and the tropics, barely there.
                cr.set_source_rgba(fg.0, fg.1, fg.2, 0.10);
                cr.set_line_width(1.0);
                for lat in [-23.4, 0.0, 23.4] {
                    let (_, y) = project(lat, 0.0, w, h);
                    cr.move_to(0.0, y.round() + 0.5);
                    cr.line_to(w, y.round() + 0.5);
                }
                let _ = cr.stroke();
                // The land, one dot per 2° cell, sized to the cell.
                let cell = (w / (360.0 / CELL)).min(h / ((NORTH - SOUTH) / CELL));
                let r = (cell * 0.34).clamp(0.7, 2.6);
                cr.set_source_rgba(fg.0, fg.1, fg.2, 0.40);
                for &(lat, lon) in land() {
                    let (x, y) = project(lat, lon, w, h);
                    cr.arc(x, y, r, 0.0, std::f64::consts::TAU);
                    let _ = cr.fill();
                }
                // Here: a filled dot inside a ring, in the accent.
                if let Some((lat, lon)) = here.get() {
                    let (x, y) = project(lat, lon, w, h);
                    let a = paint.accent;
                    cr.set_source_rgb(a.0, a.1, a.2);
                    cr.arc(x, y, 3.5, 0.0, std::f64::consts::TAU);
                    let _ = cr.fill();
                    cr.set_line_width(2.0);
                    cr.arc(x, y, 8.0, 0.0, std::f64::consts::TAU);
                    let _ = cr.stroke();
                }
            });
        }

        {
            let motion = gtk4::EventControllerMotion::new();
            let weak = Rc::downgrade(&map);
            motion.connect_motion(move |c, x, y| {
                let (Some(area), Some(m)) = (c.widget(), weak.upgrade()) else {
                    return;
                };
                let (w, h) = (f64::from(area.width()), f64::from(area.height()));
                m.hover.set(Some(unproject(x, y, w, h)));
                m.show_caption();
            });
            let weak = Rc::downgrade(&map);
            motion.connect_leave(move |_| {
                if let Some(m) = weak.upgrade() {
                    m.hover.set(None);
                    m.show_caption();
                }
            });
            area.add_controller(motion);
        }

        {
            let click = gtk4::GestureClick::new();
            click.connect_pressed(move |g, _, x, y| {
                let Some(area) = g.widget() else { return };
                let (w, h) = (f64::from(area.width()), f64::from(area.height()));
                let (lat, lon) = unproject(x, y, w, h);
                on_pick(rounded(lat), rounded(lon));
            });
            area.add_controller(click);
        }

        map
    }

    /// The map itself, for the group to lay out.
    pub fn widget(&self) -> &gtk4::AspectFrame {
        &self.frame
    }

    /// The line under the map: the place under the pointer, or the one the
    /// sun is computed for.
    pub fn caption(&self) -> &gtk4::Label {
        &self.caption
    }

    /// Show `here` as the place the sun is computed for.
    pub fn set(&self, here: Option<(f64, f64)>) {
        self.here.set(here);
        self.area.queue_draw();
        self.show_caption();
    }

    fn show_caption(&self) {
        let text = match (self.hover.get(), self.here.get()) {
            (Some((lat, lon)), _) => {
                format!("Click to pick {}", describe(rounded(lat), rounded(lon)))
            }
            (None, Some((lat, lon))) => describe(lat, lon),
            (None, None) => "No place yet: click where you are".to_string(),
        };
        self.caption.set_text(&text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_click_lands_where_the_dot_is_drawn() {
        let (w, h) = (360.0, 132.0);
        for (lat, lon) in [(59.3, 18.1), (-33.9, 151.2), (40.7, -74.0), (0.0, 0.0)] {
            let (x, y) = project(lat, lon, w, h);
            let (la, lo) = unproject(x, y, w, h);
            assert!((la - lat).abs() < 1e-9 && (lo - lon).abs() < 1e-9);
        }
        assert_eq!(rounded(59.3333), 59.3);
        assert_eq!(describe(-33.9, 151.2), "33.9° S, 151.2° E");
    }

    /// The mask has the shape the map assumes, and it is land where land is
    /// and sea where sea is.
    #[test]
    fn the_land_mask_is_the_world() {
        let rows: Vec<&str> = MASK.lines().collect();
        assert_eq!(rows.len(), ((NORTH - SOUTH) / CELL) as usize);
        assert!(rows.iter().all(|r| r.len() == (360.0 / CELL) as usize));
        let is_land = |lat: f64, lon: f64| {
            let row = ((NORTH - lat) / CELL) as usize;
            let col = ((lon + 180.0) / CELL) as usize;
            rows[row].as_bytes()[col] == b'#'
        };
        assert!(is_land(60.0, 16.0), "Sweden");
        assert!(is_land(-25.0, 134.0), "Australia");
        assert!(is_land(40.0, -100.0), "North America");
        assert!(!is_land(0.0, -30.0), "the Atlantic");
        assert!(!is_land(0.0, -150.0), "the Pacific");
    }
}
