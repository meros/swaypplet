//! System tray — SNI items as icon buttons with native GTK4 DBusMenu
//! popovers (replaces waybar's tray module).
//!
//! Left click activates the app (menu-only items open the menu instead),
//! right click opens the menu, middle click sends the secondary activate.

mod icon;
mod menu;
mod service;

pub use service::TrayService;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::time::Duration;

use gtk4::{gdk, prelude::*};
use system_tray::item::{Status, StatusNotifierItem};

use service::TrayItem;

/// Layout pushes into an *open* popover are batched — chatty apps fire
/// menu updates in bursts and a rebuild per event flickers the popover.
const MENU_DEBOUNCE_MS: u64 = 50;

/// The tray pill for one bar window, following the shared service.
pub fn build(tray: &Rc<TrayService>) -> gtk4::Box {
    let container = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .css_classes(["bar-tray"])
        .spacing(2)
        // Hidden until an item registers, so the empty pill never renders.
        .visible(false)
        .build();

    let items: Rc<RefCell<HashMap<String, ItemUi>>> = Rc::new(RefCell::new(HashMap::new()));

    let sync = {
        let (container, items, tray) = (container.clone(), items.clone(), tray.clone());
        move || sync_items(&container, &items, &tray)
    };
    tray.connect_change({
        let sync = sync.clone();
        move || sync()
    });
    sync();

    container
}

/// Reconcile item buttons against the snapshot: per-address widgets are
/// kept (so an open popover on one item survives updates to another),
/// vanished ones removed, and order fixed by cheap in-place reordering.
fn sync_items(
    container: &gtk4::Box,
    items: &Rc<RefCell<HashMap<String, ItemUi>>>,
    tray: &Rc<TrayService>,
) {
    let snapshot = tray.items();
    let mut map = items.borrow_mut();

    map.retain(|address, ui| {
        let keep = snapshot.iter().any(|i| &i.address == address);
        if !keep {
            if let Some(menu) = ui.menu.borrow_mut().take() {
                menu.unparent();
            }
            container.remove(&ui.button);
        }
        keep
    });

    let mut prev: Option<gtk4::Button> = None;
    for item in &snapshot {
        let ui = map
            .entry(item.address.clone())
            .or_insert_with(|| ItemUi::new(container, tray, item));
        ui.update(item, tray);
        container.reorder_child_after(&ui.button, prev.as_ref());
        prev = Some(ui.button.clone());
    }

    container.set_visible(!snapshot.is_empty());
}

/// Icon inputs of the last render — icon resolution (texture upload) only
/// reruns when one of them changes, not on every tray event.
#[derive(PartialEq, Eq)]
struct IconKey {
    icon_name: Option<String>,
    attention_icon_name: Option<String>,
    status: Status,
    pixmap_hash: u64,
}

impl IconKey {
    fn of(sni: &StatusNotifierItem) -> Self {
        let mut hasher = std::hash::DefaultHasher::new();
        for set in [&sni.icon_pixmap, &sni.attention_icon_pixmap] {
            for px in set.iter().flatten() {
                (px.width, px.height).hash(&mut hasher);
                px.pixels.hash(&mut hasher);
            }
        }
        Self {
            icon_name: sni.icon_name.clone(),
            attention_icon_name: sni.attention_icon_name.clone(),
            status: sni.status,
            pixmap_hash: hasher.finish(),
        }
    }
}

struct ItemUi {
    button: gtk4::Button,
    image: gtk4::Image,
    icon_key: RefCell<Option<IconKey>>,
    menu: Rc<RefCell<Option<menu::MenuUi>>>,
    menu_push_pending: Rc<Cell<bool>>,
}

