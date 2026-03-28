use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{Label, ListBox};

use super::backend::*;
use super::NetworkState;

/// Widget handles needed by the display update helpers.
pub struct DisplayWidgets {
    pub summary_icon: Label,
    pub summary_text: Label,
    pub current_icon: Label,
    pub current_ssid: Label,
    pub current_signal: Label,
    pub ip_label: Label,
    pub gateway_label: Label,
    pub dns_label: Label,
}

/// Widget handles needed by the periodic poller beyond DisplayWidgets.
pub struct PollerWidgets {
    pub display: DisplayWidgets,
    pub connectivity_label: Label,
    pub portal_btn: gtk4::Button,
    pub wifi_switch: gtk4::Switch,
    pub wifi_controls_box: gtk4::Box,
    pub power_save_row: gtk4::Box,
    pub iface_list_box: ListBox,
    pub vpn_list_box: ListBox,
}

/// State returned from the background polling thread.
struct PolledState {
    active: ActiveConnection,
    connectivity: ConnectivityState,
    wifi_radio: bool,
}

/// Cached previous state for change detection, plus polling cadence control.
struct CachedState {
    active: ActiveConnection,
    connectivity: ConnectivityState,
    wifi_radio: bool,
    accelerated: bool,
}

/// Start a periodic poller that checks network state every 5s (or 2s after changes).
/// Blocking nmcli calls run on a background thread to avoid blocking the GTK main thread.
pub fn start_periodic_poller(state: Rc<RefCell<NetworkState>>, w: PollerWidgets) {
    let w = Rc::new(w);
    let cached = Rc::new(RefCell::new(CachedState {
        active: ActiveConnection::Disconnected,
        connectivity: ConnectivityState::Unknown,
        wifi_radio: true,
        accelerated: false,
    }));

    schedule_next(state, w, cached);
}

fn schedule_next(
    state: Rc<RefCell<NetworkState>>,
    w: Rc<PollerWidgets>,
    cached: Rc<RefCell<CachedState>>,
) {
    let interval = if cached.borrow().accelerated {
        std::time::Duration::from_secs(2)
    } else {
        std::time::Duration::from_secs(5)
    };

    glib::timeout_add_local_once(interval, move || {
        let (tx, rx) = std::sync::mpsc::channel::<PolledState>();

        std::thread::spawn(move || {
            let polled = PolledState {
                active: get_active_connection(),
                connectivity: check_connectivity(),
                wifi_radio: wifi_radio_enabled(),
            };
            let _ = tx.send(polled);
        });

        let state_poll = state.clone();
        let w_poll = w.clone();
        let cached_poll = cached.clone();

        let state_resched = state;
        let w_resched = w;
        let cached_resched = cached;

        glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
            match rx.try_recv() {
                Ok(polled) => {
                    let changed =
                        apply_polled_state(&polled, &state_poll, &cached_poll, &w_poll);
                    cached_poll.borrow_mut().accelerated = changed;

                    schedule_next(
                        state_resched.clone(),
                        w_resched.clone(),
                        cached_resched.clone(),
                    );
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    schedule_next(
                        state_resched.clone(),
                        w_resched.clone(),
                        cached_resched.clone(),
                    );
                    glib::ControlFlow::Break
                }
            }
        });
    });
}

/// Compare polled state with cached state, update widgets for changed fields.
/// Returns `true` if any state changed (used to accelerate next poll).
fn apply_polled_state(
    polled: &PolledState,
    state: &Rc<RefCell<NetworkState>>,
    cached: &Rc<RefCell<CachedState>>,
    w: &PollerWidgets,
) -> bool {
    let mut prev = cached.borrow_mut();
    let mut changed = false;

    if polled.active != prev.active {
        prev.active = polled.active.clone();
        update_active_display(&polled.active, &w.display);
        state.borrow_mut().active = polled.active.clone();

        let interfaces = get_network_interfaces();
        state.borrow_mut().interfaces = interfaces;
        super::interfaces::rebuild_iface_list(&w.iface_list_box, state);

        let vpns = get_vpn_connections();
        state.borrow_mut().vpns = vpns;
        super::vpn::rebuild_vpn_list(&w.vpn_list_box, state);

        changed = true;
    }

    if polled.connectivity != prev.connectivity {
        prev.connectivity = polled.connectivity.clone();
        update_connectivity_display(
            &polled.connectivity,
            &w.connectivity_label,
            &w.portal_btn,
            &w.display.summary_text,
        );
        changed = true;
    }

    if polled.wifi_radio != prev.wifi_radio {
        prev.wifi_radio = polled.wifi_radio;
        state.borrow_mut().wifi_radio_enabled = polled.wifi_radio;
        w.wifi_switch.set_state(polled.wifi_radio);
        w.wifi_controls_box.set_visible(polled.wifi_radio);
        w.power_save_row.set_visible(
            polled.wifi_radio && matches!(state.borrow().active, ActiveConnection::Wifi { .. }),
        );
        changed = true;
    }

    changed
}

