mod backend;
mod interfaces;
mod monitor;
mod nm;
mod vpn;
mod wifi;

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{Box, Button, Label, ListBox, Revealer, RevealerTransitionType, Spinner, Switch};

use crate::spawn::spawn_work;
use crate::ui;
use backend::*;

// The quick-toggle tile drives the radio without going through this section
// (widgets/tiles.rs), so the two calls it needs are re-exported here rather
// than reaching into `backend` from outside the module.
pub use backend::{NmResult, network_manager_available, set_wifi_radio, wifi_radio_enabled};

// ── Async result types ───────────────────────────────────────────────────────

/// Data gathered on a background thread during initial construction.
struct InitResult {
    network_manager_available: bool,
    has_wifi: bool,
    wifi_radio: bool,
    active_wifi_conn_name: Option<String>,
    power_saving: bool,
}

/// Data gathered on a background thread during refresh.
struct RefreshResult {
    active: ActiveConnection,
    connectivity: ConnectivityState,
    interfaces: Vec<NetworkInterface>,
    vpns: Vec<VpnConnection>,
    /// IP info fetched in the same background task.
    ip_info: IpInfo,
}

/// IP / gateway / DNS info for the active connection device.
struct IpInfo {
    ip: Option<String>,
    gateway: Option<String>,
    dns: Vec<String>,
}

// ── Internal state ────────────────────────────────────────────────────────────

pub(crate) struct NetworkState {
    pub active: ActiveConnection,
    pub connectivity: ConnectivityState,
    pub networks: Vec<WifiNetwork>,
    pub vpns: Vec<VpnConnection>,
    pub interfaces: Vec<NetworkInterface>,
    pub wifi_radio_enabled: bool,
    pub list_visible: bool,
    pub show_all: bool,
    pub scanning: bool,
    pub search_query: String,
}

// ── NetworkSection ────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct NetworkSection {
    section: Rc<ui::Section>,
    state: Rc<RefCell<NetworkState>>,
    // The section header's icon and summary
    summary_icon: Label,
    summary_text: Label,
    // Header & radio
    wifi_switch: Switch,
    header_subtitle: Label,
    radio_row: Box,
    // Main containers
    wifi_content_box: Box,
    wifi_disabled_box: Box,
    // Hero Card
    hero_card: Box,
    current_icon_label: Label,
    current_ssid_label: Label,
    current_signal_label: Label,
    hero_status: Label,
    current_disconnect_btn: Button,
    current_spinner: Spinner,
    ip_label: Label,
    gateway_label: Label,
    dns_label: Label,
    connectivity_label: Label,
    portal_btn: Button,
    power_save_row: Box,
    // Available networks
    scan_spinner: Spinner,
    scan_status_label: Label,
    scan_btn: Button,
    search_entry: gtk4::SearchEntry,
    network_list_box: ListBox,
    // Other & Advanced
    vpn_section_box: Box,
    vpn_list_box: ListBox,
    iface_section_box: Box,
    iface_list_box: ListBox,
}

