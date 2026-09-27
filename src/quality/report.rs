//! `swaypplet report`: a screenshot, a description, and an issue.
//!
//! The capture is the screenshot flow's own (the frozen-screen selector, or
//! the window grid); Escape there means "no picture", and the card opens
//! anyway, because a report of something that is not on screen any more is
//! still a report. The card is glass on its own namespace
//! (`Namespace::Report`), takes the keyboard while it is up, and says in
//! plain words that what it sends is public.
//!
//! Send hides the card at once and hands the rest to a worker thread: the
//! PNG encode, the upload, the issue. A notification says it is on its way
//! and is replaced by one with the issue's link (or the error).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gtk4::prelude::*;

use crate::screenshot::capture::Image;
use crate::services::notifications::store::{StoreRef, store_add};
use crate::services::notifications::{Notification, Urgency};
use crate::shell::{Namespace, Surface};
use crate::ui::{self, Kind, Text, Tone};

use super::body::{self, Output, Report};

/// What to capture before the card opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Area {
    /// Drag a rectangle; a click takes the output; Escape takes nothing.
    Region,
    /// Pick a window from the grid.
    Window,
    /// The focused output, no selector (for a binding, and the harness).
    Screen,
    /// No picture.
    None,
}

impl Area {
    pub fn parse(arg: Option<&str>) -> Area {
        match arg {
            Some("window") => Area::Window,
            Some("screen") | Some("output") => Area::Screen,
            Some("none") | Some("text") => Area::None,
            _ => Area::Region,
        }
    }
}

/// What the card hands to [`send`].
pub struct Draft {
    pub description: String,
    /// The picture, when there is one and the switch was left on.
    pub image: Option<Image>,
    pub attach_log: bool,
}

thread_local! {
    /// The open card. One at a time: a second `swaypplet report` while one
    /// is up is a second press of the same key.
    static CARD: RefCell<Option<Surface>> = const { RefCell::new(None) };
    /// Notification id → issue URL, for the Open button.
    static LINKS: RefCell<HashMap<u32, String>> = RefCell::new(HashMap::new());
}

/// Wire the notification's Open button. Called once, with the store.
pub fn install(store: &StoreRef) {
    store.borrow_mut().connect_action(|id, key| {
        if key != "open-issue" {
            return;
        }
        if let Some(url) = LINKS.with(|l| l.borrow().get(&id).cloned()) {
            open_uri(&url);
        }
    });
}

pub fn open_uri(url: &str) {
    if let Err(e) =
        gtk4::gio::AppInfo::launch_default_for_uri(url, None::<&gtk4::gio::AppLaunchContext>)
    {
        log::warn!("quality: could not open {url}: {e}");
    }
}

/// Capture, then the card.
pub fn start(app: &gtk4::Application, store: &StoreRef, area: Area) {
    if CARD.with(|c| c.borrow().is_some()) {
        return;
    }
    let app_c = app.clone();
    let store = store.clone();
    let then = move |image: Option<Image>| open_card(&app_c, &store, image);
    match area {
        Area::None => then(None),
        Area::Window => crate::screenshot::window::pick(app, then),
        Area::Region => crate::screenshot::select::region(
            app,
            crate::screenshot::select::Mode::Region,
            move |selection| then(selection.map(|s| s.image)),
        ),
        Area::Screen => crate::spawn::spawn_work(
            || {
                let output = crate::sway::ipc::focused_output()?;
                crate::screenshot::capture::output(&output)
                    .map_err(|e| log::warn!("report: {e}"))
                    .ok()
            },
            then,
        ),
    }
}

fn open_card(app: &gtk4::Application, store: &StoreRef, image: Option<Image>) {
    let store = store.clone();
    let surface = card(app, image, move |draft| send(&store, draft));
    CARD.with(|c| c.replace(Some(surface)));
}

