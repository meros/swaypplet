//! The Wi-Fi page: the radios, the connection in use, the networks in
//! range, VPNs, Tailscale, saved networks and the wired adapters.
//!
//! One source of truth, [`Snapshot`], read whole from NetworkManager
//! (`services::network::snapshot`) whenever it signals a change
//! (`services::network::watch`), and only while the page is on screen.
//! Nothing polls. Opening the page draws what NetworkManager already knows
//! at once, then asks for a scan; the rows are kept per network and updated
//! in place (`wifi.rs`), so a list that is being scanned does not jump, and
//! a password being typed survives every update.
//!
//! A join is followed through the device's own states ("Getting an
//! address…") and ends in NetworkManager's reason when it fails ("Wrong
//! password" opens the password field again, with the text still in it).
//!
//! `SWAYPPLET_NET_FIXTURE=<state>` draws a canned state instead
//! (`services::network::fixture`) and makes every action a no-op, so the
//! render harness can show "connecting" or "sign-in required" without
//! touching the machine's network.

mod extras;
mod interfaces;
mod vpn;
mod wifi;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{Button, Label, ListBox, RevealerTransitionType, Spinner, Switch};

use crate::services::network::model::{self, Failure, Snapshot};
use crate::services::network::tailscale::{self, Status};
use crate::services::network::{
    ActiveConnection, ConnectivityState, ICON_DISCONNECTED, ICON_ETHERNET, NmResult,
    display_name, fixture, signal_icon, snapshot, watch,
};
use crate::spawn::spawn_work;
use crate::ui;

use extras::{Saved, SavedAsk, Tailscale, TailscaleAsk};
use wifi::{Ask, WifiRow};

/// How long after the last change signal the page rereads: a scan or a
/// join is a burst of signals, and one read covers the burst.
const SETTLE_MS: u64 = 120;

/// How long a scan may take before the spinner gives up on it.
const SCAN_PATIENCE_S: u32 = 8;

/// Rows shown before "Show all".
const FIRST_ROWS: usize = 8;

/// A signal is only coloured when it is a problem: weak is a warning, none
/// is danger, and anything usable stays in the icon's own tone.
fn signal_tone(strength: u8) -> ui::Tone {
    match strength {
        0..=20 => ui::Tone::Danger,
        21..=40 => ui::Tone::Warning,
        _ => ui::Tone::Fg,
    }
}

/// A network glyph at title size, in the tone its signal earns.
fn set_signal_glyph(icon: &Label, glyph: &str, tone: ui::Tone) {
    icon.set_label(glyph);
    ui::glyph::adopt(icon, ui::Text::Title, tone);
}

/// The connection in use: its row, a line for a sign-in page, and the
/// details and switches that open under it.
struct Current {
    card: gtk4::Box,
    row: ui::Row,
    spinner: Spinner,
    disconnect: Button,
    portal: gtk4::Box,
    lines: gtk4::Box,
    metered_row: gtk4::Box,
    metered: Switch,
    powersave_row: gtk4::Box,
    powersave: Switch,
}

struct Inner {
    section: ui::Section,
    fixture: Option<String>,
    snap: RefCell<Snapshot>,
    syncing: Cell<bool>,

    wifi_row: gtk4::Box,
    wifi_switch: Switch,
    airplane_row: gtk4::Box,
    airplane: Switch,
    off_note: gtk4::Box,
    current: Current,

    list_box: gtk4::Box,
    list: ListBox,
    rows: RefCell<HashMap<String, WifiRow>>,
    order: RefCell<Vec<String>>,
    empty: Label,
    more: Button,
    show_all: Cell<bool>,
    scan_spinner: Spinner,
    scan_btn: Button,
    scanning: Cell<bool>,
    scan_from: Cell<i64>,

    vpn_box: gtk4::Box,
    vpns: vpn::VpnList,
    tailscale: Tailscale,
    ts_status: RefCell<Option<Status>>,
    saved: Saved,
    adapters_box: gtk4::Box,
    adapters: interfaces::Adapters,

    /// The network a join was asked for, until it is up or has failed.
    joining: RefCell<Option<String>>,
    failure: RefCell<Option<Failure>>,
    watch: RefCell<Option<watch::Watch>>,
    reading: Cell<bool>,
    reread: Cell<bool>,
    settle: RefCell<Option<glib::SourceId>>,
}

