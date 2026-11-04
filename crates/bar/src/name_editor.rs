//! Naming a workspace, in the bar.
//!
//! Right-click a workspace's segment (or press the label key, `swaypplet
//! label`) and a card opens under it with a text field. Enter or a click
//! away saves, Escape drops the change, and Remove clears the name. The
//! name then shows in that segment while a screen is on the workspace
//! (`workspaces.rs`), and a toast says it when you arrive
//! (`swaypplet-label`). The editor holds the pop-up's switch too, so the one
//! card changes everything the feature does.
//!
//! The bar's window takes no keys, so the field could not be typed in. While
//! the card is open the window asks the compositor for keyboard focus when
//! it is clicked (`OnDemand`), and gives it back when the card closes.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4::gdk;
use gtk4::prelude::*;
use gtk4_layer_shell::{KeyboardMode, LayerShell as _};

use super::popover;
use swaypplet_base::sway::workspace::label_for_name;
use swaypplet_label::Mode;
use swaypplet_ui::ui;

thread_local! {
    /// Every workspace segment that can open an editor, for the label key.
    static SEGMENTS: RefCell<Vec<(glib::WeakRef<gtk4::Button>, String)>> =
        const { RefCell::new(Vec::new()) };
}

/// Give `button` (the segment of workspace `workspace`, number `num`) the
/// editor on a right-click, and answer the label key for it.
pub fn attach(button: &gtk4::Button, workspace: &str, _num: i32) {
    let state = Rc::new(Editor::new(button, workspace));
    let right = gtk4::GestureClick::builder()
        .button(gdk::BUTTON_SECONDARY)
        .build();
    {
        let state = state.clone();
        right.connect_pressed(move |gesture, _, _, _| {
            gesture.set_state(gtk4::EventSequenceState::Claimed);
            state.open();
        });
    }
    button.add_controller(right);

    SEGMENTS.with(|all| {
        let mut all = all.borrow_mut();
        all.retain(|(b, _)| b.upgrade().is_some());
        all.push((button.downgrade(), workspace.to_string()));
    });
    EDITORS.with(|all| all.borrow_mut().push(Rc::downgrade(&state)));
    ensure_request_handler();
}

thread_local! {
    static EDITORS: RefCell<Vec<std::rc::Weak<Editor>>> = const { RefCell::new(Vec::new()) };
    static HANDLER: Cell<bool> = const { Cell::new(false) };
}

/// One handler for the whole process, registered by the first segment: it
/// opens the editor of the segment on the focused output.
fn ensure_request_handler() {
    if HANDLER.with(|h| h.replace(true)) {
        return;
    }
    swaypplet_label::connect_edit(|output, workspace| {
        let editors: Vec<_> = EDITORS.with(|all| {
            let mut all = all.borrow_mut();
            all.retain(|e| e.upgrade().is_some());
            all.iter().filter_map(std::rc::Weak::upgrade).collect()
        });
        let on_output = editors
            .iter()
            .find(|e| e.workspace == workspace && e.output().as_deref() == Some(output));
        let any = editors
            .iter()
            .find(|e| e.workspace == workspace && e.button.is_mapped());
        if let Some(editor) = on_output.or(any) {
            editor.open();
        }
    });
}

struct Editor {
    workspace: String,
    button: gtk4::Button,
    popover: gtk4::Popover,
    entry: gtk4::Entry,
    title: gtk4::Label,
    remove: gtk4::Button,
    pop_up: gtk4::Switch,
    /// The window's keyboard mode before the card took it.
    keyboard: Cell<Option<KeyboardMode>>,
}