impl NetworkSection {
    pub fn new() -> Self {
        // Shown as a Helm page (`expand_for_page`), where the section drops
        // its own fill and the sub-sheet is the ground, with groups used
        // semantically inside.
        let section = Rc::new(ui::section(ICON_DISCONNECTED, "Wi-Fi", "Disconnected"));
        ui::glyph(&section.icon, ui::Text::Title, ui::Tone::Fg);
        let summary_icon = section.icon.clone();
        let summary_text = section.summary.clone();

        // ── Placeholder (hidden by default, shown if nmcli unavailable) ───────
        let placeholder = ui::text(
            "NetworkManager not available",
            ui::Text::Body,
            ui::Tone::Muted,
        );
        placeholder.add_css_class("section-empty");
        placeholder.set_visible(false);
        section.root.append(&placeholder);

        let detail_box = ui::vbox(4);
        section.body.append(&detail_box);

        // ── WiFi Header Bar: Radio switch + Status ────────────────────────────
        let radio_row = ui::group(0);
        radio_row.set_visible(false);
        let (radio, wifi_switch) = ui::switch_row("Wi-Fi", "Enabled");
        radio.icon.set_label(ICON_SIGNAL_EXCELLENT);
        radio.icon.set_visible(true);
        ui::glyph(&radio.icon, ui::Text::Title, ui::Tone::Fg);
        wifi_switch.set_sensitive(false);
        let header_subtitle = radio.subtitle.clone();
        radio_row.append(&radio.root);
        detail_box.append(&radio_row);

        // ── WiFi Disabled State (shown when radio is off) ─────────────────────
        let wifi_disabled_box = ui::vbox(1);
        wifi_disabled_box.set_halign(gtk4::Align::Center);
        wifi_disabled_box.set_valign(gtk4::Align::Center);
        wifi_disabled_box.set_vexpand(true);
        wifi_disabled_box.add_css_class("network-disabled");
        wifi_disabled_box.set_visible(false);

        let disabled_icon = gtk4::Label::new(Some(ICON_DISCONNECTED));
        ui::glyph(&disabled_icon, ui::Text::DisplaySm, ui::Tone::Muted);

        let disabled_title = ui::text("Wi-Fi is turned off", ui::Text::Body, ui::Tone::Fg);
        disabled_title.add_css_class("ui-strong");
        disabled_title.set_xalign(0.5);

        let disabled_subtitle = ui::text(
            "Turn on Wi-Fi to scan and connect to nearby networks",
            ui::Text::Label,
            ui::Tone::Muted,
        );
        disabled_subtitle.set_xalign(0.5);

        wifi_disabled_box.append(&disabled_icon);
        wifi_disabled_box.append(&disabled_title);
        wifi_disabled_box.append(&disabled_subtitle);
        detail_box.append(&wifi_disabled_box);

        // ── WiFi Content Box (visible when radio is ON) ───────────────────────
        let wifi_content_box = ui::vbox(4);
        wifi_content_box.set_visible(false);

        // ── Hero Card: Active Connection ──────────────────────────────────────
        let hero_card = ui::group(1);
        hero_card.add_css_class("network-hero");
        hero_card.set_visible(false);

        let hero = ui::row(ICON_DISCONNECTED, "", "");
        ui::glyph(&hero.icon, ui::Text::Title, ui::Tone::Fg);
        let current_icon_label = hero.icon.clone();
        let current_ssid_label = hero.title.clone();

        // Status and signal share the subtitle line, each updated on its own.
        let hero_meta_box = ui::hbox(2);
        let hero_status = ui::text("Connected", ui::Text::Label, ui::Tone::Muted);
        let current_signal_label = ui::text("", ui::Text::Label, ui::Tone::Faint);
        hero_meta_box.append(&hero_status);
        hero_meta_box.append(&current_signal_label);
        if let Some(texts) = hero.subtitle.parent().and_downcast::<gtk4::Box>() {
            texts.append(&hero_meta_box);
        }

        let current_spinner = Spinner::new();
        current_spinner.set_visible(false);

        let current_disconnect_btn = ui::small_button("Disconnect", ui::Kind::Secondary);
        current_disconnect_btn.set_visible(false);

        let details_toggle_btn = ui::small_button("Details ▸", ui::Kind::Flat);

        hero.end.append(&current_spinner);
        hero.end.append(&current_disconnect_btn);
        hero.end.append(&details_toggle_btn);
        hero_card.append(&hero.root);

        // Connectivity warning & captive portal button
        let connectivity_row = ui::hbox(3);
        connectivity_row.add_css_class("network-hero-line");

        let connectivity_label = ui::text("", ui::Text::Label, ui::Tone::Muted);
        connectivity_label.set_hexpand(true);
        connectivity_label.set_visible(false);

        let portal_btn = ui::small_button("Open portal", ui::Kind::Primary);
        portal_btn.set_visible(false);
        portal_btn.connect_clicked(|_| {
            let _ = std::process::Command::new("xdg-open")
                .arg("http://nmcheck.gnome.org/")
                .spawn();
        });

        connectivity_row.append(&connectivity_label);
        connectivity_row.append(&portal_btn);
        hero_card.append(&connectivity_row);

        // Expandable Details Drawer
        let details_revealer = Revealer::builder()
            .transition_type(RevealerTransitionType::SlideDown)
            .transition_duration(crate::anim::duration(crate::tokens::motion::EXPAND.ms) as u32)
            .reveal_child(false)
            .build();

        let details_tray = ui::vbox(2);
        details_tray.append(&ui::separator());

        let ip_box = ui::vbox(1);
        ip_box.add_css_class("network-hero-line");

        let ip_line = || {
            let l = ui::text("", ui::Text::Caption, ui::Tone::Faint);
            l.add_css_class("ui-mono");
            l.set_visible(false);
            l
        };
        let ip_label = ip_line();
        let gateway_label = ip_line();
        let dns_label = ip_line();

        ip_box.append(&ip_label);
        ip_box.append(&gateway_label);
        ip_box.append(&dns_label);
        details_tray.append(&ip_box);

        // Power saving row inside details
        let (ps, ps_switch) = ui::switch_row("WiFi Power Saving", "");
        let power_save_row = ps.root.clone();
        power_save_row.set_visible(false);
        {
            let ps_switch_c = ps_switch.clone();
            ps_switch.connect_state_set(move |_sw, active| {
                if let Some(conn_name) = get_active_wifi_conn_name() {
                    let sw_poll = ps_switch_c.clone();
                    spawn_work(
                        move || set_wifi_power_saving(&conn_name, active),
                        move |result| match result {
                            NmResult::Success => sw_poll.set_state(active),
                            NmResult::Failure(_) => sw_poll.set_state(!active),
                        },
                    );
                }
                glib::Propagation::Proceed
            });
        }
        details_tray.append(&power_save_row);

        details_revealer.set_child(Some(&details_tray));
        hero_card.append(&details_revealer);

        // Wire details toggle
        {
            let rev_c = details_revealer.clone();
            let btn_c = details_toggle_btn.clone();
            details_toggle_btn.connect_clicked(move |_| {
                let open = rev_c.reveals_child();
                rev_c.set_reveal_child(!open);
                btn_c.set_label(if open { "Details ▸" } else { "Details ▾" });
            });
        }

        wifi_content_box.append(&hero_card);

        // ── Available Networks Section (Immediate, Hero list) ─────────────────
        let available_section = ui::vbox(2);
        available_section.append(&ui::heading("Available Networks"));

        let search_bar = ui::hbox(3);

        let search_entry = gtk4::SearchEntry::builder()
            .placeholder_text("Search networks…")
            .hexpand(true)
            .build();
        ui::entry(&search_entry);

        let scan_spinner = Spinner::new();
        scan_spinner.set_visible(false);

        let scan_status_label = ui::text("", ui::Text::Caption, ui::Tone::Faint);
        scan_status_label.set_visible(false);

        let scan_btn = ui::button("󰑐 Scan", ui::Kind::Secondary);
        scan_btn.set_tooltip_text(Some("Scan for available networks"));

        search_bar.append(&search_entry);
        search_bar.append(&scan_spinner);
        search_bar.append(&scan_status_label);
        search_bar.append(&scan_btn);
        available_section.append(&search_bar);

        let no_adapter_label = ui::text("No WiFi adapter found", ui::Text::Body, ui::Tone::Muted);
        no_adapter_label.add_css_class("section-empty");
        no_adapter_label.set_visible(false);
        available_section.append(&no_adapter_label);

        let network_list_box = ui::list();
        available_section.append(&network_list_box);

        wifi_content_box.append(&available_section);

        // ── Other Connections & Advanced (Collapsible) ────────────────────────
        let other = ui::disclosure("Advanced & Other Connections");
        let other_box = ui::vbox(3);
        other.body.append(&other_box);

        // VPN subsection
        let vpn_section_box = ui::vbox(2);
        vpn_section_box.set_visible(false);
        vpn_section_box.append(&ui::heading("VPN Connections"));
        let vpn_list_box = ui::list();
        vpn_section_box.append(&vpn_list_box);
        other_box.append(&vpn_section_box);

        // Interface subsection (for physical Ethernet, etc.)
        let iface_section_box = ui::vbox(2);
        iface_section_box.set_visible(false);
        iface_section_box.append(&ui::heading("Network Adapters"));
        let iface_list_box = ui::list();
        iface_section_box.append(&iface_list_box);
        other_box.append(&iface_section_box);

        // Advanced Network Connections launcher
        let adv_btn = ui::button(
            "󰒓  Advanced Network Connections (nm-connection-editor)",
            ui::Kind::Secondary,
        );
        adv_btn.add_css_class("section-launch-btn");
        adv_btn.connect_clicked(|_| {
            let _ = std::process::Command::new("nm-connection-editor")
                .spawn()
                .or_else(|_| {
                    std::process::Command::new("ghostty")
                        .args(["-e", "nmtui"])
                        .spawn()
                })
                .or_else(|_| {
                    std::process::Command::new("foot")
                        .args(["-e", "nmtui"])
                        .spawn()
                });
        });
        other_box.append(&adv_btn);

        wifi_content_box.append(&other.root);

        detail_box.append(&wifi_content_box);

        let state_ref: Rc<RefCell<NetworkState>> = Rc::new(RefCell::new(NetworkState {
            active: ActiveConnection::Disconnected,
            connectivity: ConnectivityState::Unknown,
            networks: Vec::new(),
            vpns: Vec::new(),
            interfaces: Vec::new(),
            wifi_radio_enabled: false,
            list_visible: true,
            show_all: false,
            scanning: false,
            search_query: String::new(),
        }));

        let section = Self {
            section,
            state: state_ref,
            summary_icon,
            summary_text,
            wifi_switch,
            header_subtitle,
            radio_row,
            wifi_content_box,
            wifi_disabled_box,
            hero_card,
            current_icon_label,
            current_ssid_label,
            current_signal_label,
            hero_status,
            current_disconnect_btn,
            current_spinner,
            ip_label,
            gateway_label,
            dns_label,
            connectivity_label,
            portal_btn,
            power_save_row,
            scan_spinner,
            scan_status_label,
            scan_btn,
            search_entry,
            network_list_box,
            vpn_section_box,
            vpn_list_box,
            iface_section_box,
            iface_list_box,
        };

        // Wire disconnect button on hero card
        {
            let sec_c = section.clone();
            let btn_c = section.current_disconnect_btn.clone();
            let spin_c = section.current_spinner.clone();
            section.current_disconnect_btn.connect_clicked(move |_| {
                btn_c.set_sensitive(false);
                spin_c.set_visible(true);
                spin_c.start();
                let sec_poll = sec_c.clone();
                let btn_poll = btn_c.clone();
                let spin_poll = spin_c.clone();
                spawn_work(disconnect_active_wifi, move |_| {
                    spin_poll.stop();
                    spin_poll.set_visible(false);
                    btn_poll.set_sensitive(true);
                    sec_poll.refresh();
                });
            });
        }

        // Wire search entry
        {
            let state_search = section.state.clone();
            let list_search = section.network_list_box.clone();
            let on_change_search = section.on_change();
            section.search_entry.connect_search_changed(move |entry| {
                state_search.borrow_mut().search_query = entry.text().to_string();
                wifi::rebuild_wifi_list(&list_search, &state_search, &on_change_search);
            });
        }

        // Wire scan button
        {
            let sec_scan = section.clone();
            section.scan_btn.connect_clicked(move |_| {
                sec_scan.trigger_scan();
            });
        }

        // ── Async init: probe nmcli/adapter/radio on background thread ────
        let radio_row_c = section.radio_row.clone();
        let ps_switch_c = ps_switch;
        let no_adapter_label_c = no_adapter_label;
        let placeholder_c = placeholder;
        let wifi_switch_init = section.wifi_switch.clone();
        let wifi_content_init = section.wifi_content_box.clone();
        let wifi_disabled_init = section.wifi_disabled_box.clone();
        let subtitle_init = section.header_subtitle.clone();
        let power_save_init = section.power_save_row.clone();
        let state_init = section.state.clone();

        // Clones for the WiFi radio toggle callback
        let wifi_switch_radio = section.wifi_switch.clone();
        let state_radio_init = section.state.clone();
        let wifi_content_radio = section.wifi_content_box.clone();
        let wifi_disabled_radio = section.wifi_disabled_box.clone();
        let subtitle_radio = section.header_subtitle.clone();
        let power_save_radio = section.power_save_row.clone();
        let summary_icon_radio = section.summary_icon.clone();
        let summary_text_radio = section.summary_text.clone();

        spawn_work(
            || {
                let active_wifi_conn_name = get_active_wifi_conn_name();
                let power_saving = active_wifi_conn_name
                    .as_deref()
                    .map(get_wifi_power_saving)
                    .unwrap_or(false);
                InitResult {
                    network_manager_available: network_manager_available(),
                    has_wifi: wifi_adapter_present(),
                    wifi_radio: wifi_radio_enabled(),
                    active_wifi_conn_name,
                    power_saving,
                }
            },
            move |init| {
                if !init.network_manager_available {
                    placeholder_c.set_visible(true);
                    return;
                }

                if init.has_wifi {
                    radio_row_c.set_visible(true);
                    wifi_switch_init.set_sensitive(true);
                    wifi_switch_init.set_active(init.wifi_radio);
                    state_init.borrow_mut().wifi_radio_enabled = init.wifi_radio;

                    wifi_content_init.set_visible(init.wifi_radio);
                    wifi_disabled_init.set_visible(!init.wifi_radio);
                    if !init.wifi_radio {
                        subtitle_init.set_label("Wi-Fi is off");
                    }

                    if init.active_wifi_conn_name.is_some() {
                        ps_switch_c.set_active(init.power_saving);
                        if init.wifi_radio {
                            power_save_init.set_visible(true);
                        }
                    }

                    // Wire WiFi radio toggle now that we know adapter is present.
                    let wifi_switch_revert = wifi_switch_radio.clone();
                    wifi_switch_radio.connect_state_set(move |_sw, active| {
                        let state_poll = state_radio_init.clone();
                        let content_poll = wifi_content_radio.clone();
                        let disabled_poll = wifi_disabled_radio.clone();
                        let subtitle_poll = subtitle_radio.clone();
                        let ps_poll = power_save_radio.clone();
                        let si_poll = summary_icon_radio.clone();
                        let st_poll = summary_text_radio.clone();
                        let sw_poll = wifi_switch_revert.clone();
                        spawn_work(
                            move || set_wifi_radio(active),
                            move |result| match result {
                                NmResult::Success => {
                                    state_poll.borrow_mut().wifi_radio_enabled = active;
                                    content_poll.set_visible(active);
                                    disabled_poll.set_visible(!active);
                                    if !active {
                                        subtitle_poll.set_label("Wi-Fi is off");
                                        si_poll.set_label(ICON_DISCONNECTED);
                                        st_poll.set_label("WiFi Off");
                                    } else {
                                        subtitle_poll.set_label("Enabled");
                                    }
                                    ps_poll.set_visible(
                                        active && get_active_wifi_conn_name().is_some(),
                                    );
                                }
                                NmResult::Failure(_) => {
                                    sw_poll.set_state(!active);
                                }
                            },
                        );

                        glib::Propagation::Proceed
                    });
                } else {
                    no_adapter_label_c.set_visible(true);
                }
            },
        );

        // ── Async initial refresh ─────────────────────────────────────────
        section.refresh();

        // And again whenever the section comes back on screen. The poller
        // sleeps while nothing of ours is mapped (`monitor::schedule_next`),
        // so without this the first thing a reopened page shows is whatever
        // was true when it was last closed, for as long as a tick. `map`
        // rather than a call from the panel, because there are three ways in
        // — the panel being shown, the deck switching to the page, and the
        // Wi-Fi pill — and this covers all of them at once.
        {
            let section_c = section.clone();
            section
                .section
                .root
                .connect_map(move |_| section_c.refresh());
        }

        // Start periodic poller.
        monitor::start_periodic_poller(
            section.state.clone(),
            monitor::PollerWidgets {
                display: section.display_widgets(),
                root: section.section.root.clone(),
                connectivity_label: section.connectivity_label.clone(),
                portal_btn: section.portal_btn.clone(),
                wifi_switch: section.wifi_switch.clone(),
                wifi_content_box: section.wifi_content_box.clone(),
                wifi_disabled_box: section.wifi_disabled_box.clone(),
                power_save_row: section.power_save_row.clone(),
                iface_list_box: section.iface_list_box.clone(),
                iface_section_box: section.iface_section_box.clone(),
                vpn_list_box: section.vpn_list_box.clone(),
                vpn_section_box: section.vpn_section_box.clone(),
            },
        );

        section
    }

