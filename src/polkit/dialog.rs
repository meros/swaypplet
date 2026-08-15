//! GTK4 modal for polkit authentication.
//!
//! Visual language matches `osd.rs` and `launcher.rs`: full-screen
//! transparent layer-shell window with a centred dark card. The card
//! shows an action icon, title, polkit's `message`, a prominent
//! fingerprint pill (when the helper is asking for one), and a password
//! entry as the fallback. Cancel via button, Esc, or backdrop click.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4_layer_shell::Edge;

use crate::auth_field::{AuthField, Caption, Tone};
use crate::layer_shell::{self, LayerShellConfig};

use super::agent::AuthRequest;

static POLKIT_CONFIG: LayerShellConfig = LayerShellConfig {
    namespace: "swaypplet-polkit",
    layer: gtk4_layer_shell::Layer::Overlay,
    exclusive: false,
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

/// Nerd Font check, for the approved state.
const ICON_OK: &str = "\u{f012c}";

/// Visual treatment of the status line below the fingerprint pill.
#[derive(Clone, Copy, Default)]
pub enum StatusKind {
    #[default]
    Info,
    Error,
    Success,
}

/// Callbacks the controller installs each time it presents the dialog.
/// Defaults are no-ops so it's always safe to fire signals.
///
/// `Rc`, not `Box`, so a handler can be lifted out of the `RefCell` and the
/// borrow dropped before it runs. Every one of these ends up in the
/// orchestrator, and the orchestrator's paths reach `hide()`, which installs
/// fresh callbacks -- a `borrow_mut` while the fire site still held a
/// `borrow`. GTK signal trampolines cannot unwind, so that panic aborted the
/// process rather than raising: the agent died, its faced registration went
/// with it, and elevation quietly stopped working until systemd restarted it.
struct Callbacks {
    on_password: Rc<dyn Fn(String)>,
    on_cancel: Rc<dyn Fn()>,
    on_identity: Rc<dyn Fn(u32)>,
    /// Fired on the first keystroke in the password entry. The controller
    /// uses it to abandon a face check that is still running, so the user
    /// who has decided to type does not have to wait out the camera.
    on_typing: Rc<dyn Fn()>,
}

impl Default for Callbacks {
    fn default() -> Self {
        Self {
            on_password: Rc::new(|_| {}),
            on_cancel: Rc::new(|| {}),
            on_identity: Rc::new(|_| {}),
            on_typing: Rc::new(|| {}),
        }
    }
}

pub struct PolkitDialog {
    icon_image: gtk4::Image,
    icon_label: gtk4::Label,
    title_label: gtk4::Label,
    message_label: gtk4::Label,
    /// The password field, with the fingerprint whorl and the face ring
    /// sharing its leading mark and Caps Lock at its trailing edge. See
    /// docs/AUTH_CARD.md — the pills this replaces were containers whose only
    /// job was to hold those next to each other, and the field already is one.
    field: AuthField,
    caption: Caption,
    face_well: gtk4::Box,
    face_command: gtk4::Label,
    face_consequence: gtk4::Label,
    password_entry: gtk4::PasswordEntry,
    identity_row: gtk4::Box,
    identity_combo: gtk4::DropDown,
    details_revealer: gtk4::Revealer,
    details_label: gtk4::Label,
    auth_btn: gtk4::Button,
    card: gtk4::Box,
    reveal: crate::anim::Reveal,
    identities: Rc<RefCell<Vec<u32>>>,
    callbacks: Rc<RefCell<Callbacks>>,
    /// Caps Lock, so a rejection composes the warning onto its own line.
    caps: Cell<bool>,
    /// Is the reader armed right now? The caption's resting sentence is
    /// computed from this and nothing else — the honesty rule.
    fp_armed: Cell<bool>,
    /// The elevate path has no password to type, so its resting sentence
    /// names the camera instead.
    elevate: Cell<bool>,
    /// Does the calling user have enrolled fingerprints? Answered off-thread
    /// once, at agent start, so `present` knows whether to lay out a
    /// fingerprint row long before PAM offers one. `None` until it lands (and
    /// when fprintd would not say), which reads as yes: a slot held open for
    /// a reader that never arms is quieter than one that appears late.
    fp_expected: Cell<Option<bool>>,
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
            .spacing(0)
            .width_request(400)
            .build();
        card.add_css_class("glass-card");
        card.add_css_class("polkit-container");

        // Icon (image first, fallback nerd-font label)
        let icon_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .halign(gtk4::Align::Center)
            .build();
        icon_box.add_css_class("polkit-icon-box");
        let icon_image = gtk4::Image::builder().pixel_size(44).visible(false).build();
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

        // ── The command, for requests polkit never described ──────────
        //
        // `sudo` reaches this card through pam_race rather than through
        // polkit, so there is no action id and no vendor message to show. The
        // command line is the only thing that distinguishes a request the
        // user made from one they did not, so it gets a well of its own:
        // monospace because it is a command, recessed because it is evidence
        // rather than instruction.
        let face_command = gtk4::Label::builder()
            .halign(gtk4::Align::Start)
            .wrap(true)
            .wrap_mode(gtk4::pango::WrapMode::WordChar)
            .max_width_chars(48)
            .selectable(true)
            .build();
        face_command.add_css_class("face-confirm-command");
        let face_well = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .visible(false)
            .build();
        face_well.add_css_class("face-confirm-well");
        face_well.append(&face_command);

        let face_consequence = gtk4::Label::builder()
            .label("Runs as root")
            .halign(gtk4::Align::Center)
            .visible(false)
            .build();
        face_consequence.add_css_class("face-confirm-consequence");

        // ── Password entry (the fallback) ─────────────────────────────
        let password_entry = gtk4::PasswordEntry::builder()
            .show_peek_icon(false)
            .placeholder_text("Password")
            .hexpand(true)
            .build();
        // One box for the entry and the face ring, which is the only mark
        // left in it.
        let field = AuthField::new(&password_entry);

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

        let caption = Caption::new(46);

        // ── Details revealer (action_id, vendor, command, pid) ────────
        let details_toggle = gtk4::Button::builder()
            .label("\u{f0142}  Details")
            .has_frame(false)
            .halign(gtk4::Align::Start)
            .build();
        details_toggle.add_css_class("polkit-details-toggle");
        let details_revealer = gtk4::Revealer::builder()
            .transition_type(gtk4::RevealerTransitionType::SlideDown)
            .transition_duration(200)
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
        // Content box on the glass: fades over the full enter/exit while
        // the card (pane) tint arrives fast (motion on glass, anim.rs).
        //
        // Spacing 0, and every gap declared in the stylesheet instead. A
        // uniform gap is the one thing a card like this cannot use: the title
        // and the message are one unit and the message and the input are two,
        // so separating both by 14 px said they were equally related and left
        // the field looking crowded against the sentence that explains it.
        // The lock card has worked this way for a while; this brings the two
        // onto the same model. See the gap block in data/style.css.
        let content = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(0)
            .build();
        content.append(&icon_box);
        content.append(&title_label);
        content.append(&message_label);
        content.append(&face_well);
        content.append(&face_consequence);
        content.append(field.widget());
        content.append(caption.widget());
        content.append(&identity_row);
        content.append(&details_toggle);
        content.append(&details_revealer);
        content.append(&actions);
        card.append(&content);

        center.append(&card);
        backdrop.append(&center);
        window.set_child(Some(&backdrop));

        let reveal = crate::anim::Reveal::new(&window, &card).content(&content);

        let identities: Rc<RefCell<Vec<u32>>> = Rc::new(RefCell::new(Vec::new()));
        let callbacks: Rc<RefCell<Callbacks>> = Rc::new(RefCell::new(Callbacks::default()));

        let dialog = Rc::new(PolkitDialog {
            icon_image,
            icon_label,
            title_label,
            message_label,
            field: field.clone(),
            caption: caption.clone(),
            face_well,
            face_command,
            face_consequence,
            password_entry: password_entry.clone(),
            identity_row,
            identity_combo: identity_combo.clone(),
            details_revealer,
            details_label,
            auth_btn: auth_btn.clone(),
            card: card.clone(),
            reveal,
            identities: identities.clone(),
            callbacks: callbacks.clone(),
            caps: Cell::new(false),
            fp_armed: Cell::new(false),
            elevate: Cell::new(false),
            fp_expected: Cell::new(None),
        });

        // Ask fprintd now, while the agent is starting and nobody is waiting
        // on a dialog. The answer decides whether the card carries a
        // fingerprint row, and that has to be settled before a card is shown,
        // not when PAM gets round to offering the reader.
        {
            let dialog = dialog.clone();
            crate::spawn::spawn_work(crate::fp::self_enrolled_blocking, move |enrolled| {
                dialog.fp_expected.set(enrolled);
            });
        }

        // Wire interactions — handlers fire the closures from `callbacks`
        // so the controller can swap them per session.

        // Password submit (Enter on entry)
        {
            let cbs = callbacks.clone();
            let entry = password_entry.clone();
            password_entry.connect_activate(move |_| {
                let text = entry.text().to_string();
                entry.set_text("");
                let cb = cbs.borrow().on_password.clone();
                cb(text);
            });
        }

        // Authenticate button → submit current password text
        {
            let cbs = callbacks.clone();
            let entry = password_entry.clone();
            auth_btn.connect_clicked(move |_| {
                let text = entry.text().to_string();
                entry.set_text("");
                let cb = cbs.borrow().on_password.clone();
                cb(text);
            });
        }

        // First keystroke means the user has chosen the password. Tell the
        // controller so a running face check can get out of the way instead
        // of holding PAM until its own deadline.
        {
            let cbs = callbacks.clone();
            let caption_clear = caption.clone();
            password_entry.connect_changed(move |entry| {
                caption_clear.clear_status();
                if !entry.text().is_empty() {
                    let cb = cbs.borrow().on_typing.clone();
                    cb();
                }
            });
        }

        // Cancel button
        {
            let cbs = callbacks.clone();
            cancel_btn.connect_clicked(move |_| {
                let cb = cbs.borrow().on_cancel.clone();
                cb();
            });
        }

        // Identity dropdown
        {
            let cbs = callbacks.clone();
            let identities_c = identities.clone();
            identity_combo.connect_selected_notify(move |combo| {
                let idx = combo.selected() as usize;
                if let Some(uid) = identities_c.borrow().get(idx).copied() {
                    let cb = cbs.borrow().on_identity.clone();
                    cb(uid);
                }
            });
        }

        // Backdrop click → cancel, but only when the click really landed on
        // the apron.
        //
        // This used to be enforced by a claiming gesture on the card, which
        // is a trap: any ancestor gesture that claims a press can starve the
        // widget the press was aimed at, and the failure is silent -- the
        // button highlights under the pointer and then does nothing, while
        // the keyboard keeps working because a layer surface's keyboard grab
        // does not go through gesture propagation at all. Hit-testing the
        // apron asks the question directly and cannot interfere with anything
        // inside the card.
        {
            let cbs = callbacks.clone();
            let backdrop_gesture = gtk4::GestureClick::new();
            backdrop_gesture.connect_released(move |gesture, _, x, y| {
                let Some(apron) = gesture.widget() else {
                    return;
                };
                let landed_on_apron = apron
                    .pick(x, y, gtk4::PickFlags::DEFAULT)
                    .is_some_and(|hit| hit == apron);
                log::debug!("polkit: backdrop release, on apron: {landed_on_apron}");
                if !landed_on_apron {
                    return;
                }
                let cb = cbs.borrow().on_cancel.clone();
                cb();
            });
            backdrop.add_controller(backdrop_gesture);
        }

        // Esc cancels — capture-phase so it beats the password entry.
        {
            let cbs = callbacks.clone();
            let key = gtk4::EventControllerKey::new();
            key.set_propagation_phase(gtk4::PropagationPhase::Capture);
            key.connect_key_pressed(move |_, key, _, _| {
                if key == gtk4::gdk::Key::Escape {
                    let cb = cbs.borrow().on_cancel.clone();
                    cb();
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
            window.add_controller(key);
        }

        // Caps Lock indicator tracks the keyboard device directly; the
        // initial state is applied on each present().
        if let Some(keyboard) = keyboard_device() {
            let dialog_caps = dialog.clone();
            keyboard.connect_caps_lock_state_notify(move |_| {
                let on = caps_lock_on();
                dialog_caps.caps.set(on);
                if on {
                    dialog_caps.caption.caps_edge();
                }
            });
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
        on_typing: Box<dyn Fn()>,
    ) {
        // Reset state
        self.password_entry.set_text("");
        self.password_entry.set_sensitive(true);
        self.auth_btn.set_sensitive(true);
        self.set_status("", StatusKind::Info);
        // Every control this card can ever show is laid out now, before it is
        // presented, and nothing is added to it afterwards. The affordances
        // arrive on PAM's schedule — the fingerprint prompt first, the
        // password prompt after it times out — and revealing each as it
        // landed grew the card twice while the user was reading it, the
        // second time under a pointer already moving toward a button.
        //
        // So the pill's space is held from the start (blank until the reader
        // is armed), and the password row is simply present: text typed
        // before PAM asks for it is buffered rather than dropped, so an entry
        // that is ready early costs nothing and answers the question every
        // user of this dialog asks first — can I just type it?
        self.elevate.set(false);
        self.field.widget().set_visible(true);
        self.field.set_fp_armed(false);
        self.fp_armed.set(false);
        self.password_entry.set_visible(true);
        self.password_entry.set_placeholder_text(Some("Password"));
        self.auth_btn.set_visible(true);
        self.auth_btn.set_sensitive(true);
        self.auth_btn.set_label("Authenticate");
        self.face_well.set_visible(false);
        self.face_consequence.set_visible(false);
        self.card.remove_css_class("polkit-shake");
        self.card.remove_css_class("polkit-success");
        self.card.remove_css_class("polkit-verifying");
        self.caps.set(caps_lock_on());
        self.refresh_resting();

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
        *self.identities.borrow_mut() = request.identities.iter().map(|i| i.uid).collect();
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
            on_password: on_password.into(),
            on_cancel: on_cancel.into(),
            on_identity: on_identity.into(),
            on_typing: on_typing.into(),
        };

        // The entry is on the card from the first frame, so the caret belongs
        // in it from the first frame too — `set_password_prompt` grabs it
        // again when PAM actually asks, which is late enough to lose the
        // first keystrokes of someone who started typing immediately.
        self.password_entry.grab_focus();
        self.reveal.show();
    }

    /// Present the card for an elevation that polkit knows nothing about.
    ///
    /// `sudo` reaches face authentication through pam_race directly, so there
    /// is no polkit action, no vendor message and no identity list — only the
    /// process that asked. The card is otherwise the same card, because from
    /// the user's side it is the same decision: something wants to run as
    /// root, and they are being asked whether it may.
    ///
    /// There is no password entry here. pam_race does prompt for a password,
    /// but it prompts on the terminal that ran `sudo`, through that terminal's
    /// own conversation — this card never sees it. What it can never prompt
    /// for is the confirm, because a prompt is answerable by a pipe, which is
    /// the whole reason the confirm lives in the session. So the card carries
    /// the press and the terminal carries the password, and they race.
    pub fn present_elevate(
        &self,
        command: &str,
        on_allow: Box<dyn Fn(String)>,
        on_cancel: Box<dyn Fn()>,
    ) {
        self.set_status("", StatusKind::Info);
        // No password here (see above) and no fingerprint: the terminal owns
        // one and pam_race's own child process owns the other. The field goes
        // with them, and the face reports through the caption, which the card
        // has anyway.
        self.elevate.set(true);
        self.field.widget().set_visible(false);
        self.password_entry.set_text("");
        self.caps.set(false);
        self.refresh_resting();
        self.identity_row.set_visible(false);
        self.card.remove_css_class("polkit-shake");
        self.card.remove_css_class("polkit-success");
        self.card.remove_css_class("polkit-verifying");

        // Name the consequence, not the ceremony. "Authenticate as meros"
        // describes the mechanism; "Administrator access" describes what the
        // user is about to hand out, which is the thing worth reading.
        self.title_label.set_label("Administrator access");
        self.message_label
            .set_label("A program is asking to run as root.");
        self.set_icon("", "");

        self.face_command.set_label(command);
        self.face_well.set_visible(true);
        self.face_consequence.set_visible(true);
        self.details_label.set_label(&format!("Command: {command}"));
        self.details_revealer.set_reveal_child(false);

        // Disabled, not absent. A button that appeared only once the face
        // matched would move the layout under the user's hands at exactly the
        // moment a press becomes consequential.
        self.auth_btn.set_label("Allow");
        self.auth_btn.set_visible(true);
        self.auth_btn.set_sensitive(false);

        *self.callbacks.borrow_mut() = Callbacks {
            on_password: on_allow.into(),
            on_cancel: on_cancel.into(),
            on_identity: Rc::new(|_| {}),
            on_typing: Rc::new(|| {}),
        };

        self.reveal.show();
    }

    pub fn hide(&self) {
        self.reveal.hide();
        self.password_entry.set_text("");
        *self.callbacks.borrow_mut() = Callbacks::default();
    }

    // ─── State updates from the controller ───────────────────────────

    /// Set the status line. Its space is reserved whether or not it says
    /// anything.
    ///
    /// It used to be hidden when empty, so the first error or the final
    /// "Authorised" grew the card and shoved everything below it down -- at
    /// exactly the moment the user is reading, and for the success case at
    /// exactly the moment they are reaching for a button. A blank line costs
    /// one line of height; a card that resizes under the pointer costs a
    /// misclick.
    pub fn set_status(&self, text: &str, kind: StatusKind) {
        let tone = match kind {
            StatusKind::Info => Tone::Info,
            StatusKind::Error => Tone::Error,
            StatusKind::Success => Tone::Success,
        };
        self.caption.status(text, tone, self.caps.get());
    }

    /// The sentence under everything else: what the user may do right now.
    ///
    /// The honesty rule. It names the reader only while the reader is armed,
    /// and never on the elevate path, where there is nothing to type and the
    /// camera is the only thing being asked.
    fn refresh_resting(&self) {
        self.caption.set_resting(if self.elevate.get() {
            "Look at the camera to allow this"
        } else if self.fp_armed.get() {
            "Touch the reader or enter your password to allow this"
        } else {
            "Enter your password to allow this"
        });
    }

    /// The reader armed, or stood down. A mark and a sentence, not a row.
    pub fn show_fingerprint(&self, active: bool, label: &str) {
        // fprintd's own words when it has some; otherwise the resting
        // sentence already says what to do and repeating it is noise.
        if active && !label.is_empty() && label != "Touch fingerprint reader" {
            self.caption.fp_hint(label);
        }
        if self.fp_armed.replace(active) != active {
            self.field.set_fp_armed(active);
            self.refresh_resting();
        }
    }

    /// Show or hide the face check status on the dialog caption.
    pub fn show_face(&self, active: bool, _state: &str, label: &str) {
        if active && !label.is_empty() {
            self.caption.status(label, Tone::Info, self.caps.get());
        }
    }

    /// Arm the confirm press once the face has matched.
    ///
    /// The press stays explicit. A match is evidence that the right person is
    /// in front of the camera; it is not evidence that they asked for this,
    /// and a face is presented by walking into a room. The button is what a
    /// pipe cannot forge, so it is never skipped.
    pub fn arm_allow(&self) {
        self.auth_btn.set_label("Allow");
        self.auth_btn.set_visible(true);
        self.auth_btn.set_sensitive(true);
        self.auth_btn.grab_focus();
    }

    pub fn set_password_prompt(&self, prompt: &str) {
        // PAM gives prompts like "Password: " — strip trailing colon/space
        // for the placeholder.
        let cleaned = prompt.trim_end_matches([' ', ':']).to_string();
        let placeholder = if cleaned.is_empty() {
            "Password".to_string()
        } else {
            cleaned
        };
        self.password_entry.set_placeholder_text(Some(&placeholder));
        // PAM is requesting a password. The row has been there since the card
        // was presented — all this does is name the prompt and take the
        // caret. The button goes back to submitting a password, in case a
        // face confirm relabelled it to "Allow" earlier in the same
        // conversation.
        self.auth_btn.set_label("Authenticate");
        self.password_entry.grab_focus();
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

    /// Say it was approved, inside the card, without moving anything.
    ///
    /// Deliberately does not hide the fingerprint or face pill. Hiding them
    /// collapsed their row at the same instant the status line appeared, so
    /// the card resized twice in opposite directions. They stay where they
    /// are and simply stop asking: the pulse comes off, and what they last
    /// reported stands as the record of which method actually worked.
    pub fn flash_success(&self) {
        self.card.add_css_class("polkit-success");
        self.icon_image.set_visible(false);
        self.icon_label.set_visible(true);
        self.icon_label.set_label(ICON_OK);
        self.icon_label.add_css_class("polkit-icon-ok");
        // The pulse comes off; the marks stay lit as the record of which
        // method actually worked.
        self.field.widget().remove_css_class("auth-fp-armed");
    }

    pub fn lock_inputs(&self) {
        self.password_entry.set_sensitive(false);
        self.auth_btn.set_sensitive(false);
    }

    /// Grey out input while the helper verifies a response; re-enable after.
    pub fn set_verifying(&self, verifying: bool) {
        self.password_entry.set_sensitive(!verifying);
        self.auth_btn.set_sensitive(!verifying);
        if verifying {
            self.card.add_css_class("polkit-verifying");
        } else {
            self.card.remove_css_class("polkit-verifying");
            if self.password_entry.is_visible() {
                self.password_entry.grab_focus();
            }
        }
    }

    fn set_icon(&self, icon_name: &str, action_id: &str) {
        self.icon_label.remove_css_class("polkit-icon-ok");
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

/// Caps Lock, as the card should report it.
///
/// The env override is the dev hook the lock screen has too: the headless
/// render harness has no keyboard to latch, and this is one of the states the
/// card must be shown not to resize for.
fn caps_lock_on() -> bool {
    if let Ok(v) = std::env::var("SWAYPPLET_PREVIEW_CAPS") {
        return v == "1";
    }
    keyboard_device().is_some_and(|kb| kb.is_caps_locked())
}

fn keyboard_device() -> Option<gdk4::Device> {
    gdk4::Display::default()
        .and_then(|d| d.default_seat())
        .and_then(|seat| seat.keyboard())
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
