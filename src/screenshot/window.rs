//! Screenshot one window, picked from a grid of every window, live.
//!
//! The region selector freezes the screen, so it can only take what is on
//! it. This takes any window, on any workspace, without going there: the
//! card shows every window as a live picture (`jump::live`, which captures
//! windows on hidden workspaces too), and the one picked is captured once
//! more at its full resolution and kept like any other shot.
//!
//! Arrow keys or the pointer move, Enter or a click takes, Escape leaves.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::glib;
use gtk4::prelude::*;

use super::capture::Image;
use crate::jump::card::{self, Live, LivePicture};
use crate::jump::{live, scene};
use crate::shell::{Namespace, Surface};

/// A window's picture in the grid, at most this box, keeping its shape.
const TILE_W: i32 = 240;
const TILE_H: i32 = 150;
/// Tiles per line.
const COLUMNS: u32 = 4;
/// Live at a glance's rate; the shot itself is captured separately.
const GRID_FPS: u32 = 15;
/// Larger than any window: the shot is taken unscaled.
const FULL: u32 = 1 << 15;

/// What happens with the shot.
/// The answer: the picked window's picture, or `None` when the grid was
/// closed without a pick (Escape) or there was nothing to pick.
type Done = Box<dyn FnOnce(Option<Image>)>;

struct Picker {
    /// Dropped by `close`, which ends the picker.
    surface: RefCell<Option<Surface>>,
    stream: RefCell<Option<live::Stream>>,
    done: RefCell<Option<Done>>,
}

/// Open the picker. `done` runs with the shot, or not at all when the
/// picker is left without one.
pub fn pick(app: &gtk4::Application, done: impl FnOnce(Option<Image>) + 'static) {
    let app = app.clone();
    crate::spawn::spawn_work(
        || {
            let tree = crate::sway::ipc::connect().ok()?.get_tree().ok()?;
            Some(scene::all_windows(&tree))
        },
        move |windows| {
            let windows: Vec<_> = windows
                .unwrap_or_default()
                .into_iter()
                .filter(|(w, _, _)| w.id.is_some())
                .collect();
            if windows.is_empty() {
                done(None);
                return;
            }
            show(&app, windows, Box::new(done));
        },
    );
}

fn show(app: &gtk4::Application, windows: Vec<(scene::Window, String, String)>, done: Done) {
    let surface = Surface::builder(app, Namespace::WindowPicker)
        // It is driven by the keyboard, so it takes it.
        .keyboard(gtk4_layer_shell::KeyboardMode::Exclusive)
        .card(crate::ui::Card::Floating)
        .build();
    let window = surface.window().clone();

    let card = surface.card();
    card.set_spacing(crate::tokens::space(3));
    card.set_halign(gtk4::Align::Center);
    card.set_valign(gtk4::Align::Center);
    card.add_css_class("window-picker");
    card.append(&crate::ui::overline(
        "Screenshot a window",
        crate::ui::Tone::Muted,
    ));

    let grid = gtk4::FlowBox::builder()
        .max_children_per_line(COLUMNS)
        // A card takes its child's narrowest width, which for a flow box is
        // one column; this asks for the full row whenever there are windows
        // enough to fill it.
        .min_children_per_line((windows.len() as u32).clamp(1, COLUMNS))
        .selection_mode(gtk4::SelectionMode::Single)
        .activate_on_single_click(true)
        .homogeneous(true)
        .row_spacing(crate::tokens::space(3) as u32)
        .column_spacing(crate::tokens::space(3) as u32)
        .build();
    crate::ui::choice_grid::adopt(&grid);
    let scroller = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .propagate_natural_height(true)
        .max_content_height(640)
        .child(&grid)
        .build();
    card.append(&scroller);

    let mut live = Live::default();
    let mut ids = Vec::new();
    for (win, workspace, name) in &windows {
        let Some(id) = win.id.clone() else { continue };
        grid.append(&tile(win, workspace, name, &id, &mut live));
        ids.push(id);
    }

    let picker = Rc::new(Picker {
        surface: RefCell::new(Some(surface.clone())),
        stream: RefCell::new(None),
        done: RefCell::new(Some(done)),
    });

    // The grid, live.
    let live = Rc::new(RefCell::new(live));
    let (tx, rx) = async_channel::unbounded::<live::Frame>();
    {
        let live = live.clone();
        glib::spawn_future_local(async move {
            while let Ok(frame) = rx.recv().await {
                live.borrow().frame(frame);
            }
        });
    }
    // Cut to each tile's picture in device pixels, which are known once the
    // surface is on its output.
    {
        let picker = picker.clone();
        let live = live.clone();
        let tx = RefCell::new(Some(tx));
        let start = move |window: &gtk4::Window| {
            let Some(tx) = tx.borrow_mut().take() else {
                return;
            };
            let wants = live.borrow().wants(None, card::device_scale(window));
            picker
                .stream
                .replace(Some(live::Stream::start_wants(wants, GRID_FPS, tx)));
        };
        if window.is_mapped() {
            start(&window);
        } else {
            window.connect_map(move |w| start(w.upcast_ref()));
        }
    }

    {
        let picker = picker.clone();
        grid.connect_child_activated(move |_, child| {
            let index = child.index();
            if let Some(id) = usize::try_from(index).ok().and_then(|i| ids.get(i)) {
                take(&picker, id.clone());
            }
        });
    }
    let keys = gtk4::EventControllerKey::new();
    {
        let picker = picker.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gtk4::gdk::Key::Escape {
                close(&picker);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
    }
    window.add_controller(keys);

    // Fades in with its glass (anim::Reveal counts it, and dropping the
    // surface in `close` gives it back).
    surface.show();
    window.present();
    if let Some(first) = grid.child_at_index(0) {
        grid.select_child(&first);
        first.grab_focus();
    }

    // Harness hook: the nested session in dev/render.sh has no keyboard, so
    // `SWAYPPLET_PICK_FIRST=1` takes the first window on its own, down the
    // same path Enter takes.
    if std::env::var("SWAYPPLET_PICK_FIRST").is_ok_and(|v| !v.is_empty()) {
        let grid = grid.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(1500), move || {
            if let Some(first) = grid.child_at_index(0) {
                first.activate();
            }
        });
    }
}

