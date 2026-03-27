use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;

use gtk4::prelude::*;
use gtk4::{Box, Label, ListBox, ListBoxRow, Orientation, Spinner, Switch};

use super::backend::*;
use super::NetworkState;

/// Rebuild the interface list from current state. Single implementation used
/// both from `NetworkSection` methods and async polling callbacks.
pub fn rebuild_iface_list(list: &ListBox, state: &Rc<RefCell<NetworkState>>) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }

    let interfaces = state.borrow().interfaces.clone();

    if interfaces.is_empty() {
        let empty_lbl = Label::builder()
            .label("No interfaces found")
            .halign(gtk4::Align::Start)
            .build();
        empty_lbl.add_css_class("network-placeholder");
        let row = ListBoxRow::builder().build();
        row.set_child(Some(&empty_lbl));
        row.add_css_class("network-row");
        list.append(&row);
        return;
    }

    for iface in interfaces {
        let row_box = Box::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .margin_top(4)
            .margin_bottom(4)
            .margin_start(4)
            .margin_end(4)
            .build();

        let icon_lbl = Label::builder()
            .label(iface_type_icon(&iface.iface_type))
            .build();
        icon_lbl.add_css_class("network-icon");

        let name_lbl = Label::builder()
            .label(&iface.device)
            .halign(gtk4::Align::Start)
            .hexpand(true)
            .build();
        name_lbl.add_css_class("network-ssid");

        let friendly_type = match iface.iface_type.as_str() {
            "wifi" => "WiFi",
            "ethernet" => "Ethernet",
            "wireguard" => "WireGuard",
            "bridge" => "Bridge",
            _ => &iface.iface_type,
        };
        let type_lbl = Label::builder()
            .label(friendly_type)
            .build();
        type_lbl.add_css_class("network-signal");

        let spinner = Spinner::new();
        spinner.set_visible(false);

        let switch = Switch::builder()
            .active(iface.enabled)
            .valign(gtk4::Align::Center)
            .build();

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
                row_c.add_css_class("network-connecting");

                let (tx, rx) = mpsc::channel::<NmResult>();
                if active {
                    device_connect_async(device.clone(), tx);
                } else {
                    device_disconnect_async(device.clone(), tx);
                }

                let state_poll = state_c.clone();
                let list_poll = list_c.clone();
                let spinner_poll = spinner_c.clone();
                let switch_poll = switch_c.clone();
                let row_poll = row_c.clone();
                glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
                    let stop_spinner = || {
                        spinner_poll.stop();
                        spinner_poll.set_visible(false);
                        row_poll.remove_css_class("network-connecting");
                    };
                    match rx.try_recv() {
                        Ok(NmResult::Success) => {
                            stop_spinner();
                            let interfaces = get_network_interfaces();
                            state_poll.borrow_mut().interfaces = interfaces;
                            rebuild_iface_list(&list_poll, &state_poll);
                            glib::ControlFlow::Break
                        }
                        Ok(NmResult::Failure(_)) => {
                            stop_spinner();
                            switch_poll.set_sensitive(true);
                            switch_poll.set_active(!active);
                            glib::ControlFlow::Break
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            stop_spinner();
                            switch_poll.set_sensitive(true);
                            glib::ControlFlow::Break
                        }
                    }
                });

                glib::Propagation::Proceed
            });
        }

        row_box.append(&icon_lbl);
        row_box.append(&name_lbl);
        row_box.append(&type_lbl);
        row_box.append(&spinner);
        row_box.append(&switch);

        let list_row = ListBoxRow::builder().build();
        list_row.set_child(Some(&row_box));
        list_row.add_css_class("network-row");
        if iface.enabled {
            list_row.add_css_class("network-row-active");
        }
        list.append(&list_row);
    }
}
