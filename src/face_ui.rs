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

/// Shorten a command line for display without hiding what it does.
///
/// The tail is what matters (`systemctl restart foo`), but so is the leading
/// binary, so this keeps both ends and elides the middle.
fn summarise(cmdline: &str, exe: &str) -> String {
    let text = if cmdline.trim().is_empty() { exe } else { cmdline };
    let text = text.trim();
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
    title: Label,
    detail: Label,
    hint: Label,
    button: Button,
}

fn build(app: &gtk4::Application) -> Ui {
    let window = create_layer_window(app, &CONFIRM_CONFIG);
    window.add_css_class("face-confirm");

    let root = GtkBox::new(Orientation::Vertical, 12);
    // Same floating-surface base the OSD, launcher and polkit card use.
    root.add_css_class("glass-card");
    root.set_margin_top(20);
    root.set_margin_bottom(20);
    root.set_margin_start(24);
    root.set_margin_end(24);

    let title = Label::new(None);
    title.add_css_class("face-confirm-title");
    title.set_halign(Align::Start);
    title.set_wrap(true);

    let detail = Label::new(None);
    detail.add_css_class("face-confirm-detail");
    detail.set_halign(Align::Start);
    detail.set_wrap(true);
    detail.set_selectable(false);

    let hint = Label::new(None);
    hint.add_css_class("face-confirm-hint");
    hint.set_halign(Align::Start);
    hint.set_wrap(true);

    let button = Button::with_label("Authorise");
    button.add_css_class("face-confirm-button");
    button.set_halign(Align::End);

    root.append(&title);
    root.append(&detail);
    root.append(&hint);
    root.append(&button);
    window.set_child(Some(&root));

    Ui {
        window,
        title,
        detail,
        hint,
        button,
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
        ui.button.connect_clicked(move |_| answer(true));
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
                    ui.title
                        .set_text(&format!("Authorise as {}", req.target_user));
                    ui.detail
                        .set_text(&summarise(&req.peer_cmdline, &req.peer_exe));
                    ui.hint.set_text("Looking for your face…");
                    ui.button.set_sensitive(false);
                    ui.window.set_visible(true);
                }
                Stage::Confirm => {
                    // Only now does a press mean anything.
                    *pending.borrow_mut() = Some(req.id.clone());
                    armed.set(true);
                    ui.hint
                        .set_text("Recognised you. Press Enter to authorise, Esc to cancel.");
                    ui.button.set_sensitive(true);
                    ui.button.grab_focus();
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