#[derive(Clone)]
pub struct NetworkSection(Rc<Inner>);

impl NetworkSection {
    pub fn new() -> Self {
        let section = ui::section(ICON_DISCONNECTED, "Wi-Fi", "Disconnected");
        ui::glyph::adopt(&section.icon, ui::Text::Title, ui::Tone::Fg);
        let body = ui::vbox(4);
        section.body.append(&body);

        // ── The radios ──────────────────────────────────────────────────
        let radios = ui::group(0);
        let (wifi, wifi_switch) = ui::switch_row("Wi-Fi", "");
        wifi.icon.set_label(signal_icon(100));
        wifi.icon.set_visible(true);
        ui::glyph::adopt(&wifi.icon, ui::Text::Title, ui::Tone::Fg);
        let (air, airplane) = ui::switch_row("Airplane mode", "Wi-Fi and Bluetooth off");
        air.icon.set_label("󰀝");
        air.icon.set_visible(true);
        ui::glyph::adopt(&air.icon, ui::Text::Title, ui::Tone::Fg);
        radios.append(&wifi.root);
        radios.append(&air.root);
        body.append(&radios);

        let off_note = ui::vbox(1);
        off_note.add_css_class("network-disabled");
        let off_title = ui::text("Wi-Fi is off", ui::Text::Body, ui::Tone::Fg);
        ui::set_weight(&off_title, ui::Weight::Strong);
        let off_sub = ui::text(
            "Turn it on to see networks in range",
            ui::Text::Label,
            ui::Tone::Muted,
        );
        off_note.append(&off_title);
        off_note.append(&off_sub);
        off_note.set_visible(false);
        body.append(&off_note);

        // ── The connection in use ───────────────────────────────────────
        let card = ui::group(1);
        card.add_css_class("network-hero");
        let row = ui::row(ICON_DISCONNECTED, "", "");
        ui::glyph::adopt(&row.icon, ui::Text::Title, ui::Tone::Fg);
        let spinner = Spinner::new();
        spinner.set_visible(false);
        let disconnect = ui::button_with(
            ui::Face::Label("Disconnect"),
            ui::Kind::Secondary,
            ui::Size::Small,
        );
        let details_btn = ui::button_with(ui::Face::Label("Details"), ui::Kind::Flat, ui::Size::Small);
        row.end.append(&spinner);
        row.end.append(&disconnect);
        row.end.append(&details_btn);
        card.append(&row.root);

        let portal = ui::hbox(3);
        portal.add_css_class("network-hero-line");
        let portal_text = ui::text(
            "This network wants you to sign in",
            ui::Text::Label,
            ui::Tone::Warning,
        );
        portal_text.set_hexpand(true);
        portal_text.set_xalign(0.0);
        let portal_btn = ui::button_with(ui::Face::Label("Sign in"), ui::Kind::Primary, ui::Size::Small);
        portal.append(&portal_text);
        portal.append(&portal_btn);
        portal.set_visible(false);
        card.append(&portal);

        let details = ui::revealer(
            RevealerTransitionType::SlideDown,
            crate::tokens::motion::EXPAND,
        );
        let tray = ui::vbox(2);
        tray.append(&ui::separator(gtk4::Orientation::Horizontal));
        let lines = ui::vbox(1);
        lines.add_css_class("network-hero-line");
        tray.append(&lines);
        let (metered_r, metered) = ui::switch_row(
            "Metered connection",
            "Apps hold back big downloads on it",
        );
        let (ps_r, powersave) = ui::switch_row("Power saving", "Saves battery, adds latency");
        tray.append(&metered_r.root);
        tray.append(&ps_r.root);
        details.set_child(Some(&tray));
        card.append(&details);
        {
            let d = details.clone();
            details_btn.connect_clicked(move |b| {
                let open = !d.reveals_child();
                d.set_reveal_child(open);
                b.set_label(if open { "Less" } else { "Details" });
            });
        }
        card.set_visible(false);
        body.append(&card);

        // ── Networks in range ───────────────────────────────────────────
        let list_box = ui::vbox(2);
        let head = ui::hbox(2);
        let heading = ui::heading("Networks");
        heading.set_hexpand(true);
        heading.set_xalign(0.0);
        let scan_spinner = Spinner::new();
        scan_spinner.set_visible(false);
        let scan_btn = ui::button_with(ui::Face::Label("Scan"), ui::Kind::Flat, ui::Size::Small);
        head.append(&heading);
        head.append(&scan_spinner);
        head.append(&scan_btn);
        list_box.append(&head);
        let list = ui::list();
        list_box.append(&list);
        let empty = ui::text("Looking for networks…", ui::Text::Label, ui::Tone::Muted);
        empty.set_visible(false);
        list_box.append(&empty);
        let more = ui::button_with(ui::Face::Label("Show all"), ui::Kind::Flat, ui::Size::Small);
        more.set_halign(gtk4::Align::Center);
        more.set_visible(false);
        list_box.append(&more);
        body.append(&list_box);

        // ── VPN ─────────────────────────────────────────────────────────
        let vpn_box = ui::vbox(2);
        vpn_box.append(&ui::heading("VPN"));
        vpn_box.set_visible(false);
        body.append(&vpn_box);

        let inner = Rc::new_cyclic(|weak: &std::rc::Weak<Inner>| {
            let w = weak.clone();
            let vpns = vpn::VpnList::new(Rc::new(move |name, on| {
                if let Some(i) = w.upgrade() {
                    NetworkSection(i).toggle_vpn(name, on);
                }
            }));
            let w = weak.clone();
            let tailscale = Tailscale::new(Rc::new(move |ask| {
                if let Some(i) = w.upgrade() {
                    NetworkSection(i).tailscale(ask);
                }
            }));
            let w = weak.clone();
            let saved = Saved::new(Rc::new(move |ask| {
                if let Some(i) = w.upgrade() {
                    NetworkSection(i).saved(ask);
                }
            }));
            let w = weak.clone();
            let adapters = interfaces::Adapters::new(Rc::new(move |ask| {
                if let Some(i) = w.upgrade() {
                    NetworkSection(i).adapter(ask);
                }
            }));
            Inner {
                section,
                fixture: fixture::requested(),
                snap: RefCell::default(),
                syncing: Cell::new(false),
                wifi_row: wifi.root.clone(),
                wifi_switch,
                airplane_row: air.root.clone(),
                airplane,
                off_note,
                current: Current {
                    card,
                    row,
                    spinner,
                    disconnect,
                    portal,
                    lines,
                    metered_row: metered_r.root.clone(),
                    metered,
                    powersave_row: ps_r.root.clone(),
                    powersave,
                },
                list_box,
                list,
                rows: RefCell::default(),
                order: RefCell::default(),
                empty,
                more,
                show_all: Cell::new(false),
                scan_spinner,
                scan_btn,
                scanning: Cell::new(false),
                scan_from: Cell::new(-1),
                vpn_box,
                vpns,
                tailscale,
                ts_status: RefCell::default(),
                saved,
                adapters_box: ui::vbox(2),
                adapters,
                joining: RefCell::default(),
                failure: RefCell::default(),
                watch: RefCell::default(),
                reading: Cell::new(false),
                reread: Cell::new(false),
                settle: RefCell::default(),
            }
        });
        let this = NetworkSection(inner);
        let i = &this.0;
        i.vpn_box.append(&i.vpns.list);
        body.append(&i.tailscale.root);
        body.append(&i.saved.disclosure.root);

        // ── Advanced ────────────────────────────────────────────────────
        let adv = ui::disclosure("Adapters and advanced");
        i.adapters_box.append(&i.adapters.list);
        adv.body.append(&i.adapters_box);
        let editor = ui::button(
            "Connection editor (nm-connection-editor)",
            ui::Kind::Secondary,
        );
        editor.add_css_class("section-launch-btn");
        editor.connect_clicked(|_| {
            let _ = std::process::Command::new("nm-connection-editor").spawn();
        });
        adv.body.append(&editor);
        body.append(&adv.root);

        this.wire(portal_btn);
        {
            let s = this.clone();
            i.section.root.connect_map(move |_| s.on_screen(true));
            let s = this.clone();
            i.section.root.connect_unmap(move |_| s.on_screen(false));
        }
        // Drawn from what NetworkManager knows before the page is ever
        // opened, so the first frame of it is never empty.
        this.refresh();
        {
            let weak = Rc::downgrade(&this.0);
            crate::services::devices::observe(move || {
                if let Some(i) = weak.upgrade() {
                    NetworkSection(i).names_changed();
                }
            });
        }
        this
    }

