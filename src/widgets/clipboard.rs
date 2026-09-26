//! Clipboard history section.
//!
//! The rows come from [`crate::clipboard`], which watches the selection over
//! `ext-data-control-v1` in this process. This used to shell out to
//! `cliphist list` on every open, against a database no daemon was filling —
//! see that module's header for what that cost.

use std::rc::Rc;

use gtk4::prelude::*;

use crate::clipboard::{ClipboardService, EntryView};
use crate::ui;
use crate::ui::icons;

// ── ClipboardSection ──────────────────────────────────────────────────────────

struct Widgets {
    section: ui::Section,
    entry_list: gtk4::Box,
    clear_btn: gtk4::Button,
}

pub struct ClipboardSection {
    widgets: Rc<Widgets>,
    /// `None` on a compositor without the protocol; the section says so
    /// rather than showing an empty list that looks like an empty history.
    service: Option<Rc<ClipboardService>>,
}

impl ClipboardSection {
    pub fn new() -> Self {
        let section = ui::section(icons::CLIPBOARD, "Clipboard", "");
        ui::glyph::adopt(&section.icon, ui::Text::Title, ui::Tone::Fg);

        let detail_box = ui::vbox(2);
        section.body.append(&detail_box);

        let entry_list = ui::vbox(1);
        detail_box.append(&entry_list);

        let clear_btn = ui::button("Clear History", ui::Kind::Flat);
        detail_box.append(&clear_btn);

        let widgets = Rc::new(Widgets {
            section,
            entry_list,
            clear_btn,
        });

        let service = crate::clipboard::service();

        if let Some(svc) = &service {
            {
                let svc = svc.clone();
                let w = widgets.clone();
                widgets.clear_btn.connect_clicked(move |_| {
                    svc.clear();
                    // The observer fires from clear(), so the list redraws
                    // itself; nothing to do here but let it.
                    let _ = &w;
                });
            }
            // Rows follow the ring: a copy made while the panel is open
            // lands in the list without an open/close cycle.
            {
                let svc = svc.clone();
                let w = widgets.clone();
                svc.clone()
                    .connect_change(move || Self::render(&w, Some(&svc.entries())));
            }
        } else {
            widgets.clear_btn.set_sensitive(false);
        }

        let clipboard = ClipboardSection { widgets, service };
        clipboard.refresh();
        clipboard
    }

    /// Draw `entries`, or the unavailable notice when there is no service.
    fn render(w: &Rc<Widgets>, entries: Option<&[EntryView]>) {
        while let Some(child) = w.entry_list.first_child() {
            w.entry_list.remove(&child);
        }

        let Some(entries) = entries else {
            let notice = ui::text(
                "Clipboard history unavailable",
                ui::Text::Body,
                ui::Tone::Muted,
            );
            notice.add_css_class("section-empty");
            w.entry_list.append(&notice);
            w.section.summary.set_label("Unavailable");
            w.section.set_open(false);
            w.section.revealer.set_sensitive(false);
            return;
        };

        w.section.revealer.set_sensitive(true);
        w.clear_btn.set_sensitive(!entries.is_empty());

        if entries.is_empty() {
            w.section.summary.set_label("");
            let empty = ui::text("No clipboard history", ui::Text::Body, ui::Tone::Muted);
            empty.add_css_class("section-empty");
            w.entry_list.append(&empty);
            return;
        }

        let count = entries.len();
        w.section.summary.set_label(&format!(
            "{count} item{}",
            if count == 1 { "" } else { "s" }
        ));
        for entry in entries {
            w.entry_list
                .append(&Self::build_entry_row(entry, w.clone()));
        }
    }

    fn build_entry_row(entry: &EntryView, w: Rc<Widgets>) -> gtk4::Button {
        let (btn, _) = ui::row_button("", &entry.preview, "");

        // Clicking puts the entry back on the selection and collapses the
        // list. The set is a Wayland request, not a subprocess, so there is
        // nothing to wait for and nothing to spawn.
        let id = entry.id;
        btn.connect_clicked(move |_| {
            if let Some(svc) = crate::clipboard::service() {
                svc.restore(id);
            }
            w.section.set_open(false);
        });

        btn
    }

    // ── Public API ────────────────────────────────────────────────────────────

    /// Redraw from the current ring. Cheap now: the state is in this
    /// process, so an open costs a clone of at most ten previews.
    pub fn refresh(&self) {
        let entries = self.service.as_ref().map(|s| s.entries());
        Self::render(&self.widgets, entries.as_deref());
    }

    /// Switch into page mode: reveal detail immediately, hide the summary
    /// toggle row.
    pub fn expand_for_page(&self) {
        self.widgets.section.show_as_page();
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.widgets.section.root
    }
}
