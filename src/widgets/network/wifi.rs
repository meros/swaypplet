use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;

use gtk4::prelude::*;
use gtk4::{Box, Button, Entry, Label, ListBox, ListBoxRow, Orientation, PasswordEntry, Revealer, RevealerTransitionType, Spinner};

use super::backend::*;
use super::NetworkState;

// ── WiFi list builder ─────────────────────────────────────────────────────────

pub fn rebuild_wifi_list(list: &ListBox, state: &Rc<RefCell<NetworkState>>) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }

    let (networks, show_all) = {
        let s = state.borrow();
        (s.networks.clone(), s.show_all)
    };

    let total = networks.len();
    let visible_count = if show_all { total } else { total.min(MAX_VISIBLE_NETWORKS) };

    for network in networks.iter().take(visible_count) {
        let list_row = build_wifi_row(network);
        list.append(&list_row);
    }

    if total > MAX_VISIBLE_NETWORKS {
        let btn_label = if show_all {
            "Show fewer".to_string()
        } else {
            format!("Show all ({})", total)
        };
        let more_btn = Button::builder()
            .label(&btn_label)
            .halign(gtk4::Align::Center)
            .build();
        more_btn.add_css_class("network-show-all-btn");

        let state_c = state.clone();
        let list_c = list.clone();
        more_btn.connect_clicked(move |_| {
            state_c.borrow_mut().show_all = !state_c.borrow().show_all;
            rebuild_wifi_list(&list_c, &state_c);
        });

        let row = ListBoxRow::builder().build();
        row.set_child(Some(&more_btn));
        row.add_css_class("network-row");
        list.append(&row);
    }

    build_hidden_network_row(list);
}

fn build_wifi_row(network: &WifiNetwork) -> ListBoxRow {
    let connect_area = Box::builder()
        .orientation(Orientation::Vertical)
        .spacing(4)
        .build();

    let row_box = Box::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .margin_top(4)
        .margin_bottom(4)
        .margin_start(4)
        .margin_end(4)
        .build();

    let signal_lbl = Label::builder()
        .label(signal_icon(network.signal))
        .tooltip_text(format!("{}%", network.signal))
        .build();
    signal_lbl.add_css_class("network-icon");

    let ssid_lbl = Label::builder()
        .label(&network.ssid)
        .halign(gtk4::Align::Start)
        .hexpand(true)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .build();
    ssid_lbl.add_css_class("network-ssid");
    if network.in_use {
        ssid_lbl.add_css_class("network-active");
    }

    let lock_lbl = Label::builder()
        .label(if network.security.is_empty() || network.security == "--" { "" } else { ICON_LOCK })
        .build();
    lock_lbl.add_css_class("network-security");

    row_box.append(&signal_lbl);
    row_box.append(&ssid_lbl);
    row_box.append(&lock_lbl);
    connect_area.append(&row_box);

    if !network.in_use {
        let needs_password =
            !network.security.is_empty() && network.security != "--" && !network.is_known;

        if network.is_known {
            let (btn_row, spinner, status_lbl, connect_btn) = make_connect_row();

            let forget_btn = Button::builder().label("Forget").build();
            forget_btn.add_css_class("network-forget-btn");
            {
                let ssid = network.ssid.clone();
                forget_btn.connect_clicked(move |btn| {
                    forget_network(&ssid);
                    if let Some(row) = btn.ancestor(ListBoxRow::static_type()) {
                        row.set_sensitive(false);
                    }
                });
            }

            // Insert forget before connect.
            btn_row.prepend(&forget_btn);
            connect_area.append(&btn_row);

            let ssid = network.ssid.clone();
            wire_connect(&connect_btn, &spinner, &status_lbl, move |tx| {
                connect_known_async(ssid.clone(), tx);
            });
        } else if needs_password {
            let pw_revealer = Revealer::builder()
                .transition_type(RevealerTransitionType::SlideDown)
                .transition_duration(150)
                .reveal_child(false)
                .build();

            let pw_area = Box::builder()
                .orientation(Orientation::Vertical)
                .spacing(4)
                .build();

            let pw_row = Box::builder()
                .orientation(Orientation::Horizontal)
                .spacing(6)
                .build();

            let pw_entry = PasswordEntry::builder()
                .hexpand(true)
                .placeholder_text("Password")
                .show_peek_icon(true)
                .build();
            pw_entry.add_css_class("network-password-entry");

            let (fb_row, spinner, status_lbl, connect_btn) = make_connect_row();

            pw_row.append(&pw_entry);
            pw_row.append(&connect_btn);
            pw_area.append(&pw_row);
            pw_area.append(&fb_row);

            pw_revealer.set_child(Some(&pw_area));
            connect_area.append(&pw_revealer);

            // Enter key triggers connect.
            {
                let btn_enter = connect_btn.clone();
                pw_entry.connect_activate(move |_| btn_enter.emit_clicked());
            }

            let ssid = network.ssid.clone();
            let pw_c = pw_entry.clone();
            wire_connect(&connect_btn, &spinner, &status_lbl, move |tx| {
                connect_new_async(ssid.clone(), pw_c.text().to_string(), false, tx);
            });

            let click = gtk4::GestureClick::new();
            {
                let rev_c = pw_revealer.clone();
                let entry_c = pw_entry.clone();
                click.connect_released(move |_, _, _, _| {
                    let visible = rev_c.reveals_child();
                    rev_c.set_reveal_child(!visible);
                    if !visible {
                        entry_c.grab_focus();
                    }
                });
            }
            row_box.add_controller(click);
        } else {
            let (btn_row, spinner, status_lbl, connect_btn) = make_connect_row();
            connect_area.append(&btn_row);

            let ssid = network.ssid.clone();
            wire_connect(&connect_btn, &spinner, &status_lbl, move |tx| {
                connect_new_async(ssid.clone(), String::new(), false, tx);
            });
        }
    }

    let list_row = ListBoxRow::builder().build();
    list_row.set_child(Some(&connect_area));
    list_row.add_css_class("network-row");
    list_row
}

