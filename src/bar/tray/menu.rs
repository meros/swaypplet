//! DBusMenu layout → gio menu model + actions, rendered by a native
//! gtk4::PopoverMenu (submenus, separators as sections, check/radio via
//! stateful actions, disabled via action enabled state).
//!
//! Reconciliation: the layout's *structure* (ids, labels, separators,
//! submenu shape, icons) is captured in a signature string. A changed
//! signature rebuilds the model; anything else (enabled, toggle state)
//! updates the existing actions in place, so chatty menus don't rebuild —
//! and flicker — the open popover.

use std::cell::RefCell;
use std::fmt::Write;
use std::rc::Rc;

use gtk4::{gio, prelude::*};
use system_tray::menu::{MenuItem, MenuType, ToggleState, ToggleType, TrayMenu};

use super::service::TrayService;

/// Action group prefix; the group is inserted on the popover itself.
const GROUP: &str = "tray";

pub struct MenuUi {
    popover: gtk4::PopoverMenu,
    service: Rc<TrayService>,
    address: String,
    menu_path: String,
    group: RefCell<Option<gio::SimpleActionGroup>>,
    signature: RefCell<String>,
}

impl MenuUi {
    pub fn new(
        parent: &impl IsA<gtk4::Widget>,
        service: Rc<TrayService>,
        address: String,
        menu_path: String,
    ) -> Self {
        let popover = gtk4::PopoverMenu::from_model(gio::MenuModel::NONE);
        popover.set_parent(parent);
        // Bar sits at the bottom edge — menus open upward.
        popover.set_position(gtk4::PositionType::Top);
        Self {
            popover,
            service,
            address,
            menu_path,
            group: RefCell::new(None),
            signature: RefCell::new(String::new()),
        }
    }

    pub fn set_layout(&self, menu: &TrayMenu) {
        let sig = signature(&menu.submenus);
        if *self.signature.borrow() == sig {
            if let Some(group) = &*self.group.borrow() {
                update_states(&menu.submenus, group);
            }
            return;
        }
        // A fresh group per rebuild drops actions for removed entries.
        let group = gio::SimpleActionGroup::new();
        let model = self.build_model(&menu.submenus, &group);
        self.popover.insert_action_group(GROUP, Some(&group));
        self.popover.set_menu_model(Some(&model));
        self.group.replace(Some(group));
        self.signature.replace(sig);
    }

    pub fn popup(&self) {
        self.popover.popup();
    }

    pub fn is_open(&self) -> bool {
        self.popover.is_visible()
    }

    /// Must run before the parent button is dropped from the bar — a
    /// popover still parented to a destroyed widget trips a GTK warning.
    pub fn unparent(&self) {
        self.popover.unparent();
    }

    /// Separators split sections (gio has no separator item); everything
    /// between two separators lands in one section.
    fn build_model(&self, items: &[MenuItem], group: &gio::SimpleActionGroup) -> gio::Menu {
        let root = gio::Menu::new();
        let mut section = gio::Menu::new();
        for item in items.iter().filter(|i| i.visible) {
            if item.menu_type == MenuType::Separator {
                if section.n_items() > 0 {
                    root.append_section(None, &section);
                    section = gio::Menu::new();
                }
                continue;
            }

            let entry = gio::MenuItem::new(item.label.as_deref(), None);
            if item.children_display.as_deref() == Some("submenu") {
                entry.set_submenu(Some(&self.build_model(&item.submenu, group)));
            } else {
                let name = format!("i{}", item.id);
                group.add_action(&self.make_action(&name, item));
                match item.toggle_type {
                    // Radio renders from a string-state action + target:
                    // checked exactly when state == target.
                    ToggleType::Radio => entry.set_action_and_target_value(
                        Some(&format!("{GROUP}.{name}")),
                        Some(&"on".to_variant()),
                    ),
                    _ => entry.set_detailed_action(&format!("{GROUP}.{name}")),
                }
            }
            if let Some(icon) = item.icon_name.as_deref().filter(|n| !n.is_empty()) {
                entry.set_icon(&gio::ThemedIcon::new(icon));
            } else if let Some(png) = &item.icon_data {
                entry.set_icon(&gio::BytesIcon::new(&glib::Bytes::from(&png[..])));
            }
            section.append_item(&entry);
        }
        if section.n_items() > 0 {
            root.append_section(None, &section);
        }
        root
    }

