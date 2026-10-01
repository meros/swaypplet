//! The wired and other physical adapters, under "Advanced": one switch
//! each, to bring the device up or down, and on a wired adapter a Block
//! button that tells NetworkManager to leave it alone until it is unplugged
//! (`services::network::is_banned`). A blocked adapter stays listed with
//! Unblock in the switch's place, since NM no longer brings it up or down.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{ListBox, ListBoxRow};

use super::set_signal_glyph;
use crate::services::network::{NetworkInterface, iface_type_icon};
use crate::ui;

/// What a row asks for.
pub enum AdapterAsk {
    /// Bring the device up (`true`) or down.
    Toggle(String, bool),
    /// Block (`true`) or unblock the adapter.
    Ban(String, bool),
}

pub struct Adapters {
    pub list: ListBox,
    shown: RefCell<Vec<(String, String, bool, bool)>>,
    syncing: Rc<Cell<bool>>,
    ask: Rc<dyn Fn(AdapterAsk)>,
}

impl Adapters {
    pub fn new(ask: Rc<dyn Fn(AdapterAsk)>) -> Adapters {
        Adapters {
            list: ui::list(),
            shown: RefCell::default(),
            syncing: Rc::default(),
            ask,
        }
    }

    pub fn update(&self, interfaces: &[NetworkInterface]) {
        let now: Vec<(String, String, bool, bool)> = interfaces
            .iter()
            .map(|i| (i.device.clone(), i.iface_type.clone(), i.enabled, i.banned))
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
            let root = if iface.banned {
                self.banned_row(iface, kind)
            } else {
                self.switch_row(iface, kind)
            };
            self.list
                .append(&ListBoxRow::builder().child(&root).build());
        }
        self.syncing.set(false);
    }

    fn switch_row(&self, iface: &NetworkInterface, kind: &str) -> gtk4::Box {
        let (r, sw) = ui::switch_row(&iface.device, kind);
        set_signal_glyph(&r.icon, iface_type_icon(&iface.iface_type), ui::Tone::Fg);
        r.icon.set_visible(true);
        ui::set_selected(&r.root, iface.enabled);
        sw.set_active(iface.enabled);
        sw.set_state(iface.enabled);
        {
            let (dev, syncing, ask) =
                (iface.device.clone(), self.syncing.clone(), self.ask.clone());
            sw.connect_state_set(move |_, active| {
                if !syncing.get() {
                    ask(AdapterAsk::Toggle(dev.clone(), active));
                }
                gtk4::glib::Propagation::Proceed
            });
        }
        if iface.iface_type == "ethernet" {
            let block = self.ban_button("Block", &iface.device, true);
            block.set_tooltip_text(Some("Use Wi-Fi instead until this adapter is unplugged"));
            r.end.prepend(&block);
        }
        r.root
    }

    fn banned_row(&self, iface: &NetworkInterface, kind: &str) -> gtk4::Box {
        let r = ui::row(
            iface_type_icon(&iface.iface_type),
            &iface.device,
            &format!("{kind} · blocked until unplugged"),
        );
        set_signal_glyph(
            &r.icon,
            iface_type_icon(&iface.iface_type),
            ui::Tone::Warning,
        );
        r.end
            .append(&self.ban_button("Unblock", &iface.device, false));
        r.root
    }

    fn ban_button(&self, label: &str, device: &str, ban: bool) -> gtk4::Button {
        let b = ui::button_with(ui::Face::Label(label), ui::Kind::Flat, ui::Size::Small);
        let (dev, ask) = (device.to_string(), self.ask.clone());
        b.connect_clicked(move |_| ask(AdapterAsk::Ban(dev.clone(), ban)));
        b
    }
}