// ── Hidden network form ───────────────────────────────────────────────────────

fn build_hidden_network_row(list: &ListBox) {
    let outer = Box::builder()
        .orientation(Orientation::Vertical)
        .spacing(4)
        .build();

    let hidden_revealer = Revealer::builder()
        .transition_type(RevealerTransitionType::SlideDown)
        .transition_duration(150)
        .reveal_child(false)
        .build();

    let form = Box::builder()
        .orientation(Orientation::Vertical)
        .spacing(6)
        .margin_top(4)
        .build();
    form.add_css_class("network-hidden-form");

    let ssid_entry = Entry::builder()
        .placeholder_text("Network name (SSID)")
        .hexpand(true)
        .build();
    ssid_entry.add_css_class("network-password-entry");

    let pw_entry = PasswordEntry::builder()
        .placeholder_text("Password (leave empty for open)")
        .show_peek_icon(true)
        .hexpand(true)
        .build();
    pw_entry.add_css_class("network-password-entry");

    let (btn_row, spinner, status_lbl, connect_btn) = make_connect_row();

    form.append(&ssid_entry);
    form.append(&pw_entry);
    form.append(&btn_row);
    hidden_revealer.set_child(Some(&form));

    // Enter in password field triggers connect.
    {
        let btn_enter = connect_btn.clone();
        pw_entry.connect_activate(move |_| btn_enter.emit_clicked());
    }

    let ssid_c = ssid_entry.clone();
    let pw_c = pw_entry.clone();
    wire_connect(&connect_btn, &spinner, &status_lbl, move |tx| {
        let ssid = ssid_c.text().to_string();
        if ssid.is_empty() {
            return;
        }
        connect_new_async(ssid, pw_c.text().to_string(), true, tx);
    });

    let toggle_btn = Button::builder()
        .label("Connect to hidden network")
        .halign(gtk4::Align::Center)
        .build();
    toggle_btn.add_css_class("network-show-all-btn");
    {
        let rev_c = hidden_revealer.clone();
        let ssid_focus = ssid_entry.clone();
        toggle_btn.connect_clicked(move |_| {
            let visible = rev_c.reveals_child();
            rev_c.set_reveal_child(!visible);
            if !visible {
                ssid_focus.grab_focus();
            }
        });
    }

    outer.append(&toggle_btn);
    outer.append(&hidden_revealer);

    let list_row = ListBoxRow::builder().build();
    list_row.set_child(Some(&outer));
    list_row.add_css_class("network-row");
    list.append(&list_row);
}

// ── Unified connect wiring ────────────────────────────────────────────────────

/// Create a standard connect-feedback row: spinner + status label + connect button.
fn make_connect_row() -> (Box, Spinner, Label, Button) {
    let row = Box::builder()
        .orientation(Orientation::Horizontal)
        .halign(gtk4::Align::End)
        .spacing(6)
        .build();

    let spinner = Spinner::new();
    spinner.set_visible(false);

    let status_lbl = Label::builder().label("").build();
    status_lbl.add_css_class("network-conn-status");
    status_lbl.set_visible(false);

    let connect_btn = Button::builder().label("Connect").build();
    connect_btn.add_css_class("network-connect-btn");

    row.append(&spinner);
    row.append(&status_lbl);
    row.append(&connect_btn);

    (row, spinner, status_lbl, connect_btn)
}

/// Wire a connect button to an async nmcli operation with spinner + status feedback.
/// `dispatch` is called with a sender; it should spawn the backend async function.
fn wire_connect<F>(btn: &Button, spinner: &Spinner, status_lbl: &Label, dispatch: F)
where
    F: Fn(mpsc::Sender<NmResult>) + 'static,
{
    let btn_c = btn.clone();
    let spinner_c = spinner.clone();
    let status_c = status_lbl.clone();

    btn.connect_clicked(move |_| {
        btn_c.set_sensitive(false);
        spinner_c.set_visible(true);
        spinner_c.start();
        status_c.set_visible(false);

        let (tx, rx) = mpsc::channel::<NmResult>();
        dispatch(tx);

        let btn_poll = btn_c.clone();
        let spinner_poll = spinner_c.clone();
        let status_poll = status_c.clone();

        glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
            match rx.try_recv() {
                Ok(result) => {
                    spinner_poll.stop();
                    spinner_poll.set_visible(false);
                    btn_poll.set_sensitive(true);
                    apply_nm_result(&status_poll, &result);
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
