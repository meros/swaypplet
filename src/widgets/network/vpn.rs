use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;

use gtk4::prelude::*;
use gtk4::{Box, Button, Label, ListBox, ListBoxRow, Orientation, Spinner};

use super::backend::*;
use super::NetworkState;

pub fn rebuild_vpn_list(list: &ListBox, state: &Rc<RefCell<NetworkState>>) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }

    let vpns = state.borrow().vpns.clone();

    if vpns.is_empty() {
        return;
    }

    for vpn in vpns {
        let row_box = Box::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .margin_top(4)
            .margin_bottom(4)
            .margin_start(4)
            .margin_end(4)
            .build();

        let icon_lbl = Label::builder().label(ICON_VPN).build();
        icon_lbl.add_css_class("network-icon");
        if vpn.active {
            icon_lbl.add_css_class("network-vpn-active");
        }

        let name_lbl = Label::builder()
            .label(&vpn.name)
            .halign(gtk4::Align::Start)
            .hexpand(true)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .build();
        name_lbl.add_css_class("network-ssid");
        if vpn.active {
            name_lbl.add_css_class("network-active");
        }

        let spinner = Spinner::new();
        spinner.set_visible(false);

        let status_lbl = Label::builder().label("").build();
        status_lbl.add_css_class("network-conn-status");
        status_lbl.set_visible(false);

        let btn_label = if vpn.active { "Disconnect" } else { "Connect" };
        let action_btn = Button::builder().label(btn_label).build();
        action_btn.add_css_class("network-connect-btn");

        {
            let name_clone = vpn.name.clone();
            let is_active = vpn.active;
            let btn_c = action_btn.clone();
            let spinner_c = spinner.clone();
            let status_c = status_lbl.clone();
            action_btn.connect_clicked(move |_| {
                btn_c.set_sensitive(false);
                spinner_c.set_visible(true);
                spinner_c.start();
                status_c.set_visible(false);

                let (tx, rx) = mpsc::channel::<NmResult>();
                if is_active {
                    vpn_down_async(name_clone.clone(), tx);
                } else {
                    vpn_up_async(name_clone.clone(), tx);
                }

                let btn_poll = btn_c.clone();
                let spinner_poll = spinner_c.clone();
                let status_poll = status_c.clone();
                let was_active = is_active;
                glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
                    match rx.try_recv() {
                        Ok(result) => {
                            spinner_poll.stop();
                            spinner_poll.set_visible(false);
                            btn_poll.set_sensitive(true);
                            apply_nm_result(&status_poll, &result);
                            if matches!(result, NmResult::Success) {
                                btn_poll.set_label(if was_active {
                                    "Connect"
                                } else {
                                    "Disconnect"
                                });
                            }
                            auto_hide_status(&status_poll);
                            glib::ControlFlow::Break
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            spinner_poll.stop();
                            spinner_poll.set_visible(false);
                            btn_poll.set_sensitive(true);
                            glib::ControlFlow::Break
                        }
                    }
                });
            });
        }

        row_box.append(&icon_lbl);
        row_box.append(&name_lbl);
        row_box.append(&spinner);
        row_box.append(&status_lbl);
        row_box.append(&action_btn);

        let list_row = ListBoxRow::builder().build();
        list_row.set_child(Some(&row_box));
        list_row.add_css_class("network-row");
        if vpn.active {
            list_row.add_css_class("network-vpn-connected");
        }
        list.append(&list_row);
    }
}
