//! Peek: rest the pointer on a workspace button in the bar, and a live
//! picture of that workspace opens above it.
//!
//! The picture is a workspace view (`jump::view`), the same card a pin is,
//! in the bar's popover chassis. A click on the picture goes there, and the
//! pin button at the footer's end pins the workspace (or unpins it) and
//! closes the peek, so a glance that turns out to be worth keeping becomes
//! a pin without a detour through Super+Tab. A workspace already on a
//! screen gets no peek: you can see it.
//!
//! The delay is what separates a peek from passing over the bar on the way
//! to something else. The popover takes no grab, so it never stands in the
//! way of the click that switches. Leaving the button starts a short grace
//! instead of closing, and the pointer arriving on the card cancels it, so
//! the pin button can be reached across the gap between the two.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gtk4::glib;
use gtk4::prelude::*;

use super::popover;
use crate::jump::pin;
use crate::jump::view::{self, PIN_GLYPH, UNPIN_GLYPH, View};

/// How long the pointer rests on a button before the peek opens.
const HOVER_MS: u64 = 350;
/// How long the peek stays after the pointer leaves the button or the card,
/// for the trip from one to the other.
const GRACE_MS: u64 = 250;
#[derive(Default)]
struct Peek {
    /// The hover delay, before the peek opens.
    opening: Option<glib::SourceId>,
    /// The grace, before it closes.
    closing: Option<glib::SourceId>,
    popover: Option<gtk4::Popover>,
    view: Option<View>,
    /// Bumped on every close, so a tree read that lands after the pointer
    /// left opens nothing.
    generation: u64,
}

type State = Rc<RefCell<Peek>>;

/// Give `button` a peek at `workspace`. `on_screen` says whether the
/// workspace is showing on some output right now, read at hover time.
pub fn attach(button: &gtk4::Button, workspace: String, on_screen: Rc<dyn Fn(&str) -> bool>) {
    let state: State = Rc::default();
    let motion = gtk4::EventControllerMotion::new();
    {
        let state = state.clone();
        let button_w = button.downgrade();
        let workspace = workspace.clone();
        motion.connect_enter(move |_, _, _| {
            cancel_close(&state);
            // Back from the card, or never gone: the peek is still up.
            if state.borrow().popover.is_some() || state.borrow().opening.is_some() {
                return;
            }
            if on_screen(&workspace) {
                return;
            }
            let state_c = state.clone();
            let button_w = button_w.clone();
            let workspace = workspace.clone();
            let id = glib::timeout_add_local_once(Duration::from_millis(HOVER_MS), move || {
                state_c.borrow_mut().opening = None;
                if let Some(button) = button_w.upgrade() {
                    open(&state_c, &button, workspace);
                }
            });
            state.borrow_mut().opening = Some(id);
        });
    }
    {
        let state = state.clone();
        motion.connect_leave(move |_| {
            let opening = state.borrow_mut().opening.take();
            if let Some(id) = opening {
                crate::spawn::remove_source(id);
            }
            close_soon(&state);
        });
    }
    button.add_controller(motion);

    // Harness hook: the nested session in dev/render.sh has no pointer, so
    // `SWAYPPLET_PEEK_OPEN=<workspace>` opens that button's peek on its own,
    // down the same path a hover takes after its delay.
    if std::env::var("SWAYPPLET_PEEK_OPEN").is_ok_and(|w| w == workspace) {
        let state = state.clone();
        let button_w = button.downgrade();
        let workspace = workspace.clone();
        glib::timeout_add_local_once(Duration::from_millis(500), move || {
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

fn open(state: &State, button: &gtk4::Button, workspace: String) {
    let generation = state.borrow().generation;
    let state = state.clone();
    let button = button.clone();
    let name = workspace.clone();
    crate::spawn::spawn_work(
        move || {
            let tree = crate::sway::ipc::connect().ok()?.get_tree().ok()?;
            crate::jump::scene::scene(&tree, &name)
        },
        move |scene| {
            if state.borrow().generation != generation || button.parent().is_none() {
                return;
            }
            let Some(scene) = scene else { return };
            show(&state, &button, workspace, scene);
        },
    );
}

fn show(state: &State, button: &gtk4::Button, workspace: String, scene: crate::jump::scene::Scene) {
    let label = crate::sway::workspace::label_for_name(&workspace);
    let mut view = View::new(&label, "click to go");
    view.show_scene(Some(scene), view::FPS);

    // The pin button: pin what you are looking at, or unpin it.
    let toggle = if pin::is_pinned(&workspace) {
        view.action(UNPIN_GLYPH, "Unpin")
    } else {
        view.action(PIN_GLYPH, "Pin")
    };
    {
        let state = state.clone();
        let workspace = workspace.clone();
        toggle.connect_clicked(move |_| {
            close(&state);
            if let Some(pins) = pin::handle() {
                pins.toggle(workspace.clone());
            }
        });
    }
    // A click on the picture goes there, as on a pin.
    let click = gtk4::GestureClick::new();
    {
        let state = state.clone();
        click.connect_released(move |_, _, _, _| {
            close(&state);
            pin::go(&workspace);
        });
    }
    view.picture().add_controller(click);

    // The bar's own popover chassis, so a peek looks like every other thing
    // that opens from the bar.
    let (popover, body) = popover::chassis(button);
    body.append(view.widget());
    // The pointer on the card holds the peek open; leaving it starts the
    // grace, as leaving the button does.
    let motion = gtk4::EventControllerMotion::new();
    {
        let state = state.clone();
        motion.connect_enter(move |_, _, _| cancel_close(&state));
    }
    {
        let state = state.clone();
        motion.connect_leave(move |_| close_soon(&state));
    }
    body.add_controller(motion);
    // No grab: the pointer is still on the button, and the click that
    // switches must reach it.
    popover.set_autohide(false);
    popover.popup();

    let mut st = state.borrow_mut();
    st.popover = Some(popover);
    st.view = Some(view);
}

fn cancel_close(state: &State) {
    let closing = state.borrow_mut().closing.take();
    if let Some(id) = closing {
        crate::spawn::remove_source(id);
    }
}

/// Close after the grace, unless the pointer comes back first.
fn close_soon(state: &State) {
    cancel_close(state);
    if state.borrow().popover.is_none() {
        return;
    }
    let state_c = state.clone();
    let id = glib::timeout_add_local_once(Duration::from_millis(GRACE_MS), move || {
        state_c.borrow_mut().closing = None;
        close(&state_c);
    });
    state.borrow_mut().closing = Some(id);
}

fn close(state: &State) {
    cancel_close(state);
    let (popover, view) = {
        let mut st = state.borrow_mut();
        st.generation += 1;
        if let Some(id) = st.opening.take() {
            crate::spawn::remove_source(id);
        }
        (st.popover.take(), st.view.take())
    };
    // The capture before the card it draws into.
    drop(view);
    if let Some(popover) = popover {
        popover.popdown();
        // The chassis also unparents when the button dies; whichever runs
        // second finds nothing to do.
        if popover.parent().is_some() {
            popover.unparent();
        }
    }
}
