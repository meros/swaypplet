//! The confirm surface for face-authenticated sudo and pkexec.
//!
//! Two stages, and the split is the security property rather than polish.
//!
//! `announce` appears the moment the camera opens, before any decision is
//! asked for. It names the process requesting elevation. `confirm` replaces it
//! only once the face has actually matched, and only then does a keypress mean
//! anything. Showing a single "press to allow" prompt would be smaller code
//! and worse security: a prompt with no attribution, appearing at a moment the
//! user did not choose, trains people to press a key reflexively. That reflex
//! is the thing an attacker is trying to buy.
//!
//! The surface takes an exclusive keyboard grab. That is deliberate too: the
//! Enter that authorises must not be one the user aimed at the terminal, and a
//! grab means the keystroke cannot be delivered anywhere else. It also means
//! `sudo` cannot receive type-ahead while this is up, which closes the race
//! where a queued password and a face confirm arrive together.
//!
//! Timeout is a refusal. If the window expires without a press, the agent
//! answers `deny` rather than staying silent, so the daemon does not have to
//! wait out its own confirm window to learn the answer.

use gtk4::prelude::*;
use gtk4::{glib, Align, Box as GtkBox, Button, Label, Orientation};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer};

use crate::face::{self, Request, Stage};
use crate::layer_shell::{create_layer_window, LayerShellConfig};

static CONFIRM_CONFIG: LayerShellConfig = LayerShellConfig {
    namespace: "swaypplet-face-confirm",
    layer: Layer::Overlay,
    exclusive: false,
    default_width: Some(420),
    default_height: None,
    // No anchors: the compositor centres it. An authorisation prompt belongs
    // where the eye already is, not in the corner where notifications live and
    // get ignored.
    anchors: &[(Edge::Top, false), (Edge::Bottom, false)],
    margins: &[],
    keyboard_mode: KeyboardMode::Exclusive,
};

/// Swap the ring's state class. The visual vocabulary lives in the
/// stylesheet, shared with the lock screen's indicator, so the thing the user
/// learns to read is the same widget in both places.
fn set_ring(ring: &GtkBox, state: &str) {
    for old in ["looking", "dark", "found", "ok", "fail"] {
        ring.remove_css_class(&format!("face-ring-{old}"));
    }
    ring.add_css_class(&format!("face-ring-{state}"));
}

/// Shorten a command line for display without hiding what it does.
///
/// The tail is what matters (`systemctl restart foo`), but so is the leading
/// binary, so this keeps both ends and elides the middle.
fn prettify(word: &str) -> String {
    // /nix/store/<hash>-sudo-1.9.17p2/bin/sudo -> sudo
    //
    // The store path is true but useless: sixty characters of hash in a
    // prompt whose whole job is letting someone recognise what they asked
    // for at a glance. Everything here is a store path, so showing them
    // distinguishes nothing.
    match word.rsplit_once('/') {
        Some((head, tail)) if head.starts_with("/nix/store/") && !tail.is_empty() => {
            tail.to_string()
        }
        _ => word.to_string(),
    }
}

fn summarise(cmdline: &str, exe: &str) -> String {
    let raw = if cmdline.trim().is_empty() { exe } else { cmdline };
    let joined: Vec<String> = raw.split_whitespace().map(prettify).collect();
    let joined = joined.join(" ");
    let text = joined.trim();
    if text.chars().count() <= 72 {
        return text.to_string();
    }
    let head: String = text.chars().take(40).collect();
    let tail: String = text
        .chars()
        .skip(text.chars().count().saturating_sub(28))
        .collect();
    format!("{head}…{tail}")
}

struct Ui {
    window: gtk4::Window,
    ring: GtkBox,
    command: Label,
    status: Label,
    allow: Button,
    cancel: Button,
}

/// The card, laid out around what the user has to decide.
///
/// Four things, in the order a person actually needs them:
///
///   1. WHAT this is. "Administrator access", not "Authorise as meros". You
///      are not authorising as yourself; you are letting something run as
///      root, and the title has to name that consequence or the prompt is
///      hiding its own point.
///   2. WHAT is asking. The command, in a recessed well so it reads as
///      quoted evidence rather than as instruction. It is the only thing on
///      the card that distinguishes a request you made from one you did not.
///   3. WHAT it becomes. "Runs as root" stated plainly, because the
///      escalation is the entire reason to think before pressing.
///   4. WHAT is happening and WHAT you can do. The ring and its status share
///      a row, so motion and wording change together, and both actions are
///      always visible.
///
/// Cancel is present and enabled from the first frame. Refusing must never
/// be harder or slower than accepting; a prompt where the only easy path is
/// "yes" is a prompt that manufactures consent.
fn build(app: &gtk4::Application) -> Ui {
    let window = create_layer_window(app, &CONFIRM_CONFIG);
    window.add_css_class("face-confirm");

    let root = GtkBox::new(Orientation::Vertical, 0);
    root.add_css_class("glass-card");
    root.add_css_class("face-confirm-card");
    // Do not stretch to the layer surface, or the buttons drift off the
    // painted card onto whatever is behind it.
    root.set_halign(Align::Center);
    root.set_valign(Align::Center);
    root.set_width_request(420);

    let title = Label::new(Some("Administrator access"));
    title.add_css_class("face-confirm-title");
    title.set_halign(Align::Start);

    // The command, in a well. Monospace because it is a command, recessed
    // because it is evidence.
    let command = Label::new(None);
    command.add_css_class("face-confirm-command");
    command.set_halign(Align::Start);
    command.set_wrap(true);
    command.set_selectable(true);
    let well = GtkBox::new(Orientation::Vertical, 0);
    well.add_css_class("face-confirm-well");
    well.append(&command);
    well.set_margin_top(12);

    let consequence = Label::new(Some("Runs as root"));
    consequence.add_css_class("face-confirm-consequence");
    consequence.set_halign(Align::Start);
    consequence.set_margin_top(8);

    // Status row: the ring and the words it belongs to, together.
    let status_row = GtkBox::new(Orientation::Horizontal, 10);
    status_row.set_margin_top(16);
    let ring = GtkBox::builder()
        .width_request(20)
        .height_request(20)
        .valign(Align::Center)
        .build();
    ring.add_css_class("face-ring");
    let status = Label::new(None);
    status.add_css_class("face-confirm-status");
    status.set_halign(Align::Start);
    status.set_wrap(true);
    status_row.append(&ring);
    status_row.append(&status);

    let actions = GtkBox::new(Orientation::Horizontal, 10);
    actions.set_halign(Align::End);
    actions.set_margin_top(18);
    let cancel = Button::with_label("Cancel");
    cancel.add_css_class("face-confirm-cancel");
    let allow = Button::with_label("Allow");
    allow.add_css_class("face-confirm-button");
    // Disabled, not absent. A button that appears only once the face matches
    // would move the layout under the user's hands at the exact moment a
    // press becomes consequential.
    allow.set_sensitive(false);
    actions.append(&cancel);
    actions.append(&allow);

    root.append(&title);
    root.append(&well);
    root.append(&consequence);
    root.append(&status_row);
    root.append(&actions);
    window.set_child(Some(&root));

    Ui {
        window,
        ring,
        command,
        status,
        allow,
        cancel,
    }
}

