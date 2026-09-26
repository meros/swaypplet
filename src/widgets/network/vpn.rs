use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{ListBox, ListBoxRow, Spinner};

use super::{NetworkState, apply_nm_result, auto_hide_status, set_signal_glyph};
use crate::services::network::*;
use crate::spawn::spawn_work;
use crate::ui;

pub fn rebuild_vpn_list(list: &ListBox, state: &Rc<RefCell<NetworkState>>) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }

    let vpns = state.borrow().vpns.clone();

    if vpns.is_empty() {
        return;
    }

    for vpn in vpns {
        let r = ui::row(ICON_VPN, &vpn.name, "");
        set_signal_glyph(&r.icon, ICON_VPN, ui::Tone::Fg);
        // A connected VPN is the selected row.
        ui::set_selected(&r.root, vpn.active);

        let badge_lbl = ui::badge(&vpn.vpn_type, ui::BadgeTone::Neutral);

        let spinner = Spinner::new();
        spinner.set_visible(false);

        let status_lbl = ui::text("", ui::Text::Label, ui::Tone::Faint);
        status_lbl.set_visible(false);

        let btn_label = if vpn.active { "Disconnect" } else { "Connect" };
        let action_btn = ui::button_with(
            ui::Face::Label(btn_label),
            ui::Kind::Secondary,
            ui::Size::Small,
        );

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

                let name_bg = name_clone.clone();
                let btn_poll = btn_c.clone();
                let spinner_poll = spinner_c.clone();
                let status_poll = status_c.clone();
                let was_active = is_active;
                spawn_work(
                    move || {
                        if was_active {
                            vpn_down(&name_bg)
                        } else {
                            vpn_up(&name_bg)
                        }
                    },
                    move |result| {
                        spinner_poll.stop();
                        spinner_poll.set_visible(false);
                        btn_poll.set_sensitive(true);
                        apply_nm_result(&status_poll, &result);
                        if matches!(result, NmResult::Success) {
                            btn_poll.set_label(if was_active { "Connect" } else { "Disconnect" });
                        }
                        auto_hide_status(&status_poll);
                    },
                );
            });
        }

        r.end.append(&badge_lbl);
        r.end.append(&spinner);
        r.end.append(&status_lbl);
        r.end.append(&action_btn);

        let list_row = ListBoxRow::builder().build();
        list_row.set_child(Some(&r.root));
        list.append(&list_row);
    }
}