    fn display_widgets(&self) -> monitor::DisplayWidgets {
        monitor::DisplayWidgets {
            summary_icon: self.summary_icon.clone(),
            summary_text: self.summary_text.clone(),
            current_icon: self.current_icon_label.clone(),
            current_ssid: self.current_ssid_label.clone(),
            current_signal: self.current_signal_label.clone(),
            hero_status: self.hero_status.clone(),
            header_subtitle: self.header_subtitle.clone(),
            hero_card: self.hero_card.clone(),
            ip_label: self.ip_label.clone(),
            gateway_label: self.gateway_label.clone(),
            dns_label: self.dns_label.clone(),
            current_disconnect_btn: self.current_disconnect_btn.clone(),
            current_spinner: self.current_spinner.clone(),
        }
    }

    pub fn on_change(&self) -> Rc<dyn Fn()> {
        let section = self.clone();
        Rc::new(move || {
            section.refresh();
        })
    }

    pub fn trigger_scan(&self) {
        if !self.state.borrow().wifi_radio_enabled {
            return;
        }
        let on_change = self.on_change();
        Self::start_wifi_scan_static(
            &self.state,
            &self.scan_spinner,
            &self.scan_status_label,
            &self.scan_btn,
            &self.network_list_box,
            &on_change,
        );
    }

