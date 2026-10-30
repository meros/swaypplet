//! Boxes on the space scale, padding, separators, and the fills that
//! group controls in a row (pill group, toolbar).

use gtk4::Orientation;
use gtk4::prelude::*;

use swaypplet_tokens::space;

/// Take every child out of `parent`, through the container's own `remove`
/// where it has one: a list or flow box keeps bookkeeping (its rows, the
/// selection) that a bare `unparent` would leave behind. Any other widget
/// owns its children directly and lets them go with `unparent`.
pub fn clear_children(parent: &impl AsRef<gtk4::Widget>) {
    let parent = parent.as_ref();
    while let Some(child) = parent.first_child() {
        if let Some(b) = parent.downcast_ref::<gtk4::Box>() {
            b.remove(&child);
        } else if let Some(list) = parent.downcast_ref::<gtk4::ListBox>() {
            list.remove(&child);
        } else if let Some(flow) = parent.downcast_ref::<gtk4::FlowBox>() {
            flow.remove(&child);
        } else if let Some(grid) = parent.downcast_ref::<gtk4::Grid>() {
            grid.remove(&child);
        } else {
            child.unparent();
        }
    }
}

/// A box whose spacing is a step of the space scale (1-based, §3.5).
pub fn stack(orientation: Orientation, step: usize) -> gtk4::Box {
    gtk4::Box::new(orientation, space(step))
}

pub fn vbox(step: usize) -> gtk4::Box {
    stack(Orientation::Vertical, step)
}

pub fn hbox(step: usize) -> gtk4::Box {
    stack(Orientation::Horizontal, step)
}

/// Padding on all four sides from the space scale, as margins.
pub fn pad(w: &impl IsA<gtk4::Widget>, step: usize) {
    let p = space(step);
    w.set_margin_top(p);
    w.set_margin_bottom(p);
    w.set_margin_start(p);
    w.set_margin_end(p);
}

/// A hairline: `Horizontal` between rows, `Vertical` between runs of
/// controls in a toolbar.
pub fn separator(line: Orientation) -> gtk4::Box {
    let s = gtk4::Box::new(line, 0);
    s.add_css_class("ui-separator");
    if line == Orientation::Vertical {
        s.add_css_class("vertical");
    }
    s
}

/// Controls that belong together on one pill-shaped fill.
pub fn pill_group(step: usize) -> gtk4::Box {
    let b = hbox(step);
    b.add_css_class("ui-pill-group");
    b
}

/// A fill above the window's ground, for a toolbar.
pub fn toolbar(step: usize) -> gtk4::Box {
    let b = hbox(step);
    b.add_css_class("ui-toolbar");
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs a display; without one (a headless build) there is nothing to
    /// build a widget on, and the test says so and passes.
    #[test]
    fn every_child_goes_whatever_the_container() {
        if gtk4::init().is_err() {
            eprintln!("no display: clear_children not exercised");
            return;
        }
        let check = |parent: &gtk4::Widget, add: &dyn Fn(i32, &gtk4::Label)| {
            for i in 0..3 {
                add(i, &gtk4::Label::new(None));
            }
            assert!(parent.first_child().is_some());
            clear_children(parent);
            assert!(parent.first_child().is_none());
        };
        let b = vbox(1);
        check(b.upcast_ref(), &|_, l| b.append(l));
        let list = gtk4::ListBox::new();
        check(list.upcast_ref(), &|_, l| list.append(l));
        // Through its own remove, so the list box keeps no stale rows.
        assert!(list.row_at_index(0).is_none());
        let flow = gtk4::FlowBox::new();
        check(flow.upcast_ref(), &|_, l| flow.insert(l, -1));
        let grid = gtk4::Grid::new();
        check(grid.upcast_ref(), &|i, l| grid.attach(l, 0, i, 1, 1));
    }
}
