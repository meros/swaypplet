use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gtk4::prelude::*;
use gtk4_layer_shell::{Edge, LayerShell};

use crate::icons;
use crate::layer_shell::{self, LayerShellConfig};

use super::store::{self, NotificationStore};
use super::{CloseReason, Notification, Urgency};

const POPUP_CONTENT_HEIGHT: i32 = 100;
const SHADOW_PAD: i32 = 32;
// Total window height including shadow padding on both sides
const POPUP_WINDOW_HEIGHT: i32 = POPUP_CONTENT_HEIGHT + 2 * SHADOW_PAD;
// Gap between visible notification pills (shadow areas overlap)
const POPUP_GAP: i32 = 4;
// Stacking step = content height + visual gap (shadow pads overlap)
const POPUP_STEP: i32 = POPUP_CONTENT_HEIGHT + POPUP_GAP;
const POPUP_TOP_MARGIN: i32 = 8;
const POPUP_RIGHT_MARGIN: i32 = 8;
const POPUP_WIDTH: i32 = 360 + 2 * SHADOW_PAD;
const MAX_POPUPS: usize = 5;
const BASE_TIMEOUT_MS: u64 = 5000;
const PER_CHAR_MS: u64 = 40;

static POPUP_CONFIG: LayerShellConfig = LayerShellConfig {
    namespace: "swaypplet-notification",
    default_width: Some(POPUP_WIDTH),
    default_height: Some(POPUP_WINDOW_HEIGHT),
    anchors: &[(Edge::Top, true), (Edge::Right, true)],
    // No margin — the CSS padding inside the wrapper provides visual spacing
    margins: &[],
    keyboard_mode: gtk4_layer_shell::KeyboardMode::None,
};

struct PopupSlot {
    id: u32,
    window: gtk4::Window,
    timeout_id: Option<glib::SourceId>,
}

/// Manages popup notification windows stacked at the top-right.
pub struct PopupManager {
    app: gtk4::Application,
    slots: Vec<PopupSlot>,
    store: Rc<RefCell<NotificationStore>>,
}

impl PopupManager {
    /// Create a `PopupManager` and wire it to the store's `on_notify` callback.
    pub fn register(app: &gtk4::Application, store: Rc<RefCell<NotificationStore>>) {
        let manager = Rc::new(RefCell::new(Self {
            app: app.clone(),
            slots: Vec::new(),
            store: store.clone(),
        }));

        // Subscribe to new notifications
        let mgr = manager.clone();
        store.borrow_mut().connect_notify(move |notif| {
            mgr.borrow_mut().show(notif);
        });

        // Subscribe to closes (e.g. from CloseNotification D-Bus call)
        let mgr = manager.clone();
        store.borrow_mut().connect_close(move |id, _reason| {
            mgr.borrow_mut().dismiss(id);
        });
    }

    fn show(&mut self, notif: &Notification) {
        if !self.store.borrow().should_popup(notif) {
            return;
        }

        let id = notif.id;

        // If replacing an existing popup, update it in-place
        if let Some(slot) = self.slots.iter_mut().find(|s| s.id == id) {
            update_popup_content(&slot.window, notif);
            // Reset timeout
            if let Some(tid) = slot.timeout_id.take() {
                tid.remove();
            }
            // Schedule new timeout (can't call self method while borrowing slots)
            let new_timeout = schedule_auto_dismiss_static(&self.store, notif);
            slot.timeout_id = new_timeout;
            return;
        }

        // Limit visible popups
        if self.slots.len() >= MAX_POPUPS {
            // Dismiss the oldest
            let oldest_id = self.slots[0].id;
            self.dismiss(oldest_id);
        }

        let window = self.create_popup_window(notif);
        let timeout_id = self.schedule_auto_dismiss(notif);

        self.slots.push(PopupSlot {
            id,
            window,
            timeout_id,
        });

        self.restack();
    }

    fn dismiss(&mut self, id: u32) {
        if let Some(pos) = self.slots.iter().position(|s| s.id == id) {
            let slot = self.slots.remove(pos);
            if let Some(tid) = slot.timeout_id {
                tid.remove();
            }
            slot.window.close();
            self.restack();
        }
    }

    fn restack(&self) {
        for (i, slot) in self.slots.iter().enumerate() {
            // Each popup window includes shadow padding in the wrapper CSS.
            // Stack by content height + visual gap; shadow areas overlap.
            slot.window.set_margin(Edge::Top, (i as i32) * POPUP_STEP);
        }
    }

    fn schedule_auto_dismiss(&self, notif: &Notification) -> Option<glib::SourceId> {
        schedule_auto_dismiss_static(&self.store, notif)
    }
}

