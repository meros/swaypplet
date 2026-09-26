use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{
    Button, Entry, Label, ListBox, ListBoxRow, PasswordEntry, Revealer, RevealerTransitionType,
    Spinner,
};

use super::NetworkState;
use super::backend::*;
use crate::spawn::spawn_work;
use crate::tokens::space;
use crate::ui;

// ── WiFi list builder ─────────────────────────────────────────────────────────

pub fn rebuild_wifi_list(
    list: &ListBox,
    state: &Rc<RefCell<NetworkState>>,
    on_change: &Rc<dyn Fn()>,
) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }

    let (networks, show_all, query) = {
        let s = state.borrow();
        (
            s.networks.clone(),
            s.show_all,
            s.search_query.trim().to_lowercase(),
        )
    };

    let filtered: Vec<WifiNetwork> = if query.is_empty() {
        networks.into_iter().filter(|n| !n.in_use).collect()
    } else {
        networks
            .into_iter()
            .filter(|n| n.ssid.to_lowercase().contains(&query))
            .collect()
    };

    if filtered.is_empty() {
        let msg = if !query.is_empty() {
            format!(
                "No networks matching \"{}\"",
                state.borrow().search_query.trim()
            )
        } else if state.borrow().scanning {
            "Scanning for networks…".to_string()
        } else {
            "No networks found".to_string()
        };

        let empty_lbl = ui::text(&msg, ui::Text::Body, ui::Tone::Muted);
        empty_lbl.set_halign(gtk4::Align::Center);
        empty_lbl.set_margin_top(space(3));
        empty_lbl.set_margin_bottom(space(3));
        let row = ListBoxRow::builder().build();
        row.set_child(Some(&empty_lbl));
        list.append(&row);
    } else {
        let total = filtered.len();
        let visible_count = if !query.is_empty() || show_all {
            total
        } else {
            total.min(MAX_VISIBLE_NETWORKS)
        };

        for network in filtered.iter().take(visible_count) {
            let list_row = build_wifi_row(network, state, on_change);
            list.append(&list_row);
        }

        // "Show all" / "Show fewer" button when more networks exist and we're not searching.
        if query.is_empty() && total > MAX_VISIBLE_NETWORKS {
            let btn_label = if show_all {
                "Show fewer".to_string()
            } else {
                format!("Show all ({})", total)
            };
            let more_btn = ui::small_button(&btn_label, ui::Kind::Flat);
            more_btn.set_halign(gtk4::Align::Center);

            let state_c = state.clone();
            let list_c = list.clone();
            let on_change_c = on_change.clone();
            more_btn.connect_clicked(move |_| {
                {
                    let mut s = state_c.borrow_mut();
                    s.show_all = !s.show_all;
                }
                rebuild_wifi_list(&list_c, &state_c, &on_change_c);
            });

            let row = ListBoxRow::builder().build();
            row.set_child(Some(&more_btn));
            list.append(&row);
        }
    }

    // "Connect to hidden network" button at the bottom.
    build_hidden_network_row(list, state, on_change);
}

