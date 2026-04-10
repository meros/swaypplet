//! GTK4 modal for polkit authentication.
//!
//! Visual language matches `osd.rs` and `launcher.rs`: full-screen
//! transparent layer-shell window with a centred dark card. The card
//! shows an action icon, title, polkit's `message`, a prominent
//! fingerprint pill (when the helper is asking for one), and a password
//! entry as the fallback. Cancel via button, Esc, or backdrop click.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4_layer_shell::Edge;

use crate::layer_shell::{self, LayerShellConfig};

use super::agent::AuthRequest;

static POLKIT_CONFIG: LayerShellConfig = LayerShellConfig {
    namespace: "swaypplet-polkit",
    default_width: None,
    default_height: None,
    anchors: &[
        (Edge::Top, true),
        (Edge::Bottom, true),
        (Edge::Left, true),
        (Edge::Right, true),
    ],
    margins: &[],
    keyboard_mode: gtk4_layer_shell::KeyboardMode::Exclusive,
};

/// Visual treatment of the status line below the fingerprint pill.
#[derive(Clone, Copy)]
pub enum StatusKind {
    Info,
    Error,
    Success,
}

/// Callbacks the controller installs each time it presents the dialog.
/// Defaults are no-ops so it's always safe to fire signals.
struct Callbacks {
    on_password: Box<dyn Fn(String)>,
    on_cancel: Box<dyn Fn()>,
    on_identity: Box<dyn Fn(u32)>,
}

impl Default for Callbacks {
    fn default() -> Self {
        Self {
            on_password: Box::new(|_| {}),
            on_cancel: Box::new(|| {}),
            on_identity: Box::new(|_| {}),
        }
    }
}

pub struct PolkitDialog {
    window: gtk4::Window,
    icon_image: gtk4::Image,
    icon_label: gtk4::Label,
    title_label: gtk4::Label,
    message_label: gtk4::Label,
    fp_pill: gtk4::Box,
    fp_label: gtk4::Label,
    password_entry: gtk4::PasswordEntry,
    identity_row: gtk4::Box,
    identity_combo: gtk4::DropDown,
    status_label: gtk4::Label,
    details_revealer: gtk4::Revealer,
    details_label: gtk4::Label,
    auth_btn: gtk4::Button,
    card: gtk4::Box,
    identities: Rc<RefCell<Vec<u32>>>,
    callbacks: Rc<RefCell<Callbacks>>,
}