    /// Run all blocking network queries on a background thread, then apply
    /// results on the GTK main thread.
    pub fn refresh(&self) {
        let state_c = self.state.clone();
        let display = self.display_widgets();
        let connectivity_label = self.connectivity_label.clone();
        let portal_btn = self.portal_btn.clone();
        let summary_text = self.summary_text.clone();
        let iface_list_box = self.iface_list_box.clone();
        let iface_section_box = self.iface_section_box.clone();
        let vpn_list_box = self.vpn_list_box.clone();
        let vpn_section_box = self.vpn_section_box.clone();
        let wifi_content_box = self.wifi_content_box.clone();
        let wifi_disabled_box = self.wifi_disabled_box.clone();
        let subtitle = self.header_subtitle.clone();
        let power_save_row = self.power_save_row.clone();
        let scan_spinner = self.scan_spinner.clone();
        let scan_status_label = self.scan_status_label.clone();
        let scan_btn = self.scan_btn.clone();
        let network_list_box = self.network_list_box.clone();
        let on_change = self.on_change();

        spawn_work(
            || {
                let active = get_active_connection();
                let connectivity = check_connectivity();
                let interfaces = get_network_interfaces();
                let vpns = get_vpn_connections();

                // Fetch IP info for the active device while still on the background thread.
                let ip_info = match &active {
                    ActiveConnection::Wifi { device, .. }
                    | ActiveConnection::Ethernet { device } => {
                        let ip = get_device_ip(device);
                        let gateway = get_default_gateway();
                        let dns = get_dns_servers(device);
                        IpInfo { ip, gateway, dns }
                    }
                    ActiveConnection::Disconnected => IpInfo {
                        ip: None,
                        gateway: None,
                        dns: Vec::new(),
                    },
                };

                RefreshResult {
                    active,
                    connectivity,
                    interfaces,
                    vpns,
                    ip_info,
                }
            },
            move |result| {
                // Apply active connection display (without re-fetching IP info).
                monitor::update_active_display_with_ip(
                    &result.active,
                    &display,
                    &result.ip_info.ip,
                    &result.ip_info.gateway,
                    &result.ip_info.dns,
                );
                state_c.borrow_mut().active = result.active;

                // Connectivity.
                monitor::update_connectivity_display(
                    &result.connectivity,
                    &connectivity_label,
                    &portal_btn,
                    &summary_text,
                    &display.hero_status,
                );
                state_c.borrow_mut().connectivity = result.connectivity;

                // Interfaces.
                state_c.borrow_mut().interfaces = result.interfaces.clone();
                interfaces::rebuild_iface_list(&iface_list_box, &state_c);
                iface_section_box.set_visible(!result.interfaces.is_empty());

                // VPNs.
                state_c.borrow_mut().vpns = result.vpns.clone();
                vpn::rebuild_vpn_list(&vpn_list_box, &state_c);
                vpn_section_box.set_visible(!result.vpns.is_empty());

                // WiFi scan.
                if state_c.borrow().wifi_radio_enabled {
                    Self::start_wifi_scan_static(
                        &state_c,
                        &scan_spinner,
                        &scan_status_label,
                        &scan_btn,
                        &network_list_box,
                        &on_change,
                    );
                }

                // WiFi controls visibility.
                let radio_on = state_c.borrow().wifi_radio_enabled;
                wifi_content_box.set_visible(radio_on);
                wifi_disabled_box.set_visible(!radio_on);
                if !radio_on {
                    subtitle.set_label("Wi-Fi is off");
                }
                power_save_row.set_visible(
                    radio_on && matches!(state_c.borrow().active, ActiveConnection::Wifi { .. }),
                );
            },
        );
    }