impl Editor {
    fn new(button: &gtk4::Button, workspace: &str) -> Editor {
        let (popover, body) = popover::chassis(button);
        body.add_css_class("bar-name-editor");

        let title = ui::text("", ui::Text::Body, ui::Tone::Fg);
        ui::set_weight(&title, ui::Weight::Strong);
        body.append(&title);

        let entry = gtk4::Entry::builder()
            .placeholder_text("For example: Invoicing")
            .max_length(80)
            .width_chars(26)
            .build();
        ui::entry::adopt(&entry, ui::FieldSize::Normal);
        body.append(&entry);

        let remove = ui::button_with(ui::Face::Label("Remove name"), ui::Kind::Flat, ui::Size::Small);
        let save = ui::button_with(ui::Face::Label("Save"), ui::Kind::Primary, ui::Size::Small);
        let spacer = gtk4::Box::builder().hexpand(true).build();
        let actions = ui::hbox(3);
        actions.add_css_class("bar-name-actions");
        actions.append(&remove);
        actions.append(&spacer);
        actions.append(&save);
        body.append(&actions);

        // The one setting, as a switch: a sentence with a word at its end
        // does not say it can be pressed.
        let (pop_row, pop_up) = ui::switch_row("Show pop-up when I switch", "");
        pop_row.root.add_css_class("bar-name-switch");
        body.append(&pop_row.root);

        let editor = Editor {
            workspace: workspace.to_string(),
            button: button.clone(),
            popover,
            entry,
            title,
            remove,
            pop_up,
            keyboard: Cell::new(None),
        };
        editor.wire(&save);
        editor
    }

    fn wire(&self, save: &gtk4::Button) {
        let popover = self.popover.clone();
        {
            let popover = popover.clone();
            self.entry.connect_activate(move |_| popover.popdown());
        }
        {
            let popover = popover.clone();
            save.connect_clicked(move |_| popover.popdown());
        }
        {
            let popover = popover.clone();
            let entry = self.entry.clone();
            self.remove.connect_clicked(move |_| {
                entry.set_text("");
                popover.popdown();
            });
        }
        {
            self.pop_up.connect_active_notify(move |switch| {
                swaypplet_label::set_mode(if switch.is_active() {
                    Mode::Auto
                } else {
                    Mode::Hidden
                });
            });
        }
        // Escape drops the change. Capture phase: the entry would take it.
        let esc = gtk4::EventControllerKey::new();
        esc.set_propagation_phase(gtk4::PropagationPhase::Capture);
        // Escape lives in a closure that cannot hold `self`, so the flag is
        // shared with the close handler, which reads it and resets it.
        let cancelled = Rc::new(Cell::new(false));
        {
            let cancelled = cancelled.clone();
            let popover = popover.clone();
            esc.connect_key_pressed(move |_, key, _, _| {
                if key == gdk::Key::Escape {
                    cancelled.set(true);
                    popover.popdown();
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
        }
        popover.add_controller(esc);
        self.save_on_close(&cancelled);
    }

    /// Closing saves the field, unless Escape closed it.
    fn save_on_close(&self, flag: &Rc<Cell<bool>>) {
        let workspace = self.workspace.clone();
        let entry = self.entry.clone();
        let button = self.button.downgrade();
        let flag = flag.clone();
        self.popover.connect_closed(move |_| {
            let dropped = flag.replace(false);
            if !dropped {
                swaypplet_label::set_name(&workspace, entry.text().as_str());
            }
            if let Some(button) = button.upgrade() {
                restore_keyboard(&button);
            }
        });
    }

    /// The output this segment's bar is on.
    fn output(&self) -> Option<String> {
        let surface = self.button.native()?.surface()?;
        let monitor = self.button.display().monitor_at_surface(&surface)?;
        monitor.connector().map(|c| c.to_string())
    }

    fn open(&self) {
        if self.popover.is_visible() {
            return;
        }
        let mark = label_for_name(&self.workspace);
        self.title.set_label(&format!("Name workspace {mark}"));
        let current = swaypplet_label::name_of(&self.workspace).unwrap_or_default();
        self.entry.set_text(&current);
        self.remove.set_visible(!current.is_empty());
        self.pop_up.set_active(swaypplet_label::mode() == Mode::Auto);
        if let Some(window) = self.button.root().and_downcast::<gtk4::Window>()
            && window.is_layer_window()
        {
            if self.keyboard.get().is_none() {
                self.keyboard.set(Some(window.keyboard_mode()));
            }
            window.set_keyboard_mode(KeyboardMode::OnDemand);
        }
        self.popover.popup();
        let entry = self.entry.clone();
        glib::idle_add_local_once(move || {
            entry.grab_focus();
            entry.select_region(0, -1);
        });
    }
}

fn restore_keyboard(button: &gtk4::Button) {
    if let Some(window) = button.root().and_downcast::<gtk4::Window>()
        && window.is_layer_window()
    {
        window.set_keyboard_mode(KeyboardMode::None);
    }
}
