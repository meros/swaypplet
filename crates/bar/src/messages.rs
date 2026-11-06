//! Messages cluster: Slack, Teams, Discord and Telegram as small marks that
//! show while the client runs or has messages waiting, with the unread count
//! beside the icon. A click brings the client's window back.
//!
//! Chat clients hide to the tray when their window closes, and a hidden
//! Slack was invisible here: the tray shows `NeedsAttention` items only and
//! Slack's item is `Active`. This cluster is the exception the vision doc
//! names (docs/BAR_VISION.md, "Messages"): it lists only the clients in
//! `swaypplet_base::clients::CLIENTS`, only while one runs or has something
//! waiting, and it draws no red, which stays "act now".
//!
//! Three event sources feed one model, with no polling: sway window events
//! (the clients' windows and titles), the tray (the clients' items and tool
//! tips), and the notification daemon (what arrived since the window was
//! last in front). What the model decides is `swaypplet_base::clients::merge`;
//! this file is the part with GTK in it.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use gio::prelude::*;
use gtk4::{gdk, prelude::*};

use swaypplet_base::clients::{
    Chip, Client, Missed, Presence, Unread, badge_text, client_for_notification, merge,
};
use swaypplet_base::service::Observed;
use swaypplet_base::sway::ipc::SwayService;
use swaypplet_base::sway::tree::show_window;
use swaypplet_notifyd::store::StoreRef;
use swaypplet_system::tray::TrayService;
use swaypplet_ui::ui;

/// How long a click on a tray-only client waits for its window to appear
/// before it starts the client instead. Activate over the tray is a request
/// the tray service cannot see the outcome of.
const ACTIVATE_WAIT: Duration = Duration::from_secs(2);

/// What the cluster shows, shared by every output's bar.
pub struct Messages {
    sway: Rc<SwayService>,
    tray: Rc<TrayService>,
    missed: RefCell<Missed>,
    chips: Observed<Vec<Chip>>,
}

impl Messages {
    /// `notifications` is `None` in the standalone bar, which has no
    /// notification daemon: the tray and the windows still count.
    pub fn start(
        sway: &Rc<SwayService>,
        tray: &Rc<TrayService>,
        notifications: Option<&StoreRef>,
    ) -> Rc<Self> {
        let this = Rc::new(Self {
            sway: sway.clone(),
            tray: tray.clone(),
            missed: RefCell::new(Missed::default()),
            chips: Observed::new(Vec::new()),
        });

        let refresh = |weak: std::rc::Weak<Self>| {
            move || {
                if let Some(this) = weak.upgrade() {
                    this.refresh();
                }
            }
        };
        sway.connect_client_windows(refresh(Rc::downgrade(&this)));
        tray.connect_presence_change(refresh(Rc::downgrade(&this)));
        if let Some(store) = notifications {
            let weak = Rc::downgrade(&this);
            store.borrow_mut().connect_notify(move |n| {
                if let Some(this) = weak.upgrade() {
                    this.notified(n.id, &n.app_name, n.desktop_entry.as_deref());
                }
            });
        }
        this.refresh();
        this
    }

    /// A notification arrived. It counts when it is from a client and that
    /// client's window is not the one in front. Do Not Disturb and a
    /// screen-share hold only stop the card, and those are the messages a
    /// person most wants to know they missed, so nothing here asks.
    fn notified(&self, id: u32, app_name: &str, desktop_entry: Option<&str>) {
        let Some(client) = client_for_notification(app_name, desktop_entry) else {
            // Teams in Chrome arrives as a Chrome notification. Until the
            // matcher has seen a real one, say what the others looked like.
            if app_name.to_lowercase().contains("chrome")
                || desktop_entry.is_some_and(|e| e.contains("chrome"))
            {
                log::debug!(
                    "messages: unmatched notification app_name={app_name:?} desktop_entry={desktop_entry:?}"
                );
            }
            return;
        };
        let looking = self
            .sway
            .client_windows()
            .iter()
            .any(|w| w.client == client.id && w.focused);
        if !looking {
            self.missed.borrow_mut().record(client.id, id);
            self.refresh();
        }
    }

