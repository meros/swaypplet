//! Where one pin stands, chosen by hand: a see-through sheet on every
//! screen, the pin's outline under the pointer, and a click to put it there.
//!
//! A pin follows focus by default (`pin.rs`): it stands in the setting's
//! corner of whichever screen you are working on. Moving one fixes it: from
//! then on it stays on the screen it was put on, at that place, whatever has
//! the focus, until it is told to follow again (this sheet's "Follow focus",
//! or the bar's popover).
//!
//! A layer surface cannot be dragged: it belongs to one output and its place
//! is margins from the edges it is anchored to, and a drag inside it loses
//! track of the pointer as soon as the surface moves under it. So the move
//! is a placement instead, the way the region selector picks a rectangle
//! (`swaypplet_screenshot::screenshot::select`): one sheet per screen, with
//! no glass (`Namespace::PinSpot`), where pointer coordinates are the
//! screen's own and stay put. The outline follows the pointer, kept a margin
//! from the edges and snapped to that margin when it is close, and a click
//! drops it.
//!
//! A place is kept as the corner nearest to it and the gaps to that corner's
//! two edges ([`Spot`]), so a pin near the bottom right stays near the bottom
//! right when the screen changes size, rather than drifting off it.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{gdk, glib};
use gtk4_layer_shell::KeyboardMode;

use swaypplet_model::store::Corner;
use swaypplet_ui::shell::{Namespace, Surface, layer};
use swaypplet_ui::ui;

/// The least gap between the outline and a screen's edge.
pub const EDGE: i32 = swaypplet_tokens::space(5);
/// How close to [`EDGE`] counts as against it.
const SNAP: i32 = swaypplet_tokens::space(7);

/// A fixed pin's place: the screen, the corner nearest it, and the gaps from
/// that corner's side edge (`x`) and top or bottom edge (`y`) to the card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spot {
    pub output: String,
    pub corner: Corner,
    pub x: i32,
    pub y: i32,
}

/// What the sheet was closed with.
pub enum Choice {
    /// Put the pin here, and keep it here.
    Here(Spot),
    /// Let the pin follow focus again, in the setting's corner.
    Follow,
    /// Leave it as it was.
    Cancel,
}

type Done = Box<dyn FnOnce(Choice)>;

struct Sheet {
    output: String,
    surface: Surface,
    area: gtk4::DrawingArea,
    /// The outline's top-left corner, while the pointer is on this screen.
    ghost: Rc<std::cell::Cell<Option<(f64, f64)>>>,
}

struct Session {
    sheets: Vec<Sheet>,
    done: RefCell<Option<Done>>,
    /// The card's size, in GTK's pixels.
    size: (f64, f64),
}

thread_local! {
    /// The one sheet open, so a second Move does not stack another on it.
    static OPEN: RefCell<Option<Rc<Session>>> = const { RefCell::new(None) };
}

/// Whether the sheet is open.
pub fn is_open() -> bool {
    OPEN.with(|o| o.borrow().is_some())
}

