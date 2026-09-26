//! The bar's components: categorical tones, the segmented track and its
//! segments, marks, the task bays, the meter, the rail, and the receded group.
//!
//! `ui::segmented()`, `ui::segment::adopt(&w, quiet)`, `ui::mark(child,
//! quiet)`, `ui::mark::adopt(&b, quiet)`, `ui::bay(child, task)`,
//! `ui::bay_chip()`, `ui::meter::adopt(&area)`, `ui::rail(slot)`; the
//! `set_*` below at runtime.

use gtk4::prelude::*;
use gtk4::{Align, Orientation};

use super::class::{swap, toggle};

/// The categorical slots (§3.1): identity only, 1-based.
pub const CATEGORIES: usize = 6;

/// Text in a categorical tone, `n` in 1..=6; anything else clears it.
pub fn set_category(w: &impl IsA<gtk4::Widget>, n: usize) {
    let family: Vec<String> = (1..=CATEGORIES).map(|i| format!("ui-cat-{i}")).collect();
    let one = (1..=CATEGORIES).contains(&n).then(|| format!("ui-cat-{n}"));
    swap(w, family.iter().map(String::as_str), one.as_deref());
}

/// Step a group back as a whole, or bring it forward.
pub fn set_receded(w: &impl IsA<gtk4::Widget>, receded: bool) {
    toggle(w, "ui-receded", receded);
}

pub mod meter {
    use gtk4::prelude::*;

    /// A Cairo-drawn meter: its `color()` is the accent fill.
    pub fn adopt(area: &gtk4::DrawingArea) {
        area.add_css_class("ui-meter");
    }
}

/// A track of fused segments; only its ends round. No gap: the segments
/// touch, and a divider separates them.
pub fn segmented() -> gtk4::Box {
    let b = gtk4::Box::new(Orientation::Horizontal, 0);
    b.add_css_class("ui-segmented");
    b
}

pub mod segment {
    use gtk4::prelude::*;

    /// Make `w` a segment. `quiet` keeps its label muted until it is
    /// selected or under the pointer.
    pub fn adopt(w: &impl IsA<gtk4::Widget>, quiet: bool) {
        w.add_css_class("ui-segment");
        super::set_quiet(w, quiet);
    }
}

/// A segment's or a mark's label one level down (muted, or faint on a
/// mark) until the pointer or the selection brings it up.
pub fn set_quiet(w: &impl IsA<gtk4::Widget>, quiet: bool) {
    toggle(w, "quiet", quiet);
}

/// Where a segment stands in its control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selection {
    Idle,
    /// What its screen shows.
    Current,
    /// Current, on the screen holding input.
    Focused,
}

pub fn set_selection(w: &impl IsA<gtk4::Widget>, s: Selection) {
    toggle(w, "current", s != Selection::Idle);
    toggle(w, "focused", s == Selection::Focused);
}

/// The one red: act now (an urgent workspace, a dying battery).
pub fn set_danger(w: &impl IsA<gtk4::Widget>, danger: bool) {
    toggle(w, "danger", danger);
}

/// What a segment's ribbon lane says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ribbon {
    Off,
    /// A line: something is live here.
    Working,
    /// The categorical tone `n` (1..=4): this one wants you.
    Category(usize),
}

/// What a segment's 2 px ribbon lane says. The first call gives the segment
/// its lane, transparent at `Off`, so set it once when the segment is built
/// and its height never changes after.
pub fn set_ribbon(w: &impl IsA<gtk4::Widget>, r: Ribbon) {
    w.add_css_class("ribboned");
    toggle(w, "ribbon-working", r == Ribbon::Working);
    for n in 1..=4 {
        toggle(w, &format!("ribbon-cat-{n}"), r == Ribbon::Category(n));
    }
}

/// A quiet button straight on thin glass: no fill at rest, muted, ink
/// under the pointer. `quiet` sits it one level lower, at faint.
pub fn mark(child: &impl IsA<gtk4::Widget>, quiet: bool) -> gtk4::Button {
    let b = gtk4::Button::builder().child(child).build();
    mark::adopt(&b, quiet);
    b
}

pub mod mark {
    use gtk4::prelude::*;

    /// Make a button the caller built a mark.
    pub fn adopt(b: &(impl IsA<gtk4::Button> + IsA<gtk4::Widget>), quiet: bool) {
        b.add_css_class("ui-mark");
        super::set_quiet(b, quiet);
    }
}

/// One task's slot on the board; `task` (1..=4) picks its tone.
pub fn bay(child: &impl IsA<gtk4::Widget>, task: usize) -> gtk4::Button {
    let b = gtk4::Button::builder().child(child).build();
    b.add_css_class("ui-bay");
    b.add_css_class(&format!("cat-{task}"));
    b
}

/// A bay's state, as the classes the component styles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BayState {
    Socket,
    Working,
    /// Halted on a prompt; rides the unacked fill.
    Blocked,
    Waiting {
        acked: bool,
        overdue: bool,
    },
    Stopped,
    Stale,
}

impl BayState {
    /// Every class a state can set, so a change can clear the old one.
    const FAMILY: [&'static str; 8] = [
        "socket", "working", "waiting", "blocked", "unacked", "overdue", "stopped", "stale",
    ];

    pub fn classes(self) -> &'static [&'static str] {
        match self {
            BayState::Socket => &["socket"],
            BayState::Working => &["working"],
            BayState::Blocked => &["waiting", "unacked", "blocked"],
            BayState::Waiting { acked: true, .. } => &["waiting"],
            BayState::Waiting {
                acked: false,
                overdue: false,
            } => &["waiting", "unacked"],
            BayState::Waiting {
                acked: false,
                overdue: true,
            } => &["waiting", "unacked", "overdue"],
            BayState::Stopped => &["stopped"],
            BayState::Stale => &["stale"],
        }
    }
}

pub fn set_bay_state(b: &gtk4::Button, state: BayState, local: bool) {
    for c in BayState::FAMILY {
        toggle(b, c, state.classes().contains(&c));
    }
    toggle(b, "local", local);
}

/// The age chip beside a bay's numeral.
pub fn bay_chip() -> gtk4::Label {
    let l = gtk4::Label::new(None);
    l.add_css_class("ui-bay-chip");
    l
}

/// A thin pill down a card's leading edge, in categorical slot `n`
/// (1..=`CATEGORIES`), or in the muted text level when there is none.
pub fn rail(n: Option<usize>) -> gtk4::Box {
    let r = gtk4::Box::new(Orientation::Vertical, 0);
    r.add_css_class("ui-rail");
    r.set_valign(Align::Fill);
    if let Some(n) = n.filter(|n| (1..=CATEGORIES).contains(n)) {
        r.add_css_class(&format!("cat-{n}"));
    }
    r
}