    fn wire(&self, portal_btn: Button) {
        let i = &self.0;
        {
            let s = self.clone();
            i.wifi_switch.connect_state_set(move |_, on| {
                if !s.0.syncing.get() {
                    s.act(move || crate::services::network::set_wifi_radio(on), |_| {});
                }
                glib::Propagation::Proceed
            });
        }
        {
            let s = self.clone();
            i.airplane.connect_state_set(move |_, on| {
                if !s.0.syncing.get() {
                    s.act(move || snapshot::set_airplane(on), |_| {});
                }
                glib::Propagation::Proceed
            });
        }
        {
            let s = self.clone();
            i.current.disconnect.connect_clicked(move |_| {
                *s.0.joining.borrow_mut() = None;
                s.act(snapshot::disconnect_wifi, |_| {});
            });
        }
        {
            let s = self.clone();
            i.current.metered.connect_state_set(move |_, on| {
                if !s.0.syncing.get() {
                    let d = s.0.snap.borrow().details.clone();
                    if let Some(d) = d
                        && let Some(id) = d.connection_id
                    {
                        s.act(move || snapshot::set_metered(&id, &d.device, on), |_| {});
                    }
                }
                glib::Propagation::Proceed
            });
        }
        {
            let s = self.clone();
            i.current.powersave.connect_state_set(move |_, on| {
                if !s.0.syncing.get() {
                    let id = s
                        .0
                        .snap
                        .borrow()
                        .details
                        .as_ref()
                        .and_then(|d| d.connection_id.clone());
                    if let Some(id) = id {
                        s.act(move || snapshot::set_power_saving(&id, on), |_| {});
                    }
                }
                glib::Propagation::Proceed
            });
        }
        {
            let s = self.clone();
            portal_btn.connect_clicked(move |_| {
                let uri = s
                    .0
                    .snap
                    .borrow()
                    .portal_uri
                    .clone()
                    .unwrap_or_else(|| "http://nmcheck.gnome.org/".into());
                if s.0.fixture.is_none() {
                    let _ = std::process::Command::new("xdg-open").arg(uri).spawn();
                }
            });
        }
        {
            let s = self.clone();
            i.scan_btn.connect_clicked(move |_| s.trigger_scan());
        }
        {
            let s = self.clone();
            i.more.connect_clicked(move |_| {
                s.0.show_all.set(!s.0.show_all.get());
                s.draw_list();
            });
        }
    }