fn schedule_auto_dismiss_static(
    store: &Rc<RefCell<NotificationStore>>,
    notif: &Notification,
) -> Option<glib::SourceId> {
    // Critical notifications with no explicit timeout are persistent
    if notif.urgency == Urgency::Critical && notif.expire_timeout <= 0 {
        return None;
    }

    // Timeout 0 means persistent (spec: server decides; we honor 0 as "never")
    if notif.expire_timeout == 0 {
        return None;
    }

    let timeout_ms = if notif.expire_timeout > 0 {
        notif.expire_timeout as u64
    } else {
        // -1 means server decides
        let char_count = notif.summary.len() + notif.body.len();
        BASE_TIMEOUT_MS + (char_count as u64) * PER_CHAR_MS
    };

    let id = notif.id;
    let store = store.clone();
    Some(glib::timeout_add_local_once(
        Duration::from_millis(timeout_ms),
        move || {
            store::store_close(&store, id, CloseReason::Expired);
        },
    ))
}

impl PopupManager {
    fn create_popup_window(&self, notif: &Notification) -> gtk4::Window {
        let window = layer_shell::create_layer_window(&self.app, &POPUP_CONFIG);
        window.add_css_class("notification-popup");

        if notif.urgency == Urgency::Critical {
            window.add_css_class("critical");
        }

        build_popup_content(&window, notif, self.store.clone());
        window.present();
        window
    }
}

fn build_popup_content(
    window: &gtk4::Window,
    notif: &Notification,
    store: Rc<RefCell<NotificationStore>>,
) {
    let hbox = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(10)
        .build();
    hbox.add_css_class("notification-popup-content");

    // Text content
    let vbox = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .valign(gtk4::Align::Center)
        .build();

    if !notif.app_name.is_empty() {
        let app_label = gtk4::Label::builder()
            .label(notif.app_name.to_uppercase())
            .halign(gtk4::Align::Start)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .build();
        app_label.add_css_class("notification-app-name");
        vbox.append(&app_label);
    }

    let summary = gtk4::Label::builder()
        .label(&notif.summary)
        .halign(gtk4::Align::Start)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .build();
    summary.add_css_class("notification-summary");
    vbox.append(&summary);

    if !notif.body.is_empty() {
        let markup = super::markup::sanitize(&notif.body);
        let body = gtk4::Label::builder()
            .label(&markup)
            .use_markup(true)
            .halign(gtk4::Align::Start)
            .wrap(true)
            .wrap_mode(gtk4::pango::WrapMode::WordChar)
            .max_width_chars(50)
            .lines(3)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .build();
        body.add_css_class("notification-body");
        vbox.append(&body);
    }

    // Progress bar
    if let Some(progress) = notif.progress {
        let bar = gtk4::ProgressBar::builder()
            .fraction(progress as f64 / 100.0)
            .hexpand(true)
            .build();
        bar.add_css_class("notification-progress");
        vbox.append(&bar);
    }

    // Action buttons
    if !notif.actions.is_empty() {
        let actions_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(4)
            .build();
        actions_box.add_css_class("notification-actions");

        for (key, label) in &notif.actions {
            if key == "default" {
                continue; // default action is handled by clicking the popup body
            }
            let btn = gtk4::Button::builder().label(label).build();
            btn.add_css_class("flat");
            btn.add_css_class("notification-action-btn");

            let id = notif.id;
            let store_c = store.clone();
            let key_c = key.clone();
            btn.connect_clicked(move |_| {
                log::info!("Action invoked: notification {id}, action {key_c}");
                store::store_close(&store_c, id, CloseReason::Dismissed);
            });
            actions_box.append(&btn);
        }

        vbox.append(&actions_box);
    }

    hbox.append(&vbox);

    // Close button
    let close_btn = gtk4::Button::builder()
        .label(icons::CLOSE)
        .valign(gtk4::Align::Start)
        .build();
    close_btn.add_css_class("flat");
    close_btn.add_css_class("notification-close-btn");

    let id = notif.id;
    let store_c = store.clone();
    close_btn.connect_clicked(move |_| {
        store::store_close(&store_c, id, CloseReason::Dismissed);
    });
    hbox.append(&close_btn);

    // Click on body = focus the app's window, then dismiss
    let gesture = gtk4::GestureClick::new();
    let id = notif.id;
    let app_name = notif.app_name.clone();
    let store_c = store;
    gesture.connect_released(move |_, _, _, _| {
        focus_app_window(&app_name);
        store::store_close(&store_c, id, CloseReason::Dismissed);
    });
    hbox.add_controller(gesture);

    // Wrapper provides transparent padding for drop shadow room
    let wrapper = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .build();
    wrapper.add_css_class("notification-popup-wrapper");
    wrapper.append(&hbox);
    window.set_child(Some(&wrapper));
}

