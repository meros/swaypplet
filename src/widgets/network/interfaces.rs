use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{ListBox, ListBoxRow, Spinner};

use super::NetworkState;
use super::backend::*;
use crate::spawn::spawn_work;
use crate::ui;

/// Rebuild the interface list from current state. Single implementation used
/// both from `NetworkSection` methods and async polling callbacks.
pub fn rebuild_iface_list(list: &ListBox, state: &Rc<RefCell<NetworkState>>) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }

    let interfaces = state.borrow().interfaces.clone();

    if interfaces.is_empty() {
        list.set_visible(false);
        return;
    }
    list.set_visible(true);

    for iface in interfaces {
        let friendly_type = match iface.iface_type.as_str() {
            "wifi" => "WiFi",
            "ethernet" => "Ethernet",
            "wireguard" => "WireGuard",
            "bridge" => "Bridge",
            _ => &iface.iface_type,
        };
        let r = ui::row("", &iface.device, friendly_type);
        set_signal_glyph(&r.icon, iface_type_icon(&iface.iface_type), ui::Tone::Fg);
        r.icon.set_visible(true);
        // An enabled adapter is the selected row.
        ui::set_selected(&r.root, iface.enabled);
        let row_box = r.root.clone();

        let spinner = Spinner::new();
        spinner.set_visible(false);

        let switch = ui::switch();
        switch.set_active(iface.enabled);

        {
            let device = iface.device.clone();
            let state_c = state.clone();
            let list_c = list.clone();
            let spinner_c = spinner.clone();
            let switch_c = switch.clone();
            let row_c = row_box.clone();
            switch.connect_state_set(move |_sw, active| {
                switch_c.set_sensitive(false);
                spinner_c.set_visible(true);
                spinner_c.start();
                ui::set_class(&row_c, "busy", true);

                let device_bg = device.clone();
                let state_poll = state_c.clone();
                let list_poll = list_c.clone();
                let spinner_poll = spinner_c.clone();
                let switch_poll = switch_c.clone();
                let row_poll = row_c.clone();
                spawn_work(
                    move || {
                        if active {
                            device_connect(&device_bg)
                        } else {
                            device_disconnect(&device_bg)
                        }
                    },
                    move |result| {
                        spinner_poll.stop();
                        spinner_poll.set_visible(false);
                        ui::set_class(&row_poll, "busy", false);
                        match result {
                            NmResult::Success => {
                                let interfaces = get_network_interfaces();
                                state_poll.borrow_mut().interfaces = interfaces;
                                rebuild_iface_list(&list_poll, &state_poll);
                            }
                            NmResult::Failure(_) => {
                                switch_poll.set_sensitive(true);
                                switch_poll.set_active(!active);
                            }
                        }
                    },
                );

                glib::Propagation::Proceed
            });
        }

        r.end.append(&spinner);
        r.end.append(&switch);

        let list_row = ListBoxRow::builder().build();
        list_row.set_child(Some(&row_box));
        list.append(&list_row);
    }
}
