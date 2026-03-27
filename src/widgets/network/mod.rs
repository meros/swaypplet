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

        // ── nmcli / adapter guard ─────────────────────────────────────────────
        if !nmcli_available() {
            let placeholder = Label::builder()
                .label("NetworkManager not available")
                .halign(gtk4::Align::Start)
                .build();
            placeholder.add_css_class("network-placeholder");
            root.append(&placeholder);

            return Self::minimal(root, summary_icon, summary_text, summary_arrow);
        }

        let has_wifi = wifi_adapter_present();

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

        // ── WiFi radio toggle row ─────────────────────────────────────────────
        let wifi_switch = Switch::builder()
            .active(if has_wifi { wifi_radio_enabled() } else { false })
            .valign(gtk4::Align::Center)
            .sensitive(has_wifi)
            .build();

        let radio_row = Box::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .build();
        radio_row.add_css_class("network-switch-row");

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
        if has_wifi {
            detail_box.append(&radio_row);
        }

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

        // Initialize power saving state.
        if has_wifi && let Some(conn_name) = get_active_wifi_conn_name() {
            ps_switch.set_active(get_wifi_power_saving(&conn_name));
            power_save_row.set_visible(true);
        }

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
                                // Revert on failure.
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
        if has_wifi {
            detail_box.append(&power_save_row);
        }

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

        // ── WiFi controls box (hidden when radio off) ─────────────────────────
        let wifi_controls_box = Box::builder()
            .orientation(Orientation::Vertical)
            .spacing(4)
            .build();

        // Available Networks toggle.
        let toggle_button = Button::builder().label("▸ Available Networks").build();
        toggle_button.add_css_class("section-expander");

        let no_adapter_label = Label::builder()
            .label("No WiFi adapter found")
            .halign(gtk4::Align::Start)
            .build();
        no_adapter_label.add_css_class("network-placeholder");

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

        if !has_wifi {
            revealer_box.append(&no_adapter_label);
        }

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
                wifi_radio_enabled: has_wifi && wifi_radio_enabled(),
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

            // ── Wire WiFi radio toggle ────────────────────────────────────────
            if has_wifi {
                let state_radio = state_ref.clone();
                let wifi_controls_c = wifi_controls_box.clone();
                let power_save_c = power_save_row.clone();
                let summary_icon_c = summary_icon.clone();
                let summary_text_c = summary_text.clone();
                wifi_switch.connect_state_set(move |_sw, active| {
                    let (tx, rx) = mpsc::channel::<NmResult>();
                    set_wifi_radio_async(active, tx);

                    let state_poll = state_radio.clone();
                    let controls_poll = wifi_controls_c.clone();
                    let ps_poll = power_save_c.clone();
                    let si_poll = summary_icon_c.clone();
                    let st_poll = summary_text_c.clone();
                    glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
                        match rx.try_recv() {
                            Ok(NmResult::Success) => {
                                state_poll.borrow_mut().wifi_radio_enabled = active;
                                controls_poll.set_visible(active);
                                ps_poll.set_visible(active && get_active_wifi_conn_name().is_some());
                                if !active {
                                    si_poll.set_label(ICON_WIFI_OFF);
                                    st_poll.set_label("WiFi Off");
                                }
                                glib::ControlFlow::Break
                            }
                            Ok(NmResult::Failure(_)) => {
                                // Revert on failure.
                                glib::ControlFlow::Break
                            }
                            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                            Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
                        }
                    });

                    glib::Propagation::Proceed
                });
            }

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

    fn minimal(root: Box, summary_icon: Label, summary_text: Label, summary_arrow: Label) -> Self {
        Self {
            root,
            state: Rc::new(RefCell::new(NetworkState {
                active: ActiveConnection::Disconnected,
                connectivity: ConnectivityState::Unknown,
                networks: Vec::new(),
                vpns: Vec::new(),
                interfaces: Vec::new(),
                wifi_radio_enabled: false,
                list_visible: false,
                show_all: false,
                scanning: false,
            })),
            summary_icon,
            summary_text,
            summary_arrow,
            detail_revealer: Revealer::new(),
            current_icon_label: Label::new(None),
            current_ssid_label: Label::new(None),
            current_signal_label: Label::new(None),
            ip_label: Label::new(None),
            gateway_label: Label::new(None),
            dns_label: Label::new(None),
            connectivity_label: Label::new(None),
            portal_btn: Button::new(),
            wifi_switch: Switch::new(),
            wifi_controls_box: Box::new(Orientation::Vertical, 0),
            power_save_row: Box::new(Orientation::Horizontal, 0),
            scan_spinner: Spinner::new(),
            scan_status_label: Label::new(None),
            toggle_button: Button::new(),
            revealer: Revealer::new(),
            network_list_box: ListBox::new(),
            vpn_list_box: ListBox::new(),
            iface_list_box: ListBox::new(),
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

    pub fn refresh(&self) {
        let active = get_active_connection();
        monitor::update_active_display(&active, &self.display_widgets());
        self.state.borrow_mut().active = active;

        // Connectivity.
        let connectivity = check_connectivity();
        monitor::update_connectivity_display(
            &connectivity,
            &self.connectivity_label,
            &self.portal_btn,
            &self.summary_text,
        );
        self.state.borrow_mut().connectivity = connectivity;

        // Interfaces.
        let ifaces = get_network_interfaces();
        self.state.borrow_mut().interfaces = ifaces;
        interfaces::rebuild_iface_list(&self.iface_list_box, &self.state);

        // VPNs.
        let vpns = get_vpn_connections();
        self.state.borrow_mut().vpns = vpns;
        vpn::rebuild_vpn_list(&self.vpn_list_box, &self.state);

        // WiFi scan.
        if self.state.borrow().wifi_radio_enabled {
            self.start_wifi_scan();
        }

        // WiFi controls visibility.
        let radio_on = self.state.borrow().wifi_radio_enabled;
        self.wifi_controls_box.set_visible(radio_on);
        self.power_save_row.set_visible(
            radio_on && matches!(self.state.borrow().active, ActiveConnection::Wifi { .. }),
        );
    }

    fn start_wifi_scan(&self) {
        if self.state.borrow().scanning {
            return;
        }
        self.state.borrow_mut().scanning = true;

        self.scan_spinner.set_visible(true);
        self.scan_spinner.start();
        self.scan_status_label.set_label("Scanning…");
        self.scan_status_label.set_visible(true);

        let (tx, rx) = mpsc::channel::<Vec<WifiNetwork>>();

        std::thread::spawn(move || {
            let raw = scan_wifi_raw();
            let known = get_known_ssids();
            let networks = parse_wifi_list(&raw, &known);
            let _ = tx.send(networks);
        });

        let scan_spinner_c = self.scan_spinner.clone();
        let scan_status_c = self.scan_status_label.clone();
        let network_list_box_c = self.network_list_box.clone();
        let state_c = self.state.clone();

        glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
            match rx.try_recv() {
                Ok(networks) => {
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
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    scan_spinner_c.stop();
                    scan_spinner_c.set_visible(false);
                    scan_status_c.set_visible(false);
                    state_c.borrow_mut().scanning = false;
                    glib::ControlFlow::Break
                }
            }
        });
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }
}
