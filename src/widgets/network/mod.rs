mod backend;
mod interfaces;
mod monitor;
mod vpn;
mod wifi;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;

use gtk4::prelude::*;
use gtk4::{Box, Button, Label, ListBox, Orientation, Revealer, RevealerTransitionType, Spinner, Switch};

use backend::*;
use crate::spawn::spawn_work;

// ── Async result types ───────────────────────────────────────────────────────

/// Data gathered on a background thread during initial construction.
struct InitResult {
    nmcli_available: bool,
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
}

// ── NetworkSection ────────────────────────────────────────────────────────────

#[allow(dead_code)]
pub struct NetworkSection {
    root: Box,
    state: Rc<RefCell<NetworkState>>,
    // Summary row
    summary_icon: Label,
    summary_text: Label,
    summary_arrow: Label,
    detail_revealer: Revealer,
    // Detail widgets
    current_icon_label: Label,
    current_ssid_label: Label,
    current_signal_label: Label,
    ip_label: Label,
    gateway_label: Label,
    dns_label: Label,
    connectivity_label: Label,
    portal_btn: Button,
    wifi_switch: Switch,
    wifi_controls_box: Box,
    power_save_row: Box,
    // Scan status
    scan_spinner: Spinner,
    scan_status_label: Label,
    // Toggle / lists
    toggle_button: Button,
    revealer: Revealer,
    network_list_box: ListBox,
    vpn_list_box: ListBox,
    iface_list_box: ListBox,
}

