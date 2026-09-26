//! Bluetooth section: power, connected devices, your devices, and nearby
//! ones while the section is on screen.
//!
//! State comes from [`crate::services::bluetooth`], which follows BlueZ's
//! signals, so a device that connects by itself appears here without a
//! refresh, and what a device is doing (connecting, pairing, a code to
//! confirm, why it failed) is part of that state rather than of the row's
//! widgets. Rows are kept per device and updated in place; a group is
//! rebuilt only when who is in it changes, so a button never loses focus
//! to an unrelated battery reading.
//!
//! Scanning follows the section's visibility: it starts when the section is
//! mapped (its sheet on screen) and stops when it is unmapped. No timer.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gtk4::prelude::*;

use crate::services::bluetooth::{BluetoothService, BtState, Command, Op};
use crate::services::bluez::Device;
use crate::ui;
use crate::ui::icons;

/// The glyph for a device, from BlueZ's `Icon` hint.
pub fn device_icon(hint: Option<&str>) -> &'static str {
    match hint.unwrap_or_default() {
        h if h.contains("headset") => icons::HEADSET,
        h if h.contains("headphone") || h.starts_with("audio") => icons::HEADPHONES,
        h if h.contains("keyboard") => icons::KEYBOARD,
        h if h.contains("mouse") || h.contains("tablet") => icons::MOUSE,
        h if h.contains("phone") => icons::PHONE,
        h if h.contains("watch") => icons::WATCH,
        h if h.contains("gaming") || h.contains("joystick") => icons::GAMEPAD,
        h if h.contains("computer") => icons::COMPUTER,
        h if h.contains("video") || h.contains("display") => icons::TV,
        _ => icons::BLUETOOTH,
    }
}

/// Which list a device belongs in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Group {
    Connected,
    Mine,
    Nearby,
}

/// Where each device goes, in order. Nearby shows only named devices seen
/// by a scan (or with something under way), strongest first: an address
/// alone is not something a person can pick.
pub fn grouped(state: &BtState) -> Vec<(Group, &Device)> {
    let mut out: Vec<(Group, &Device)> = Vec::new();
    for d in &state.snapshot.devices {
        let group = if d.connected {
            Group::Connected
        } else if d.paired {
            Group::Mine
        } else if d.named && (d.rssi.is_some() || state.ops.contains_key(&d.mac)) {
            Group::Nearby
        } else {
            continue;
        };
        out.push((group, d));
    }
    let order = |g: &Group| match g {
        Group::Connected => 0,
        Group::Mine => 1,
        Group::Nearby => 2,
    };
    out.sort_by(|(ga, a), (gb, b)| {
        order(ga).cmp(&order(gb)).then_with(|| match ga {
            Group::Nearby => b.rssi.cmp(&a.rssi),
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        })
    });
    out
}

/// The line under a device's name, and its tone.
pub fn subtitle(d: &Device, group: Group, op: Option<&Op>) -> (String, ui::Tone) {
    match op {
        Some(Op::Connecting) => ("Connecting…".into(), ui::Tone::Muted),
        Some(Op::Disconnecting) => ("Disconnecting…".into(), ui::Tone::Muted),
        Some(Op::Pairing) => ("Pairing…".into(), ui::Tone::Muted),
        Some(Op::Forgetting) => ("Forgetting…".into(), ui::Tone::Muted),
        Some(Op::Confirm(code)) => (format!("Does {} show {code}?", d.name), ui::Tone::Fg),
        Some(Op::Show(code)) => (format!("Type {code} on it, then Enter"), ui::Tone::Fg),
        Some(Op::Failed(why)) => (why.clone(), ui::Tone::Danger),
        None => match (group, d.battery) {
            (Group::Connected, Some(p)) => {
                (format!("{} {p} %", icons::battery(p)), ui::Tone::Muted)
            }
            (Group::Connected, None) => ("Connected".into(), ui::Tone::Muted),
            (Group::Mine, _) => ("Not connected".into(), ui::Tone::Faint),
            (Group::Nearby, _) => ("Not paired".into(), ui::Tone::Faint),
        },
    }
}

