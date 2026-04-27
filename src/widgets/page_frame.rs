use gtk4::prelude::*;

/// A thin wrapper that adds a title row + optional subtitle above a section widget,
/// giving each dispatch-board lane the feel of a settings page.
pub struct PageFrame {
    root: gtk4::Box,
}

impl PageFrame {
    pub fn new(title: &str, subtitle: Option<&str>, content: &impl IsA<gtk4::Widget>) -> Self {
        let root = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(0)
            .hexpand(true)
            .vexpand(true)
            .build();
        root.add_css_class("page-frame");

        // ── Header ─────────────────────────────────────────────────────────
        let header = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(2)
            .build();
        header.add_css_class("page-frame-header");

        let title_lbl = gtk4::Label::builder()
            .label(title)
            .halign(gtk4::Align::Start)
            .build();
        title_lbl.add_css_class("page-title");
        header.append(&title_lbl);

        if let Some(sub) = subtitle {
            let sub_lbl = gtk4::Label::builder()
                .label(sub)
                .halign(gtk4::Align::Start)
                .build();
            sub_lbl.add_css_class("page-subtitle");
            header.append(&sub_lbl);
        }

        root.append(&header);
        root.append(content);

        Self { root }
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }
}