impl PolkitDialog {
    pub fn new(app: &gtk4::Application) -> Rc<Self> {
        let window = layer_shell::create_layer_window(app, &POLKIT_CONFIG);
        window.add_css_class("polkit");
        window.set_visible(false);

        // ── Backdrop fills the whole screen; click anywhere → cancel ──
        let backdrop = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .halign(gtk4::Align::Fill)
            .valign(gtk4::Align::Fill)
            .hexpand(true)
            .vexpand(true)
            .build();
        backdrop.add_css_class("polkit-backdrop");

        // Centring wrapper
        let center = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .halign(gtk4::Align::Center)
            .valign(gtk4::Align::Center)
            .hexpand(true)
            .vexpand(true)
            .build();

        // ── The card ─────────────────────────────────────────────────
        let card = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(14)
            .width_request(440)
            .build();
        card.add_css_class("polkit-container");

        // Icon (image first, fallback nerd-font label)
        let icon_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .halign(gtk4::Align::Center)
            .build();
        let icon_image = gtk4::Image::builder()
            .pixel_size(56)
            .visible(false)
            .build();
        icon_image.add_css_class("polkit-icon");
        let icon_label = gtk4::Label::builder().label("\u{f0483}").build();
        icon_label.add_css_class("polkit-icon-glyph");
        icon_box.append(&icon_image);
        icon_box.append(&icon_label);

        let title_label = gtk4::Label::builder()
            .label("Authentication Required")
            .halign(gtk4::Align::Center)
            .build();
        title_label.add_css_class("polkit-title");

        let message_label = gtk4::Label::builder()
            .halign(gtk4::Align::Center)
            .justify(gtk4::Justification::Center)
            .wrap(true)
            .wrap_mode(gtk4::pango::WrapMode::WordChar)
            .max_width_chars(48)
            .build();
        message_label.add_css_class("polkit-message");

        // ── Fingerprint pill (hidden by default) ──────────────────────
        let fp_pill = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(10)
            .halign(gtk4::Align::Center)
            .visible(false)
            .build();
        fp_pill.add_css_class("polkit-fp-pill");
        let fp_glyph = gtk4::Label::builder().label("\u{f0577}").build();
        fp_glyph.add_css_class("polkit-fp-glyph");
        let fp_label = gtk4::Label::builder().label("Touch fingerprint reader").build();
        fp_label.add_css_class("polkit-fp-label");
        fp_pill.append(&fp_glyph);
        fp_pill.append(&fp_label);

        // ── Password entry (the fallback) ─────────────────────────────
        let password_entry = gtk4::PasswordEntry::builder()
            .show_peek_icon(false)
            .placeholder_text("Password")
            .hexpand(true)
            .build();
        password_entry.add_css_class("polkit-entry");

        // ── Identity picker (hidden when only one identity) ───────────
        let identity_row = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(10)
            .visible(false)
            .build();
        identity_row.add_css_class("polkit-identity-row");
        let identity_lbl = gtk4::Label::builder().label("Run as").build();
        identity_lbl.add_css_class("polkit-identity-label");
        let identity_combo = gtk4::DropDown::builder().hexpand(true).build();
        identity_combo.add_css_class("polkit-identity-combo");
        identity_row.append(&identity_lbl);
        identity_row.append(&identity_combo);

        // ── Status line (errors / info) ───────────────────────────────
        let status_label = gtk4::Label::builder()
            .halign(gtk4::Align::Center)
            .visible(false)
            .build();
        status_label.add_css_class("polkit-status");

        // ── Details revealer (action_id, vendor, command, pid) ────────
        let details_toggle = gtk4::Button::builder()
            .label("\u{f0142}  Details")
            .has_frame(false)
            .halign(gtk4::Align::Start)
            .build();
        details_toggle.add_css_class("polkit-details-toggle");
        let details_revealer = gtk4::Revealer::builder()
            .transition_type(gtk4::RevealerTransitionType::SlideDown)
            .transition_duration(180)
            .reveal_child(false)
            .build();
        let details_label = gtk4::Label::builder()
            .halign(gtk4::Align::Start)
            .justify(gtk4::Justification::Left)
            .wrap(true)
            .wrap_mode(gtk4::pango::WrapMode::WordChar)
            .max_width_chars(56)
            .selectable(true)
            .build();
        details_label.add_css_class("polkit-details");
        details_revealer.set_child(Some(&details_label));
        {
            let revealer = details_revealer.clone();
            let toggle = details_toggle.clone();
            details_toggle.connect_clicked(move |_| {
                let revealed = !revealer.reveals_child();
                revealer.set_reveal_child(revealed);
                toggle.set_label(if revealed {
                    "\u{f0140}  Details"
                } else {
                    "\u{f0142}  Details"
                });
            });
        }

        // ── Action buttons ───────────────────────────────────────────
        let actions = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(10)
            .halign(gtk4::Align::End)
            .build();
        actions.add_css_class("polkit-actions");
        let cancel_btn = gtk4::Button::builder().label("Cancel").build();
        cancel_btn.add_css_class("polkit-cancel");
        let auth_btn = gtk4::Button::builder().label("Authenticate").build();
        auth_btn.add_css_class("polkit-auth-btn");
        auth_btn.add_css_class("suggested-action");
        actions.append(&cancel_btn);
        actions.append(&auth_btn);

        // ── Assemble the card ────────────────────────────────────────
        card.append(&icon_box);
        card.append(&title_label);
        card.append(&message_label);
        card.append(&fp_pill);
        card.append(&password_entry);
        card.append(&identity_row);
        card.append(&status_label);
        card.append(&details_toggle);
        card.append(&details_revealer);
        card.append(&actions);

        center.append(&card);
        backdrop.append(&center);
        window.set_child(Some(&backdrop));

        let identities: Rc<RefCell<Vec<u32>>> = Rc::new(RefCell::new(Vec::new()));
        let callbacks: Rc<RefCell<Callbacks>> = Rc::new(RefCell::new(Callbacks::default()));

        let dialog = Rc::new(PolkitDialog {
            window: window.clone(),
            icon_image,
            icon_label,
            title_label,
            message_label,
            fp_pill,
            fp_label,
            password_entry: password_entry.clone(),
            identity_row,
            identity_combo: identity_combo.clone(),
            status_label,
            details_revealer,
            details_label,
            auth_btn: auth_btn.clone(),
            card: card.clone(),
            identities: identities.clone(),
            callbacks: callbacks.clone(),
        });

        // Wire interactions — handlers fire the closures from `callbacks`
        // so the controller can swap them per session.

        // Password submit (Enter on entry)
        {
            let cbs = callbacks.clone();
            let entry = password_entry.clone();
            password_entry.connect_activate(move |_| {
                let text = entry.text().to_string();
                entry.set_text("");
                (cbs.borrow().on_password)(text);
            });
        }

        // Authenticate button → submit current password text
        {
            let cbs = callbacks.clone();
            let entry = password_entry.clone();
            auth_btn.connect_clicked(move |_| {
                let text = entry.text().to_string();
                entry.set_text("");
                (cbs.borrow().on_password)(text);
            });
        }

        // Cancel button
        {
            let cbs = callbacks.clone();
            cancel_btn.connect_clicked(move |_| {
                (cbs.borrow().on_cancel)();
            });
        }

        // Identity dropdown
        {
            let cbs = callbacks.clone();
            let identities_c = identities.clone();
            identity_combo.connect_selected_notify(move |combo| {
                let idx = combo.selected() as usize;
                if let Some(uid) = identities_c.borrow().get(idx).copied() {
                    (cbs.borrow().on_identity)(uid);
                }
            });
        }

        // Backdrop click → cancel
        {
            let cbs = callbacks.clone();
            let backdrop_gesture = gtk4::GestureClick::new();
            backdrop_gesture.connect_released(move |_, _, _, _| {
                (cbs.borrow().on_cancel)();
            });
            backdrop.add_controller(backdrop_gesture);
        }

        // Swallow clicks on the card so they never reach the backdrop.
        {
            let card_gesture = gtk4::GestureClick::new();
            card_gesture.set_propagation_phase(gtk4::PropagationPhase::Capture);
            card_gesture.connect_pressed(|gesture, _, _, _| {
                gesture.set_state(gtk4::EventSequenceState::Claimed);
            });
            card.add_controller(card_gesture);
        }

        // Esc cancels — capture-phase so it beats the password entry.
        {
            let cbs = callbacks.clone();
            let key = gtk4::EventControllerKey::new();
            key.set_propagation_phase(gtk4::PropagationPhase::Capture);
            key.connect_key_pressed(move |_, key, _, _| {
                if key == gtk4::gdk::Key::Escape {
                    (cbs.borrow().on_cancel)();
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
            window.add_controller(key);
        }

        dialog
    }

    // ─── Lifecycle ────────────────────────────────────────────────────

    pub fn present(
        &self,
        request: &AuthRequest,
        on_password: Box<dyn Fn(String)>,
        on_cancel: Box<dyn Fn()>,
        on_identity: Box<dyn Fn(u32)>,
    ) {
        // Reset state
        self.password_entry.set_text("");
        self.password_entry.set_sensitive(true);
        self.auth_btn.set_sensitive(true);
        self.set_status("", StatusKind::Info);
        self.show_fingerprint(false, "Touch fingerprint reader");
        self.card.remove_css_class("polkit-shake");
        self.card.remove_css_class("polkit-success");

        // Title + message
        self.title_label.set_label("Authentication Required");
        self.message_label.set_label(if request.message.is_empty() {
            "An action requires authorization."
        } else {
            request.message.as_str()
        });

        // Icon
        self.set_icon(&request.icon_name, &request.action_id);

        // Identities
        *self.identities.borrow_mut() =
            request.identities.iter().map(|i| i.uid).collect();
        if request.identities.len() <= 1 {
            self.identity_row.set_visible(false);
        } else {
            let model = gtk4::StringList::new(&[]);
            for ident in &request.identities {
                model.append(&ident.username);
            }
            self.identity_combo.set_model(Some(&model));
            self.identity_combo.set_selected(0);
            self.identity_row.set_visible(true);
        }

        // Details
        self.details_label.set_label(&format_details(request));
        self.details_revealer.set_reveal_child(false);

        // Install fresh callbacks
        *self.callbacks.borrow_mut() = Callbacks {
            on_password,
            on_cancel,
            on_identity,
        };

        self.window.set_visible(true);
        self.password_entry.grab_focus();
    }

    pub fn hide(&self) {
        self.window.set_visible(false);
        self.password_entry.set_text("");
        *self.callbacks.borrow_mut() = Callbacks::default();
    }

    // ─── State updates from the controller ───────────────────────────

    pub fn set_status(&self, text: &str, kind: StatusKind) {
        if text.is_empty() {
            self.status_label.set_visible(false);
            self.status_label.set_label("");
        } else {
            self.status_label.set_visible(true);
            self.status_label.set_label(text);
        }
        self.status_label.remove_css_class("polkit-status-error");
        self.status_label.remove_css_class("polkit-status-success");
        self.status_label.remove_css_class("polkit-status-info");
        match kind {
            StatusKind::Info => self.status_label.add_css_class("polkit-status-info"),
            StatusKind::Error => self.status_label.add_css_class("polkit-status-error"),
            StatusKind::Success => self.status_label.add_css_class("polkit-status-success"),
        }
    }

    pub fn show_fingerprint(&self, active: bool, label: &str) {
        self.fp_pill.set_visible(active);
        if active {
            self.fp_label.set_label(label);
            self.fp_pill.add_css_class("polkit-fp-active");
        } else {
            self.fp_pill.remove_css_class("polkit-fp-active");
        }
    }

    pub fn set_password_prompt(&self, prompt: &str) {
        // PAM gives prompts like "Password: " — strip trailing colon/space
        // for the placeholder.
        let cleaned = prompt
            .trim_end_matches([' ', ':'])
            .to_string();
        let placeholder = if cleaned.is_empty() {
            "Password".to_string()
        } else {
            cleaned
        };
        self.password_entry.set_placeholder_text(Some(&placeholder));
    }

    pub fn shake(&self) {
        // Re-trigger the CSS keyframe by removing then adding the class.
        let card = self.card.clone();
        card.remove_css_class("polkit-shake");
        let c = card.clone();
        glib::idle_add_local_once(move || {
            c.add_css_class("polkit-shake");
        });
    }

    pub fn flash_success(&self) {
        self.card.add_css_class("polkit-success");
    }

    pub fn lock_inputs(&self) {
        self.password_entry.set_sensitive(false);
        self.auth_btn.set_sensitive(false);
    }

    fn set_icon(&self, icon_name: &str, action_id: &str) {
        // Try the icon name from polkit first.
        if !icon_name.is_empty() {
            let display = gtk4::prelude::WidgetExt::display(&self.icon_image);
            if gtk4::IconTheme::for_display(&display).has_icon(icon_name) {
                self.icon_image.set_icon_name(Some(icon_name));
                self.icon_image.set_visible(true);
                self.icon_label.set_visible(false);
                return;
            }
        }
        // Fall back to a Nerd Font glyph based on the action id.
        self.icon_image.set_visible(false);
        self.icon_label.set_visible(true);
        self.icon_label.set_label(glyph_for_action(action_id));
    }
}

fn glyph_for_action(action_id: &str) -> &'static str {
    let id = action_id.to_ascii_lowercase();
    if id.contains("shutdown") || id.contains("halt") || id.contains("power-off") {
        "\u{f0425}" // 󰐥
    } else if id.contains("reboot") || id.contains("restart") {
        "\u{f0709}" // 󰜉
    } else if id.contains("suspend") {
        "\u{f0904}" // 󰤄
    } else if id.contains("hibernate") {
        "\u{f02ca}" // 󰋊
    } else if id.contains("network") || id.contains("wifi") || id.contains("nm-") {
        "\u{f1bbb}" // 󱮻
    } else if id.contains("bluetooth") {
        "\u{f00af}" // 󰂯
    } else if id.contains("mount") || id.contains("udisks") || id.contains("disk") {
        "\u{f02ca}" // disk-ish
    } else if id.contains("update") || id.contains("install") || id.contains("packagekit") {
        "\u{f01da}" // 󰇚
    } else {
        "\u{f0483}" // 󰒃 shield
    }
}

fn format_details(request: &AuthRequest) -> String {
    let mut lines = Vec::new();
    lines.push(format!("Action: {}", request.action_id));
    if let Some(vendor) = request.details.get("polkit.message") {
        lines.push(format!("Message: {vendor}"));
    }
    if let Some(cmd) = request
        .details
        .get("command_line")
        .or_else(|| request.details.get("polkit.command_line"))
    {
        lines.push(format!("Command: {cmd}"));
    }
    if let Some(pid) = request.details.get("process") {
        lines.push(format!("Process: {pid}"));
    }
    for (k, v) in &request.details {
        if matches!(
            k.as_str(),
            "polkit.message" | "command_line" | "polkit.command_line" | "process"
        ) {
            continue;
        }
        lines.push(format!("{k}: {v}"));
    }
    lines.join("\n")
}
