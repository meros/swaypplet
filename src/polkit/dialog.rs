//! The elevation card: one GTK4 modal for `pkexec` and `sudo`.
//!
//! Visual language matches `osd.rs` and `launcher.rs`: full-screen
//! transparent layer-shell window with a centred card. What is on it is
//! described once, by a [`Card`], and which methods are accepting input at
//! this instant by a [`Methods`]; the orchestrator (`mod.rs`) computes both
//! and this file only paints them. There is no elevate mode and no polkit
//! mode, only a card with or without a command well, with or without a
//! password field, and a caption that names what works right now.
//!
//! Cancel via button, Esc, or backdrop click.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4_layer_shell::Edge;

use crate::auth_field::{AuthField, Caption, Tone};
use crate::layer_shell::{self, LayerShellConfig};

use super::agent::ResolvedIdentity;

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

/// Visual treatment of the status line.
#[derive(Clone, Copy, Default)]
pub enum StatusKind {
    #[default]
    Info,
    Error,
    Success,
}

/// Everything the card says about the request. Decided before the card is
/// presented and never changed while it is up: a card that resizes under a
/// pointer already moving toward a button is the bug docs/AUTH_CARD.md was
/// written to kill.
pub struct Card<'a> {
    pub title: &'a str,
    pub message: &'a str,
    /// polkit's icon, and the action id its glyph falls back on. Empty for
    /// a request polkit never saw.
    pub icon_name: &'a str,
    pub action_id: &'a str,
    /// The command line, in a well of its own, for a request that has no
    /// vendor message to describe it: `sudo` reaches this card through
    /// pam_race, and the command is the only thing that distinguishes a
    /// request the user made from one they did not.
    pub command: Option<&'a str>,
    /// The accounts polkit will accept. One or none hides the picker.
    pub identities: &'a [ResolvedIdentity],
    pub details: String,
    /// Whether a password typed here reaches the conversation. False only
    /// for a face check with no pam_race link behind it, where the terminal
    /// alone owns the prompt.
    pub password: bool,
}

/// Which methods are accepting input at this instant. The caption's resting
/// sentence is computed from this and nothing else — the honesty rule.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Methods {
    pub face: bool,
    pub fp: bool,
    pub password: bool,
}

impl Methods {
    /// What the user may do right now, as one sentence. Every combination
    /// has words, so the line is never empty.
    pub fn resting(self) -> &'static str {
        match (self.face, self.fp, self.password) {
            (true, true, true) => "Look at the camera, touch the reader or enter your password",
            (true, false, true) => "Look at the camera or enter your password to allow this",
            (false, true, true) => "Touch the reader or enter your password to allow this",
            (false, false, true) => "Enter your password to allow this",
            (true, true, false) => "Look at the camera or touch the reader to allow this",
            (true, false, false) => "Look at the camera to allow this",
            (false, true, false) => "Touch the reader to allow this",
            (false, false, false) => "Enter your password in the terminal to allow this",
        }
    }
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
pub struct Callbacks {
    /// Enter or the button. The text is the password; empty is the Allow
    /// press, which only means something once a face has matched.
    pub on_submit: Rc<dyn Fn(String)>,
    pub on_cancel: Rc<dyn Fn()>,
    pub on_identity: Rc<dyn Fn(u32)>,
    /// Fired on the first keystroke in the password entry. The controller
    /// uses it to abandon a face check that is still running, so the user
    /// who has decided to type does not have to wait out the camera.
    pub on_typing: Rc<dyn Fn()>,
}

impl Default for Callbacks {
    fn default() -> Self {
        Self {
            on_submit: Rc::new(|_| {}),
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
    /// What is accepting input right now. See [`Methods`].
    methods: Cell<Methods>,
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
            methods: Cell::new(Methods::default()),
        });

        // Wire interactions — handlers fire the closures from `callbacks`
        // so the controller can swap them per session.

