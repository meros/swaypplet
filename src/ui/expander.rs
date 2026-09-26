//! Section and disclosure: a header that opens a body under it.

use gtk4::Align;
use gtk4::prelude::*;

use super::class::toggle;
use super::*;

/// A collapsible group: a header that says what is inside and its state,
/// and a body that opens under it.
pub struct Section {
    pub root: gtk4::Box,
    pub header: gtk4::Button,
    pub icon: gtk4::Label,
    pub summary: gtk4::Label,
    pub body: gtk4::Box,
    pub revealer: gtk4::Revealer,
}

pub fn section(icon: &str, title: &str, summary: &str) -> Section {
    let root = vbox(0);
    root.add_css_class("ui-section");
    let header = gtk4::Button::new();
    header.add_css_class("ui-section-header");
    let line = hbox(3);
    let icon_l = gtk4::Label::new(Some(icon));
    icon_l.add_css_class("ui-row-icon");
    let title_l = gtk4::Label::new(Some(title));
    title_l.add_css_class("ui-section-title");
    title_l.set_xalign(0.0);
    let summary_l = gtk4::Label::new(Some(summary));
    summary_l.add_css_class("ui-section-summary");
    summary_l.set_hexpand(true);
    summary_l.set_xalign(1.0);
    summary_l.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    let chevron = gtk4::Image::from_icon_name("pan-end-symbolic");
    chevron.add_css_class("ui-section-chevron");
    line.append(&icon_l);
    line.append(&title_l);
    line.append(&summary_l);
    line.append(&chevron);
    header.set_child(Some(&line));
    let body = vbox(1);
    body.add_css_class("ui-section-body");
    let revealer = revealer(
        gtk4::RevealerTransitionType::SlideDown,
        crate::tokens::motion::EXPAND,
    );
    revealer.set_child(Some(&body));
    root.append(&header);
    root.append(&revealer);
    {
        let (root, revealer) = (root.clone(), revealer.clone());
        header.connect_clicked(move |_| {
            let open = !revealer.reveals_child();
            revealer.set_reveal_child(open);
            toggle(&root, "open", open);
        });
    }
    Section {
        root,
        header,
        icon: icon_l,
        summary: summary_l,
        body,
        revealer,
    }
}

impl Section {
    pub fn set_open(&self, open: bool) {
        self.revealer.set_reveal_child(open);
        toggle(&self.root, "open", open);
    }
}

impl Section {
    /// The section as a page of its own (a Helm sub-sheet): no header, the
    /// body open at once, and no fill, because the sheet is already the
    /// group and a fill on it would only be a second ground.
    pub fn show_as_page(&self) {
        self.header.set_visible(false);
        let duration = self.revealer.transition_duration();
        self.revealer.set_transition_duration(0);
        self.set_open(true);
        self.revealer.set_transition_duration(duration);
        self.root.add_css_class("page");
    }
}

/// A disclosure inside a section: a quiet line that opens a body under
/// it. A section header would be a second group inside the first.
pub struct Disclosure {
    pub root: gtk4::Box,
    pub button: gtk4::Button,
    pub body: gtk4::Box,
    pub revealer: gtk4::Revealer,
}

pub fn disclosure(label: &str) -> Disclosure {
    let root = vbox(0);
    root.add_css_class("ui-disclosure");
    let button = gtk4::Button::new();
    button.add_css_class("ui-disclosure-button");
    button.set_halign(Align::Start);
    let line = hbox(2);
    let chevron = gtk4::Image::from_icon_name("pan-end-symbolic");
    chevron.add_css_class("ui-disclosure-chevron");
    let l = gtk4::Label::new(Some(label));
    line.append(&chevron);
    line.append(&l);
    button.set_child(Some(&line));
    let body = vbox(1);
    let revealer = revealer(
        gtk4::RevealerTransitionType::SlideDown,
        crate::tokens::motion::EXPAND,
    );
    revealer.set_child(Some(&body));
    root.append(&button);
    root.append(&revealer);
    {
        let (root, revealer) = (root.clone(), revealer.clone());
        button.connect_clicked(move |_| {
            let open = !revealer.reveals_child();
            revealer.set_reveal_child(open);
            toggle(&root, "open", open);
        });
    }
    Disclosure {
        root,
        button,
        body,
        revealer,
    }
}

impl Disclosure {
    pub fn set_open(&self, open: bool) {
        self.revealer.set_reveal_child(open);
        toggle(&self.root, "open", open);
    }
}
