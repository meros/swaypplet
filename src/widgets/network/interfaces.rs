//! The wired and other physical adapters, under "Advanced": one switch
//! each, to bring the device up or down.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{ListBox, ListBoxRow};

use super::set_signal_glyph;
use crate::services::network::{NetworkInterface, iface_type_icon};
use crate::ui;

pub struct Adapters {
    pub list: ListBox,
    shown: RefCell<Vec<(String, String, bool)>>,
    syncing: Rc<Cell<bool>>,
    on_toggle: Rc<dyn Fn(String, bool)>,
}

impl Adapters {
    pub fn new(on_toggle: Rc<dyn Fn(String, bool)>) -> Adapters {
        Adapters {
            list: ui::list(),
            shown: RefCell::default(),
            syncing: Rc::default(),
            on_toggle,
        }
    }

    pub fn update(&self, interfaces: &[NetworkInterface]) {
        let now: Vec<(String, String, bool)> = interfaces
            .iter()
            .map(|i| (i.device.clone(), i.iface_type.clone(), i.enabled))
            .collect();
        if *self.shown.borrow() == now {
            return;
        }
        *self.shown.borrow_mut() = now;
        while let Some(c) = self.list.first_child() {
            self.list.remove(&c);
        }
        self.syncing.set(true);
        for iface in interfaces {
            let kind = match iface.iface_type.as_str() {
                "ethernet" => "Ethernet",
                "wireguard" => "WireGuard",
                "bluetooth" => "Bluetooth",
                other => other,
            };
            let (r, sw) = ui::switch_row(&iface.device, kind);
            set_signal_glyph(&r.icon, iface_type_icon(&iface.iface_type), ui::Tone::Fg);
            r.icon.set_visible(true);
            ui::set_selected(&r.root, iface.enabled);
            sw.set_active(iface.enabled);
            sw.set_state(iface.enabled);
            {
                let (dev, syncing, on) = (
                    iface.device.clone(),
                    self.syncing.clone(),
                    self.on_toggle.clone(),
                );
                sw.connect_state_set(move |_, active| {
                    if !syncing.get() {
                        on(dev.clone(), active);
                    }
                    gtk4::glib::Propagation::Proceed
                });
            }
            self.list
                .append(&ListBoxRow::builder().child(&r.root).build());
        }
        self.syncing.set(false);
    }
}