    fn refresh(&self) {
        let windows = self.sway.client_windows();
        {
            // Looking at a client clears what it had missed.
            let mut missed = self.missed.borrow_mut();
            for w in windows.iter().filter(|w| w.focused) {
                missed.clear(w.client);
            }
        }
        let chips = merge(&self.tray.presence(), &windows, &self.missed.borrow());
        self.chips.set_if_changed(chips);
    }

    fn chips(&self) -> Vec<Chip> {
        self.chips.with(Clone::clone)
    }

    fn chip(&self, client: &str) -> Option<Chip> {
        self.chips().into_iter().find(|c| c.client.id == client)
    }

    /// What a click on the client's mark does.
    fn activate(self: &Rc<Self>, client: &'static Client) {
        let Some(chip) = self.chip(client.id) else {
            return;
        };
        match chip.presence {
            Presence::Open { focused: true, .. } => {}
            Presence::Open {
                con_id,
                workspace,
                scratchpad,
                ..
            } => show_window(con_id, workspace.as_deref(), scratchpad),
            Presence::Background { address } => {
                self.tray.activate_default(&address);
                // The tray call has no answer to wait on, so the window
                // appearing is the answer. No window by then: start it.
                let this = self.clone();
                glib::timeout_add_local_once(ACTIVATE_WAIT, move || {
                    let open = this
                        .chip(client.id)
                        .is_some_and(|c| matches!(c.presence, Presence::Open { .. }));
                    if !open {
                        launch(client);
                    }
                });
            }
            Presence::Gone => launch(client),
        }
    }
}

/// Start the client from its desktop file, with the activation token the
/// display hands out so the new window may take focus.
fn launch(client: &Client) {
    let Some(info) = app_info(client) else {
        log::warn!("messages: no desktop file {}.desktop", client.desktop_id);
        return;
    };
    let context = gdk::Display::default().map(|d| d.app_launch_context());
    if let Err(e) = info.launch(&[], context.as_ref()) {
        log::warn!("messages: could not start {}: {e}", client.name);
    }
}

/// The client's desktop entry, if it is installed.
fn app_info(client: &Client) -> Option<gio::AppInfo> {
    let id = format!("{}.desktop", client.desktop_id);
    gio::AppInfo::all()
        .into_iter()
        .find(|app| app.id().is_some_and(|i| i.as_str() == id))
}

/// The words on a mark's tooltip.
fn tooltip(chip: &Chip) -> String {
    let place = match &chip.presence {
        Presence::Open {
            workspace: Some(ws),
            scratchpad: false,
            ..
        } => format!("workspace {ws}"),
        Presence::Open { .. } => "window open".to_string(),
        Presence::Background { .. } => "in the tray, no window. Click to open it".to_string(),
        Presence::Gone => "not running. Click to start it".to_string(),
    };
    let unread = match chip.unread {
        Unread::None => String::new(),
        Unread::Dot => " · something unread".to_string(),
        Unread::Count(n) => format!(" · {n} unread"),
    };
    format!("{} · {place}{unread}", chip.client.name)
}

// ── Widget ──────────────────────────────────────────────────────────────

struct ChipUi {
    button: gtk4::Button,
    badge: gtk4::Label,
}

/// The cluster for one bar window, following the shared model.
pub fn build(model: &Rc<Messages>, _tray: &Rc<TrayService>) -> gtk4::Box {
    let container = ui::hbox(1);
    ui::segment::adopt(&container, false);
    container.add_css_class("bar-messages");
    // Hidden until a client runs, so the empty pill never renders. Like the
    // tray, the cluster owns its visibility: the Bar tab's switch is one
    // input and "a mark is there" the other.
    container.set_visible(false);

    let marks: Rc<RefCell<HashMap<&'static str, ChipUi>>> = Rc::new(RefCell::new(HashMap::new()));

    let sync = {
        let (container, marks, model) = (container.clone(), marks.clone(), model.clone());
        move || sync_marks(&container, &marks, &model)
    };
    model.chips.connect_change({
        let sync = sync.clone();
        move || sync()
    });
    sync();

    let weak = container.downgrade();
    let marks_seen = marks.clone();
    swaypplet_model::store::observe(move || {
        if let Some(container) = weak.upgrade() {
            show_if_wanted(&container, !marks_seen.borrow().is_empty());
        }
    });

    container
}