/// What the second button on a row does.
#[derive(Clone)]
enum Secondary {
    Hidden,
    Send(&'static str, Command),
    /// Forget, which asks first.
    AskForget,
    /// Back out of the forget question.
    CancelForget,
}

struct DeviceRow {
    row: ui::Row,
    primary: gtk4::Button,
    secondary: gtk4::Button,
    action: Rc<RefCell<Option<Command>>>,
    second: Rc<RefCell<Secondary>>,
    /// Forget was pressed once; the row asks before doing it.
    asking: Rc<Cell<bool>>,
}

impl DeviceRow {
    fn new(service: &Rc<BluetoothService>, d: &Device, redraw: &Rc<dyn Fn()>) -> Self {
        let row = ui::row(device_icon(d.icon_hint.as_deref()), &d.name, "");
        let primary = ui::button_with(
            ui::Face::Label("Connect"),
            ui::Kind::Secondary,
            ui::Size::Small,
        );
        let secondary = ui::button_with(ui::Face::Label("Forget"), ui::Kind::Flat, ui::Size::Small);
        row.end.append(&primary);
        row.end.append(&secondary);
        let action: Rc<RefCell<Option<Command>>> = Rc::default();
        let second = Rc::new(RefCell::new(Secondary::Hidden));
        let asking = Rc::new(Cell::new(false));
        {
            let (service, action) = (service.clone(), action.clone());
            primary.connect_clicked(move |_| {
                if let Some(c) = action.borrow().clone() {
                    service.send(c);
                }
            });
        }
        {
            let (service, second, asking, redraw) =
                (service.clone(), second.clone(), asking.clone(), redraw.clone());
            secondary.connect_clicked(move |_| {
                let what = second.borrow().clone();
                match what {
                    Secondary::Hidden => {}
                    Secondary::Send(_, c) => service.send(c),
                    Secondary::AskForget => {
                        asking.set(true);
                        redraw();
                    }
                    Secondary::CancelForget => {
                        asking.set(false);
                        redraw();
                    }
                }
            });
        }
        DeviceRow {
            row,
            primary,
            secondary,
            action,
            second,
            asking,
        }
    }