    // ── Reading ────────────────────────────────────────────────────────

    /// Follow NetworkManager while the page is on screen, and not at all
    /// while it is not.
    fn on_screen(&self, shown: bool) {
        let i = &self.0;
        if !shown {
            i.watch.borrow_mut().take();
            // A fresh open ranks by signal again (`model::settle`).
            i.order.borrow_mut().clear();
            return;
        }
        if i.fixture.is_none() && i.watch.borrow().is_none() {
            let (tx, rx) = async_channel::bounded::<()>(1);
            *i.watch.borrow_mut() = Some(watch::start(tx));
            let weak = Rc::downgrade(&self.0);
            glib::spawn_future_local(async move {
                while rx.recv().await.is_ok() {
                    let Some(i) = weak.upgrade() else { break };
                    NetworkSection(i).changed();
                }
            });
        }
        self.refresh();
        self.read_tailscale();
    }

    /// A change signal: reread once the burst is over.
    fn changed(&self) {
        let i = &self.0;
        if i.settle.borrow().is_some() {
            return;
        }
        let s = self.clone();
        *i.settle.borrow_mut() = Some(glib::timeout_add_local_once(
            std::time::Duration::from_millis(SETTLE_MS),
            move || {
                s.0.settle.borrow_mut().take();
                s.refresh();
            },
        ));
    }

    /// Read the whole state and draw it. One read at a time; a request
    /// during a read runs once after it.
    pub fn refresh(&self) {
        let i = &self.0;
        if let Some(name) = &i.fixture {
            let snap = fixture::snapshot(name);
            if let Some(p) = fixture::pending(name) {
                *i.joining.borrow_mut() = Some(p);
            }
            *i.ts_status.borrow_mut() = fixture::tailscale(name);
            self.apply(snap);
            return;
        }
        if i.reading.replace(true) {
            i.reread.set(true);
            return;
        }
        let s = self.clone();
        spawn_work(snapshot::read, move |snap| {
            s.0.reading.set(false);
            s.apply(snap);
            if s.0.reread.replace(false) {
                s.refresh();
            }
        });
    }

