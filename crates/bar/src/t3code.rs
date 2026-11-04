//! T3 Code segment: the host's `t3code.service` as one glyph on the
//! instrument track. A click starts the server when it is down and stops it
//! when it is up.
//!
//! The unit is the host's (the system manager), so starting and stopping it
//! needs the polkit grant in `modules/nixos/services/t3code.nix` (nixos
//! repo). A host that has no such unit shows nothing: the segment is hidden
//! while systemd answers `LoadState=not-found`, which is the answer on every
//! machine that does not run the server.
//!
//! State is established, not assumed. A click runs the verb, waits for the
//! job, then reads the unit back, so a refused stop leaves the glyph as it
//! was and puts the reason in the tooltip. The unit can also change without
//! this bar (`systemctl`, a crash, `nx` restarting it), so a slow poll reads
//! it every [`POLL_SECONDS`]. glib batches second-granularity timers with
//! the others, and the read is one `systemctl show`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4::glib;
use gtk4::prelude::*;

use swaypplet_base::spawn::spawn_work;
use swaypplet_model::store;
use swaypplet_system::systemd::{Unit, Verb};
use swaypplet_ui::ui;

const UNIT: &str = "t3code.service";
const POLL_SECONDS: u32 = 10;

// Nerd Font: md-server / md-server-off.
const ICON_UP: &str = "\u{f048b}";
const ICON_DOWN: &str = "\u{f048f}";

/// What the segment knows about the unit.
#[derive(Clone, Debug, PartialEq, Eq)]
enum State {
    /// The host has no such unit: the segment is hidden.
    Absent,
    Up,
    Down,
    /// The unit exited with an error; a click starts it again.
    Failed,
    /// `systemctl` could not be asked.
    Unknown,
}

impl State {
    /// From `LoadState` and `ActiveState` as `systemctl show` prints them.
    /// A unit on its way up or down counts as up: a click then stops it,
    /// which is what someone watching a start that hangs wants.
    fn read(load: &str, active: &str) -> State {
        if load == "not-found" {
            return State::Absent;
        }
        match active {
            "active" | "activating" | "reloading" => State::Up,
            "failed" => State::Failed,
            "inactive" | "deactivating" => State::Down,
            _ => State::Unknown,
        }
    }

    /// What a click does from this state.
    fn click(&self) -> Option<Verb> {
        match self {
            State::Up => Some(Verb::Stop),
            State::Down | State::Failed => Some(Verb::Start),
            State::Absent | State::Unknown => None,
        }
    }

    fn icon(&self) -> &'static str {
        match self {
            State::Up => ICON_UP,
            _ => ICON_DOWN,
        }
    }

    /// Running is plain ink and stopped is faint, the same rest tones as
    /// the other instruments. A failed unit is the one amber state.
    fn tone(&self) -> ui::Tone {
        match self {
            State::Up => ui::Tone::Fg,
            State::Failed => ui::Tone::Warning,
            State::Down | State::Absent | State::Unknown => ui::Tone::Faint,
        }
    }

    fn tooltip(&self) -> &'static str {
        match self {
            State::Up => "T3 Code server runs. Click to stop it.",
            State::Down => "T3 Code server is stopped. Click to start it.",
            State::Failed => "T3 Code server failed. Click to start it again.",
            State::Unknown => "T3 Code server: systemctl did not answer.",
            State::Absent => "",
        }
    }
}

/// The Bar tab's switch for this segment.
fn wanted() -> bool {
    store::with(|s| s.bar().t3code)
}

fn read() -> State {
    match Unit::system(UNIT).show(&["LoadState", "ActiveState"]) {
        Ok(p) => State::read(p.get("LoadState"), p.get("ActiveState")),
        Err(e) => {
            log::warn!("t3code: {e}");
            State::Unknown
        }
    }
}

pub fn build() -> gtk4::Button {
    let glyph = gtk4::Label::new(Some(ICON_DOWN));
    let btn = gtk4::Button::builder()
        .child(&glyph)
        .css_classes(["bar-seg"])
        .visible(false)
        .build();
    ui::segment::adopt(&btn, false);

    let state = Rc::new(RefCell::new(State::Absent));
    // A start or stop is in flight: a second click would queue the opposite
    // verb behind it.
    let busy = Rc::new(Cell::new(false));
    // The last refusal, shown in the tooltip until the next read agrees.
    let error = Rc::new(RefCell::new(None::<String>));

    let show = {
        let (btn, glyph, state, error) = (
            btn.downgrade(),
            glyph.downgrade(),
            state.clone(),
            error.clone(),
        );
        Rc::new(move |next: State| {
            let (Some(btn), Some(glyph)) = (btn.upgrade(), glyph.upgrade()) else {
                return;
            };
            btn.set_visible(wanted() && next != State::Absent);
            glyph.set_label(next.icon());
            ui::set_tone(&glyph, next.tone());
            let tip = match error.borrow().as_deref() {
                Some(e) => format!("{}\n{e}", next.tooltip()),
                None => next.tooltip().to_string(),
            };
            btn.set_tooltip_text(Some(&tip));
            state.replace(next);
        })
    };

    let refresh = {
        let (show, busy) = (show.clone(), busy.clone());
        Rc::new(move || {
            if busy.get() {
                return;
            }
            let show = show.clone();
            spawn_work(read, move |next| show(next));
        })
    };

    {
        let (state, busy, error, show) =
            (state.clone(), busy.clone(), error.clone(), show.clone());
        btn.connect_clicked(move |_| {
            let Some(verb) = state.borrow().click() else {
                return;
            };
            if busy.replace(true) {
                return;
            }
            let (busy, error, show) = (busy.clone(), error.clone(), show.clone());
            spawn_work(
                move || {
                    let result = Unit::system(UNIT).run(verb);
                    (result, read())
                },
                move |(result, next)| {
                    busy.set(false);
                    error.replace(result.err());
                    show(next);
                },
            );
        });
    }

    // The segment follows the Bar tab's switch itself, as the tray does:
    // `follow_setting` would show it on a host that has no such unit.
    {
        let (weak, state) = (btn.downgrade(), state.clone());
        store::observe(move || {
            if let Some(btn) = weak.upgrade() {
                btn.set_visible(wanted() && *state.borrow() != State::Absent);
            }
        });
    }

    refresh();
    let weak = btn.downgrade();
    glib::timeout_add_seconds_local(POLL_SECONDS, move || {
        if weak.upgrade().is_none() {
            return glib::ControlFlow::Break;
        }
        refresh();
        glib::ControlFlow::Continue
    });

    btn
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_host_without_the_unit_shows_nothing() {
        assert_eq!(State::read("not-found", "inactive"), State::Absent);
        assert_eq!(State::Absent.click(), None);
    }

    #[test]
    fn a_unit_on_its_way_counts_by_its_direction() {
        assert_eq!(State::read("loaded", "active"), State::Up);
        assert_eq!(State::read("loaded", "activating"), State::Up);
        assert_eq!(State::read("loaded", "deactivating"), State::Down);
        assert_eq!(State::read("loaded", "inactive"), State::Down);
        assert_eq!(State::read("loaded", "failed"), State::Failed);
        assert_eq!(State::read("loaded", ""), State::Unknown);
    }

    #[test]
    fn a_click_reverses_the_state_and_a_failed_unit_starts() {
        assert_eq!(State::Up.click(), Some(Verb::Stop));
        assert_eq!(State::Down.click(), Some(Verb::Start));
        assert_eq!(State::Failed.click(), Some(Verb::Start));
        assert_eq!(State::Unknown.click(), None);
    }
}
