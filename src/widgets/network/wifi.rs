use std::cell::{Cell, RefCell};
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

    // "Show all" / "Show fewer" button when more networks exist.
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
            {
                let mut s = state_c.borrow_mut();
                s.show_all = !s.show_all;
            }
            rebuild_wifi_list(&list_c, &state_c);
        });

        let row = ListBoxRow::builder().build();
        row.set_child(Some(&more_btn));
        row.add_css_class("network-row");
        list.append(&row);
    }

    // "Connect to hidden network" button at the bottom.
    build_hidden_network_row(list, state);
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
    signal_lbl.add_css_class(signal_css_class(network.signal));

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
    if let Some(freq) = network.freq_mhz {
        let band_lbl = Label::builder()
            .label(freq_band_short(freq))
            .build();
        band_lbl.add_css_class("network-band");
        row_box.append(&band_lbl);
    }
    row_box.append(&lock_lbl);

    if network.in_use {
        let dot = Label::builder().label("●").build();
        dot.add_css_class("network-active-dot");
        row_box.prepend(&dot);
    }

    connect_area.append(&row_box);

    if !network.in_use {
        let needs_password =
            !network.security.is_empty() && network.security != "--" && !network.is_known;

        if network.is_known {
            let btn_row = Box::builder()
                .orientation(Orientation::Horizontal)
                .halign(gtk4::Align::End)
                .spacing(6)
                .build();

            let spinner = Spinner::new();
            spinner.set_visible(false);

            let status_lbl = Label::builder().label("").build();
            status_lbl.add_css_class("network-conn-status");
            status_lbl.set_visible(false);

            let forget_btn = Button::builder().label("Forget").build();
            forget_btn.add_css_class("network-forget-btn");
            {
                let ssid = network.ssid.clone();
                let confirmed = Rc::new(Cell::new(false));
                let confirmed_c = confirmed.clone();
                let btn_c = forget_btn.clone();
                forget_btn.connect_clicked(move |btn| {
                    if !confirmed.get() {
                        confirmed.set(true);
                        btn.set_label("Sure?");
                        btn.remove_css_class("network-forget-btn");
                        btn.add_css_class("network-forget-confirm-btn");
                        // Auto-revert after 3 seconds
                        let btn_revert = btn_c.clone();
                        let confirmed_revert = confirmed_c.clone();
                        glib::timeout_add_local_once(std::time::Duration::from_secs(3), move || {
                            if confirmed_revert.get() {
                                confirmed_revert.set(false);
                                btn_revert.set_label("Forget");
                                btn_revert.remove_css_class("network-forget-confirm-btn");
                                btn_revert.add_css_class("network-forget-btn");
                            }
                        });
                    } else {
                        confirmed.set(false);
                        forget_network(&ssid);
                        if let Some(row) = btn.ancestor(ListBoxRow::static_type()) {
                            row.set_sensitive(false);
                        }
                    }
                });
            }

            let connect_btn = Button::builder().label("Connect").build();
            connect_btn.add_css_class("network-connect-btn");

            btn_row.append(&spinner);
            btn_row.append(&status_lbl);
            btn_row.append(&forget_btn);
            btn_row.append(&connect_btn);
            connect_area.append(&btn_row);

            wire_connect_known(&connect_btn, &spinner, &status_lbl, network.ssid.clone());
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

            let connect_btn = Button::builder().label("Connect").build();
            connect_btn.add_css_class("network-connect-btn");

            pw_row.append(&pw_entry);
            pw_row.append(&connect_btn);
            pw_area.append(&pw_row);

            let fb_row = Box::builder()
                .orientation(Orientation::Horizontal)
                .halign(gtk4::Align::End)
                .spacing(6)
                .build();

            let spinner = Spinner::new();
            spinner.set_visible(false);

            let status_lbl = Label::builder().label("").build();
            status_lbl.add_css_class("network-conn-status");
            status_lbl.set_visible(false);

            fb_row.append(&spinner);
            fb_row.append(&status_lbl);
            pw_area.append(&fb_row);

            pw_revealer.set_child(Some(&pw_area));
            connect_area.append(&pw_revealer);

            wire_connect_new(&connect_btn, &pw_entry, &spinner, &status_lbl, network.ssid.clone(), false);

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
            let btn_row = Box::builder()
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

            btn_row.append(&spinner);
            btn_row.append(&status_lbl);
            btn_row.append(&connect_btn);
            connect_area.append(&btn_row);

            wire_connect_open(&connect_btn, &spinner, &status_lbl, network.ssid.clone());
        }
    }

    let list_row = ListBoxRow::builder().build();
    list_row.set_child(Some(&connect_area));
    list_row.add_css_class("network-row");
    if network.in_use {
        list_row.add_css_class("network-row-active");
    }
    list_row
}

// ── Hidden network form ───────────────────────────────────────────────────────