fn show_if_wanted(container: &gtk4::Box, has_marks: bool) {
    let wanted = swaypplet_model::store::with(|s| s.bar().messages);
    container.set_visible(wanted && has_marks);
}

/// Reconcile marks against the model: a mark is kept per client, so a
/// pointer resting on one survives the others changing.
fn sync_marks(
    container: &gtk4::Box,
    marks: &Rc<RefCell<HashMap<&'static str, ChipUi>>>,
    model: &Rc<Messages>,
) {
    let chips = model.chips();
    let mut map = marks.borrow_mut();

    map.retain(|id, ui| {
        let keep = chips.iter().any(|c| c.client.id == *id);
        if !keep {
            container.remove(&ui.button);
        }
        keep
    });

    let mut prev: Option<gtk4::Button> = None;
    for chip in &chips {
        let ui = map
            .entry(chip.client.id)
            .or_insert_with(|| new_mark(container, model, chip.client));
        update_mark(ui, chip);
        container.reorder_child_after(&ui.button, prev.as_ref());
        prev = Some(ui.button.clone());
    }

    show_if_wanted(container, !chips.is_empty());
}

fn new_mark(container: &gtk4::Box, model: &Rc<Messages>, client: &'static Client) -> ChipUi {
    let image = gtk4::Image::new();
    set_icon(&image, client);
    let badge = ui::badge("", ui::BadgeTone::Neutral);
    badge.set_visible(false);
    let row = ui::hbox(1);
    row.append(&image);
    row.append(&badge);

    let button = ui::mark(&row, false);
    button.add_css_class("bar-messages-item");
    container.append(&button);
    button.connect_clicked({
        let model = model.clone();
        move |_| model.activate(client)
    });
    ChipUi { button, badge }
}

fn update_mark(ui: &ChipUi, chip: &Chip) {
    match chip.unread {
        Unread::None => ui.badge.set_visible(false),
        Unread::Dot => {
            ui.badge.set_label("•");
            ui.badge.set_visible(true);
        }
        Unread::Count(n) => {
            ui.badge.set_label(&badge_text(n));
            ui.badge.set_visible(true);
        }
    }
    // In the tray with no window reads as "there, but not up".
    ui::set_quiet(
        &ui.button,
        matches!(chip.presence, Presence::Background { .. }),
    );
    ui.button.set_tooltip_text(Some(&tooltip(chip)));
}

/// The client's own icon, from its desktop file; the theme's name for it
/// next; a generic mail glyph last.
fn set_icon(image: &gtk4::Image, client: &Client) {
    image.set_pixel_size(16);
    if let Some(info) = app_info(client)
        && let Some(icon) = info.icon()
    {
        image.set_from_gicon(&icon);
        return;
    }
    let themed = gdk::Display::default()
        .is_some_and(|d| gtk4::IconTheme::for_display(&d).has_icon(client.id));
    image.set_icon_name(Some(if themed {
        client.id
    } else {
        "mail-unread-symbolic"
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use swaypplet_base::clients::CLIENTS;

    fn chip(id: &str, presence: Presence, unread: Unread) -> Chip {
        Chip {
            client: CLIENTS.iter().find(|c| c.id == id).unwrap(),
            presence,
            unread,
        }
    }

    #[test]
    fn tooltips_say_where_the_client_is_and_how_much_waits() {
        let open = chip(
            "slack",
            Presence::Open {
                con_id: 1,
                workspace: Some("38".into()),
                focused: false,
                scratchpad: false,
            },
            Unread::Count(3),
        );
        assert_eq!(tooltip(&open), "Slack · workspace 38 · 3 unread");
        let tray = chip(
            "slack",
            Presence::Background {
                address: ":1.2".into(),
            },
            Unread::Count(1),
        );
        assert_eq!(
            tooltip(&tray),
            "Slack · in the tray, no window. Click to open it · 1 unread"
        );
        let gone = chip("teams", Presence::Gone, Unread::Dot);
        assert_eq!(
            tooltip(&gone),
            "Teams · not running. Click to start it · something unread"
        );
    }
}