impl NetworkSection {
    pub fn new() -> Self {
        let root = Box::builder()
            .orientation(Orientation::Vertical)
            .spacing(6)
            .build();
        root.add_css_class("section");

        // ── Summary row ───────────────────────────────────────────────────────
        let summary_content = Box::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .build();

        let summary_icon = Label::builder().label(ICON_DISCONNECTED).build();
        summary_icon.add_css_class("section-summary-icon");

        let summary_text = Label::builder()
            .label("Disconnected")
            .hexpand(true)
            .xalign(0.0)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .build();
        summary_text.add_css_class("section-summary-label");

        let summary_arrow = Label::builder().label("▸").build();
        summary_arrow.add_css_class("section-expand-arrow");

        summary_content.append(&summary_icon);
        summary_content.append(&summary_text);
        summary_content.append(&summary_arrow);

        let summary_btn = Button::builder().child(&summary_content).build();
        summary_btn.add_css_class("section-summary");
        root.append(&summary_btn);

        // ── Placeholder (hidden by default, shown if nmcli unavailable) ───────
        let placeholder = Label::builder()
            .label("NetworkManager not available")
            .halign(gtk4::Align::Start)
            .build();
        placeholder.add_css_class("network-placeholder");
        placeholder.set_visible(false);
        root.append(&placeholder);

        // ── Detail revealer ───────────────────────────────────────────────────
        let detail_revealer = Revealer::builder()
            .transition_type(RevealerTransitionType::SlideDown)
            .transition_duration(200)
            .reveal_child(false)
            .build();

        let detail_box = Box::builder()
            .orientation(Orientation::Vertical)
            .spacing(4)
            .build();

        // ── WiFi radio toggle row (hidden until async init confirms adapter) ──
        let wifi_switch = Switch::builder()
            .active(false)
            .valign(gtk4::Align::Center)
            .sensitive(false)
            .build();

        let radio_row = Box::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .build();
        radio_row.add_css_class("network-switch-row");
        radio_row.set_visible(false);

        let wifi_icon = Label::builder().label(ICON_SIGNAL_EXCELLENT).build();
        wifi_icon.add_css_class("network-icon");

        let wifi_label = Label::builder()
            .label("WiFi")
            .halign(gtk4::Align::Start)
            .hexpand(true)
            .build();
        wifi_label.add_css_class("network-ssid");

        radio_row.append(&wifi_icon);
        radio_row.append(&wifi_label);
        radio_row.append(&wifi_switch);
        detail_box.append(&radio_row);

        // ── Current connection row ────────────────────────────────────────────
        let current_row = Box::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .build();
        current_row.add_css_class("network-current");

        let current_icon_label = Label::builder()
            .label(ICON_DISCONNECTED)
            .build();
        current_icon_label.add_css_class("network-icon");

        let current_ssid_label = Label::builder()
            .label("")
            .halign(gtk4::Align::Start)
            .hexpand(true)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .build();
        current_ssid_label.add_css_class("network-ssid");
        current_ssid_label.add_css_class("network-active");

        let current_signal_label = Label::builder().label("").build();
        current_signal_label.add_css_class("network-signal");

        current_row.append(&current_icon_label);
        current_row.append(&current_ssid_label);
        current_row.append(&current_signal_label);
        detail_box.append(&current_row);

        // ── Connectivity state ────────────────────────────────────────────────
        let connectivity_row = Box::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .build();

        let connectivity_label = Label::builder()
            .label("")
            .halign(gtk4::Align::Start)
            .hexpand(true)
            .build();
        connectivity_label.add_css_class("network-connectivity");
        connectivity_label.set_visible(false);

        let portal_btn = Button::builder()
            .label("Open portal")
            .build();
        portal_btn.add_css_class("network-connect-btn");
        portal_btn.set_visible(false);
        portal_btn.connect_clicked(|_| {
            let _ = std::process::Command::new("xdg-open")
                .arg("http://nmcheck.gnome.org/")
                .spawn();
        });

        connectivity_row.append(&connectivity_label);
        connectivity_row.append(&portal_btn);
        detail_box.append(&connectivity_row);

        // ── IP / Gateway / DNS info ───────────────────────────────────────────
        let ip_box = Box::builder()
            .orientation(Orientation::Vertical)
            .spacing(2)
            .build();
        ip_box.add_css_class("network-ip-info");

        let ip_label = Label::builder()
            .label("")
            .halign(gtk4::Align::Start)
            .build();
        ip_label.add_css_class("network-ip");
        ip_label.set_visible(false);

        let gateway_label = Label::builder()
            .label("")
            .halign(gtk4::Align::Start)
            .build();
        gateway_label.add_css_class("network-ip");
        gateway_label.set_visible(false);

        let dns_label = Label::builder()
            .label("")
            .halign(gtk4::Align::Start)
            .build();
        dns_label.add_css_class("network-ip");
        dns_label.set_visible(false);

        ip_box.append(&ip_label);
        ip_box.append(&gateway_label);
        ip_box.append(&dns_label);
        detail_box.append(&ip_box);

        // ── WiFi power saving toggle ──────────────────────────────────────────
        let power_save_row = Box::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .build();
        power_save_row.add_css_class("network-switch-row");
        power_save_row.set_visible(false);

        let ps_label = Label::builder()
            .label("WiFi Power Saving")
            .halign(gtk4::Align::Start)
            .hexpand(true)
            .build();
        ps_label.add_css_class("network-ssid");

        let ps_switch = Switch::builder()
            .valign(gtk4::Align::Center)
            .build();

        {
            let ps_switch_c = ps_switch.clone();
            ps_switch.connect_state_set(move |_sw, active| {
                if let Some(conn_name) = get_active_wifi_conn_name() {
                    let (tx, rx) = mpsc::channel::<NmResult>();
                    set_wifi_power_saving_async(conn_name, active, tx);

                    let sw_poll = ps_switch_c.clone();
                    glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
                        match rx.try_recv() {
                            Ok(NmResult::Success) => {
                                sw_poll.set_state(active);
                                glib::ControlFlow::Break
                            }
                            Ok(NmResult::Failure(_)) => {
                                sw_poll.set_state(!active);
                                glib::ControlFlow::Break
                            }
                            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                            Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
                        }
                    });
                }
                glib::Propagation::Proceed
            });
        }

        power_save_row.append(&ps_label);
        power_save_row.append(&ps_switch);
        detail_box.append(&power_save_row);

        // ── Interfaces subsection ─────────────────────────────────────────────
        let iface_title = Label::builder()
            .label("Interfaces")
            .halign(gtk4::Align::Start)
            .build();
        iface_title.add_css_class("network-subsection-title");
        detail_box.append(&iface_title);

        let iface_list_box = ListBox::builder()
            .selection_mode(gtk4::SelectionMode::None)
            .build();
        iface_list_box.add_css_class("network-list");
        detail_box.append(&iface_list_box);

        // ── Scan status row ───────────────────────────────────────────────────
        let scan_row = Box::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .halign(gtk4::Align::Center)
            .build();

        let scan_spinner = Spinner::new();
        scan_spinner.set_visible(false);

        let scan_status_label = Label::builder()
            .label("")
            .build();
        scan_status_label.add_css_class("network-scan-status");
        scan_status_label.set_visible(false);

        scan_row.append(&scan_spinner);
        scan_row.append(&scan_status_label);
        detail_box.append(&scan_row);

        // ── WiFi controls box (hidden by default) ─────────────────────────────
        let wifi_controls_box = Box::builder()
            .orientation(Orientation::Vertical)
            .spacing(4)
            .build();
        wifi_controls_box.set_visible(false);

        // Available Networks toggle.
        let toggle_button = Button::builder().label("▸ Available Networks").build();
        toggle_button.add_css_class("section-expander");

        let no_adapter_label = Label::builder()
            .label("No WiFi adapter found")
            .halign(gtk4::Align::Start)
            .build();
        no_adapter_label.add_css_class("network-placeholder");
        no_adapter_label.set_visible(false);

        let revealer = Revealer::builder()
            .transition_type(RevealerTransitionType::SlideDown)
            .transition_duration(200)
            .reveal_child(false)
            .build();

        let revealer_box = Box::builder()
            .orientation(Orientation::Vertical)
            .spacing(4)
            .build();

        let network_list_box = ListBox::builder()
            .selection_mode(gtk4::SelectionMode::None)
            .build();
        network_list_box.add_css_class("network-list");

        revealer_box.append(&no_adapter_label);
        revealer_box.append(&network_list_box);

        // VPN subsection.
        let vpn_title = Label::builder()
            .label("VPN")
            .halign(gtk4::Align::Start)
            .build();
        vpn_title.add_css_class("network-subsection-title");
        revealer_box.append(&vpn_title);

        let vpn_list_box = ListBox::builder()
            .selection_mode(gtk4::SelectionMode::None)
            .build();
        vpn_list_box.add_css_class("network-list");
        revealer_box.append(&vpn_list_box);

        revealer.set_child(Some(&revealer_box));

        wifi_controls_box.append(&toggle_button);
        wifi_controls_box.append(&revealer);
        detail_box.append(&wifi_controls_box);

        detail_revealer.set_child(Some(&detail_box));
        root.append(&detail_revealer);

        // ── Wire summary toggle ───────────────────────────────────────────────
        {
            let rev_c = detail_revealer.clone();
            let arrow_c = summary_arrow.clone();
            summary_btn.connect_clicked(move |_| {
                let expanded = rev_c.reveals_child();
                rev_c.set_reveal_child(!expanded);
                arrow_c.set_label(if expanded { "▸" } else { "▾" });
            });
        }

        // ── Wire available networks toggle ────────────────────────────────────
        {
            let rev_c = revealer.clone();
            let state_ref: Rc<RefCell<NetworkState>> = Rc::new(RefCell::new(NetworkState {
                active: ActiveConnection::Disconnected,
                connectivity: ConnectivityState::Unknown,
                networks: Vec::new(),
                vpns: Vec::new(),
                interfaces: Vec::new(),
                wifi_radio_enabled: false,
                list_visible: false,
                show_all: false,
                scanning: false,
            }));

            let state_toggle = state_ref.clone();
            let toggle_btn_c = toggle_button.clone();
            let toggle_btn_field = toggle_button.clone();
            toggle_button.connect_clicked(move |_| {
                let mut s = state_toggle.borrow_mut();
                s.list_visible = !s.list_visible;
                rev_c.set_reveal_child(s.list_visible);
                toggle_btn_c.set_label(if s.list_visible {
                    "▾ Available Networks"
                } else {
                    "▸ Available Networks"
                });
            });

            // ── Build section ─────────────────────────────────────────────────
            let section = Self {
                root,
                state: state_ref,
                summary_icon,
                summary_text,
                summary_arrow,
                detail_revealer,
                current_icon_label,
                current_ssid_label,
                current_signal_label,
                ip_label,
                gateway_label,
                dns_label,
                connectivity_label,
                portal_btn,
                wifi_switch,
                wifi_controls_box,
                power_save_row,
                scan_spinner,
                scan_status_label,
                toggle_button: toggle_btn_field,
                revealer,
                network_list_box,
                vpn_list_box,
                iface_list_box,
            };

            // ── Async init: probe nmcli/adapter/radio on background thread ────
            let radio_row_c = radio_row;
            let ps_switch_c = ps_switch;
            let no_adapter_label_c = no_adapter_label;
            let placeholder_c = placeholder;
            let wifi_switch_init = section.wifi_switch.clone();
            let wifi_controls_init = section.wifi_controls_box.clone();
            let power_save_init = section.power_save_row.clone();
            let state_init = section.state.clone();
            let summary_icon_init = section.summary_icon.clone();
            let summary_text_init = section.summary_text.clone();

            // Clones for the WiFi radio toggle callback (wired inside the async callback).
            let wifi_switch_radio = section.wifi_switch.clone();
            let state_radio_init = section.state.clone();
            let wifi_controls_radio = section.wifi_controls_box.clone();
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
                        nmcli_available: nmcli_available(),
                        has_wifi: wifi_adapter_present(),
                        wifi_radio: wifi_radio_enabled(),
                        active_wifi_conn_name,
                        power_saving,
                    }
                },
                move |init| {
                    if !init.nmcli_available {
                        placeholder_c.set_visible(true);
                        return;
                    }

                    if init.has_wifi {
                        radio_row_c.set_visible(true);
                        wifi_switch_init.set_sensitive(true);
                        wifi_switch_init.set_active(init.wifi_radio);
                        state_init.borrow_mut().wifi_radio_enabled = init.wifi_radio;

                        if init.wifi_radio {
                            wifi_controls_init.set_visible(true);
                        }

                        if init.active_wifi_conn_name.is_some() {
                            ps_switch_c.set_active(init.power_saving);
                            if init.wifi_radio {
                                power_save_init.set_visible(true);
                            }
                        }

                        // Wire WiFi radio toggle now that we know adapter is present.
                        wifi_switch_radio.connect_state_set(move |_sw, active| {
                            let (tx, rx) = mpsc::channel::<NmResult>();
                            set_wifi_radio_async(active, tx);

                            let state_poll = state_radio_init.clone();
                            let controls_poll = wifi_controls_radio.clone();
                            let ps_poll = power_save_radio.clone();
                            let si_poll = summary_icon_radio.clone();
                            let st_poll = summary_text_radio.clone();
                            glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
                                match rx.try_recv() {
                                    Ok(NmResult::Success) => {
                                        state_poll.borrow_mut().wifi_radio_enabled = active;
                                        controls_poll.set_visible(active);
                                        ps_poll.set_visible(active && get_active_wifi_conn_name().is_some());
                                        if !active {
                                            si_poll.set_label(ICON_DISCONNECTED);
                                            st_poll.set_label("WiFi Off");
                                        }
                                        glib::ControlFlow::Break
                                    }
                                    Ok(NmResult::Failure(_)) => glib::ControlFlow::Break,
                                    Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                                    Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
                                }
                            });

                            glib::Propagation::Proceed
                        });
                    } else {
                        no_adapter_label_c.set_visible(true);
                    }
                },
            );

            // ── Async initial refresh ─────────────────────────────────────────
            section.refresh();

            // Start periodic poller.
            monitor::start_periodic_poller(
                section.state.clone(),
                monitor::PollerWidgets {
                    display: monitor::DisplayWidgets {
                        summary_icon: section.summary_icon.clone(),
                        summary_text: section.summary_text.clone(),
                        current_icon: section.current_icon_label.clone(),
                        current_ssid: section.current_ssid_label.clone(),
                        current_signal: section.current_signal_label.clone(),
                        ip_label: section.ip_label.clone(),
                        gateway_label: section.gateway_label.clone(),
                        dns_label: section.dns_label.clone(),
                    },
                    connectivity_label: section.connectivity_label.clone(),
                    portal_btn: section.portal_btn.clone(),
                    wifi_switch: section.wifi_switch.clone(),
                    wifi_controls_box: section.wifi_controls_box.clone(),
                    power_save_row: section.power_save_row.clone(),
                    iface_list_box: section.iface_list_box.clone(),
                    vpn_list_box: section.vpn_list_box.clone(),
                },
            );

            section
        }
    }

    fn display_widgets(&self) -> monitor::DisplayWidgets {
        monitor::DisplayWidgets {
            summary_icon: self.summary_icon.clone(),
            summary_text: self.summary_text.clone(),
            current_icon: self.current_icon_label.clone(),
            current_ssid: self.current_ssid_label.clone(),
            current_signal: self.current_signal_label.clone(),
            ip_label: self.ip_label.clone(),
            gateway_label: self.gateway_label.clone(),
            dns_label: self.dns_label.clone(),
        }
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
        let vpn_list_box = self.vpn_list_box.clone();
        let wifi_controls_box = self.wifi_controls_box.clone();
        let power_save_row = self.power_save_row.clone();
        let scan_spinner = self.scan_spinner.clone();
        let scan_status_label = self.scan_status_label.clone();
        let network_list_box = self.network_list_box.clone();

        spawn_work(
            || {
                let active = get_active_connection();
                let connectivity = check_connectivity();
                let interfaces = get_network_interfaces();
                let vpns = get_vpn_connections();

                // Fetch IP info for the active device while still on the background thread.
                let ip_info = match &active {
                    ActiveConnection::Wifi { device, .. } | ActiveConnection::Ethernet { device } => {
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

                RefreshResult { active, connectivity, interfaces, vpns, ip_info }
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
                );
                state_c.borrow_mut().connectivity = result.connectivity;

                // Interfaces.
                state_c.borrow_mut().interfaces = result.interfaces;
                interfaces::rebuild_iface_list(&iface_list_box, &state_c);

                // VPNs.
                state_c.borrow_mut().vpns = result.vpns;
                vpn::rebuild_vpn_list(&vpn_list_box, &state_c);

                // WiFi scan.
                if state_c.borrow().wifi_radio_enabled {
                    Self::start_wifi_scan_static(
                        &state_c,
                        &scan_spinner,
                        &scan_status_label,
                        &network_list_box,
                    );
                }

                // WiFi controls visibility.
                let radio_on = state_c.borrow().wifi_radio_enabled;
                wifi_controls_box.set_visible(radio_on);
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
        network_list_box: &ListBox,
    ) {
        if state.borrow().scanning {
            return;
        }
        state.borrow_mut().scanning = true;

        scan_spinner.set_visible(true);
        scan_spinner.start();
        scan_status_label.set_label("Scanning…");
        scan_status_label.set_visible(true);

        let scan_spinner_c = scan_spinner.clone();
        let scan_status_c = scan_status_label.clone();
        let network_list_box_c = network_list_box.clone();
        let state_c = state.clone();

        spawn_work(
            || {
                let raw = scan_wifi_raw();
                let known = get_known_ssids();
                parse_wifi_list(&raw, &known)
            },
            move |networks| {
                scan_spinner_c.stop();
                scan_spinner_c.set_visible(false);
                scan_status_c.set_visible(false);

                {
                    let mut s = state_c.borrow_mut();
                    s.networks = networks;
                    s.scanning = false;
                    s.show_all = false;
                }

                wifi::rebuild_wifi_list(&network_list_box_c, &state_c);
            },
        );
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }
}
