//! Transitions and loops that go through `anim::duration`, so Look → Motion
//! and reduced motion reach every one (design lint `motion-bypass`).
//!
//! `ui::revealer(transition, motion)`, `ui::page_stack(transition,
//! motion)`; `ui::set_breathing` at runtime.
//!
//! # Live lengths
//!
//! GTK keeps a revealer's or a stack's `transition_duration` as a number, so
//! one set when the widget is built would keep the Motion setting of that
//! moment for as long as the widget lives. Instead every revealer and stack
//! built here is remembered, weakly and with its motion token, in a
//! per-thread [`Registry`], and the registry sets each live one's length
//! again whenever the stylesheet reloads (`theme::observe`, which fires when
//! any theme input moved, the Motion scale among them) and whenever GTK's
//! own reduced-motion switch flips. So a Look → Motion change reaches the
//! revealers already on screen in the same tick as the CSS durations beside
//! them. A widget that has gone is dropped from the registry on the next
//! pass.

use std::cell::{Cell, RefCell};

use gtk4::prelude::*;

use super::class::toggle;
use crate::tokens::motion::Motion;

/// A GTK transition length for a motion token, through `anim::ms`, so a
/// revealer or a stack follows Look → Motion and reduced motion like every
/// other animation. The only place Rust sets a `transition_duration`.
fn transition_ms(motion: Motion) -> u32 {
    crate::anim::ms(motion).round() as u32
}

/// A revealer that moves at `motion`, closed.
pub fn revealer(transition: gtk4::RevealerTransitionType, motion: Motion) -> gtk4::Revealer {
    let r = gtk4::Revealer::builder()
        .transition_type(transition)
        .transition_duration(transition_ms(motion))
        .build();
    follow(&r, motion, gtk4::Revealer::set_transition_duration);
    r
}

/// A stack of pages that changes page at `motion`.
pub fn page_stack(transition: gtk4::StackTransitionType, motion: Motion) -> gtk4::Stack {
    let s = gtk4::Stack::builder()
        .transition_type(transition)
        .transition_duration(transition_ms(motion))
        .build();
    follow(&s, motion, gtk4::Stack::set_transition_duration);
    s
}

/// One transition the registry keeps in step.
struct Entry {
    motion: Motion,
    /// Sets the length, or says the widget is gone.
    apply: Box<dyn Fn(u32) -> bool>,
    /// Whether the widget is still there, without touching it.
    alive: Box<dyn Fn() -> bool>,
}

/// The transitions built on this thread, weakly held.
#[derive(Default)]
struct Registry {
    entries: Vec<Entry>,
    /// The size at which [`Registry::add`] next sweeps out the dead, so a
    /// process that builds and drops rows all day (and never reloads the
    /// theme) does not grow the list without bound.
    sweep_at: usize,
}

/// The smallest list worth sweeping.
const SWEEP_FLOOR: usize = 64;

impl Registry {
    fn add(&mut self, entry: Entry) {
        if self.entries.len() >= self.sweep_at.max(SWEEP_FLOOR) {
            self.entries.retain(|e| (e.alive)());
            self.sweep_at = self.entries.len() * 2;
        }
        self.entries.push(entry);
    }

    /// Set every live transition to `len` of its motion; forget the rest.
    fn refresh(&mut self, len: impl Fn(Motion) -> u32) {
        self.entries.retain(|e| (e.apply)(len(e.motion)));
    }
}

thread_local! {
    static LIVE: RefCell<Registry> = RefCell::new(Registry::default());
    /// Whether this thread follows the reloads yet: hooked on the first
    /// transition built, so a process that never builds one never hooks.
    static HOOKED: Cell<bool> = const { Cell::new(false) };
}

/// Remember `w` (weakly) so its length follows the Motion setting.
fn follow<W: IsA<gtk4::glib::Object>>(w: &W, motion: Motion, set: fn(&W, u32)) {
    let (weak, check) = (w.downgrade(), w.downgrade());
    LIVE.with(|l| {
        l.borrow_mut().add(Entry {
            motion,
            apply: Box::new(move |ms| weak.upgrade().map(|w| set(&w, ms)).is_some()),
            alive: Box::new(move || check.upgrade().is_some()),
        })
    });
    if !HOOKED.replace(true) {
        crate::theme::observe(refresh);
        if let Some(settings) = gtk4::Settings::default() {
            settings.connect_gtk_enable_animations_notify(|_| refresh());
        }
    }
}