        // Submit: Enter on the entry, or the button. Same closure, because
        // they are the same act.
        {
            let cbs = callbacks.clone();
            let entry = password_entry.clone();
            let submit = Rc::new(move || {
                let text = entry.text().to_string();
                entry.set_text("");
                let cb = cbs.borrow().on_submit.clone();
                cb(text);
            });
            let s = submit.clone();
            password_entry.connect_activate(move |_| s());
            auth_btn.connect_clicked(move |_| submit());
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

    /// Show the card. Every control it can ever show is laid out now, before
    /// it is presented, and nothing is added afterwards. Affordances arrive
    /// on PAM's schedule — the reader arms, the camera opens, the prompt
    /// lands — and each of them is a paint change on a slot held from the
    /// first frame. Text typed before PAM asks for it is buffered by the
    /// orchestrator rather than dropped, so an entry that is ready early
    /// costs nothing and answers the question every user of this dialog
    /// asks first: can I just type it?
    pub fn present(&self, card: &Card, callbacks: Callbacks) {
        self.password_entry.set_text("");
        self.password_entry.set_placeholder_text(Some("Password"));
        self.set_status("", StatusKind::Info);
        self.card.remove_css_class("polkit-shake");
        self.card.remove_css_class("polkit-success");
        self.card.remove_css_class("polkit-verifying");
        self.field.set_busy(false);
        self.field.set_fp_armed(false);
        self.caps.set(caps_lock_on());

        // The field is the one thing decided per card rather than per
        // instant: a face check with no conversation behind it has nothing
        // to type into, and holding an entry open that leads nowhere would
        // be the card lying about what it can do.
        self.field.widget().set_visible(card.password);
        self.password_entry.set_visible(card.password);
        self.password_entry.set_sensitive(card.password);
        self.methods.set(Methods {
            password: card.password,
            ..Methods::default()
        });
        self.caption.set_resting(self.methods.get().resting());

        self.title_label.set_label(card.title);
        self.message_label.set_label(card.message);
        self.set_icon(card.icon_name, card.action_id);

        match card.command {
            Some(command) => {
                self.face_command.set_label(command);
                self.face_well.set_visible(true);
                self.face_consequence.set_visible(true);
            }
            None => {
                self.face_well.set_visible(false);
                self.face_consequence.set_visible(false);
            }
        }

        *self.identities.borrow_mut() = card.identities.iter().map(|i| i.uid).collect();
        if card.identities.len() <= 1 {
            self.identity_row.set_visible(false);
        } else {
            let model = gtk4::StringList::new(&[]);
            for ident in card.identities {
                model.append(&ident.username);
            }
            self.identity_combo.set_model(Some(&model));
            self.identity_combo.set_selected(0);
            self.identity_row.set_visible(true);
        }

        self.details_label.set_label(&card.details);
        self.details_revealer.set_reveal_child(false);

        // The button submits a password when there is one to submit. With
        // no field it is the Allow press and nothing else, and it waits,
        // disabled rather than absent, for a face to match: a button that
        // appeared only then would move the layout under the user's hands at
        // exactly the moment a press becomes consequential.
        self.auth_btn.set_visible(true);
        self.auth_btn.set_label(if card.password {
            "Authenticate"
        } else {
            "Allow"
        });
        self.auth_btn.set_sensitive(card.password);

        *self.callbacks.borrow_mut() = callbacks;

        // The entry is on the card from the first frame, so the caret belongs
        // in it from the first frame too — `set_password_prompt` grabs it
        // again when PAM actually asks, which is late enough to lose the
        // first keystrokes of someone who started typing immediately.
        if card.password {
            self.password_entry.grab_focus();
        }
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

    /// What is accepting input right now. Repaints the resting sentence and
    /// the field's arm pulse; never touches an allocation. The password
    /// half is the card's and cannot be turned on here.
    pub fn set_methods(&self, methods: Methods) {
        let methods = Methods {
            password: self.methods.get().password,
            ..methods
        };
        if self.methods.replace(methods) == methods {
            return;
        }
        self.field.set_fp_armed(methods.fp);
        self.caption.set_resting(methods.resting());
    }

    /// A word from a method still running — a reader hint, what the camera
    /// saw. Holds a moment, then the resting sentence returns.
    pub fn hint(&self, text: &str) {
        if !text.is_empty() {
            self.caption.hint(text);
        }
    }

    /// Rejected: the words, the shake, and the inputs back.
    pub fn reject(&self, text: &str) {
        self.set_verifying(false);
        self.set_status(text, StatusKind::Error);
        self.field.flash_reject();
        self.shake();
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
        // for the placeholder. pam_race's own prompt names the user and the
        // other methods ("Password, finger or face for meros"); the caption
        // already says all of that, so the placeholder keeps the one word.
        let cleaned = prompt.trim_end_matches([' ', ':']);
        let placeholder = match cleaned.split_once(" for ") {
            Some((head, _)) if head.starts_with("Password") => "Password",
            _ if cleaned.is_empty() => "Password",
            _ => cleaned,
        };
        self.password_entry.set_placeholder_text(Some(placeholder));
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

    /// The stack is checking a password; hold the inputs until it answers.
    /// A border and a word on the field rather than a card-wide dimming:
    /// greying the whole card for a field-scale event reads as a fault.
    pub fn set_verifying(&self, verifying: bool) {
        let password = self.methods.get().password;
        self.password_entry.set_sensitive(password && !verifying);
        self.auth_btn.set_sensitive(password && !verifying);
        self.field.set_busy(verifying);
        if verifying {
            self.set_status("Checking\u{2026}", StatusKind::Info);
        } else {
            self.caption.clear_status();
            if password && self.password_entry.is_visible() {
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

/// polkit's details, as the card's Details drawer shows them.
pub fn format_details(request: &super::agent::AuthRequest) -> String {
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