fn build_hidden_network_row(list: &ListBox, _state: &Rc<RefCell<NetworkState>>) {
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

    let btn_row = Box::builder()
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

    btn_row.append(&spinner);
    btn_row.append(&status_lbl);
    btn_row.append(&connect_btn);

    form.append(&ssid_entry);
    form.append(&pw_entry);
    form.append(&btn_row);
    hidden_revealer.set_child(Some(&form));

    // Wire connect button for hidden network.
    {
        let ssid_c = ssid_entry.clone();
        let pw_c = pw_entry.clone();
        let btn_c = connect_btn.clone();
        let spinner_c = spinner.clone();
        let status_c = status_lbl.clone();

        // Enter in password field triggers connect.
        {
            let btn_enter = connect_btn.clone();
            pw_entry.connect_activate(move |_| {
                btn_enter.emit_clicked();
            });
        }

        connect_btn.connect_clicked(move |_| {
            let ssid = ssid_c.text().to_string();
            if ssid.is_empty() {
                return;
            }
            let password = pw_c.text().to_string();

            btn_c.set_sensitive(false);
            spinner_c.set_visible(true);
            spinner_c.start();
            status_c.set_visible(false);

            let (tx, rx) = mpsc::channel::<NmResult>();
            connect_new_async(ssid, password, true, tx);

            let btn_poll = btn_c.clone();
            let spinner_poll = spinner_c.clone();
            let status_poll = status_c.clone();

            glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
                match rx.try_recv() {
                    Ok(NmResult::Success) => {
                        spinner_poll.stop();
                        spinner_poll.set_visible(false);
                        btn_poll.set_sensitive(true);
                        status_poll.set_label("✓");
                        status_poll.add_css_class("network-status-ok");
                        status_poll.remove_css_class("network-status-err");
                        status_poll.set_visible(true);
                        auto_hide_status(&status_poll);
                        glib::ControlFlow::Break
                    }
                    Ok(NmResult::Failure(msg)) => {
                        spinner_poll.stop();
                        spinner_poll.set_visible(false);
                        btn_poll.set_sensitive(true);
                        let display = if msg.is_empty() { "Failed".to_string() } else { msg };
                        status_poll.set_label(&display);
                        status_poll.add_css_class("network-status-err");
                        status_poll.remove_css_class("network-status-ok");
                        status_poll.set_visible(true);
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

    let toggle_btn = Button::builder()
        .label("Connect to hidden network")
        .halign(gtk4::Align::Center)
        .build();
    toggle_btn.add_css_class("network-show-all-btn");
    {
        let rev_c = hidden_revealer.clone();
        let ssid_c = ssid_entry.clone();
        toggle_btn.connect_clicked(move |_| {
            let visible = rev_c.reveals_child();
            rev_c.set_reveal_child(!visible);
            if !visible {
                ssid_c.grab_focus();
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

// ── Connection wiring helpers ─────────────────────────────────────────────────

fn wire_connect_known(btn: &Button, spinner: &Spinner, status_lbl: &Label, ssid: String) {
    let btn_c = btn.clone();
    let spinner_c = spinner.clone();
    let status_c = status_lbl.clone();

    btn.connect_clicked(move |_| {
        btn_c.set_sensitive(false);
        spinner_c.set_visible(true);
        spinner_c.start();
        status_c.set_visible(false);

        let (tx, rx) = mpsc::channel::<NmResult>();
        connect_known_async(ssid.clone(), tx);

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

fn wire_connect_new(
    btn: &Button,
    pw_entry: &PasswordEntry,
    spinner: &Spinner,
    status_lbl: &Label,
    ssid: String,
    hidden: bool,
) {
    {
        let btn_enter = btn.clone();
        pw_entry.connect_activate(move |_| {
            btn_enter.emit_clicked();
        });
    }

    let btn_c = btn.clone();
    let pw_c = pw_entry.clone();
    let spinner_c = spinner.clone();
    let status_c = status_lbl.clone();

    btn.connect_clicked(move |_| {
        let password = pw_c.text().to_string();
        btn_c.set_sensitive(false);
        spinner_c.set_visible(true);
        spinner_c.start();
        status_c.set_visible(false);

        let (tx, rx) = mpsc::channel::<NmResult>();
        connect_new_async(ssid.clone(), password, hidden, tx);

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

fn wire_connect_open(btn: &Button, spinner: &Spinner, status_lbl: &Label, ssid: String) {
    let btn_c = btn.clone();
    let spinner_c = spinner.clone();
    let status_c = status_lbl.clone();

    btn.connect_clicked(move |_| {
        btn_c.set_sensitive(false);
        spinner_c.set_visible(true);
        spinner_c.start();
        status_c.set_visible(false);

        let (tx, rx) = mpsc::channel::<NmResult>();
        connect_new_async(ssid.clone(), String::new(), false, tx);

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

// ── Shared status helpers ─────────────────────────────────────────────────────

fn apply_nm_result(status_lbl: &Label, result: &NmResult) {
    match result {
        NmResult::Success => {
            status_lbl.set_label("✓");
            status_lbl.add_css_class("network-status-ok");
            status_lbl.remove_css_class("network-status-err");
        }
        NmResult::Failure(msg) => {
            let display = if msg.is_empty() { "Failed" } else { msg.as_str() };
            status_lbl.set_label(display);
            status_lbl.add_css_class("network-status-err");
            status_lbl.remove_css_class("network-status-ok");
        }
    }
    status_lbl.set_visible(true);
}

pub fn auto_hide_status(status_lbl: &Label) {
    let status_hide = status_lbl.clone();
    glib::timeout_add_local_once(std::time::Duration::from_secs(4), move || {
        status_hide.set_visible(false);
    });
}