fn build_wifi_row(
    network: &WifiNetwork,
    _state: &Rc<RefCell<NetworkState>>,
    on_change: &Rc<dyn Fn()>,
) -> ListBoxRow {
    let connect_area = ui::vbox(2);

    let r = ui::row("", &network.ssid, "");
    set_signal_glyph(
        &r.icon,
        signal_icon(network.signal),
        signal_tone(network.signal),
    );
    r.icon.set_visible(true);
    r.icon
        .set_tooltip_text(Some(&format!("{}%", network.signal)));
    // The network in use is the selected row.
    ui::set_selected(&r.root, network.in_use);
    let row_box = r.root.clone();

    if let Some(freq) = network.freq_mhz {
        let band_lbl = ui::badge(freq_band_short(freq));
        band_lbl.add_css_class("neutral");
        r.end.append(&band_lbl);
    }

    if !network.security.is_empty() && network.security != "--" {
        let lock_lbl = Label::builder().label(ICON_LOCK).build();
        lock_lbl.set_tooltip_text(Some("Secured"));
        r.end.append(&lock_lbl);
    }

    let needs_password =
        !network.security.is_empty() && network.security != "--" && !network.is_known;

    if network.in_use {
        // Connected network: provide Disconnect button
        let btn_row = ui::hbox(2);
        btn_row.set_halign(gtk4::Align::End);

        let spinner = Spinner::new();
        spinner.set_visible(false);

        let status_lbl = ui::text("", ui::Text::Label, ui::Tone::Faint);
        status_lbl.set_visible(false);

        let disconnect_btn = ui::small_button("Disconnect", ui::Kind::Secondary);

        btn_row.append(&spinner);
        btn_row.append(&status_lbl);
        btn_row.append(&disconnect_btn);
        r.end.append(&btn_row);

        connect_area.append(&row_box);

        wire_disconnect(
            &Busy::new(&disconnect_btn, &spinner, &status_lbl),
            network.ssid.clone(),
            on_change.clone(),
        );
    } else if network.is_known {
        let btn_row = ui::hbox(2);
        btn_row.set_halign(gtk4::Align::End);

        let spinner = Spinner::new();
        spinner.set_visible(false);

        let status_lbl = ui::text("", ui::Text::Label, ui::Tone::Faint);
        status_lbl.set_visible(false);

        let forget_btn = ui::small_button("Forget", ui::Kind::Flat);
        wire_forget(
            &Busy::new(&forget_btn, &spinner, &status_lbl),
            network.ssid.clone(),
            on_change.clone(),
        );

        let connect_btn = ui::small_button("Connect", ui::Kind::Secondary);
        wire_connect_known(
            &Busy::new(&connect_btn, &spinner, &status_lbl),
            network.ssid.clone(),
            on_change.clone(),
        );

        btn_row.append(&spinner);
        btn_row.append(&status_lbl);
        btn_row.append(&forget_btn);
        btn_row.append(&connect_btn);
        r.end.append(&btn_row);

        connect_area.append(&row_box);

        // Clicking the row activates connect
        let click = gtk4::GestureClick::new();
        {
            let conn_c = connect_btn.clone();
            click.connect_released(move |_, _, _, _| {
                conn_c.emit_clicked();
            });
        }
        row_box.add_controller(click);
    } else if needs_password {
        let btn_row = ui::hbox(2);
        btn_row.set_halign(gtk4::Align::End);

        let toggle_btn = ui::small_button("Connect", ui::Kind::Secondary);
        btn_row.append(&toggle_btn);
        r.end.append(&btn_row);

        connect_area.append(&row_box);

        let pw_revealer = Revealer::builder()
            .transition_type(RevealerTransitionType::SlideDown)
            .transition_duration(crate::anim::duration(crate::tokens::motion::EXPAND.ms) as u32)
            .reveal_child(false)
            .build();

        let pw_area = ui::vbox(2);
        pw_area.set_margin_start(space(3));
        pw_area.set_margin_end(space(3));
        pw_area.set_margin_bottom(space(2));

        let pw_row = ui::hbox(2);

        let pw_entry = PasswordEntry::builder()
            .hexpand(true)
            .placeholder_text("Password")
            .show_peek_icon(true)
            .build();
        ui::entry(&pw_entry);

        let join_btn = ui::small_button("Join", ui::Kind::Primary);

        pw_row.append(&pw_entry);
        pw_row.append(&join_btn);
        pw_area.append(&pw_row);

        let fb_row = ui::hbox(2);
        fb_row.set_halign(gtk4::Align::End);

        let spinner = Spinner::new();
        spinner.set_visible(false);

        let status_lbl = ui::text("", ui::Text::Label, ui::Tone::Faint);
        status_lbl.set_visible(false);

        fb_row.append(&spinner);
        fb_row.append(&status_lbl);
        pw_area.append(&fb_row);

        pw_revealer.set_child(Some(&pw_area));
        connect_area.append(&pw_revealer);

        let ssid = network.ssid.clone();
        wire_connect_new(
            &Busy::new(&join_btn, &spinner, &status_lbl),
            &pw_entry,
            move || ssid.clone(),
            network.security.clone(),
            false,
            on_change.clone(),
        );

        let rev_c = pw_revealer.clone();
        let entry_c = pw_entry.clone();
        toggle_btn.connect_clicked(move |_| {
            let visible = rev_c.reveals_child();
            rev_c.set_reveal_child(!visible);
            if !visible {
                entry_c.grab_focus();
            }
        });

        let click = gtk4::GestureClick::new();
        {
            let rev_c2 = pw_revealer.clone();
            let entry_c2 = pw_entry.clone();
            click.connect_released(move |_, _, _, _| {
                let visible = rev_c2.reveals_child();
                rev_c2.set_reveal_child(!visible);
                if !visible {
                    entry_c2.grab_focus();
                }
            });
        }
        row_box.add_controller(click);
    } else {
        let btn_row = ui::hbox(2);
        btn_row.set_halign(gtk4::Align::End);

        let spinner = Spinner::new();
        spinner.set_visible(false);

        let status_lbl = ui::text("", ui::Text::Label, ui::Tone::Faint);
        status_lbl.set_visible(false);

        let connect_btn = ui::small_button("Connect", ui::Kind::Secondary);

        btn_row.append(&spinner);
        btn_row.append(&status_lbl);
        btn_row.append(&connect_btn);
        r.end.append(&btn_row);

        connect_area.append(&row_box);

        wire_connect_open(
            &Busy::new(&connect_btn, &spinner, &status_lbl),
            network.ssid.clone(),
            on_change.clone(),
        );

        let click = gtk4::GestureClick::new();
        {
            let conn_c = connect_btn.clone();
            click.connect_released(move |_, _, _, _| {
                conn_c.emit_clicked();
            });
        }
        row_box.add_controller(click);
    }

    let list_row = ListBoxRow::builder().build();
    list_row.set_child(Some(&connect_area));
    list_row
}