    fn update(&self, d: &Device, group: Group, op: Option<&Op>) {
        self.row.title.set_label(&d.name);
        self.row.icon.set_label(device_icon(d.icon_hint.as_deref()));
        ui::set_selected(&self.row.root, group == Group::Connected);
        ui::set_busy(&self.row.root, op.is_some_and(Op::busy));
        if op.is_some() {
            self.asking.set(false);
        }

        let (text, tone) = if self.asking.get() {
            (format!("Forget {}? It will need pairing again.", d.name), ui::Tone::Fg)
        } else {
            subtitle(d, group, op)
        };
        self.row.subtitle.set_label(&text);
        ui::set_text_style(&self.row.subtitle, ui::Text::Caption, tone);
        self.row.subtitle.set_visible(!text.is_empty());
        // A reason or a code is read whole; a status fits one line.
        let whole = self.asking.get() || matches!(op, Some(Op::Failed(_) | Op::Confirm(_) | Op::Show(_)));
        self.row.subtitle.set_wrap(whole);
        self.row.subtitle.set_ellipsize(if whole {
            gtk4::pango::EllipsizeMode::None
        } else {
            gtk4::pango::EllipsizeMode::End
        });

        let mac = d.mac.clone();
        let (primary, second) = if self.asking.get() {
            (
                Some(("Forget", Command::Forget(mac), ui::Kind::Destructive)),
                Secondary::CancelForget,
            )
        } else {
            match op {
                Some(Op::Confirm(_)) => (
                    Some((
                        "Pair",
                        Command::Answer {
                            mac: mac.clone(),
                            accept: true,
                        },
                        ui::Kind::Primary,
                    )),
                    Secondary::Send("Cancel", Command::Answer { mac, accept: false }),
                ),
                Some(Op::Failed(_)) => {
                    let retry = match group {
                        Group::Connected => Command::Disconnect(mac.clone()),
                        Group::Mine => Command::Connect(mac.clone()),
                        Group::Nearby => Command::Pair(mac.clone()),
                    };
                    (
                        Some(("Try again", retry, ui::Kind::Secondary)),
                        Secondary::Send("Dismiss", Command::Dismiss(mac)),
                    )
                }
                Some(_) => (None, Secondary::Hidden),
                None => {
                    let (label, command) = match group {
                        Group::Connected => ("Disconnect", Command::Disconnect(mac)),
                        Group::Mine => ("Connect", Command::Connect(mac)),
                        Group::Nearby => ("Pair", Command::Pair(mac)),
                    };
                    // A device you do not own has nothing to forget.
                    let second = if group == Group::Nearby {
                        Secondary::Hidden
                    } else {
                        Secondary::AskForget
                    };
                    (Some((label, command, ui::Kind::Secondary)), second)
                }
            }
        };

        match primary {
            Some((label, command, kind)) => {
                self.primary.set_visible(true);
                self.primary.set_label(label);
                ui::set_button_kind(&self.primary, kind);
                *self.action.borrow_mut() = Some(command);
            }
            None => {
                self.primary.set_visible(false);
                *self.action.borrow_mut() = None;
            }
        }
        let label = match &second {
            Secondary::Hidden => None,
            Secondary::Send(label, _) => Some(*label),
            Secondary::AskForget => Some("Forget"),
            Secondary::CancelForget => Some("Cancel"),
        };
        self.secondary.set_visible(label.is_some());
        if let Some(label) = label {
            self.secondary.set_label(label);
        }
        *self.second.borrow_mut() = second;
    }
}

struct Widgets {
    section: ui::Section,
    power: gtk4::Switch,
    off_note: gtk4::Label,
    lists: Vec<(Group, gtk4::Box, gtk4::Label)>,
    rows: RefCell<HashMap<String, DeviceRow>>,
    /// Who is in each group, in order, as last drawn.
    members: RefCell<HashMap<Group, Vec<String>>>,
    /// Set while this code moves the switch, so it is not read as a click.
    updating: Cell<bool>,
}

pub struct BluetoothSection {
    widgets: Rc<Widgets>,
    service: Rc<BluetoothService>,
    redraw: RefCell<Option<Rc<dyn Fn()>>>,
}

impl BluetoothSection {
    pub fn new(service: Rc<BluetoothService>) -> Rc<Self> {
        let section = ui::section(icons::BLUETOOTH, "Bluetooth", "");
        ui::glyph::adopt(&section.icon, ui::Text::Title, ui::Tone::Fg);

        let (power_row, power) = ui::switch_row("Bluetooth", "");
        section.body.append(&power_row.root);
        let off_note = ui::text(
            "Turn Bluetooth on to use your devices and find new ones.",
            ui::Text::Caption,
            ui::Tone::Faint,
        );
        off_note.set_wrap(true);
        off_note.set_xalign(0.0);
        section.body.append(&off_note);

        let mut lists = Vec::new();
        for (group, title) in [
            (Group::Connected, "Connected"),
            (Group::Mine, "My devices"),
            (Group::Nearby, "Nearby"),
        ] {
            let head = ui::overline(title, ui::Tone::Muted);
            head.set_xalign(0.0);
            let list = ui::vbox(1);
            section.body.append(&head);
            section.body.append(&list);
            lists.push((group, list, head));
        }

        // A full manager when one is installed, for what the panel does not
        // do (renaming, device classes). Hidden when there is none.
        let manager = ui::button("Bluetooth manager…", ui::Kind::Secondary);
        manager.set_halign(gtk4::Align::Start);
        manager.set_visible(glib::find_program_in_path("blueman-manager").is_some());
        manager.connect_clicked(|_| {
            let _ = std::process::Command::new("blueman-manager").spawn();
        });
        section.body.append(&manager);

        let widgets = Rc::new(Widgets {
            section,
            power,
            off_note,
            lists,
            rows: RefCell::default(),
            members: RefCell::default(),
            updating: Cell::new(false),
        });

        {
            let (w, service) = (widgets.clone(), service.clone());
            widgets.power.connect_active_notify(move |s| {
                if !w.updating.get() {
                    service.send(Command::Power(s.is_active()));
                }
            });
        }
        // Scan while on screen, and only then.
        {
            let service = service.clone();
            widgets
                .section
                .root
                .connect_map(move |_| service.send(Command::Discover(true)));
        }
        {
            let service = service.clone();
            widgets
                .section
                .root
                .connect_unmap(move |_| service.send(Command::Discover(false)));
        }

        let this = Rc::new(BluetoothSection {
            widgets,
            service,
            redraw: RefCell::new(None),
        });
        let redraw: Rc<dyn Fn()> = {
            let weak = Rc::downgrade(&this);
            Rc::new(move || {
                if let Some(this) = weak.upgrade() {
                    this.draw();
                }
            })
        };
        *this.redraw.borrow_mut() = Some(redraw.clone());
        this.service.connect_change(move || redraw());
        this.draw();
        this
    }