/// Open the sheet on every screen pins may stand on, for a card `size` big.
/// `fixed` says whether the pin is fixed now, which offers "Follow focus".
/// `done` runs once, with what was chosen; nothing happens while a sheet is
/// already open.
pub fn choose(
    app: &gtk4::Application,
    label: &str,
    size: (i32, i32),
    fixed: bool,
    done: impl FnOnce(Choice) + 'static,
) {
    if is_open() {
        return;
    }
    let connected = layer::connectors();
    let allowed = swaypplet_ui::shell::extra::Kind::Pins.allowed();
    let mut sheets = Vec::new();
    for output in &connected {
        if !swaypplet_ui::shell::extra::shows(allowed, output, &connected) {
            continue;
        }
        let monitor = layer::monitor_by_connector(output);
        let surface = Surface::builder(app, Namespace::PinSpot)
            .monitor(monitor.as_ref())
            .fill()
            // The whole screen, bar included: a pin may stand over it, and
            // a fixed pin's gaps are measured from the screen's edges too.
            .over_exclusive_zones()
            // Escape has to reach the sheet.
            .keyboard(KeyboardMode::Exclusive)
            .no_card()
            .build();
        let area = gtk4::DrawingArea::builder()
            .hexpand(true)
            .vexpand(true)
            .build();
        area.set_cursor(gdk::Cursor::from_name("move", None).as_ref());
        let overlay = gtk4::Overlay::new();
        overlay.set_child(Some(&area));
        overlay.add_overlay(&hint(label, fixed));
        overlay.set_hexpand(true);
        overlay.set_vexpand(true);
        surface.root().append(&overlay);
        sheets.push(Sheet {
            output: output.clone(),
            surface,
            area,
            ghost: Rc::default(),
        });
    }
    if sheets.is_empty() {
        done(Choice::Cancel);
        return;
    }
    let session = Rc::new(Session {
        sheets,
        done: RefCell::new(Some(Box::new(done))),
        size: (f64::from(size.0), f64::from(size.1)),
    });
    OPEN.with(|o| o.replace(Some(session.clone())));
    for i in 0..session.sheets.len() {
        wire(&session, i);
    }
    // The hint's buttons find the session through OPEN.
    for sheet in &session.sheets {
        sheet.surface.window().present();
    }
    seed(&session);
}

/// Close the sheet with `choice`. Safe to call twice: the second finds
/// nothing open.
fn finish(choice: Choice) {
    let Some(session) = OPEN.with(|o| o.borrow_mut().take()) else {
        return;
    };
    let done = session.done.borrow_mut().take();
    // The surfaces go with the session; let the handler that called this
    // return first, since it runs on one of them.
    glib::idle_add_local_once(move || drop(session));
    if let Some(done) = done {
        done(choice);
    }
}

/// The card at the top of each screen: what this is, and the ways out.
fn hint(label: &str, fixed: bool) -> gtk4::Widget {
    let card = ui::vbox(3);
    ui::card::adopt(&card, ui::Card::Solid);
    ui::pad(&card, 4);
    card.set_halign(gtk4::Align::Center);
    card.set_valign(gtk4::Align::Start);
    card.set_margin_top(swaypplet_tokens::space(7));
    let title = ui::text(&format!("Move {label}"), ui::Text::Body, ui::Tone::Fg);
    ui::set_weight(&title, ui::Weight::Strong);
    title.set_xalign(0.5);
    card.append(&title);
    let line = ui::text(
        "Click where the pin should stay, on any screen \u{00b7} Esc to cancel",
        ui::Text::Caption,
        ui::Tone::Muted,
    );
    line.set_xalign(0.5);
    card.append(&line);
    let buttons = ui::hbox(2);
    buttons.set_halign(gtk4::Align::Center);
    if fixed {
        let follow = ui::button_with(
            ui::Face::Label("Follow focus again"),
            ui::Kind::Secondary,
            ui::Size::Small,
        );
        follow.connect_clicked(|_| finish(Choice::Follow));
        buttons.append(&follow);
    }
    let cancel = ui::button_with(
        ui::Face::Label("Cancel"),
        ui::Kind::Secondary,
        ui::Size::Small,
    );
    cancel.connect_clicked(|_| finish(Choice::Cancel));
    buttons.append(&cancel);
    card.append(&buttons);
    card.upcast()
}