    fn make_action(&self, name: &str, item: &MenuItem) -> gio::SimpleAction {
        let action = match item.toggle_type {
            ToggleType::Checkmark => {
                gio::SimpleAction::new_stateful(name, None, &checked(item).to_variant())
            }
            ToggleType::Radio => gio::SimpleAction::new_stateful(
                name,
                Some(glib::VariantTy::STRING),
                &radio_state(item).to_variant(),
            ),
            ToggleType::CannotBeToggled => gio::SimpleAction::new(name, None),
        };
        action.set_enabled(item.enabled);
        let service = self.service.clone();
        let (address, menu_path, id) = (self.address.clone(), self.menu_path.clone(), item.id);
        // State is deliberately not flipped here: the app owns it, and its
        // toggle-state update flows back through the menu diff events.
        action.connect_activate(move |_, _| service.menu_item_clicked(&address, &menu_path, id));
        action
    }
}

fn checked(item: &MenuItem) -> bool {
    item.toggle_state == ToggleState::On
}

fn radio_state(item: &MenuItem) -> &'static str {
    if checked(item) { "on" } else { "off" }
}

/// In-place refresh of the mutable-without-rebuild bits.
fn update_states(items: &[MenuItem], group: &gio::SimpleActionGroup) {
    for item in items {
        let action = group
            .lookup_action(&format!("i{}", item.id))
            .and_then(|a| a.downcast::<gio::SimpleAction>().ok());
        if let Some(action) = action {
            action.set_enabled(item.enabled);
            match item.toggle_type {
                ToggleType::Checkmark => action.set_state(&checked(item).to_variant()),
                ToggleType::Radio => action.set_state(&radio_state(item).to_variant()),
                ToggleType::CannotBeToggled => {}
            }
        }
        update_states(&item.submenu, group);
    }
}

/// Everything that forces a model rebuild when it changes. Enabled and
/// toggle *state* stay out — they live in the actions.
fn signature(items: &[MenuItem]) -> String {
    let mut out = String::new();
    walk(items, &mut out);
    out
}

fn walk(items: &[MenuItem], out: &mut String) {
    for item in items.iter().filter(|i| i.visible) {
        let _ = write!(
            out,
            "{}:{:?}:{:?}:{}:{}:{};",
            item.id,
            item.menu_type,
            item.toggle_type,
            item.label.as_deref().unwrap_or(""),
            item.icon_name.as_deref().unwrap_or(""),
            item.icon_data.as_ref().map_or(0, Vec::len),
        );
        if item.children_display.as_deref() == Some("submenu") {
            out.push('[');
            walk(&item.submenu, out);
            out.push(']');
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: i32) -> MenuItem {
        MenuItem {
            id,
            label: Some(format!("Item {id}")),
            enabled: true,
            visible: true,
            ..Default::default()
        }
    }

    #[test]
    fn toggle_state_and_enabled_do_not_change_the_signature() {
        let a = vec![item(1), item(2)];
        let mut b = a.clone();
        b[0].toggle_state = ToggleState::Off;
        b[1].enabled = false;
        assert_eq!(signature(&a), signature(&b));
    }

    #[test]
    fn label_change_forces_a_rebuild() {
        let a = vec![item(1)];
        let mut b = a.clone();
        b[0].label = Some("Copy 3 items".into());
        assert_ne!(signature(&a), signature(&b));
    }

    #[test]
    fn visibility_and_separators_shape_the_signature() {
        let a = vec![item(1), item(2)];
        let mut hidden = a.clone();
        hidden[1].visible = false;
        assert_ne!(signature(&a), signature(&hidden));

        let mut with_sep = a.clone();
        with_sep.insert(
            1,
            MenuItem {
                id: 9,
                menu_type: MenuType::Separator,
                visible: true,
                ..Default::default()
            },
        );
        assert_ne!(signature(&a), signature(&with_sep));
    }

    #[test]
    fn submenu_structure_is_part_of_the_signature() {
        let mut parent = item(1);
        parent.children_display = Some("submenu".into());
        parent.submenu = vec![item(10)];
        let a = vec![parent.clone()];
        parent.submenu.push(item(11));
        let b = vec![parent];
        assert_ne!(signature(&a), signature(&b));
    }
}