    fn draw(&self) {
        let Some(redraw) = self.redraw.borrow().clone() else {
            return;
        };
        let state = self.service.state();
        let w = &self.widgets;
        let snap = &state.snapshot;

        w.updating.set(true);
        w.power.set_active(snap.powered);
        w.power.set_sensitive(snap.available);
        w.updating.set(false);

        let on = snap.available && snap.powered;
        w.off_note.set_visible(snap.available && !snap.powered);
        let groups = if on { grouped(&state) } else { Vec::new() };

        let connected: Vec<&Device> = groups
            .iter()
            .filter(|(g, _)| *g == Group::Connected)
            .map(|(_, d)| *d)
            .collect();
        let (icon, summary) = if !snap.available {
            (icons::BLUETOOTH_OFF, "Unavailable".to_string())
        } else if !snap.powered {
            (icons::BLUETOOTH_OFF, "Off".to_string())
        } else {
            match connected.as_slice() {
                [] => (icons::BLUETOOTH, "On".to_string()),
                [d] => (
                    icons::BLUETOOTH_CONNECTED,
                    match d.battery {
                        Some(p) => format!("{} · {p} %", d.name),
                        None => d.name.clone(),
                    },
                ),
                many => (
                    icons::BLUETOOTH_CONNECTED,
                    format!("{} devices", many.len()),
                ),
            }
        };
        w.section.icon.set_label(icon);
        w.section.summary.set_label(&summary);

        for (group, list, head) in &w.lists {
            let members: Vec<&Device> = groups
                .iter()
                .filter(|(g, _)| g == group)
                .map(|(_, d)| *d)
                .collect();
            let macs: Vec<String> = members.iter().map(|d| d.mac.clone()).collect();
            let searching = *group == Group::Nearby && on && snap.discovering;
            head.set_visible(!macs.is_empty() || searching);
            if *group == Group::Nearby {
                // An overline is upper case (`ui::overline`); a relabel
                // has to say so itself.
                head.set_label(&if searching {
                    "Nearby · searching…"
                } else {
                    "Nearby"
                }
                .to_uppercase());
            }

            let changed = w.members.borrow().get(group) != Some(&macs);
            let mut rows = w.rows.borrow_mut();
            if changed {
                while let Some(child) = list.first_child() {
                    list.remove(&child);
                }
                for d in &members {
                    // A row that moved from another group starts fresh.
                    rows.remove(&d.mac);
                }
            }
            for d in &members {
                let row = rows
                    .entry(d.mac.clone())
                    .or_insert_with(|| DeviceRow::new(&self.service, d, &redraw));
                if changed {
                    list.append(&row.row.root);
                }
                row.update(d, *group, state.ops.get(&d.mac));
            }
            w.members.borrow_mut().insert(*group, macs);
        }
        let listed: HashSet<&String> = groups.iter().map(|(_, d)| &d.mac).collect();
        w.rows.borrow_mut().retain(|mac, _| listed.contains(mac));
    }

    /// Kept for the panel's open hook; the state is already current.
    pub fn refresh(&self) {
        self.draw();
    }

    pub fn expand_for_page(&self) {
        self.widgets.section.show_as_page();
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.widgets.section.root
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::bluez::Snapshot;

    fn dev(mac: &str, name: &str, connected: bool, paired: bool, rssi: Option<i16>) -> Device {
        Device {
            path: format!("/org/bluez/hci0/dev_{}", mac.replace(':', "_")),
            mac: mac.into(),
            name: name.into(),
            icon_hint: None,
            connected,
            paired,
            trusted: paired,
            battery: None,
            rssi,
            named: true,
        }
    }

    #[test]
    fn devices_fall_into_connected_mine_and_nearby() {
        let mut anonymous = dev("00:00:00:00:00:06", "00:00:00:00:00:06", false, false, Some(-30));
        anonymous.named = false;
        let state = BtState {
            snapshot: Snapshot {
                available: true,
                powered: true,
                devices: vec![
                    dev("00:00:00:00:00:01", "Keyboard", true, true, None),
                    dev("00:00:00:00:00:02", "Headphones", false, true, None),
                    // Unpaired and not seen by this scan: not listed.
                    dev("00:00:00:00:00:03", "Stale", false, false, None),
                    dev("00:00:00:00:00:04", "Far", false, false, Some(-90)),
                    dev("00:00:00:00:00:05", "Near", false, false, Some(-40)),
                    anonymous,
                ],
                ..Default::default()
            },
            ops: Default::default(),
        };
        let names: Vec<(Group, &str)> = grouped(&state)
            .into_iter()
            .map(|(g, d)| (g, d.name.as_str()))
            .collect();
        assert_eq!(
            names,
            vec![
                (Group::Connected, "Keyboard"),
                (Group::Mine, "Headphones"),
                (Group::Nearby, "Near"),
                (Group::Nearby, "Far"),
            ]
        );
    }

    #[test]
    fn a_row_says_what_its_device_is_doing() {
        let mut d = dev("00:00:00:00:00:01", "Buds", true, true, None);
        d.battery = Some(72);
        assert!(subtitle(&d, Group::Connected, None).0.ends_with("72 %"));
        let (text, tone) = subtitle(&d, Group::Mine, Some(&Op::Failed("Not responding.".into())));
        assert_eq!((text.as_str(), tone), ("Not responding.", ui::Tone::Danger));
        assert_eq!(
            subtitle(&d, Group::Nearby, Some(&Op::Confirm("123 456".into()))).0,
            "Does Buds show 123 456?"
        );
    }

    #[test]
    fn device_hints_pick_their_glyph() {
        assert_eq!(device_icon(Some("audio-headset")), icons::HEADSET);
        assert_eq!(device_icon(Some("audio-headphones")), icons::HEADPHONES);
        assert_eq!(device_icon(Some("input-keyboard")), icons::KEYBOARD);
        assert_eq!(device_icon(None), icons::BLUETOOTH);
    }
}
