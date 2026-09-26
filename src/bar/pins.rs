//! Pins mark: where pinned workspaces live in the bar (`jump/pin.rs`).
//!
//! A dim pin glyph in the right cluster while anything is pinned, with the
//! count beside it from two up; zero width when nothing is. Filled while the
//! pins float on screen, hollow while they are tucked into the bar. It
//! reveals at the structural 200 ms when the first pin lands and leaves the
//! same way with the last (BAR_VISION P2: motion at onset only; P10: the
//! last unpin is its stand-down).
//!
//! A click opens the read-layer popover (bar/popover.rs): every pin, live,
//! with a button to go there and one to unpin, and a tuck that takes the
//! floating pins away without unpinning anything. Everything is a button,
//! so nothing needs a pointer to dwell (P8). The pictures stream only while
//! the popover is open.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::glib;
use gtk4::prelude::*;

use super::popover;
use crate::ui;
use crate::jump::card::{self, Live};
use crate::jump::{live, pin};

const PIN: &str = "\u{f0403}";
const PIN_OUTLINE: &str = "\u{f0931}";

/// A pin's picture in the popover, 16:10.
const ROW_W: i32 = 192;
const ROW_H: i32 = 120;
/// Enough to read as live in a popover that is open for a glance.
const ROW_FPS: u32 = 15;

struct Ui {
    revealer: gtk4::Revealer,
    glyph: gtk4::Label,
    count: gtk4::Label,
    btn: gtk4::Button,
    pop: gtk4::Popover,
    body: gtk4::Box,
    live: Rc<RefCell<Live>>,
    stream: RefCell<Option<live::Stream>>,
}

pub fn build() -> gtk4::Widget {
    let glyph = gtk4::Label::new(Some(PIN));
    let count = gtk4::Label::new(None);
    ui::set_text_style(&count, ui::Text::Caption, ui::Tone::Fg);
    let inner = ui::hbox(1);
    inner.append(&glyph);
    inner.append(&count);
    // Dim and achromatic like the media mark; a level lower still while the
    // pins are tucked.
    let btn = ui::mark(&inner, false);
    btn.add_css_class("bar-pinmark");
    btn.set_tooltip_text(Some("Pinned workspaces"));
    let revealer = gtk4::Revealer::builder()
        .transition_type(gtk4::RevealerTransitionType::SlideLeft)
        .transition_duration(crate::tokens::DURATION[1].1)
        .child(&btn)
        .build();

    let (pop, body) = popover::chassis(&btn);
    let ui = Rc::new(Ui {
        revealer: revealer.clone(),
        glyph,
        count,
        btn: btn.clone(),
        pop: pop.clone(),
        body,
        live: Rc::default(),
        stream: RefCell::new(None),
    });

    {
        let ui = ui.clone();
        btn.connect_clicked(move |_| {
            fill(&ui);
            ui.pop.popup();
        });
    }
    {
        let ui = ui.clone();
        pop.connect_closed(move |_| {
            ui.stream.replace(None);
        });
    }

    // The registry outlives a bar unplugged with its output; a dead weak
    // reference makes the leftover callback a no-op.
    let weak = Rc::downgrade(&ui);
    pin::connect_changed(move || {
        if let Some(ui) = weak.upgrade() {
            update(&ui);
            if ui.pop.is_visible() {
                fill(&ui);
            }
        }
    });
    update(&ui);

    // Harness hook: the nested session in dev/render.sh has no pointer, so
    // `SWAYPPLET_PINS_OPEN=1` opens the popover on its own once pins exist,
    // down the same path a click takes.
    if std::env::var("SWAYPPLET_PINS_OPEN").is_ok_and(|v| !v.is_empty()) {
        let weak = Rc::downgrade(&ui);
        let opened = std::cell::Cell::new(false);
        pin::connect_changed(move || {
            let Some(ui) = weak.upgrade() else { return };
            if opened.get() || pin::pinned().is_empty() {
                return;
            }
            opened.set(true);
            // After the mark has revealed, so the popover has a parent on
            // screen.
            glib::timeout_add_local_once(std::time::Duration::from_millis(600), move || {
                fill(&ui);
                ui.pop.popup();
            });
        });
    }
    revealer.upcast()
}

