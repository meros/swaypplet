use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;

use gtk4::prelude::*;
use gtk4::{Box, Label, ListBox, ListBoxRow, Orientation, Switch};

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

        let type_lbl = Label::builder()
            .label(&iface.iface_type)
            .build();
        type_lbl.add_css_class("network-signal");

        let switch = Switch::builder()
            .active(iface.enabled)
            .valign(gtk4::Align::Center)
            .build();

        {
            let device = iface.device.clone();
            let state_c = state.clone();
            let list_c = list.clone();
            switch.connect_state_set(move |_sw, active| {
                let (tx, rx) = mpsc::channel::<NmResult>();
                if active {
                    device_connect_async(device.clone(), tx);
                } else {
                    device_disconnect_async(device.clone(), tx);
                }

                let state_poll = state_c.clone();
                let list_poll = list_c.clone();
                glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
                    match rx.try_recv() {
                        Ok(_) => {
                            let interfaces = get_network_interfaces();
                            state_poll.borrow_mut().interfaces = interfaces;
                            rebuild_iface_list(&list_poll, &state_poll);
                            glib::ControlFlow::Break
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
                    }
                });

                glib::Propagation::Proceed
            });
        }

        row_box.append(&icon_lbl);
        row_box.append(&name_lbl);
        row_box.append(&type_lbl);
        row_box.append(&switch);

        let list_row = ListBoxRow::builder().build();
        list_row.set_child(Some(&row_box));
        list_row.add_css_class("network-row");
        list.append(&list_row);
    }
}
