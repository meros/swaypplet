//! The wired and other physical adapters, under "Advanced": one switch
//! each, to bring the device up or down, and on a wired adapter a Block
//! button that tells NetworkManager to leave it alone until it is unplugged
//! (`services::network::is_banned`). A blocked adapter stays listed with
//! Unblock in the switch's place, since NM no longer brings it up or down.
//!
//! Each row reads as the adapter's name ("Ethernet on Lenovo dock",
//! `services::network::naming`, or the one the person gave it) over the
//! chipset and the kernel name. Rename (`widgets::rename`) stores a name
//! under the adapter's MAC address.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{ListBox, ListBoxRow};

use super::set_signal_glyph;
use crate::services::network::{NetworkInterface, display_name, iface_type_icon};
use crate::ui;

/// What a row asks for.
pub enum AdapterAsk {
    /// Bring the device up (`true`) or down.
    Toggle(String, bool),
    /// Block (`true`) or unblock the adapter.
    Ban(String, bool),
    /// Store a name for the adapter with this MAC; empty clears it.
    Rename(String, String),
}

/// What a row draws, for telling whether the list changed.
type Shown = (String, String, bool, bool, String, Option<String>);

pub struct Adapters {
    pub list: ListBox,
    shown: RefCell<Vec<Shown>>,
    syncing: Rc<Cell<bool>>,
    ask: Rc<dyn Fn(AdapterAsk)>,
}

/// "Realtek RTL8153 · enp0s13f0u2u1", or the kernel name alone.
fn subtitle(lead: Option<&str>, device: &str) -> String {
    match lead {
        Some(lead) => format!("{lead} · {device}"),
        None => device.to_string(),
    }
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
        let now: Vec<Shown> = interfaces
            .iter()
            .map(|i| {
                (
                    i.device.clone(),
                    i.iface_type.clone(),
                    i.enabled,
                    i.banned,
                    display_name(&i.label, i.mac.as_deref()),
                    i.chipset.clone(),
                )
            })
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
            let name = display_name(&iface.label, iface.mac.as_deref());
            let r = if iface.banned {
                self.banned_row(iface, &name)
            } else {
                self.switch_row(iface, &name)
            };
            if let Some(mac) = &iface.mac {
                let (ask, mac) = (self.ask.clone(), mac.clone());
                r.end.prepend(&crate::widgets::rename::button(
                    &r.title,
                    &name,
                    &iface.label,
                    move |name| ask(AdapterAsk::Rename(mac.clone(), name)),
                ));
            }
            self.list
                .append(&ListBoxRow::builder().child(&r.root).build());
        }
        self.syncing.set(false);
    }

    fn switch_row(&self, iface: &NetworkInterface, name: &str) -> ui::Row {
        let (r, sw) = ui::switch_row(name, &subtitle(iface.chipset.as_deref(), &iface.device));
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
        r
    }

    fn banned_row(&self, iface: &NetworkInterface, name: &str) -> ui::Row {
        let r = ui::row(
            iface_type_icon(&iface.iface_type),
            name,
            &subtitle(Some("Blocked until unplugged"), &iface.device),
        );
        set_signal_glyph(
            &r.icon,
            iface_type_icon(&iface.iface_type),
            ui::Tone::Warning,
        );
        r.end
            .append(&self.ban_button("Unblock", &iface.device, false));
        r
    }

    fn ban_button(&self, label: &str, device: &str, ban: bool) -> gtk4::Button {
        let b = ui::button_with(ui::Face::Label(label), ui::Kind::Flat, ui::Size::Small);
        let (dev, ask) = (device.to_string(), self.ask.clone());
        b.connect_clicked(move |_| ask(AdapterAsk::Ban(dev.clone(), ban)));
        b
    }
}