// ── Hidden network form ───────────────────────────────────────────────────────

fn build_hidden_network_row(
    list: &ListBox,
    _state: &Rc<RefCell<NetworkState>>,
    on_change: &Rc<dyn Fn()>,
) {
    let outer = ui::vbox(2);

    let hidden_revealer = Revealer::builder()
        .transition_type(RevealerTransitionType::SlideDown)
        .transition_duration(crate::anim::duration(crate::tokens::motion::EXPAND.ms) as u32)
        .reveal_child(false)
        .build();

    let form = ui::vbox(2);
    form.set_margin_top(space(2));
    form.set_margin_start(space(3));
    form.set_margin_end(space(3));

    let ssid_entry = Entry::builder()
        .placeholder_text("Network name (SSID)")
        .hexpand(true)
        .build();
    ui::entry(&ssid_entry);

    let pw_entry = PasswordEntry::builder()
        .placeholder_text("Password (leave empty for open)")
        .show_peek_icon(true)
        .hexpand(true)
        .build();
    ui::entry(&pw_entry);

    let btn_row = ui::hbox(2);
    btn_row.set_halign(gtk4::Align::End);

    let spinner = Spinner::new();
    spinner.set_visible(false);

    let status_lbl = ui::text("", ui::Text::Label, ui::Tone::Faint);
    status_lbl.set_visible(false);

    let connect_btn = ui::small_button("Connect", ui::Kind::Secondary);

    btn_row.append(&spinner);
    btn_row.append(&status_lbl);
    btn_row.append(&connect_btn);

    form.append(&ssid_entry);
    form.append(&pw_entry);
    form.append(&btn_row);
    hidden_revealer.set_child(Some(&form));

    // The SSID comes from the entry at the moment the button is pressed, and
    // an empty one is not a network: `wire_connect_new` drops the click.
    {
        let typed = ssid_entry.clone();
        wire_connect_new(
            &Busy::new(&connect_btn, &spinner, &status_lbl),
            &pw_entry,
            move || typed.text().to_string(),
            String::new(),
            true,
            on_change.clone(),
        );
    }

    let toggle_btn = ui::small_button("Connect to hidden network", ui::Kind::Flat);
    toggle_btn.set_halign(gtk4::Align::Center);
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
    list.append(&list_row);
}

// ── Connection wiring helpers ─────────────────────────────────────────────────

/// A row's button while a NetworkManager call is in flight, and what it does
/// with the answer.
///
/// Six actions on these rows — connect, connect with a password, connect to a
/// hidden network, connect to an open one, disconnect, forget —differ  only in the
/// call they make. Each used to carry its own copy of: disable the button,
/// show and start the spinner, hide the status, spawn, then stop, re-enable,
/// and either refresh the section or show the failure for four seconds. Six
/// copies is six places for the spinner to be left spinning on a path someone
/// forgot.
struct Busy {
    btn: Button,
    spinner: Spinner,
    status: Label,
}