    fn read_tailscale(&self) {
        if self.0.fixture.is_some() {
            return;
        }
        let s = self.clone();
        spawn_work(tailscale::status, move |status| {
            *s.0.ts_status.borrow_mut() = status;
            s.0.tailscale.update(s.0.ts_status.borrow().as_ref());
        });
    }

    /// Ask for a scan. The results arrive as change signals; the spinner
    /// runs until `LastScan` moves or [`SCAN_PATIENCE_S`] pass.
    pub fn trigger_scan(&self) {
        let i = &self.0;
        if !i.snap.borrow().wifi_enabled || i.scanning.get() {
            return;
        }
        i.scanning.set(true);
        i.scan_from.set(i.snap.borrow().last_scan);
        i.scan_spinner.set_visible(true);
        i.scan_spinner.start();
        i.scan_btn.set_sensitive(false);
        if i.fixture.is_some() {
            // Nothing to wait for: the fixture's list is the result.
            self.scan_done();
            return;
        }
        spawn_work(snapshot::request_scan, |_| {});
        let s = self.clone();
        glib::timeout_add_seconds_local_once(SCAN_PATIENCE_S, move || s.scan_done());
    }

    fn scan_done(&self) {
        let i = &self.0;
        if !i.scanning.replace(false) {
            return;
        }
        i.scan_spinner.stop();
        i.scan_spinner.set_visible(false);
        i.scan_btn.set_sensitive(true);
        self.draw_list();
    }

    // ── Acting ─────────────────────────────────────────────────────────

    /// Run `work` on a worker, then reread. Under a fixture, nothing runs.
    fn act(
        &self,
        work: impl FnOnce() -> NmResult + Send + 'static,
        on_done: impl FnOnce(&NmResult) + 'static,
    ) {
        if let Some(name) = &self.0.fixture {
            log::info!("network: fixture {name}: an action was asked for and not run");
            return;
        }
        let s = self.clone();
        spawn_work(work, move |result| {
            if let NmResult::Failure(msg) = &result {
                log::info!("network: {msg}");
            }
            on_done(&result);
            s.refresh();
        });
    }

    fn ask(&self, ask: Ask) {
        let i = &self.0;
        *i.failure.borrow_mut() = None;
        let ssid = match &ask {
            Ask::Saved(id) => i
                .snap
                .borrow()
                .saved
                .iter()
                .find(|s| s.id == *id)
                .map_or_else(|| id.clone(), |s| s.ssid.clone()),
            Ask::Join(ssid, ..) => ssid.clone(),
        };
        *i.joining.borrow_mut() = Some(ssid.clone());
        self.draw_list();
        let s = self.clone();
        let fail = move |r: &NmResult| {
            if let NmResult::Failure(msg) = r {
                let reason = s.0.snap.borrow().wifi_reason;
                let (text, needs) = if reason != 0 {
                    (model::failure_text(reason), model::wants_password(reason))
                } else {
                    let m = msg.to_lowercase();
                    (msg.clone(), m.contains("password") || m.contains("secret"))
                };
                *s.0.failure.borrow_mut() = Some(Failure {
                    ssid: ssid.clone(),
                    text,
                    needs_password: needs,
                });
                *s.0.joining.borrow_mut() = None;
            } else {
                *s.0.joining.borrow_mut() = None;
            }
        };
        match ask {
            Ask::Saved(id) => self.act(move || snapshot::connect_saved(&id), fail),
            Ask::Join(ssid, pw, sec) => {
                self.act(move || snapshot::join(&ssid, &pw, &sec, false), fail)
            }
        }
    }

    fn toggle_vpn(&self, name: String, on: bool) {
        let s = self.clone();
        let n = name.clone();
        self.act(move || snapshot::vpn(&name, on), move |r| {
            s.0.vpns.set_error(
                &n,
                match r {
                    NmResult::Failure(m) => Some(m.clone()),
                    NmResult::Success => None,
                },
            );
        });
    }

