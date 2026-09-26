//! The VPNs: one switch row each, first-class rather than behind
//! "Advanced". After Wi-Fi itself, a VPN is the network setting that
//! changes most often in a day.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{ListBox, ListBoxRow, Switch};

use crate::services::network::{ICON_VPN, VpnConnection};
use crate::ui;

use super::set_signal_glyph;

pub struct VpnList {
    pub list: ListBox,
    rows: RefCell<Vec<(String, ui::Row, Switch)>>,
    /// Set while the switches are moved to match the snapshot, so the
    /// handlers know it was not a hand.
    syncing: Rc<Cell<bool>>,
    on_toggle: Rc<dyn Fn(String, bool)>,
    errors: RefCell<Vec<(String, String)>>,
}

impl VpnList {
    pub fn new(on_toggle: Rc<dyn Fn(String, bool)>) -> VpnList {
        VpnList {
            list: ui::list(),
            rows: RefCell::default(),
            syncing: Rc::default(),
            on_toggle,
            errors: RefCell::default(),
        }
    }

    /// Show `name`'s last failure under it, or clear it.
    pub fn set_error(&self, name: &str, error: Option<String>) {
        let mut e = self.errors.borrow_mut();
        e.retain(|(n, _)| n != name);
        if let Some(msg) = error {
            e.push((name.to_string(), msg));
        }
    }

    pub fn update(&self, vpns: &[VpnConnection]) {
        let same = {
            let rows = self.rows.borrow();
            rows.len() == vpns.len() && rows.iter().zip(vpns).all(|((n, ..), v)| *n == v.name)
        };
        if !same {
            while let Some(c) = self.list.first_child() {
                self.list.remove(&c);
            }
            let mut rows = self.rows.borrow_mut();
            rows.clear();
            for v in vpns {
                let (r, sw) = ui::switch_row(&v.name, &v.vpn_type);
                set_signal_glyph(&r.icon, ICON_VPN, ui::Tone::Fg);
                r.icon.set_visible(true);
                {
                    let (name, syncing, on) =
                        (v.name.clone(), self.syncing.clone(), self.on_toggle.clone());
                    sw.connect_state_set(move |_, active| {
                        if !syncing.get() {
                            on(name.clone(), active);
                        }
                        gtk4::glib::Propagation::Proceed
                    });
                }
                self.list
                    .append(&ListBoxRow::builder().child(&r.root).build());
                rows.push((v.name.clone(), r, sw));
            }
        }
        self.syncing.set(true);
        let errors = self.errors.borrow();
        for ((_, r, sw), v) in self.rows.borrow().iter().zip(vpns) {
            sw.set_active(v.active);
            sw.set_state(v.active);
            ui::set_selected(&r.root, v.active);
            match errors.iter().find(|(n, _)| *n == v.name) {
                Some((_, msg)) => {
                    r.subtitle.set_label(msg);
                    ui::set_tone(&r.subtitle, ui::Tone::Danger);
                }
                None => {
                    let text = if v.active {
                        format!("{} · Connected", v.vpn_type)
                    } else {
                        v.vpn_type.clone()
                    };
                    r.subtitle.set_label(&text);
                    ui::set_tone(&r.subtitle, ui::Tone::Muted);
                }
            }
            r.subtitle.set_visible(true);
        }
        self.syncing.set(false);
    }
}