/// Start the confirm agent and the surface that answers it.
pub fn register(app: &gtk4::Application) {
    let (tx, rx) = async_channel::unbounded::<Request>();
    face::start(tx);

    let ui = build(app);
    // The request currently on screen. `None` means nothing is pending, and
    // any stray keypress or click is ignored rather than answering an
    // identifier we no longer hold.
    let pending: std::rc::Rc<std::cell::RefCell<Option<String>>> =
        std::rc::Rc::new(std::cell::RefCell::new(None));
    // Armed only in the confirm stage. A press during announce must not
    // authorise: the face has not matched yet, and accepting early would make
    // the match irrelevant.
    let armed = std::rc::Rc::new(std::cell::Cell::new(false));

    let answer = {
        let pending = pending.clone();
        let armed = armed.clone();
        let window = ui.window.clone();
        move |allow: bool| {
            let Some(id) = pending.borrow_mut().take() else {
                return;
            };
            if allow && !armed.get() {
                // Pressed during announce. Put it back and keep waiting; the
                // user pressed before there was anything to agree to.
                *pending.borrow_mut() = Some(id);
                return;
            }
            armed.set(false);
            window.set_visible(false);
            face::reply(&id, allow);
        }
    };

    {
        let answer = answer.clone();
        ui.allow.connect_clicked(move |_| answer(true));
    }
    {
        let answer = answer.clone();
        ui.cancel.connect_clicked(move |_| answer(false));
    }

    let keys = gtk4::EventControllerKey::new();
    {
        let answer = answer.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            match key {
                gtk4::gdk::Key::Return | gtk4::gdk::Key::KP_Enter | gtk4::gdk::Key::space => {
                    answer(true);
                    glib::Propagation::Stop
                }
                gtk4::gdk::Key::Escape => {
                    answer(false);
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
    }
    ui.window.add_controller(keys);

    // Closing the surface by any other route is a refusal, not a dismissal.
    {
        let answer = answer.clone();
        ui.window.connect_close_request(move |_| {
            answer(false);
            glib::Propagation::Stop
        });
    }

    glib::spawn_future_local(async move {
        while let Ok(req) = rx.recv().await {
            match req.stage {
                Stage::Announce => {
                    *pending.borrow_mut() = Some(req.id.clone());
                    armed.set(false);
                    ui.command
                        .set_text(&summarise(&req.peer_cmdline, &req.peer_exe));
                    set_ring(&ui.ring, "looking");
                    ui.status.set_text("Checking your face…");
                    ui.allow.set_sensitive(false);
                    ui.window.set_visible(true);
                }
                Stage::Confirm => {
                    // Only now does a press mean anything.
                    *pending.borrow_mut() = Some(req.id.clone());
                    armed.set(true);
                    set_ring(&ui.ring, "ok");
                    ui.status.set_text("Recognised you. Enter to allow, Esc to cancel.");
                    ui.allow.set_sensitive(true);
                    ui.allow.grab_focus();
                    ui.window.set_visible(true);
                }
                Stage::Cancel => {
                    // The daemon gave up. Drop the request without answering:
                    // it is already gone on the other side, and a late reply
                    // would apply to nothing.
                    if pending.borrow().as_deref() == Some(req.id.as_str()) {
                        pending.borrow_mut().take();
                        armed.set(false);
                        ui.window.set_visible(false);
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::summarise;

    #[test]
    fn a_short_command_is_shown_whole() {
        assert_eq!(
            summarise("sudo systemctl restart foo", "/run/wrappers/bin/sudo"),
            "sudo systemctl restart foo"
        );
    }

    #[test]
    fn a_long_command_keeps_both_ends() {
        let long = format!("sudo sh -c '{}' && systemctl restart foo", "x".repeat(90));
        let out = summarise(&long, "");
        assert!(out.contains('…'), "expected elision, got {out}");
        assert!(out.starts_with("sudo sh -c"), "lost the head: {out}");
        assert!(out.ends_with("systemctl restart foo"), "lost the tail: {out}");
    }

    #[test]
    fn an_empty_cmdline_falls_back_to_the_binary() {
        assert_eq!(summarise("   ", "/run/wrappers/bin/pkexec"), "/run/wrappers/bin/pkexec");
    }
}