    fn adapter(&self, ask: interfaces::AdapterAsk) {
        use crate::services::network;
        match ask {
            interfaces::AdapterAsk::Toggle(dev, on) => self.act(
                move || {
                    if on {
                        network::device_connect(&dev)
                    } else {
                        network::device_disconnect(&dev)
                    }
                },
                |_| {},
            ),
            interfaces::AdapterAsk::Ban(dev, ban) => {
                self.act(move || network::set_banned(&dev, ban), |_| {})
            }
            // Settings, not NetworkManager: stored on this thread, and the
            // list redrawn from the snapshot it already has.
            // Settings, not NetworkManager: stored on this thread; the
            // names redraw through `devices::observe` (`names_changed`).
            interfaces::AdapterAsk::Rename(mac, name) => {
                crate::services::devices::rename(
                    &crate::services::devices::DeviceKey::Net(mac),
                    &name,
                );
            }
        }
    }

    fn tailscale(&self, ask: TailscaleAsk) {
        if self.0.fixture.is_some() {
            return;
        }
        let s = self.clone();
        spawn_work(
            move || match ask {
                TailscaleAsk::Up(up) => tailscale::set_up(up),
                TailscaleAsk::ExitNode(ip) => tailscale::set_exit_node(ip.as_deref()),
            },
            move |r| {
                if let Err(e) = r {
                    log::info!("network: tailscale: {e}");
                }
                s.read_tailscale();
            },
        );
    }

    fn saved(&self, ask: SavedAsk) {
        match ask {
            SavedAsk::Autoconnect(id, on) => {
                self.act(move || snapshot::set_autoconnect(&id, on), |_| {})
            }
            SavedAsk::Forget(id) => self.act(move || snapshot::forget(&id), |_| {}),
        }
    }

    // ── Drawing ────────────────────────────────────────────────────────

    /// A device name may have changed: redraw what shows adapter names
    /// from the snapshot already held.
    fn names_changed(&self) {
        let i = &self.0;
        let snap = i.snap.borrow().clone();
        i.syncing.set(true);
        self.draw_current(&snap);
        i.syncing.set(false);
        i.adapters.update(&snap.interfaces);
        self.draw_summary(&snap);
    }

    fn apply(&self, snap: Snapshot) {
        let i = &self.0;
        // A join that ended: up, or failed with a reason.
        let joining = i.joining.borrow().clone();
        if let Some(ssid) = joining {
            let up = matches!(&snap.active, ActiveConnection::Wifi { ssid: s, .. } if *s == ssid);
            let failed = snap.activating.is_none()
                && matches!(snap.wifi_state, 30 | 120)
                && snap.wifi_reason != 0;
            if up {
                *i.joining.borrow_mut() = None;
                *i.failure.borrow_mut() = None;
            } else if failed {
                *i.failure.borrow_mut() = Some(Failure {
                    ssid: ssid.clone(),
                    text: model::failure_text(snap.wifi_reason),
                    needs_password: model::wants_password(snap.wifi_reason),
                });
                *i.joining.borrow_mut() = None;
            }
        }
        if i.scanning.get() && snap.last_scan > i.scan_from.get() {
            *i.snap.borrow_mut() = snap;
            self.scan_done();
        } else {
            *i.snap.borrow_mut() = snap;
        }
        let snap = i.snap.borrow().clone();

        i.syncing.set(true);
        self.draw_radios(&snap);
        self.draw_current(&snap);
        i.syncing.set(false);
        self.draw_list();
        i.vpn_box.set_visible(!snap.vpns.is_empty());
        i.vpns.update(&snap.vpns);
        i.tailscale.update(i.ts_status.borrow().as_ref());
        i.saved.update(&snap.saved);
        i.adapters.update(&snap.interfaces);
        i.adapters_box.set_visible(!snap.interfaces.is_empty());
        self.draw_summary(&snap);
    }

    fn draw_radios(&self, s: &Snapshot) {
        let i = &self.0;
        i.wifi_row.set_visible(s.has_wifi);
        i.wifi_switch.set_active(s.wifi_enabled);
        i.wifi_switch.set_state(s.wifi_enabled);
        let airplane = model::airplane(s);
        i.airplane.set_active(airplane);
        i.airplane.set_state(airplane);
        i.airplane_row
            .set_visible(s.has_wifi || s.bluetooth_powered.is_some());
        let off = s.has_wifi && !s.wifi_enabled;
        i.off_note.set_visible(off);
        i.list_box.set_visible(s.has_wifi && s.wifi_enabled);
        if !s.available {
            i.off_note.set_visible(true);
        }
    }

