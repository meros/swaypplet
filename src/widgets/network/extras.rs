//! The groups under the network list: Tailscale, and the saved networks.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{DropDown, ListBox, ListBoxRow, Switch};

use crate::services::network::model::SavedNetwork;
use crate::services::network::tailscale::Status;
use crate::ui;

/// Tailscale: up or down, where it is, and which exit node carries the
/// traffic. Hidden on a machine without the CLI.
pub struct Tailscale {
    pub root: gtk4::Box,
    row: ui::Row,
    switch: Switch,
    exit: DropDown,
    /// The exit node each dropdown entry stands for; `None` is "none".
    exit_ips: Rc<RefCell<Vec<Option<String>>>>,
    syncing: Rc<Cell<bool>>,
}

pub enum TailscaleAsk {
    Up(bool),
    ExitNode(Option<String>),
}

impl Tailscale {
    pub fn new(ask: Rc<dyn Fn(TailscaleAsk)>) -> Tailscale {
        let root = ui::group(0);
        root.set_visible(false);
        let (row, switch) = ui::switch_row("Tailscale", "");
        row.icon.set_label("󰖂");
        row.icon.set_visible(true);
        ui::glyph::adopt(&row.icon, ui::Text::Title, ui::Tone::Fg);
        root.append(&row.root);

        let exit_row = ui::row("", "Exit node", "Send all traffic through a device");
        let exit = ui::dropdown(&["None"]);
        exit_row.end.append(&exit);
        root.append(&exit_row.root);

        let this = Tailscale {
            root,
            row,
            switch,
            exit,
            exit_ips: Rc::default(),
            syncing: Rc::default(),
        };
        {
            let (syncing, ask) = (this.syncing.clone(), ask.clone());
            this.switch.connect_state_set(move |_, on| {
                if !syncing.get() {
                    ask(TailscaleAsk::Up(on));
                }
                gtk4::glib::Propagation::Proceed
            });
        }
        {
            let syncing = this.syncing.clone();
            let ips_c = this.exit_ips.clone();
            this.exit.connect_selected_notify(move |d| {
                if syncing.get() {
                    return;
                }
                let ip = ips_c.borrow().get(d.selected() as usize).cloned();
                if let Some(ip) = ip {
                    ask(TailscaleAsk::ExitNode(ip));
                }
            });
        }
        this
    }

    pub fn update(&self, status: Option<&Status>) {
        let Some(s) = status else {
            self.root.set_visible(false);
            return;
        };
        self.root.set_visible(true);
        self.syncing.set(true);
        let up = s.running();
        self.switch.set_active(up);
        self.switch.set_state(up);
        ui::set_selected(&self.row.root, up);
        let sub = if up {
            let mut parts = Vec::new();
            if let Some(t) = &s.tailnet {
                parts.push(t.clone());
            }
            if let Some(ip) = &s.self_ip {
                parts.push(ip.clone());
            }
            parts.push(format!("{} of {} online", s.peers_online, s.peers_total));
            parts.join(" · ")
        } else if s.state == "NeedsLogin" {
            "Needs you to log in (tailscale up)".to_string()
        } else {
            s.state.clone()
        };
        self.row.subtitle.set_label(&sub);
        self.row.subtitle.set_visible(true);

        let mut labels = vec!["None".to_string()];
        let mut ips = vec![None];
        for p in &s.exit_options {
            labels.push(if p.online {
                p.name.clone()
            } else {
                format!("{} (offline)", p.name)
            });
            ips.push(Some(p.ip.clone()));
        }
        let current = s
            .exit_node
            .as_ref()
            .and_then(|n| s.exit_options.iter().position(|p| p.name == *n))
            .map_or(0, |i| i + 1);
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        self.exit.set_model(Some(&gtk4::StringList::new(&refs)));
        self.exit.set_selected(current as u32);
        self.exit.set_sensitive(up && !s.exit_options.is_empty());
        *self.exit_ips.borrow_mut() = ips;
        self.syncing.set(false);
    }
}

pub enum SavedAsk {
    Autoconnect(String, bool),
    Forget(String),
}

/// The saved networks, most recently used first: whether each joins by
/// itself, and forget.
pub struct Saved {
    pub disclosure: ui::Disclosure,
    list: ListBox,
    ask: Rc<dyn Fn(SavedAsk)>,
    shown: RefCell<Vec<SavedNetwork>>,
}

impl Saved {
    pub fn new(ask: Rc<dyn Fn(SavedAsk)>) -> Saved {
        let disclosure = ui::disclosure("Saved networks");
        let list = ui::list();
        disclosure.body.append(&list);
        Saved {
            disclosure,
            list,
            ask,
            shown: RefCell::default(),
        }
    }

    pub fn update(&self, saved: &[SavedNetwork]) {
        if *self.shown.borrow() == saved {
            return;
        }
        *self.shown.borrow_mut() = saved.to_vec();
        while let Some(c) = self.list.first_child() {
            self.list.remove(&c);
        }
        self.disclosure.root.set_visible(!saved.is_empty());
        for s in saved {
            let sub = if s.ssid != s.id {
                format!("{} · {}", s.ssid, joins(s.autoconnect))
            } else {
                joins(s.autoconnect).to_string()
            };
            let r = ui::row("", &s.id, &sub);
            let auto = ui::switch();
            auto.set_active(s.autoconnect);
            auto.set_tooltip_text(Some("Join by itself when in range"));
            {
                let (id, ask) = (s.id.clone(), self.ask.clone());
                auto.connect_state_set(move |_, on| {
                    ask(SavedAsk::Autoconnect(id.clone(), on));
                    gtk4::glib::Propagation::Proceed
                });
            }
            let forget = ui::button_with(ui::Face::Label("Forget"), ui::Kind::Flat, ui::Size::Small);
            wire_forget(&forget, s.id.clone(), self.ask.clone());
            r.end.append(&forget);
            r.end.append(&auto);
            self.list
                .append(&ListBoxRow::builder().child(&r.root).build());
        }
    }
}

fn joins(auto: bool) -> &'static str {
    if auto { "Joins by itself" } else { "Joins when asked" }
}

/// Forget asks twice: the first click arms the button for three seconds.
fn wire_forget(b: &gtk4::Button, id: String, ask: Rc<dyn Fn(SavedAsk)>) {
    let armed = Rc::new(Cell::new(false));
    b.connect_clicked(move |b| {
        if !armed.get() {
            armed.set(true);
            b.set_label("Sure?");
            ui::set_button_kind(b, ui::Kind::Destructive);
            let (b, armed) = (b.clone(), armed.clone());
            gtk4::glib::timeout_add_local_once(std::time::Duration::from_secs(3), move || {
                if armed.replace(false) {
                    b.set_label("Forget");
                    ui::set_button_kind(&b, ui::Kind::Flat);
                }
            });
            return;
        }
        armed.set(false);
        ask(SavedAsk::Forget(id.clone()));
    });
}