/// The mark: shown while anything is pinned, filled or hollow, and a count.
fn update(ui: &Ui) {
    let pinned = pin::pinned();
    ui.revealer.set_reveal_child(!pinned.is_empty());
    let tucked = pin::tucked();
    ui.glyph.set_label(if tucked { PIN_OUTLINE } else { PIN });
    ui::set_class(&ui.btn, "quiet", tucked);
    ui.count.set_visible(pinned.len() > 1);
    ui.count.set_label(&pinned.len().to_string());
}

/// Build the popover's rows from the tree, and start their pictures.
fn fill(ui: &Rc<Ui>) {
    let names = pin::pinned();
    let ui = ui.clone();
    crate::spawn::spawn_work(
        move || {
            let tree = crate::sway_ipc::connect().ok()?.get_tree().ok()?;
            Some(
                names
                    .into_iter()
                    .map(|n| {
                        let s = crate::jump::scene::scene(&tree, &n);
                        (n, s)
                    })
                    .collect::<Vec<_>>(),
            )
        },
        move |scenes| {
            let Some(scenes) = scenes else { return };
            rows(&ui, &scenes);
        },
    );
}

fn rows(ui: &Rc<Ui>, scenes: &[(String, Option<crate::jump::scene::Scene>)]) {
    while let Some(child) = ui.body.first_child() {
        ui.body.remove(&child);
    }
    ui.body.append(&popover::title("PINNED"));

    *ui.live.borrow_mut() = Live::default();
    for (name, scene) in scenes {
        let row = ui::hbox(3);
        row.add_css_class("bar-pinned-row");
        let picture = card::preview(scene.as_ref(), ROW_W, ROW_H, &mut ui.live.borrow_mut());
        row.append(&picture);

        let side = ui::vbox(3);
        side.set_valign(gtk4::Align::Center);
        let label = ui::text(
            &crate::jump::rows::label_for(&crate::jump::place::Place {
                num: name
                    .split(':')
                    .next()
                    .and_then(|n| n.parse().ok())
                    .unwrap_or(-1),
                name: name.clone(),
                output: String::new(),
            }),
            ui::Text::Body,
            ui::Tone::Fg,
        );
        label.add_css_class("ui-strong");
        side.append(&label);
        let go = ui::small_button("Go there", ui::Kind::Secondary);
        let unpin = ui::small_button("Unpin", ui::Kind::Secondary);
        for (b, action) in [(&go, "go"), (&unpin, "unpin")] {
            let name = name.clone();
            let pop = ui.pop.clone();
            b.connect_clicked(move |_| {
                let Some(pins) = pin::handle() else { return };
                if action == "go" {
                    pop.popdown();
                    pins.go(&name);
                } else {
                    pins.unpin(&name);
                }
            });
        }
        side.append(&go);
        side.append(&unpin);
        row.append(&side);
        ui.body.append(&row);
    }

    let tucked = pin::tucked();
    let tuck = ui::small_button(
        if tucked {
            "Show pins on screen"
        } else {
            "Tuck pins into the bar"
        },
        ui::Kind::Secondary,
    );
    tuck.connect_clicked(move |_| {
        if let Some(pins) = pin::handle() {
            pins.set_tucked(!tucked);
        }
    });
    ui.body.append(&tuck);

    let ids = ui.live.borrow().window_ids();
    ui.stream.replace(None);
    if !ids.is_empty() {
        let (tx, rx) = async_channel::unbounded::<live::Frame>();
        let live = ui.live.clone();
        glib::spawn_future_local(async move {
            while let Ok(frame) = rx.recv().await {
                live.borrow().frame(frame);
            }
        });
        ui.stream.replace(Some(live::Stream::start(
            ids,
            (ROW_W * 2) as u32,
            ROW_FPS,
            tx,
        )));
    }
}