    fn draw_current(&self, s: &Snapshot) {
        let c = &self.0.current;
        let joining = s.activating.as_ref().map(|a| a.ssid.clone());
        match (&s.active, &joining) {
            (ActiveConnection::Wifi { ssid, signal, .. }, _) => {
                set_signal_glyph(&c.row.icon, signal_icon(*signal), signal_tone(*signal));
                c.row.title.set_label(ssid);
                c.disconnect.set_label("Disconnect");
                c.disconnect.set_visible(true);
                c.spinner.set_visible(false);
            }
            (ActiveConnection::Ethernet { device }, _) => {
                set_signal_glyph(&c.row.icon, ICON_ETHERNET, ui::Tone::Fg);
                // The adapter's name, as the Advanced list shows it; the
                // chipset and kernel name go to the details' Device line.
                let name = s
                    .interfaces
                    .iter()
                    .find(|i| &i.device == device)
                    .map(|i| display_name(&i.label, i.mac.as_deref()))
                    .unwrap_or_else(|| format!("Wired · {device}"));
                c.row.title.set_label(&name);
                c.disconnect.set_visible(false);
                c.spinner.set_visible(false);
            }
            (ActiveConnection::Disconnected, Some(ssid)) => {
                set_signal_glyph(&c.row.icon, signal_icon(60), ui::Tone::Fg);
                c.row.title.set_label(ssid);
                c.disconnect.set_label("Cancel");
                c.disconnect.set_visible(true);
                c.spinner.set_visible(true);
                c.spinner.start();
            }
            (ActiveConnection::Disconnected, None) => {
                c.card.set_visible(false);
                return;
            }
        }
        c.card.set_visible(true);

        let (sub, tone) = match (&s.active, &s.activating) {
            (ActiveConnection::Disconnected, Some(a)) => (a.stage.to_string(), ui::Tone::Accent),
            _ => (
                model::active_subtitle(&s.connectivity, s.details.as_ref()),
                match s.connectivity {
                    ConnectivityState::Full | ConnectivityState::Unknown => ui::Tone::Muted,
                    _ => ui::Tone::Warning,
                },
            ),
        };
        c.row.subtitle.set_label(&sub);
        c.row.subtitle.set_visible(true);
        ui::set_tone(&c.row.subtitle, tone);
        c.portal
            .set_visible(matches!(s.connectivity, ConnectivityState::Portal));

        while let Some(ch) = c.lines.first_child() {
            c.lines.remove(&ch);
        }
        if let Some(d) = &s.details {
            let add = |k: &str, v: &str| {
                let l = ui::text(&format!("{k:<9} {v}"), ui::Text::Caption, ui::Tone::Muted);
                ui::set_mono(&l, true);
                l.set_xalign(0.0);
                l.set_selectable(true);
                c.lines.append(&l);
            };
            if let Some(v) = &d.ip4 {
                add("Address", v);
            }
            if let Some(v) = &d.ip6 {
                add("IPv6", v);
            }
            if let Some(v) = &d.gateway {
                add("Gateway", v);
            }
            if !d.dns.is_empty() {
                add("DNS", &d.dns.join(", "));
            }
            let mut link = Vec::new();
            if let Some(b) = d.bitrate_mbps.filter(|b| *b > 0) {
                link.push(format!("{b} Mb/s"));
            }
            if let Some(f) = d.freq_mhz {
                link.push(format!("{f} MHz"));
            }
            if !d.security.is_empty() {
                link.push(d.security.clone());
            }
            if !link.is_empty() {
                add("Link", &link.join(" · "));
            }
            if let ActiveConnection::Ethernet { device } = &s.active {
                let chipset = s
                    .interfaces
                    .iter()
                    .find(|i| &i.device == device)
                    .and_then(|i| i.chipset.clone());
                add(
                    "Device",
                    &match chipset {
                        Some(c) => format!("{c} · {device}"),
                        None => device.clone(),
                    },
                );
            }
            if let Some(v) = &d.hw_address {
                add("Hardware", v);
            }
            let wifi = matches!(s.active, ActiveConnection::Wifi { .. });
            c.metered_row.set_visible(d.connection_id.is_some());
            c.metered.set_active(d.metered.is_metered());
            c.metered.set_state(d.metered.is_metered());
            c.powersave_row.set_visible(wifi && d.connection_id.is_some());
            c.powersave.set_active(d.power_saving);
            c.powersave.set_state(d.power_saving);
        } else {
            c.metered_row.set_visible(false);
            c.powersave_row.set_visible(false);
        }
    }

