//! Peek: rest the pointer on a workspace button in the bar, and a live
//! picture of that workspace opens above it.
//!
//! The same picture as a Super+Tab tile (`card::preview`), in a popover, fed
//! by its own `live::Stream` for as long as the pointer stays. A workspace
//! already on a screen gets no peek: you can see it.
//!
//! The delay is what separates a peek from passing over the bar on the way
//! to something else, and the popover takes no grab, so it never stands in
//! the way of the click that switches.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::glib;
use gtk4::prelude::*;

use super::card::{self, Live};
use super::live;

/// How long the pointer rests on a button before the peek opens.
const HOVER_MS: u64 = 350;
/// The picture, at the Super+Tab tile's size.
const PEEK_W: i32 = super::rows::PREVIEW_W;
const PEEK_H: i32 = super::rows::PREVIEW_H;
/// Frames per second per window. Enough to read as live; a peek is a glance.
const PEEK_FPS: u32 = 30;

#[derive(Default)]
struct Peek {
    timer: Option<glib::SourceId>,
    popover: Option<gtk4::Popover>,
    stream: Option<live::Stream>,
    live: Rc<RefCell<Live>>,
    /// Bumped on every close, so a tree read that lands after the pointer
    /// left opens nothing.
    generation: u64,
}

/// Give `button` a peek at `workspace`. `on_screen` says whether the
/// workspace is showing on some output right now, read at hover time.
pub fn attach(button: &gtk4::Button, workspace: String, on_screen: Rc<dyn Fn(&str) -> bool>) {
    let state: Rc<RefCell<Peek>> = Rc::default();
    let motion = gtk4::EventControllerMotion::new();
    {
        let state = state.clone();
        let button_w = button.downgrade();
        let workspace = workspace.clone();
        motion.connect_enter(move |_, _, _| {
            close(&state);
            if on_screen(&workspace) {
                return;
            }
            let state_c = state.clone();
            let button_w = button_w.clone();
            let workspace = workspace.clone();
            let id = glib::timeout_add_local_once(
                std::time::Duration::from_millis(HOVER_MS),
                move || {
                    state_c.borrow_mut().timer = None;
                    if let Some(button) = button_w.upgrade() {
                        open(&state_c, &button, workspace);
                    }
                },
            );
            state.borrow_mut().timer = Some(id);
        });
    }
    {
        let state = state.clone();
        motion.connect_leave(move |_| close(&state));
    }
    button.add_controller(motion);

    // Harness hook: the nested session in dev/render.sh has no pointer, so
    // `SWAYPPLET_PEEK_OPEN=<workspace>` opens that button's peek on its own,
    // down the same path a hover takes after its delay.
    if std::env::var("SWAYPPLET_PEEK_OPEN").is_ok_and(|w| w == workspace) {
        let state = state.clone();
        let button_w = button.downgrade();
        let workspace = workspace.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(500), move || {
            if let Some(button) = button_w.upgrade() {
                open(&state, &button, workspace);
            }
        });
    }
    // The click switches; the peek has nothing left to show.
    let state_c = state.clone();
    button.connect_clicked(move |_| close(&state_c));
    // A button destroyed under the pointer (the bar rebuilt its pills) must
    // not leave a popover or a capture behind.
    button.connect_destroy(move |_| close(&state));
}

fn open(state: &Rc<RefCell<Peek>>, button: &gtk4::Button, workspace: String) {
    let generation = state.borrow().generation;
    let state = state.clone();
    let button = button.clone();
    crate::spawn::spawn_work(
        move || {
            let tree = crate::sway_ipc::connect().ok()?.get_tree().ok()?;
            super::scene::scene(&tree, &workspace)
        },
        move |scene| {
            if state.borrow().generation != generation || button.parent().is_none() {
                return;
            }
            let Some(scene) = scene else { return };
            show(&state, &button, &scene);
        },
    );
}

fn show(state: &Rc<RefCell<Peek>>, button: &gtk4::Button, scene: &super::scene::Scene) {
    let live = state.borrow().live.clone();
    *live.borrow_mut() = Live::default();
    let picture = card::preview(Some(scene), PEEK_W, PEEK_H, &mut live.borrow_mut());
    let ids = live.borrow().window_ids();

    // The bar's own popover chassis, so a peek looks like every other thing
    // that opens from the bar.
    let (popover, body) = crate::bar::popover::chassis(button);
    body.add_css_class("jump-peek");
    body.append(&picture);
    // No grab: the pointer is still on the button, and the click that
    // switches must reach it.
    popover.set_autohide(false);
    popover.popup();

    let stream = if ids.is_empty() {
        None
    } else {
        let (tx, rx) = async_channel::unbounded::<live::Frame>();
        let live = live.clone();
        glib::spawn_future_local(async move {
            while let Ok(frame) = rx.recv().await {
                live.borrow().frame(frame);
            }
        });
        Some(live::Stream::start(ids, (PEEK_W * 2) as u32, PEEK_FPS, tx))
    };

    let mut st = state.borrow_mut();
    st.popover = Some(popover);
    st.stream = stream;
}

fn close(state: &Rc<RefCell<Peek>>) {
    let mut st = state.borrow_mut();
    st.generation += 1;
    if let Some(id) = st.timer.take() {
        crate::spawn::remove_source(id);
    }
    st.stream = None;
    if let Some(popover) = st.popover.take() {
        popover.popdown();
        // The chassis also unparents when the button dies; whichever runs
        // second finds nothing to do.
        if popover.parent().is_some() {
            popover.unparent();
        }
    }
    *st.live.borrow_mut() = Live::default();
}