fn wire(session: &Rc<Session>, index: usize) {
    let sheet = &session.sheets[index];
    let size = session.size;

    let ghost = sheet.ghost.clone();
    let paint = swaypplet_ui::theme::paint();
    sheet.area.set_draw_func(move |_, cr, w, h| {
        // Dim the screen, and clear the outline back to it: where the pin
        // will stand is the one bright place.
        ui::set_source(cr, swaypplet_tokens::Rgb::BLACK, 0.42);
        cr.rectangle(0.0, 0.0, f64::from(w), f64::from(h));
        if let Some((x, y)) = ghost.get() {
            cr.rectangle(x, y + size.1, size.0, -size.1);
        }
        cr.set_fill_rule(gtk4::cairo::FillRule::EvenOdd);
        let _ = cr.fill();
        if let Some((x, y)) = ghost.get() {
            ui::set_source(cr, paint.accent, 0.18);
            cr.rectangle(x, y, size.0, size.1);
            let _ = cr.fill();
            ui::set_source(cr, paint.accent, 1.0);
            cr.set_line_width(2.0);
            cr.rectangle(x + 1.0, y + 1.0, size.0 - 2.0, size.1 - 2.0);
            let _ = cr.stroke();
        }
    });

    // Follow the pointer; a screen it left shows no outline.
    let motion = gtk4::EventControllerMotion::new();
    {
        let (ghost, area) = (sheet.ghost.clone(), sheet.area.clone());
        motion.connect_motion(move |_, x, y| {
            let screen = (f64::from(area.width()), f64::from(area.height()));
            ghost.set(Some(ghost_at((x, y), size, screen)));
            area.queue_draw();
        });
    }
    {
        let (ghost, area) = (sheet.ghost.clone(), sheet.area.clone());
        motion.connect_leave(move |_| {
            ghost.set(None);
            area.queue_draw();
        });
    }
    sheet.area.add_controller(motion);

    // A click puts it there; right or middle click backs out.
    let click = gtk4::GestureClick::new();
    click.set_button(0);
    {
        let (ghost, area, output) = (
            sheet.ghost.clone(),
            sheet.area.clone(),
            sheet.output.clone(),
        );
        click.connect_released(move |g, _, x, y| {
            if g.current_button() != gdk::BUTTON_PRIMARY {
                finish(Choice::Cancel);
                return;
            }
            let screen = (f64::from(area.width()), f64::from(area.height()));
            // A click with no motion before it still lands where it is.
            let at = ghost
                .get()
                .unwrap_or_else(|| ghost_at((x, y), size, screen));
            let (corner, gx, gy) = spot_for(at, size, screen);
            finish(Choice::Here(Spot {
                output: output.clone(),
                corner,
                x: gx,
                y: gy,
            }));
        });
    }
    sheet.area.add_controller(click);

    let keys = gtk4::EventControllerKey::new();
    keys.connect_key_pressed(|_, key, _, _| {
        if key == gdk::Key::Escape {
            finish(Choice::Cancel);
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    sheet.surface.window().add_controller(keys);
}

/// Harness hook: the nested session in `dev/render.sh` has no pointer, so
/// `SWPP_PIN_SPOT=x,y` draws the outline as if the pointer were there on the
/// first screen, and `SWPP_PIN_SPOT_DROP=1` clicks there a moment later.
fn seed(session: &Rc<Session>) {
    let Ok(spec) = std::env::var("SWPP_PIN_SPOT") else {
        return;
    };
    let v: Vec<f64> = spec
        .split(',')
        .filter_map(|n| n.trim().parse().ok())
        .collect();
    let [px, py] = v[..] else { return };
    let size = session.size;
    let sheet = &session.sheets[0];
    let (ghost, area, output) = (
        sheet.ghost.clone(),
        sheet.area.clone(),
        sheet.output.clone(),
    );
    glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || {
        let screen = (f64::from(area.width()), f64::from(area.height()));
        let at = ghost_at((px, py), size, screen);
        ghost.set(Some(at));
        area.queue_draw();
        if std::env::var("SWPP_PIN_SPOT_DROP").is_ok_and(|v| !v.is_empty()) {
            glib::timeout_add_local_once(std::time::Duration::from_millis(1200), move || {
                let (corner, x, y) = spot_for(at, size, screen);
                finish(Choice::Here(Spot {
                    output,
                    corner,
                    x,
                    y,
                }));
            });
        }
    });
}

/// The outline's top-left corner for the pointer at `p`: centred on it,
/// kept [`EDGE`] from every edge, and against that margin when within
/// [`SNAP`] of it.
pub fn ghost_at(p: (f64, f64), size: (f64, f64), screen: (f64, f64)) -> (f64, f64) {
    let axis = |p: f64, len: f64, room: f64| {
        let edge = f64::from(EDGE);
        let snap = f64::from(SNAP);
        let lo = edge;
        let hi = (room - len - edge).max(lo);
        let v = (p - len / 2.0).clamp(lo, hi);
        if v - lo < snap {
            lo
        } else if hi - v < snap {
            hi
        } else {
            v.round()
        }
    };
    (axis(p.0, size.0, screen.0), axis(p.1, size.1, screen.1))
}

/// The corner nearest the outline at `at`, and its gaps to that corner's
/// side and top or bottom edge.
pub fn spot_for(at: (f64, f64), size: (f64, f64), screen: (f64, f64)) -> (Corner, i32, i32) {
    let left = at.0 + size.0 / 2.0 < screen.0 / 2.0;
    let top = at.1 + size.1 / 2.0 < screen.1 / 2.0;
    let x = if left { at.0 } else { screen.0 - at.0 - size.0 };
    let y = if top { at.1 } else { screen.1 - at.1 - size.1 };
    let corner = match (top, left) {
        (true, true) => Corner::TopLeft,
        (true, false) => Corner::TopRight,
        (false, true) => Corner::BottomLeft,
        (false, false) => Corner::BottomRight,
    };
    (corner, x.round().max(0.0) as i32, y.round().max(0.0) as i32)
}

#[cfg(test)]
mod tests {
    use super::{EDGE, SNAP, ghost_at, spot_for};
    use swaypplet_model::store::Corner;

    const SCREEN: (f64, f64) = (1920.0, 1080.0);
    const CARD: (f64, f64) = (424.0, 314.0);

    #[test]
    fn the_outline_centres_on_the_pointer_away_from_the_edges() {
        assert_eq!(ghost_at((960.0, 540.0), CARD, SCREEN), (748.0, 383.0));
    }

    #[test]
    fn the_outline_stays_on_the_screen_and_snaps_to_its_margin() {
        let e = f64::from(EDGE);
        // Off the top left: against the margin.
        assert_eq!(ghost_at((0.0, 0.0), CARD, SCREEN), (e, e));
        // Off the bottom right: against the margin there.
        assert_eq!(
            ghost_at(SCREEN, CARD, SCREEN),
            (SCREEN.0 - CARD.0 - e, SCREEN.1 - CARD.1 - e)
        );
        // Close to the margin, inside the snap: against it.
        let near = e + f64::from(SNAP) - 1.0 + CARD.0 / 2.0;
        assert_eq!(ghost_at((near, 540.0), CARD, SCREEN).0, e);
    }

    #[test]
    fn a_card_bigger_than_the_screen_still_lands_at_the_margin() {
        let e = f64::from(EDGE);
        assert_eq!(
            ghost_at((50.0, 50.0), (900.0, 900.0), (600.0, 600.0)),
            (e, e)
        );
    }

    #[test]
    fn a_place_is_kept_from_its_nearest_corner() {
        // The bottom right, 40 from the right and 60 from the bottom.
        let at = (SCREEN.0 - CARD.0 - 40.0, SCREEN.1 - CARD.1 - 60.0);
        assert_eq!(spot_for(at, CARD, SCREEN), (Corner::BottomRight, 40, 60));
        // The top left, as it stands.
        assert_eq!(
            spot_for((30.0, 20.0), CARD, SCREEN),
            (Corner::TopLeft, 30, 20)
        );
        // Top right and bottom left.
        let at = (SCREEN.0 - CARD.0 - 10.0, 15.0);
        assert_eq!(spot_for(at, CARD, SCREEN), (Corner::TopRight, 10, 15));
        let at = (25.0, SCREEN.1 - CARD.1 - 35.0);
        assert_eq!(spot_for(at, CARD, SCREEN), (Corner::BottomLeft, 25, 35));
    }
}