// ── Display update helpers ────────────────────────────────────────────────────

/// Update active connection display, fetching IP info synchronously.
/// Used by the periodic poller (which already runs on a background thread).
pub fn update_active_display(active: &ActiveConnection, w: &DisplayWidgets) {
    let device = update_active_labels(active, w);

    if let Some(dev) = device {
        let ip = get_device_ip(dev);
        let gateway = get_default_gateway();
        let dns = get_dns_servers(dev);
        apply_ip_info(w, &ip, &gateway, &dns);
    } else {
        apply_ip_info(w, &None, &None, &[]);
    }
}

/// Update active connection display with pre-fetched IP info.
/// Used by the async refresh path where IP info was fetched on the background thread.
pub fn update_active_display_with_ip(
    active: &ActiveConnection,
    w: &DisplayWidgets,
    ip: &Option<String>,
    gateway: &Option<String>,
    dns: &[String],
) {
    update_active_labels(active, w);
    apply_ip_info(w, ip, gateway, dns);
}

/// Set connection labels (icon, SSID, signal, summary). Returns the device name if connected.
fn update_active_labels<'a>(active: &'a ActiveConnection, w: &DisplayWidgets) -> Option<&'a str> {
    match active {
        ActiveConnection::Wifi { ssid, signal, device, freq_mhz } => {
            w.current_icon.set_label(signal_icon(*signal));
            w.current_ssid.set_label(ssid);
            let signal_text = match freq_mhz {
                Some(freq) => format!("{}% · {}", signal, freq_band_label(*freq)),
                None => format!("{}%", signal),
            };
            w.current_signal.set_label(&signal_text);
            w.summary_icon.set_label(signal_icon(*signal));
            let summary_label = match freq_mhz {
                Some(freq) => format!("{} · {}", ssid, freq_band_label(*freq)),
                None => ssid.clone(),
            };
            w.summary_text.set_label(&summary_label);
            Some(device.as_str())
        }
        ActiveConnection::Ethernet { device } => {
            w.current_icon.set_label(ICON_ETHERNET);
            w.current_ssid.set_label("Ethernet");
            w.current_signal.set_label("");
            w.summary_icon.set_label(ICON_ETHERNET);
            w.summary_text.set_label("Wired");
            Some(device.as_str())
        }
        ActiveConnection::Disconnected => {
            w.current_icon.set_label(ICON_DISCONNECTED);
            w.current_ssid.set_label("Disconnected");
            w.current_signal.set_label("");
            w.summary_icon.set_label(ICON_DISCONNECTED);
            w.summary_text.set_label("Disconnected");
            None
        }
    }
}

/// Apply pre-fetched IP / gateway / DNS info to the display widgets.
fn apply_ip_info(w: &DisplayWidgets, ip: &Option<String>, gateway: &Option<String>, dns: &[String]) {
    match ip {
        Some(ip) => {
            w.ip_label.set_label(&format!("IP: {}", ip));
            w.ip_label.set_visible(true);
        }
        None => w.ip_label.set_visible(false),
    }
    match gateway {
        Some(gw) => {
            w.gateway_label.set_label(&format!("Gateway: {}", gw));
            w.gateway_label.set_visible(true);
        }
        None => w.gateway_label.set_visible(false),
    }
    if dns.is_empty() {
        w.dns_label.set_visible(false);
    } else {
        w.dns_label.set_label(&format!("DNS: {}", dns.join(", ")));
        w.dns_label.set_visible(true);
    }
}

pub fn update_connectivity_display(
    connectivity: &ConnectivityState,
    label: &Label,
    portal_btn: &gtk4::Button,
    summary_text: &Label,
) {
    label.set_label(connectivity.label());

    label.remove_css_class("network-connectivity-ok");
    label.remove_css_class("network-connectivity-warn");
    match connectivity {
        ConnectivityState::Full => label.add_css_class("network-connectivity-ok"),
        _ => label.add_css_class("network-connectivity-warn"),
    }

    portal_btn.set_visible(matches!(connectivity, ConnectivityState::Portal));

    // Append badge to summary, or strip stale badge when connectivity improves.
    match connectivity.summary_badge() {
        Some(badge) => {
            let current = summary_text.label();
            let current_str = current.as_str();
            if !current_str.contains('⚠') {
                summary_text.set_label(&format!("{}{}", current_str, badge));
            }
        }
        None => {
            // Strip any stale badge from the summary text.
            let current = summary_text.label();
            let current_str = current.as_str();
            if let Some(pos) = current_str.find(" · ⚠") {
                summary_text.set_label(&current_str[..pos]);
            }
        }
    }

    label.set_visible(!matches!(connectivity, ConnectivityState::Full));
}