/// Build and show the card; `on_send` gets the draft when Send is pressed.
/// The preview harness calls this with a picture of its own.
pub fn card(
    app: &gtk4::Application,
    image: Option<Image>,
    on_send: impl Fn(Draft) + 'static,
) -> Surface {
    let surface = Surface::builder(app, Namespace::Report)
        .keyboard(gtk4_layer_shell::KeyboardMode::Exclusive)
        .card(ui::Card::Floating)
        .width(520)
        .build();
    let window = surface.window().clone();

    let content = ui::vbox(4);
    ui::pad(&content, 5);
    content.append(&ui::overline("Report a problem", Tone::Muted));

    // The picture, as big as the card's width allows and no taller than a
    // third of a screen.
    let picture = image.as_ref().map(|image| {
        let texture = crate::screenshot::deliver::texture(image);
        let pic = gtk4::Picture::for_paintable(&texture);
        pic.set_content_fit(gtk4::ContentFit::Contain);
        pic.set_can_shrink(true);
        let frame = ui::thumb();
        frame.set_halign(gtk4::Align::Fill);
        frame.set_size_request(-1, 220);
        pic.set_hexpand(true);
        frame.append(&pic);
        content.append(&frame);
        frame
    });

    let prompt = ui::text(
        "What went wrong, and what did you expect?",
        Text::Label,
        Tone::Muted,
    );
    prompt.set_xalign(0.0);
    content.append(&prompt);
    let (area, view) = ui::text_area(5);
    content.append(&area);
    if let Ok(seed) = std::env::var("SWAYPPLET_REPORT_TEXT") {
        view.buffer().set_text(&seed);
    }

    // A group, so the rows' own inset reads as a block of options.
    let switches = ui::group(0);
    let (shot_row, shot) = ui::switch_row(
        "Attach screenshot",
        if image.is_some() {
            "The picture above, as a PNG"
        } else {
            "No picture was taken"
        },
    );
    shot.set_active(image.is_some());
    shot.set_sensitive(image.is_some());
    switches.append(&shot_row.root);
    let (log_row, log) = ui::switch_row(
        "Attach recent log",
        &format!("The last {} lines this shell logged", super::LOG_LINES),
    );
    log.set_active(true);
    switches.append(&log_row.root);
    content.append(&switches);
    if let Some(frame) = &picture {
        let frame = frame.clone();
        shot.connect_active_notify(move |s| {
            frame.set_opacity(if s.is_active() { 1.0 } else { 0.4 })
        });
    }

    let note = ui::text(
        &format!(
            "Public: the issue, the picture and the log lines go to github.com/{}, where anyone can read them.",
            super::repo()
        ),
        Text::Caption,
        Tone::Warning,
    );
    note.set_wrap(true);
    note.set_max_width_chars(60);
    note.set_xalign(0.0);
    content.append(&note);

    let buttons = ui::hbox(3);
    buttons.set_halign(gtk4::Align::End);
    let cancel = ui::button("Cancel", Kind::Secondary);
    let send_b = ui::button("Send", Kind::Primary);
    buttons.append(&cancel);
    buttons.append(&send_b);
    content.append(&buttons);

    surface.card().append(&content);
    surface.set_content(&content);

    let image = Rc::new(image);
    let on_send = Rc::new(on_send);
    let submit = {
        let view = view.clone();
        let shot = shot.clone();
        let log = log.clone();
        let image = image.clone();
        let surface = surface.clone();
        move || {
            let buffer = view.buffer();
            let description = buffer
                .text(&buffer.start_iter(), &buffer.end_iter(), false)
                .to_string();
            let draft = Draft {
                description,
                image: if shot.is_active() {
                    (*image).clone()
                } else {
                    None
                },
                attach_log: log.is_active(),
            };
            surface.hide();
            on_send(draft);
        }
    };
    let submit = Rc::new(submit);
    {
        let submit = submit.clone();
        send_b.connect_clicked(move |_| submit());
    }
    {
        let surface = surface.clone();
        cancel.connect_clicked(move |_| surface.hide());
    }
    let keys = gtk4::EventControllerKey::new();
    // Capture, so Ctrl+Enter reaches the card before the text view takes
    // Enter as a new line.
    keys.set_propagation_phase(gtk4::PropagationPhase::Capture);
    {
        let surface = surface.clone();
        let submit = submit.clone();
        keys.connect_key_pressed(move |_, key, _, mods| {
            if key == gtk4::gdk::Key::Escape {
                surface.hide();
                return glib::Propagation::Stop;
            }
            let enter = key == gtk4::gdk::Key::Return || key == gtk4::gdk::Key::KP_Enter;
            if enter && mods.contains(gtk4::gdk::ModifierType::CONTROL_MASK) {
                submit();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
    }
    window.add_controller(keys);
    // Dropped once faded out: the surface's teardown (docs/design-system.md §6.1).
    surface.connect_hidden(|| {
        let surface = CARD.with(|c| c.borrow_mut().take());
        glib::idle_add_local_once(move || drop(surface));
    });

    surface.show();
    window.present();
    view.grab_focus();

    // Harness hook: the nested session has no keyboard, so
    // `SWAYPPLET_REPORT_SEND=1` presses Send after two seconds, down the
    // same path the button takes.
    if std::env::var("SWAYPPLET_REPORT_SEND").is_ok_and(|v| !v.is_empty()) {
        glib::timeout_add_local_once(std::time::Duration::from_secs(2), move || submit());
    }
    surface
}

/// File the draft: encode, upload, create, on a worker; notifications on
/// either side.
pub fn send(store: &StoreRef, draft: Draft) {
    let dry = super::dry_run();
    let pending = store_add(
        store,
        Notification {
            app_name: "Report".into(),
            timestamp: std::time::SystemTime::now(),
            summary: "Filing the report\u{2026}".into(),
            body: if dry {
                "Dry run: printing what would be sent".into()
            } else {
                format!("To github.com/{}", super::repo())
            },
            urgency: Urgency::Low,
            expire_timeout: 0,
            ..Default::default()
        },
    );
    let home = super::home();
    let log: Vec<String> = if draft.attach_log {
        super::logring::tail(super::LOG_LINES)
            .iter()
            .map(|l| super::redact(l, &home))
            .collect()
    } else {
        Vec::new()
    };
    let mode = match crate::theme::shown().mode {
        crate::tokens::Mode::Dark => "dark",
        crate::tokens::Mode::Light => "light",
    }
    .to_string();
    let Draft {
        description, image, ..
    } = draft;

    let store = store.clone();
    crate::spawn::spawn_work(
        move || file(description, image, log, mode),
        move |result| {
            let (summary, body, actions, url) = match result {
                Ok(url) => (
                    "Report filed".to_string(),
                    url.clone(),
                    vec![("open-issue".to_string(), "Open".to_string())],
                    Some(url),
                ),
                Err(e) => {
                    log::error!("report: {e}");
                    ("Report not filed".to_string(), e, Vec::new(), None)
                }
            };
            let id = store_add(
                &store,
                Notification {
                    app_name: "Report".into(),
                    timestamp: std::time::SystemTime::now(),
                    summary,
                    body,
                    actions,
                    replaces_id: pending,
                    urgency: Urgency::Normal,
                    expire_timeout: 10_000,
                    resident: true,
                    ..Default::default()
                },
            );
            if let Some(url) = url {
                LINKS.with(|l| {
                    let mut l = l.borrow_mut();
                    // A handful is all a history can still show a button for.
                    if l.len() > 8 {
                        l.clear();
                    }
                    l.insert(id, url);
                });
            }
        },
    );
}

/// The worker half of [`send`]: the issue's URL.
fn file(
    description: String,
    image: Option<Image>,
    log: Vec<String>,
    mode: String,
) -> Result<String, String> {
    let screenshot = match image {
        Some(image) => {
            let png = crate::screenshot::deliver::texture(&image).save_to_png_bytes();
            let path = format!("reports/{}.png", super::stamp());
            Some(super::gh::upload_asset(&path, &png)?)
        }
        None => None,
    };
    let report = Report {
        description,
        rev: super::rev().to_string(),
        outputs: outputs(),
        mode,
        log,
        screenshot,
    };
    super::gh::create_issue(
        &body::report_title(&report.description),
        &body::report_body(&report),
        &[super::REPORT],
    )
}

/// The outputs as sway reports them: fractional scale included, which GDK
/// 4.12 rounds.
fn outputs() -> Vec<Output> {
    let Ok(mut conn) = crate::sway::ipc::connect() else {
        return Vec::new();
    };
    conn.get_outputs()
        .map(|outs| {
            outs.into_iter()
                .filter(|o| o.active)
                .map(|o| Output {
                    name: o.name,
                    width: o.current_mode.map_or(o.rect.width, |m| m.width),
                    height: o.current_mode.map_or(o.rect.height, |m| m.height),
                    scale: o.scale.unwrap_or(1.0),
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_report_area_is_a_region() {
        assert_eq!(Area::parse(None), Area::Region);
        assert_eq!(Area::parse(Some("window")), Area::Window);
        assert_eq!(Area::parse(Some("screen")), Area::Screen);
        assert_eq!(Area::parse(Some("none")), Area::None);
    }
}