    /// Add, update, remove and (rarely) reorder the rows.
    fn draw_list(&self) {
        let i = &self.0;
        let snap = i.snap.borrow().clone();
        // The network in use, or the one coming up, has the card above; it
        // is not a row too.
        let coming = snap.activating.as_ref().map(|a| a.ssid.as_str());
        let fresh: Vec<_> = snap
            .networks
            .iter()
            .filter(|n| !n.in_use && Some(n.ssid.as_str()) != coming)
            .cloned()
            .collect();
        let settled = model::settle(&i.order.borrow(), fresh);
        let order: Vec<String> = settled.iter().map(|n| n.ssid.clone()).collect();

        let ask: Rc<dyn Fn(Ask)> = {
            let weak = Rc::downgrade(&self.0);
            Rc::new(move |a| {
                if let Some(i) = weak.upgrade() {
                    NetworkSection(i).ask(a);
                }
            })
        };
        let joining = i.joining.borrow().clone();
        let failure = i.failure.borrow().clone();
        let mut rows = i.rows.borrow_mut();
        rows.retain(|ssid, row| {
            let keep = order.contains(ssid);
            if !keep {
                i.list.remove(&row.list_row);
            }
            keep
        });
        let reorder = *i.order.borrow() != order;
        for n in &settled {
            match rows.get(&n.ssid) {
                Some(r) => r.update(n, &snap, joining.as_deref(), failure.as_ref()),
                None => {
                    let r = WifiRow::new(n, &snap, ask.clone());
                    r.update(n, &snap, joining.as_deref(), failure.as_ref());
                    rows.insert(n.ssid.clone(), r);
                }
            }
        }
        if reorder {
            // Detach and re-append in order. Rows keep their widgets, so a
            // field being typed in keeps its text.
            for n in &settled {
                let r = &rows[&n.ssid];
                if r.list_row.parent().is_some() {
                    i.list.remove(&r.list_row);
                }
                i.list.append(&r.list_row);
            }
            *i.order.borrow_mut() = order.clone();
        }
        let all = i.show_all.get();
        for (k, n) in settled.iter().enumerate() {
            rows[&n.ssid]
                .list_row
                .set_visible(all || k < FIRST_ROWS || joining.as_deref() == Some(n.ssid.as_str()));
        }
        i.more.set_visible(settled.len() > FIRST_ROWS);
        i.more.set_label(if all {
            "Show fewer".to_string()
        } else {
            format!("Show all {}", settled.len())
        }
        .as_str());
        i.empty.set_visible(settled.is_empty());
        i.empty.set_label(if i.scanning.get() {
            "Looking for networks…"
        } else {
            "No other networks in range"
        });
    }

    fn draw_summary(&self, s: &Snapshot) {
        let i = &self.0;
        let (icon, text) = if !s.available {
            (ICON_DISCONNECTED, "Unavailable".to_string())
        } else if model::airplane(s) {
            ("󰀝", "Airplane mode".to_string())
        } else if let Some(a) = &s.activating {
            (signal_icon(60), format!("{}…", a.ssid))
        } else {
            match &s.active {
                ActiveConnection::Wifi { ssid, signal, .. } => {
                    let mut t = ssid.clone();
                    match s.connectivity {
                        ConnectivityState::Portal => t.push_str(" · Sign in"),
                        ConnectivityState::Limited | ConnectivityState::None => {
                            t.push_str(" · No internet")
                        }
                        _ => {}
                    }
                    (signal_icon(*signal), t)
                }
                // The adapter's name, as Wi-Fi shows the network's.
                ActiveConnection::Ethernet { device } => (
                    ICON_ETHERNET,
                    s.interfaces
                        .iter()
                        .find(|i| &i.device == device)
                        .map(|i| display_name(&i.label, i.mac.as_deref()))
                        .unwrap_or_else(|| "Wired".to_string()),
                ),
                ActiveConnection::Disconnected if !s.wifi_enabled => {
                    (ICON_DISCONNECTED, "Off".to_string())
                }
                ActiveConnection::Disconnected => {
                    (ICON_DISCONNECTED, "Not connected".to_string())
                }
            }
        };
        i.section.icon.set_label(icon);
        i.section.summary.set_label(&text);
    }

    pub fn expand_for_page(&self) {
        self.0.section.show_as_page();
        self.trigger_scan();
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.0.section.root
    }
}