impl Busy {
    fn new(btn: &Button, spinner: &Spinner, status: &Label) -> Rc<Self> {
        Rc::new(Self {
            btn: btn.clone(),
            spinner: spinner.clone(),
            status: status.clone(),
        })
    }

    /// Run `work` on a worker, with the button held and the spinner turning
    /// until it answers. Success refreshes the section through `on_change`,
    /// which rebuilds this row and drops these widgets; failure says so on
    /// the row itself and clears itself after a few seconds.
    fn run(
        self: &Rc<Self>,
        work: impl FnOnce() -> NmResult + Send + 'static,
        on_change: Rc<dyn Fn()>,
    ) {
        self.btn.set_sensitive(false);
        self.spinner.set_visible(true);
        self.spinner.start();
        self.status.set_visible(false);

        let this = self.clone();
        spawn_work(work, move |result| {
            this.spinner.stop();
            this.spinner.set_visible(false);
            this.btn.set_sensitive(true);
            match &result {
                NmResult::Success => on_change(),
                NmResult::Failure(_) => {
                    apply_nm_result(&this.status, &result);
                    auto_hide_status(&this.status);
                }
            }
        });
    }
}

fn wire_disconnect(busy: &Rc<Busy>, ssid: String, on_change: Rc<dyn Fn()>) {
    let busy_c = busy.clone();
    busy.btn.clone().connect_clicked(move |_| {
        let ssid = ssid.clone();
        busy_c.run(move || disconnect_network(&ssid), on_change.clone());
    });
}

/// Forget asks twice. The first click turns the button into "Sure?" for three
/// seconds; only the second one removes the saved connection, because the
/// button sits beside Connect on a row the user is aiming at.
fn wire_forget(busy: &Rc<Busy>, ssid: String, on_change: Rc<dyn Fn()>) {
    let confirmed = Rc::new(Cell::new(false));
    let busy_c = busy.clone();
    busy.btn.clone().connect_clicked(move |b| {
        if !confirmed.get() {
            confirmed.set(true);
            b.set_label("Sure?");
            ui::set_button_kind(b, ui::Kind::Destructive);
            let revert = b.clone();
            let confirmed_revert = confirmed.clone();
            glib::timeout_add_local_once(std::time::Duration::from_secs(3), move || {
                if confirmed_revert.get() {
                    confirmed_revert.set(false);
                    revert.set_label("Forget");
                    ui::set_button_kind(&revert, ui::Kind::Flat);
                }
            });
            return;
        }
        confirmed.set(false);
        let ssid = ssid.clone();
        busy_c.run(move || forget_network(&ssid), on_change.clone());
    });
}

fn wire_connect_known(busy: &Rc<Busy>, ssid: String, on_change: Rc<dyn Fn()>) {
    let busy_c = busy.clone();
    busy.btn.clone().connect_clicked(move |_| {
        let ssid = ssid.clone();
        busy_c.run(move || connect_known(&ssid), on_change.clone());
    });
}

/// An open network: the same call with no password and no security.
fn wire_connect_open(busy: &Rc<Busy>, ssid: String, on_change: Rc<dyn Fn()>) {
    let busy_c = busy.clone();
    busy.btn.clone().connect_clicked(move |_| {
        let ssid = ssid.clone();
        busy_c.run(move || connect_new(&ssid, "", "", false), on_change.clone());
    });
}

/// A secured network, with the password from `pw_entry`. Enter in the field
/// is the same as pressing the button.
///
/// `ssid` is a closure rather than a string because the hidden-network row
/// reads its SSID from an entry the user is still typing into; a visible row
/// hands back the name it was built with.
fn wire_connect_new(
    busy: &Rc<Busy>,
    pw_entry: &PasswordEntry,
    ssid: impl Fn() -> String + 'static,
    security: String,
    hidden: bool,
    on_change: Rc<dyn Fn()>,
) {
    {
        let btn_enter = busy.btn.clone();
        pw_entry.connect_activate(move |_| btn_enter.emit_clicked());
    }
    let busy_c = busy.clone();
    let pw_c = pw_entry.clone();
    busy.btn.clone().connect_clicked(move |_| {
        let ssid = ssid();
        if ssid.is_empty() {
            return;
        }
        let password = pw_c.text().to_string();
        let security = security.clone();
        busy_c.run(
            move || connect_new(&ssid, &password, &security, hidden),
            on_change.clone(),
        );
    });
}
