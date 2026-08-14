//! Clipboard history section.
//!
//! The rows come from [`crate::clipboard`], which watches the selection over
//! `ext-data-control-v1` in this process. This used to shell out to
//! `cliphist list` on every open, against a database no daemon was filling —
//! see that module's header for what that cost.

use std::rc::Rc;

use gtk4::prelude::*;

use crate::clipboard::{ClipboardService, EntryView};
use crate::icons;

// ── ClipboardSection ──────────────────────────────────────────────────────────

struct Widgets {
    summary_btn: gtk4::Button,
    summary_text: gtk4::Label,
    summary_arrow: gtk4::Label,
    detail_revealer: gtk4::Revealer,
    entry_list: gtk4::Box,
    clear_btn: gtk4::Button,
}

pub struct ClipboardSection {
    root: gtk4::Box,
    widgets: Rc<Widgets>,
    /// `None` on a compositor without the protocol; the section says so
    /// rather than showing an empty list that looks like an empty history.
    service: Option<Rc<ClipboardService>>,
}

impl ClipboardSection {
    pub fn new() -> Self {
        let root = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(6)
            .build();
        root.add_css_class("section");

        // ── Summary row (always visible, toggles detail revealer) ─────────────
        let summary_content = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(6)
            .build();

        let summary_icon = gtk4::Label::new(Some(icons::CLIPBOARD));
        summary_icon.add_css_class("section-summary-icon");

        let summary_text = gtk4::Label::new(Some("Clipboard"));
        summary_text.add_css_class("section-summary-label");
        summary_text.set_hexpand(true);
        summary_text.set_xalign(0.0);
        summary_text.set_ellipsize(gtk4::pango::EllipsizeMode::End);

        let summary_arrow = gtk4::Label::new(Some("▸"));
        summary_arrow.add_css_class("section-expand-arrow");

        summary_content.append(&summary_icon);
        summary_content.append(&summary_text);
        summary_content.append(&summary_arrow);

        let summary_btn = gtk4::Button::builder().child(&summary_content).build();
        summary_btn.add_css_class("section-summary");

        // ── Detail revealer (collapsed by default) ───────────────────────────
        let detail_revealer = gtk4::Revealer::builder()
            .transition_type(gtk4::RevealerTransitionType::SlideDown)
            .transition_duration(200)
            .reveal_child(false)
            .build();

        {
            let rev = detail_revealer.clone();
            let arrow = summary_arrow.clone();
            summary_btn.connect_clicked(move |_| {
                let revealed = rev.reveals_child();
                rev.set_reveal_child(!revealed);
                arrow.set_label(if revealed { "▸" } else { "▾" });
            });
        }

        root.append(&summary_btn);
        root.append(&detail_revealer);

        let detail_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(4)
            .build();
        detail_revealer.set_child(Some(&detail_box));

        let entry_list = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(2)
            .build();
        entry_list.add_css_class("device-list");
        detail_box.append(&entry_list);

        let clear_btn = gtk4::Button::with_label("Clear History");
        clear_btn.add_css_class("flat");
        detail_box.append(&clear_btn);

        let widgets = Rc::new(Widgets {
            summary_btn,
            summary_text,
            summary_arrow,
            detail_revealer,
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

        let section = ClipboardSection {
            root,
            widgets,
            service,
        };
        section.refresh();
        section
    }

    /// Draw `entries`, or the unavailable notice when there is no service.
    fn render(w: &Rc<Widgets>, entries: Option<&[EntryView]>) {
        while let Some(child) = w.entry_list.first_child() {
            w.entry_list.remove(&child);
        }

        let Some(entries) = entries else {
            let notice = gtk4::Label::new(Some("Clipboard history unavailable"));
            notice.set_xalign(0.0);
            notice.add_css_class("device-row");
            w.entry_list.append(&notice);
            w.summary_text.set_label("Unavailable");
            w.summary_arrow.set_label("▸");
            w.detail_revealer.set_reveal_child(false);
            w.detail_revealer.set_sensitive(false);
            return;
        };

        w.detail_revealer.set_sensitive(true);
        w.clear_btn.set_sensitive(!entries.is_empty());

        if entries.is_empty() {
            w.summary_text.set_label("Clipboard");
            let empty = gtk4::Label::new(Some("No clipboard history"));
            empty.set_xalign(0.0);
            empty.add_css_class("device-row");
            w.entry_list.append(&empty);
            return;
        }

        let count = entries.len();
        w.summary_text.set_label(&format!(
            "Clipboard · {count} item{}",
            if count == 1 { "" } else { "s" }
        ));
        for entry in entries {
            w.entry_list
                .append(&Self::build_entry_row(entry, w.clone()));
        }
    }

    fn build_entry_row(entry: &EntryView, w: Rc<Widgets>) -> gtk4::Box {
        let row = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(8)
            .build();
        row.add_css_class("device-row");
        row.set_focusable(true);
        row.set_can_focus(true);

        let preview_label = gtk4::Label::new(Some(&entry.preview));
        preview_label.set_hexpand(true);
        preview_label.set_xalign(0.0);
        preview_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        row.append(&preview_label);

        // Clicking puts the entry back on the selection and collapses the
        // list. The set is a Wayland request, not a subprocess, so there is
        // nothing to wait for and nothing to spawn.
        let id = entry.id;
        let gesture = gtk4::GestureClick::new();
        gesture.connect_released(move |_, _, _, _| {
            if let Some(svc) = crate::clipboard::service() {
                svc.restore(id);
            }
            w.detail_revealer.set_reveal_child(false);
            w.summary_arrow.set_label("▸");
        });
        row.add_controller(gesture);

        row
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
        self.widgets.summary_btn.set_visible(false);
        self.widgets.detail_revealer.set_transition_duration(0);
        self.widgets.detail_revealer.set_reveal_child(true);
        self.widgets.detail_revealer.set_transition_duration(200);
        self.widgets.summary_arrow.set_label("▾");
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }
}