/// Try to focus a Sway window matching the notification's app name.
/// Uses `swaymsg -t get_tree` to find the window, then `[con_id=N] focus`.
fn focus_app_window(app_name: &str) {
    if app_name.is_empty() {
        return;
    }

    let output = match std::process::Command::new("swaymsg")
        .args(["-t", "get_tree", "--raw"])
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            log::warn!("swaymsg get_tree failed: {}", e);
            return;
        }
    };

    let text = String::from_utf8_lossy(&output.stdout);
    let app_lower = app_name.to_lowercase();

    // Parse the JSON tree to find a matching window con_id.
    // We look for "app_id" or "class" matching the app_name (case-insensitive).
    if let Some(con_id) = find_con_id_in_tree(&text, &app_lower) {
        let cmd = format!("[con_id={}] focus", con_id);
        let _ = std::process::Command::new("swaymsg")
            .arg(&cmd)
            .spawn()
            .map_err(|e| log::warn!("swaymsg focus failed: {}", e));
    }
}

/// Walk the swaymsg JSON tree (simple string scanning — avoids serde dependency)
/// to find a container whose app_id or class matches `app_lower`.
fn find_con_id_in_tree(json: &str, app_lower: &str) -> Option<u64> {
    // Strategy: find "app_id":"<match>" or "class":"<match>" near an "id":<num>.
    // We split by `{` to get rough "node" chunks and look for matches.
    let mut best_id: Option<u64> = None;
    let mut best_focused = false;

    for chunk in json.split('{') {
        // Check if this chunk has a matching app_id or window class
        let matches = match_field(chunk, "app_id", app_lower)
            || match_field(chunk, "class", app_lower);

        if !matches {
            continue;
        }

        // Extract the "id" field
        if let Some(id) = extract_u64_field(chunk, "\"id\"") {
            let focused = chunk.contains("\"focused\":true");
            // Prefer focused window, otherwise take the first match
            if best_id.is_none() || (focused && !best_focused) {
                best_id = Some(id);
                best_focused = focused;
            }
        }
    }

    best_id
}

fn match_field(chunk: &str, field: &str, app_lower: &str) -> bool {
    let pattern = format!("\"{}\"", field);
    if let Some(pos) = chunk.find(&pattern) {
        let rest = &chunk[pos + pattern.len()..];
        // Skip : and whitespace, find the quoted value
        if let Some(q1) = rest.find('"') {
            let after_q = &rest[q1 + 1..];
            if let Some(q2) = after_q.find('"') {
                let value = &after_q[..q2];
                return value.to_lowercase().contains(app_lower)
                    || app_lower.contains(&value.to_lowercase());
            }
        }
    }
    false
}

fn extract_u64_field(chunk: &str, field: &str) -> Option<u64> {
    let pos = chunk.find(field)?;
    let rest = &chunk[pos + field.len()..];
    // Skip `:` and whitespace
    let rest = rest.trim_start().strip_prefix(':')?;
    let rest = rest.trim_start();
    // Parse the number
    let num_str: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    num_str.parse().ok()
}

fn update_popup_content(window: &gtk4::Window, notif: &Notification) {
    // For simplicity on replace, just rebuild the content
    if let Some(child) = window.child() {
        window.set_child(None::<&gtk4::Widget>);
        let _ = child;
    }
    // We need a store reference — but for in-place updates we just rebuild.
    // The close/action buttons won't work without a store ref, but replaces_id
    // updates are typically for progress bars where actions aren't needed.
    let vbox = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(10)
        .build();
    vbox.add_css_class("notification-popup-content");

    let text_box = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .valign(gtk4::Align::Center)
        .build();

    if !notif.app_name.is_empty() {
        let app_label = gtk4::Label::builder()
            .label(notif.app_name.to_uppercase())
            .halign(gtk4::Align::Start)
            .build();
        app_label.add_css_class("notification-app-name");
        text_box.append(&app_label);
    }

    let summary = gtk4::Label::builder()
        .label(&notif.summary)
        .halign(gtk4::Align::Start)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .build();
    summary.add_css_class("notification-summary");
    text_box.append(&summary);

    if !notif.body.is_empty() {
        let markup = super::markup::sanitize(&notif.body);
        let body = gtk4::Label::builder()
            .label(&markup)
            .use_markup(true)
            .halign(gtk4::Align::Start)
            .wrap(true)
            .wrap_mode(gtk4::pango::WrapMode::WordChar)
            .max_width_chars(50)
            .lines(3)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .build();
        body.add_css_class("notification-body");
        text_box.append(&body);
    }

    if let Some(progress) = notif.progress {
        let bar = gtk4::ProgressBar::builder()
            .fraction(progress as f64 / 100.0)
            .hexpand(true)
            .build();
        bar.add_css_class("notification-progress");
        text_box.append(&bar);
    }

    vbox.append(&text_box);

    let wrapper = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .build();
    wrapper.add_css_class("notification-popup-wrapper");
    wrapper.append(&vbox);
    window.set_child(Some(&wrapper));
}