impl ItemUi {
    fn new(container: &gtk4::Box, tray: &Rc<TrayService>, item: &TrayItem) -> Self {
        let image = gtk4::Image::new();
        let button = gtk4::Button::builder()
            .css_classes(["bar-tray-item"])
            .has_frame(false)
            .child(&image)
            .build();
        container.append(&button);

        let menu: Rc<RefCell<Option<menu::MenuUi>>> = Rc::new(RefCell::new(None));

        button.connect_clicked({
            let (tray, menu, address) = (tray.clone(), menu.clone(), item.address.clone());
            move |btn| {
                let Some(item) = tray.item(&address) else {
                    return;
                };
                if item.sni.item_is_menu {
                    open_menu(&tray, &item, &menu, btn);
                } else {
                    tray.activate_default(&address);
                }
            }
        });

        // Button only reports primary clicks; right/middle need a gesture.
        let gesture = gtk4::GestureClick::new();
        gesture.set_button(0);
        gesture.connect_pressed({
            let (tray, menu, address) = (tray.clone(), menu.clone(), item.address.clone());
            let weak = button.downgrade();
            move |g, _, _, _| match g.current_button() {
                gdk::BUTTON_SECONDARY => {
                    let (Some(btn), Some(item)) = (weak.upgrade(), tray.item(&address)) else {
                        return;
                    };
                    open_menu(&tray, &item, &menu, &btn);
                }
                gdk::BUTTON_MIDDLE => tray.activate_secondary(&address),
                _ => {}
            }
        });
        button.add_controller(gesture);

        Self {
            button,
            image,
            icon_key: RefCell::new(None),
            menu,
            menu_push_pending: Rc::new(Cell::new(false)),
        }
    }

    fn update(&self, item: &TrayItem, tray: &Rc<TrayService>) {
        let key = IconKey::of(&item.sni);
        if self.icon_key.borrow().as_ref() != Some(&key) {
            icon::apply(&self.image, &item.sni);
            self.icon_key.replace(Some(key));
        }

        let status = item.sni.status;
        set_class(&self.button, "passive", status == Status::Passive);
        set_class(
            &self.button,
            "needs-attention",
            status == Status::NeedsAttention,
        );

        let tooltip = item
            .sni
            .tool_tip
            .as_ref()
            .map(|t| t.title.clone())
            .filter(|t| !t.is_empty())
            .or_else(|| item.sni.title.clone());
        self.button.set_tooltip_text(tooltip.as_deref());

        // A closed popover re-syncs on open; only a visible one needs the
        // fresh layout pushed, and then debounced.
        let open = self
            .menu
            .borrow()
            .as_ref()
            .is_some_and(menu::MenuUi::is_open);
        if open && !self.menu_push_pending.get() {
            self.menu_push_pending.set(true);
            let (menu, pending) = (self.menu.clone(), self.menu_push_pending.clone());
            let (tray, address) = (tray.clone(), item.address.clone());
            glib::timeout_add_local_once(Duration::from_millis(MENU_DEBOUNCE_MS), move || {
                pending.set(false);
                let layout = tray.item(&address).and_then(|i| i.menu);
                if let (Some(ui), Some(layout)) = (&*menu.borrow(), layout)
                    && ui.is_open()
                {
                    ui.set_layout(&layout);
                }
            });
        }
    }
}

fn open_menu(
    tray: &Rc<TrayService>,
    item: &TrayItem,
    slot: &Rc<RefCell<Option<menu::MenuUi>>>,
    button: &gtk4::Button,
) {
    let Some(path) = item.sni.menu.clone() else {
        return;
    };
    tray.about_to_show(&item.address, &path);
    // No layout yet (client still fetching, or the app populates only on
    // AboutToShow): nothing to pop — the update lands as a tray event.
    let Some(layout) = &item.menu else {
        return;
    };
    let mut slot = slot.borrow_mut();
    let ui = slot
        .get_or_insert_with(|| menu::MenuUi::new(button, tray.clone(), item.address.clone(), path));
    ui.set_layout(layout);
    ui.popup();
}

fn set_class(widget: &impl IsA<gtk4::Widget>, class: &str, on: bool) {
    if on {
        widget.add_css_class(class);
    } else {
        widget.remove_css_class(class);
    }
}
