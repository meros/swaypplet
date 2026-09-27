//! A box that wraps: children left to right at their natural widths, and a
//! new line when the next one does not fit.
//!
//! GTK 4 has no such container (libadwaita's `WrapBox` is not a dependency
//! here). `FlowBox` is not one: it lines its children up in shared columns,
//! so one wide child on the second line widens a column and opens gaps
//! between the children on the first. This is the panel's switch strip's
//! layout, where the tiles and the one-click actions are of very different
//! widths.

use std::cell::Cell;

use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use gtk4::{glib, Orientation, SizeRequestMode};

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct WrapBox {
        /// Between children on a line, in px.
        pub spacing: Cell<i32>,
        /// Between lines, in px.
        pub line_spacing: Cell<i32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for WrapBox {
        const NAME: &'static str = "SwayppletWrapBox";
        type Type = super::WrapBox;
        type ParentType = gtk4::Widget;
    }

    impl ObjectImpl for WrapBox {
        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for WrapBox {
        fn request_mode(&self) -> SizeRequestMode {
            SizeRequestMode::HeightForWidth
        }

        fn measure(&self, orientation: Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let widths = self.obj().widths();
            match orientation {
                Orientation::Horizontal => {
                    // At least the widest child; at most everything on one line.
                    let min = widths.iter().map(|w| w.0).max().unwrap_or(0);
                    let nat = widths.iter().map(|w| w.1).sum::<i32>()
                        + self.spacing.get() * (widths.len() as i32 - 1).max(0);
                    (min, nat.max(min), -1, -1)
                }
                _ => {
                    let width = if for_size < 0 { i32::MAX } else { for_size };
                    let h = self.obj().lay_out(width).height;
                    (h, h, -1, -1)
                }
            }
        }

        fn size_allocate(&self, width: i32, _height: i32, _baseline: i32) {
            let layout = self.obj().lay_out(width);
            for (child, rect) in layout.children {
                child.size_allocate(&gtk4::Allocation::new(rect.0, rect.1, rect.2, rect.3), -1);
            }
        }
    }
}

glib::wrapper! {
    pub struct WrapBox(ObjectSubclass<imp::WrapBox>)
        @extends gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

/// Where each child goes, and the height it all takes.
struct Layout {
    children: Vec<(gtk4::Widget, (i32, i32, i32, i32))>,
    height: i32,
}

impl WrapBox {
    fn visible(&self) -> Vec<gtk4::Widget> {
        std::iter::successors(self.first_child(), |c| c.next_sibling())
            .filter(|c| c.should_layout())
            .collect()
    }

    /// Each visible child's (minimum, natural) width.
    fn widths(&self) -> Vec<(i32, i32)> {
        self.visible()
            .iter()
            .map(|c| {
                let (min, nat, _, _) = c.measure(Orientation::Horizontal, -1);
                (min, nat)
            })
            .collect()
    }

    /// Lines at `width`: each child at its natural width (never wider than
    /// the box), a line's height its tallest child's, children centred in
    /// their line.
    fn lay_out(&self, width: i32) -> Layout {
        let imp = self.imp();
        let (gap, line_gap) = (imp.spacing.get(), imp.line_spacing.get());
        let mut lines: Vec<Vec<(gtk4::Widget, i32, i32)>> = vec![Vec::new()];
        let mut x = 0;
        for child in self.visible() {
            let (min, nat, _, _) = child.measure(Orientation::Horizontal, -1);
            let w = nat.min(width).max(min);
            let line = lines.last_mut().expect("one line at least");
            if !line.is_empty() && x + gap + w > width {
                lines.push(Vec::new());
                x = 0;
            }
            let line = lines.last_mut().expect("one line at least");
            if !line.is_empty() {
                x += gap;
            }
            let (_, h, _, _) = child.measure(Orientation::Vertical, w);
            line.push((child, w, h));
            x += w;
        }
        let mut children = Vec::new();
        let mut y = 0;
        for (i, line) in lines.iter().filter(|l| !l.is_empty()).enumerate() {
            if i > 0 {
                y += line_gap;
            }
            let line_h = line.iter().map(|c| c.2).max().unwrap_or(0);
            let mut x = 0;
            for (j, (child, w, h)) in line.iter().enumerate() {
                if j > 0 {
                    x += gap;
                }
                children.push((child.clone(), (x, y + (line_h - h) / 2, *w, *h)));
                x += w;
            }
            y += line_h;
        }
        Layout {
            children,
            height: y,
        }
    }

    pub fn append(&self, child: &impl IsA<gtk4::Widget>) {
        child.set_parent(self);
    }
}

/// A wrapping box whose spacing, between children and between lines, is a
/// step of the space scale.
pub fn wrap_box(step: usize) -> WrapBox {
    let b: WrapBox = glib::Object::new();
    let s = crate::tokens::space(step);
    b.imp().spacing.set(s);
    b.imp().line_spacing.set(s);
    b
}