    fn start_wifi_scan_static(
        state: &Rc<RefCell<NetworkState>>,
        scan_spinner: &Spinner,
        scan_status_label: &Label,
        scan_btn: &Button,
        network_list_box: &ListBox,
        on_change: &Rc<dyn Fn()>,
    ) {
        if state.borrow().scanning {
            return;
        }
        state.borrow_mut().scanning = true;

        scan_btn.set_sensitive(false);
        scan_spinner.set_visible(true);
        scan_spinner.start();
        scan_status_label.set_label("Scanning…");
        scan_status_label.set_visible(true);

        let scan_spinner_c = scan_spinner.clone();
        let scan_status_c = scan_status_label.clone();
        let scan_btn_c = scan_btn.clone();
        let network_list_box_c = network_list_box.clone();
        let state_c = state.clone();
        let on_change_c = on_change.clone();

        spawn_work(
            scan_wifi,
            move |result: Result<Vec<WifiNetwork>, String>| {
                scan_spinner_c.stop();
                scan_spinner_c.set_visible(false);
                scan_btn_c.set_sensitive(true);
                state_c.borrow_mut().scanning = false;

                match result {
                    Ok(networks) => {
                        scan_status_c.set_visible(false);
                        {
                            let mut s = state_c.borrow_mut();
                            s.networks = networks;
                        }
                        wifi::rebuild_wifi_list(&network_list_box_c, &state_c, &on_change_c);
                    }
                    Err(msg) => {
                        scan_status_c.set_label(&msg);
                        auto_hide_status(&scan_status_c);
                    }
                }
            },
        );
    }

    pub fn expand_for_page(&self) {
        self.section.show_as_page();
        self.state.borrow_mut().list_visible = true;
        self.trigger_scan();
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.section.root
    }
}
