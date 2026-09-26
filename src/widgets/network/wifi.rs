//! One network's row, kept for as long as the network is in the list.
//!
//! The list used to be torn down and rebuilt on every refresh: a scan
//! result, a join, the section coming back on screen. A half-typed password
//! vanished with its row, rows jumped under the pointer as signals moved,
//! and the list went empty for the length of a scan. Now each network has
//! one row, keyed by SSID, updated in place (`update`); the section only
//! adds, removes and — rarely — reorders them (`model::settle`).

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{ListBoxRow, PasswordEntry, Revealer, RevealerTransitionType, Spinner};

use crate::services::network::model::{self, Failure, Snapshot};
use crate::services::network::{WifiNetwork, signal_icon};
use crate::ui;

use super::{set_signal_glyph, signal_tone};

/// What a row asks the section to do.
pub enum Ask {
    /// Bring up the saved connection with this id.
    Saved(String),
    /// Join: SSID, password (empty for open), security label.
    Join(String, String, String),
}

pub struct WifiRow {
    pub list_row: ListBoxRow,
    row: ui::Row,
    spinner: Spinner,
    pw: Revealer,
    pw_entry: PasswordEntry,
    pw_note: gtk4::Label,
    /// The network as last drawn, for the click.
    net: Rc<RefCell<WifiNetwork>>,
    /// The saved networks as `(id, ssid)`, for the click: a saved network
    /// joins by its stored connection, whose name need not be its SSID.
    saved: Rc<RefCell<Vec<(String, String)>>>,
}

impl WifiRow {
    pub fn new(net: &WifiNetwork, snap: &Snapshot, ask: Rc<dyn Fn(Ask)>) -> WifiRow {
        let body = ui::vbox(1);
        let row = ui::row("", &net.ssid, "");
        row.icon.set_visible(true);
        let spinner = Spinner::new();
        spinner.set_visible(false);
        row.end.append(&spinner);
        body.append(&row.root);

        let pw = ui::revealer(
            RevealerTransitionType::SlideDown,
            crate::tokens::motion::EXPAND,
        );
        let pw_box = ui::vbox(1);
        pw_box.add_css_class("network-hero-line");
        let pw_line = ui::hbox(2);
        let pw_entry = PasswordEntry::builder()
            .hexpand(true)
            .placeholder_text("Password")
            .show_peek_icon(true)
            .build();
        ui::entry::adopt(&pw_entry, ui::FieldSize::Normal);
        let join = ui::button_with(ui::Face::Label("Join"), ui::Kind::Primary, ui::Size::Small);
        pw_line.append(&pw_entry);
        pw_line.append(&join);
        let pw_note = ui::text("", ui::Text::Caption, ui::Tone::Danger);
        pw_note.set_xalign(0.0);
        pw_note.set_visible(false);
        pw_box.append(&pw_line);
        pw_box.append(&pw_note);
        pw.set_child(Some(&pw_box));
        body.append(&pw);

        let list_row = ListBoxRow::builder().child(&body).build();
        let this = WifiRow {
            list_row,
            row,
            spinner,
            pw,
            pw_entry,
            pw_note,
            net: Rc::new(RefCell::new(net.clone())),
            saved: Rc::default(),
        };

        // Join from the field: the button and Enter.
        {
            let (net, entry, ask) = (this.net.clone(), this.pw_entry.clone(), ask.clone());
            let go: Rc<dyn Fn()> = Rc::new(move || {
                let n = net.borrow().clone();
                let password = entry.text().to_string();
                if !password.is_empty() {
                    ask(Ask::Join(n.ssid, password, n.security));
                }
            });
            let g = go.clone();
            join.connect_clicked(move |_| g());
            this.pw_entry.connect_activate(move |_| go());
        }

        // A click on the row: a saved or open network joins at once, a
        // secured stranger opens its password field.
        {
            let (net, pw, entry) = (this.net.clone(), this.pw.clone(), this.pw_entry.clone());
            let saved_c = this.saved.clone();
            let click = gtk4::GestureClick::new();
            click.connect_released(move |_, _, _, _| {
                // Copies, not borrows: the ask redraws this very row.
                let n = net.borrow().clone();
                if n.in_use {
                    return;
                }
                let saved_id = saved_c
                    .borrow()
                    .iter()
                    .find(|(_, s)| *s == n.ssid)
                    .map(|(id, _)| id.clone());
                if let Some(id) = saved_id {
                    ask(Ask::Saved(id));
                } else if n.security.is_empty() {
                    ask(Ask::Join(n.ssid.clone(), String::new(), String::new()));
                } else {
                    let open = !pw.reveals_child();
                    pw.set_reveal_child(open);
                    if open {
                        entry.grab_focus();
                    }
                }
            });
            this.row.root.add_controller(click);
        }

        this.update(net, snap, None, None);
        this
    }

    /// Draw `net` as the snapshot has it, with the join in progress (its
    /// stage) or the failure, if they are this network's.
    pub fn update(
        &self,
        net: &WifiNetwork,
        snap: &Snapshot,
        joining: Option<&str>,
        failure: Option<&Failure>,
    ) {
        *self.net.borrow_mut() = net.clone();
        *self.saved.borrow_mut() = snap
            .saved
            .iter()
            .map(|sv| (sv.id.clone(), sv.ssid.clone()))
            .collect();

        set_signal_glyph(
            &self.row.icon,
            signal_icon(net.signal),
            signal_tone(net.signal),
        );
        self.row
            .icon
            .set_tooltip_text(Some(&format!("Signal {}%", net.signal)));
        if self.row.title.label() != net.ssid {
            self.row.title.set_label(&net.ssid);
        }

        let mine = failure.filter(|f| f.ssid == net.ssid);
        // The device's own stage first; the join just asked for says
        // "Connecting…" until the device has one, unless it already failed.
        let stage = snap
            .activating
            .as_ref()
            .filter(|a| a.ssid == net.ssid)
            .map(|a| a.stage)
            .or_else(|| {
                (mine.is_none() && joining == Some(net.ssid.as_str())).then_some("Connecting…")
            });

        let (text, tone) = match (stage, mine) {
            (Some(s), _) => (s.to_string(), ui::Tone::Accent),
            (None, Some(f)) => (f.text.clone(), ui::Tone::Danger),
            (None, None) => (model::row_subtitle(net), ui::Tone::Muted),
        };
        self.row.subtitle.set_label(&text);
        self.row.subtitle.set_visible(true);
        ui::set_tone(&self.row.subtitle, tone);

        let busy = stage.is_some();
        self.spinner.set_visible(busy);
        if busy {
            self.spinner.start();
        } else {
            self.spinner.stop();
        }
        ui::set_busy(&self.row.root, busy);
        ui::set_selected(&self.row.root, net.in_use);

        // A failure a password fixes opens the field, with the reason
        // beside it, and keeps whatever was typed.
        match mine {
            Some(f) if f.needs_password => {
                self.pw_note.set_label(&f.text);
                self.pw_note.set_visible(true);
                if !self.pw.reveals_child() {
                    self.pw.set_reveal_child(true);
                    self.pw_entry.grab_focus();
                }
            }
            _ => self.pw_note.set_visible(false),
        }
        if net.in_use || busy {
            self.pw.set_reveal_child(false);
        }
    }
}
