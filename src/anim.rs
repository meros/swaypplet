//! Shared motion scale for all animated surfaces.
//!
//! One easing (cubic ease-out) and three durations, so every surface moves
//! the same way:
//! - micro interactions (hover/press color changes): 150ms, CSS only
//! - structural reveal/collapse: 200ms (GtkRevealer `transition_duration`)
//! - enter/move: [`ENTER_MS`]/[`MOVE_MS`]; exit/fade: [`EXIT_MS`]
//!
//! The CSS side of this scale is documented in `data/style.css`.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;

pub const ENTER_MS: f64 = 300.0;
pub const MOVE_MS: f64 = 300.0;
pub const EXIT_MS: f64 = 200.0;

pub fn ease_out_cubic(t: f64) -> f64 {
    1.0 - (1.0 - t).powi(3)
}

struct FadeState {
    from: f64,
    to: f64,
    start: i64, // glib::monotonic_time() µs
    ms: f64,
    ticking: bool,
    on_done: Option<Box<dyn FnOnce()>>,
}

/// Retargetable opacity fade on a single widget, driven by a frame-clock
/// tick callback. Calling [`Fade::to`] mid-flight redirects the running
/// animation from the current opacity (and drops the pending `on_done`).
#[derive(Clone)]
pub struct Fade {
    widget: gtk4::Widget,
    state: Rc<RefCell<FadeState>>,
}

impl Fade {
    pub fn new(widget: &impl IsA<gtk4::Widget>) -> Self {
        Fade {
            widget: widget.clone().upcast(),
            state: Rc::new(RefCell::new(FadeState {
                from: 1.0,
                to: 1.0,
                start: 0,
                ms: EXIT_MS,
                ticking: false,
                on_done: None,
            })),
        }
    }

    /// Set opacity immediately, cancelling any pending `on_done`.
    pub fn jump(&self, value: f64) {
        let mut s = self.state.borrow_mut();
        s.from = value;
        s.to = value;
        s.on_done = None;
        self.widget.set_opacity(value);
    }

    pub fn to(&self, target: f64, ms: f64, on_done: Option<Box<dyn FnOnce()>>) {
        let start_tick = {
            let mut s = self.state.borrow_mut();
            s.from = self.widget.opacity();
            s.to = target;
            s.start = glib::monotonic_time();
            s.ms = ms;
            s.on_done = on_done;
            if s.ticking {
                false
            } else {
                s.ticking = true;
                true
            }
        };
        if !start_tick {
            return;
        }

        let state = self.state.clone();
        self.widget.add_tick_callback(move |widget, _| {
            let done_cb = {
                let mut s = state.borrow_mut();
                let t = (((glib::monotonic_time() - s.start) as f64 / 1000.0) / s.ms)
                    .clamp(0.0, 1.0);
                widget.set_opacity(s.from + (s.to - s.from) * ease_out_cubic(t));
                if t >= 1.0 {
                    s.ticking = false;
                    s.on_done.take()
                } else {
                    return glib::ControlFlow::Continue;
                }
            };
            if let Some(cb) = done_cb {
                cb();
            }
            glib::ControlFlow::Break
        });
    }
}