/// Every live revealer and stack to its motion's length as this session
/// plays it now. The list is taken out while the setters run, so a notify
/// handler that builds another revealer adds to an unborrowed registry.
fn refresh() {
    let mut taken = LIVE.with(|l| std::mem::take(&mut *l.borrow_mut()));
    taken.refresh(transition_ms);
    LIVE.with(|l| {
        let mut l = l.borrow_mut();
        taken.entries.append(&mut l.entries);
        l.entries = taken.entries;
        l.sweep_at = l.sweep_at.max(taken.sweep_at);
    });
}

/// Shake `w` once: a "no" on the card that asked (a rejected password).
/// Every call replays it: the class comes off now and back on the next
/// main-loop turn, so the style recomputes between the two.
pub fn shake(w: &impl IsA<gtk4::Widget>) {
    w.remove_css_class("ui-shake");
    let w = w.clone().upcast::<gtk4::Widget>();
    gtk4::glib::idle_add_local_once(move || w.add_css_class("ui-shake"));
}

/// Stop a shake's class from lingering (a card being reset for reuse).
pub fn clear_shake(w: &impl IsA<gtk4::Widget>) {
    w.remove_css_class("ui-shake");
}

/// Breathe: the attention loop for something working in the background
/// (a notification's progress, the player's art).
pub fn set_breathing(w: &impl IsA<gtk4::Widget>, breathing: bool) {
    toggle(w, "ui-breathing", breathing);
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::{Rc, Weak};

    use super::*;
    use crate::tokens::motion::{ENTER, EXPAND};

    /// A stand-in for a widget: its length, held weakly as GTK's would be.
    fn entry(motion: Motion, target: &Rc<Cell<u32>>) -> Entry {
        let (weak, check): (Weak<Cell<u32>>, Weak<Cell<u32>>) =
            (Rc::downgrade(target), Rc::downgrade(target));
        Entry {
            motion,
            apply: Box::new(move |ms| weak.upgrade().map(|c| c.set(ms)).is_some()),
            alive: Box::new(move || check.upgrade().is_some()),
        }
    }

    /// Half speed, as Look → Motion's middle step plays it.
    fn half(m: Motion) -> u32 {
        (m.ms / 2.0) as u32
    }

    #[test]
    fn a_refresh_sets_every_live_transition_to_its_own_motion() {
        let mut r = Registry::default();
        let (a, b) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
        r.add(entry(EXPAND, &a));
        r.add(entry(ENTER, &b));
        r.refresh(half);
        assert_eq!((a.get(), b.get()), (100, 150));
        r.refresh(|m| m.ms as u32);
        assert_eq!((a.get(), b.get()), (200, 300));
    }

    #[test]
    fn a_dropped_widget_leaves_the_registry() {
        let mut r = Registry::default();
        let kept = Rc::new(Cell::new(0));
        r.add(entry(EXPAND, &kept));
        r.add(entry(ENTER, &Rc::new(Cell::new(0))));
        r.refresh(half);
        assert_eq!(r.entries.len(), 1);
        assert_eq!(kept.get(), 100);
    }

    #[test]
    fn adding_sweeps_the_dead_without_a_refresh() {
        let mut r = Registry::default();
        for _ in 0..SWEEP_FLOOR * 4 {
            r.add(entry(EXPAND, &Rc::new(Cell::new(0))));
        }
        assert!(r.entries.len() <= SWEEP_FLOOR + 1, "{}", r.entries.len());
        // The live ones survive a sweep.
        let kept: Vec<_> = (0..SWEEP_FLOOR * 2)
            .map(|_| Rc::new(Cell::new(0)))
            .collect();
        for c in &kept {
            r.add(entry(ENTER, c));
        }
        r.refresh(half);
        assert!(kept.iter().all(|c| c.get() == 150));
        assert_eq!(r.entries.len(), kept.len());
    }
}