/// One window's tile: its live picture over its title and where it is.
fn tile(win: &scene::Window, workspace: &str, name: &str, id: &str, live: &mut Live) -> gtk4::Box {
    let b = crate::ui::vbox(2);

    let (s, _, _) = scene::fit(win.w, win.h, TILE_W, TILE_H);
    let picture = LivePicture::new();
    picture.set_size_request(
        ((f64::from(win.w) * s).round() as i32).max(8),
        ((f64::from(win.h) * s).round() as i32).max(8),
    );
    picture.set_halign(gtk4::Align::Center);
    picture.set_valign(gtk4::Align::Center);
    // The box keeps every tile one size whatever the window's shape.
    let frame = crate::ui::vbox(0);
    frame.set_size_request(TILE_W, TILE_H);
    frame.append(&picture);
    picture.set_vexpand(true);
    b.append(&frame);
    live.add(id.to_string(), picture);

    let title = crate::ui::text(
        if name.is_empty() { &win.app } else { name },
        crate::ui::Text::Body,
        crate::ui::Tone::Fg,
    );
    title.set_max_width_chars(28);
    title.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    b.append(&title);
    let place = crate::ui::text(
        &format!("{} \u{00b7} {}", win.app, workspace),
        crate::ui::Text::Caption,
        crate::ui::Tone::Muted,
    );
    place.set_max_width_chars(28);
    place.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    b.append(&place);
    b
}

/// Capture the picked window once, unscaled, and hand it over.
fn take(picker: &Rc<Picker>, id: String) {
    // The grid's stream stops; the shot has a stream of its own.
    picker.stream.replace(None);
    // Out of the shot at once, not faded: the capture starts now.
    if let Some(surface) = &*picker.surface.borrow() {
        surface.window().set_visible(false);
    }
    let (tx, rx) = async_channel::bounded::<live::Frame>(1);
    let shot = live::Stream::start(vec![id], FULL, 0, tx);
    let picker = picker.clone();
    glib::spawn_future_local(async move {
        // One frame is the shot; dropping the stream ends the capture.
        let frame = rx.recv().await;
        drop(shot);
        if let (Ok(frame), Some(done)) = (frame, picker.done.borrow_mut().take()) {
            done(Some(to_image(frame)));
        }
        close(&picker);
    });
}

fn close(picker: &Rc<Picker>) {
    picker.stream.replace(None);
    // Still here: closed without a pick.
    let unanswered = picker.done.borrow_mut().take();
    if let Some(done) = unanswered {
        done(None);
    }
    // Taken before it drops: the surface's own handlers hold the picker.
    let surface = picker.surface.borrow_mut().take();
    drop(surface);
}

/// A live frame (premultiplied BGRA) as a screenshot image (straight RGBA).
fn to_image(frame: live::Frame) -> Image {
    // `Stream::start` sends frames in memory, never on the GPU.
    let mut pixels = frame.pixels.into_memory().unwrap_or_default();
    for px in pixels.chunks_exact_mut(4) {
        let (b, g, r, a) = (px[0], px[1], px[2], px[3]);
        let un = |c: u8| {
            if a == 0 || a == 255 {
                c
            } else {
                ((u32::from(c) * 255 + u32::from(a) / 2) / u32::from(a)).min(255) as u8
            }
        };
        px[0] = un(r);
        px[1] = un(g);
        px[2] = un(b);
        px[3] = a;
    }
    Image {
        width: frame.width,
        height: frame.height,
        pixels,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(px: [u8; 4]) -> live::Frame {
        live::Frame {
            id: String::new(),
            width: 1,
            height: 1,
            pixels: live::Pixels::Memory(px.to_vec()),
        }
    }

    #[test]
    fn an_opaque_pixel_only_swaps_to_rgba() {
        assert_eq!(
            to_image(frame([10, 20, 30, 255])).pixels,
            vec![30, 20, 10, 255]
        );
    }

    #[test]
    fn a_translucent_pixel_is_unpremultiplied() {
        // Premultiplied at half alpha: 100 of a straight 200.
        assert_eq!(
            to_image(frame([50, 100, 0, 128])).pixels,
            vec![0, 199, 100, 128]
        );
    }
}
