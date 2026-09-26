//! One popup card's content: its header, body, pictures, reply field and
//! actions, the gestures on it, and where a click on it goes.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;

use super::menu::card_menu;
use super::stack::{CARD_WIDTH, State, reflow};
use super::timers::{pause_timers, resume_timers};
use crate::anim;
use crate::services::notifications::group::describe;
use crate::services::notifications::store::{self, NotificationStore};
use crate::services::notifications::{CloseReason, ImageSource, Notification, Urgency};

/// The action key a sender uses to ask for a reply field (KDE's convention,
/// which is what the `inline-reply` capability promises).
const INLINE_REPLY_KEY: &str = "inline-reply";

// Drag-to-dismiss: how far before the gesture takes the event sequence off
// the click handler, and how far before releasing means "gone".
const DRAG_CLAIM_PX: f64 = 8.0;
const DRAG_DISMISS_PX: f64 = 72.0;

/// How many sender colours there are: the categorical slots of the design
/// system (`--cat-1` … `--cat-6`, `ui::rail` and `ui::set_category`), none of
/// them the red the URGENT chip owns.
pub const ACCENTS: u64 = crate::ui::CATEGORIES as u64;

/// Which accent a sender gets, 1 to [`ACCENTS`], from its name.
///
/// The point is that two senders look different and one sender looks the
/// same every time, including across restarts — so this is a written-out
/// FNV-1a rather than `DefaultHasher`, whose value is a std implementation
/// detail and not something a colour should depend on.
///
/// Case-folded, because "Backup" and "BACKUP" are one sender wearing two
/// hats, and the header prints the name uppercased anyway.
fn accent_for(app: &str) -> u8 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in app.to_lowercase().bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    (hash % ACCENTS) as u8 + 1
}

/// Icon size in the card's leading slot.
const ICON_PX: i32 = 24;
/// Thumbnail size when the notification carries its own picture.
const THUMB_PX: i32 = 40;
/// A picture at least this wide, and at least this much wider than it is
/// tall, is a screenshot or a banner rather than an avatar, so it goes under
/// the text at full card width instead of into the leading slot.
const WIDE_MIN_PX: i32 = 192;
const WIDE_MIN_ASPECT: f64 = 1.6;

/// Build a paintable widget for an image source, or `None` when it cannot be
/// resolved — a missing file or an icon name no theme has. Callers fall back
/// rather than showing a broken-image placeholder, because a card with no
/// picture reads better than a card with a question mark in it.
fn image_widget(src: &ImageSource, px: i32) -> Option<gtk4::Widget> {
    use gtk4::prelude::*;
    let image = match src {
        ImageSource::Named(name) => {
            let display = gtk4::gdk::Display::default()?;
            let theme = gtk4::IconTheme::for_display(&display);
            if !theme.has_icon(name) {
                return None;
            }
            gtk4::Image::from_icon_name(name)
        }
        ImageSource::Path(path) => {
            if !path.is_file() {
                return None;
            }
            gtk4::Image::from_file(path)
        }
        ImageSource::Data(d) => {
            let bytes = gtk4::glib::Bytes::from(&d.data);
            let pixbuf = gtk4::gdk_pixbuf::Pixbuf::from_bytes(
                &bytes,
                gtk4::gdk_pixbuf::Colorspace::Rgb,
                d.has_alpha,
                d.bits_per_sample,
                d.width,
                d.height,
                d.rowstride,
            );
            let texture = gtk4::gdk::Texture::for_pixbuf(&pixbuf);
            gtk4::Image::from_paintable(Some(&texture))
        }
    };
    image.set_pixel_size(px);
    Some(image.upcast())
}

/// Whether a picture should be laid out as a wide banner. Only the geometry
/// decides, and for files it is read from the header rather than by decoding
/// the whole image.
fn is_wide_picture(src: &ImageSource) -> bool {
    let (w, h) = match src {
        ImageSource::Data(d) => (d.width, d.height),
        ImageSource::Path(path) => match gtk4::gdk_pixbuf::Pixbuf::file_info(path) {
            Some((_, w, h)) => (w, h),
            None => return false,
        },
        // A themed name is an icon by definition.
        ImageSource::Named(_) => return false,
    };
    if h <= 0 {
        return false;
    }
    w >= WIDE_MIN_PX && (w as f64 / h as f64) >= WIDE_MIN_ASPECT
}

/// How old a notification reads on the card. Short forms, because this sits
/// in a header beside the app name and competes with it for width.
///
/// Anything under a minute is "now": a card that just arrived does not need
/// its age, and a per-second countdown would be a loop the bar's cadence
/// budget (P7) does not pay for.
pub(super) fn age_label(ts: std::time::SystemTime, now: std::time::SystemTime) -> Option<String> {
    let secs = now.duration_since(ts).ok()?.as_secs();
    Some(match secs {
        0..=59 => return None,
        60..=3599 => format!("{}m", secs / 60),
        3600..=86_399 => format!("{}h", secs / 3600),
        _ => format!("{}d", secs / 86_400),
    })
}

/// A body long enough that the 3-line cap will hide part of it, so the card
/// earns an expander. Counted on the text rather than measured after layout:
/// the answer is needed while building the card, before it has an allocation.
fn body_is_truncated(body: &str) -> bool {
    body.lines().count() > 3 || body.chars().count() > 140
}

/// Critical, in the material instead of around it. What this replaced was a
/// 2 px red border pulsing at 0.7 s over squared-off corners: an error
/// dialog's language, drawn on top of the glass rather than in it, and the
/// one thing on screen shouting while the rest of the panel stays calm. The
/// urgency is three quiet channels now: the glass itself runs warm, its
/// hairline picks up the same red, and the header carries the word. The
/// card's radius stays, because a corner only reads as "squarer" beside a
/// card that isn't, and the shape channel is the chip, which needs nothing
/// to compare itself against.
pub(super) fn set_critical_class(card: &gtk4::Box, notif: &Notification) {
    crate::ui::set_card_tint(
        card,
        crate::ui::CardTint::Danger,
        notif.urgency == Urgency::Critical,
    );
}

/// Categories whose notifications are status reports rather than messages:
/// nothing to read, nothing to act on, and a full card around three words
/// reads as an empty box. Prefixes, because the spec's categories are
/// dotted and senders extend them (`device.added`, `device.removed`).
const COMPACT_CATEGORIES: &[&str] = &["device", "x-swaypplet.osd"];

/// Whether a notification puts a text field on its card.
pub(super) fn wants_keyboard(notif: &Notification) -> bool {
    notif.actions.iter().any(|(key, _)| key == INLINE_REPLY_KEY)
}

/// Whether a notification wants the one-line template.
///
/// Two ways in. A transient toast with nothing to read or press is one by
/// construction — volume, brightness and this machine's own "nothing to jump
/// to" all land here. A category can also say so outright, which catches the
/// senders that describe what they are without setting `transient`.
fn is_compact(notif: &Notification) -> bool {
    if !notif.actions.is_empty() || !notif.body.is_empty() {
        return false;
    }
    if notif.transient {
        return true;
    }
    notif.category.as_deref().is_some_and(|c| {
        COMPACT_CATEGORIES
            .iter()
            .any(|p| c == *p || c.starts_with(&format!("{p}.")))
    })
}

/// (Re)build a card's content. The card box itself persists across
/// `replaces_id` updates (keeping its z-order and transform); everything
/// inside — including gesture handlers — is rebuilt with fresh store refs.
///
/// `earlier` is the rest of the sender's group behind this card, newest
/// first (`services::notifications::group`): counted in the header and
/// listed under a disclosure. Empty for a card that stands alone.
///
/// Returns the age label, if the card has one, so the stack can retitle it on
/// the minute tick without rebuilding the card underneath the pointer.
pub(super) fn populate_card(
    card: &gtk4::Box,
    notif: &Notification,
    store: &Rc<RefCell<NotificationStore>>,
    st: &Rc<RefCell<State>>,
    earlier: &[Notification],
) -> Option<gtk4::Label> {
    while let Some(child) = card.first_child() {
        card.remove(&child);
    }

    let compact = is_compact(notif);
    if compact {
        card.add_css_class("compact");
    } else {
        card.remove_css_class("compact");
    }

    let hbox = crate::ui::hbox(3);

    // A picture wide enough to be a screenshot or a banner goes under the
    // text at full width; anything else is a thumbnail in the leading slot,
    // which is what a chat avatar or an app icon wants to be.
    let wide = notif.image.as_ref().filter(|i| is_wide_picture(i));
    if let Some(widget) = leading_picture(notif, wide) {
        hbox.append(&widget);
    }

    // Text content
    let vbox = crate::ui::vbox(1);
    vbox.set_hexpand(true);
    vbox.set_valign(gtk4::Align::Center);

    // max_width_chars(1) collapses each label's natural width so the card's
    // CARD_WIDTH size request is what drives allocation — a larger cap
    // becomes the natural width and can push the card past the window (see
    // reflow). Fill + xalign(0) makes the label span that allocation and
    // ellipsize/wrap there instead of shrinking to the collapsed natural.
    let count = earlier.len() + 1;
    let age_label_handle = header(notif, count).map(|(header, age)| {
        vbox.append(&header);
        age
    });

    let summary = crate::ui::text(&notif.summary, crate::ui::Text::Title, crate::ui::Tone::Fg);
    summary.set_halign(gtk4::Align::Fill);
    summary.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    summary.set_max_width_chars(1);
    vbox.append(&summary);

    body_text(&vbox, notif, st);

    if let Some(widget) = wide.and_then(|src| image_widget(src, CARD_WIDTH)) {
        let frame = crate::ui::thumb();
        frame.append(&widget);
        frame.add_css_class("notification-picture");
        frame.set_halign(gtk4::Align::Fill);
        vbox.append(&frame);
    }

    // Progress bar
    if let Some(progress) = notif.progress {
        let bar = crate::ui::progress(progress as f64 / 100.0);
        bar.set_hexpand(true);
        bar.add_css_class("notification-progress");
        crate::ui::set_breathing(&bar, true);
        vbox.append(&bar);
    }

    if let Some(entry) = reply_field(notif, store, st) {
        vbox.append(&entry);
    }

    if let Some(actions) = action_buttons(notif, store) {
        vbox.append(&actions);
    }

    if let Some(list) = earlier_list(earlier, st) {
        vbox.append(&list);
    }

    hbox.append(&vbox);

    if !compact {
        hbox.append(&menu_button(notif, store, st));
    }

    hbox.add_controller(drag_to_dismiss(notif, store, st));

    let gesture = click_gesture(notif, store);
    // The rail and the content, side by side. The gesture rides the row
    // rather than the content, so a click on the rail is a click on the card
    // like any other.
    //
    // The rail is the sender's colour: which slot is a hash of the app name
    // (`accent_for`), so senders look different and one sender looks the
    // same every time. Urgency does not touch the rail: that channel is the
    // warm glass and the URGENT chip, and a rail that meant severity on some
    // cards and sender on others would mean neither.
    let row = crate::ui::hbox(3);
    let rail = crate::ui::rail(
        (!notif.app_name.is_empty()).then(|| usize::from(accent_for(&notif.app_name))),
    );
    hbox.set_hexpand(true);
    row.append(&rail);
    row.append(&hbox);
    row.add_controller(gesture);

    card.append(&row);
    // A screen reader hears the count with the card, not only a number in
    // its header.
    let label = match describe(count, &notif.app_name) {
        Some(group) => format!("{group}. Newest: {}", notif.summary),
        None => notif.summary.clone(),
    };
    card.update_property(&[gtk4::accessible::Property::Label(&label)]);
    age_label_handle
}

/// The sender's earlier notifications, under a disclosure: a line each,
/// newest first. The card grows when it opens, so the stack reflows once the
/// reveal has finished.
fn earlier_list(earlier: &[Notification], st: &Rc<RefCell<State>>) -> Option<gtk4::Widget> {
    if earlier.is_empty() {
        return None;
    }
    let d = crate::ui::disclosure(&format!("{} earlier", earlier.len()));
    let now = std::time::SystemTime::now();
    for n in earlier {
        let line = crate::ui::hbox(2);
        let summary = crate::ui::text(&n.summary, crate::ui::Text::Body, crate::ui::Tone::Muted);
        summary.set_halign(gtk4::Align::Fill);
        summary.set_hexpand(true);
        summary.set_xalign(0.0);
        summary.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        summary.set_max_width_chars(1);
        line.append(&summary);
        let age = crate::ui::text(
            &age_label(n.timestamp, now).unwrap_or_default(),
            crate::ui::Text::Caption,
            crate::ui::Tone::Faint,
        );
        line.append(&age);
        d.body.append(&line);
    }
    let st = st.clone();
    d.revealer.connect_child_revealed_notify(move |_| reflow(&st));
    Some(d.root.upcast())
}

/// The picture in the card's leading slot: the notification's own picture
/// as a framed thumbnail, or its app icon. `wide` is the picture that goes
/// under the text instead, when there is one.
fn leading_picture(notif: &Notification, wide: Option<&ImageSource>) -> Option<gtk4::Widget> {
    let leading = if wide.is_some() {
        notif.icon.as_ref()
    } else {
        notif.image.as_ref().or(notif.icon.as_ref())
    };
    let leading_px = if wide.is_none() && notif.image.is_some() {
        THUMB_PX
    } else {
        ICON_PX
    };
    let widget = leading.and_then(|src| image_widget(src, leading_px))?;
    // A picture the notification supplied is content, so it gets the
    // treatment content gets: a rounded, clipped frame. An app icon is
    // not.
    let widget = if notif.image.is_some() && wide.is_none() {
        let frame = crate::ui::thumb();
        frame.append(&widget);
        frame.upcast()
    } else {
        widget
    };
    widget.set_valign(gtk4::Align::Start);
    Some(widget)
}

/// The header row, and the age label in it. Task attribution (vision O2 —
/// "T<N>" says whose background session this is) ahead of the app name, with
/// the card's age closing it out on the right. One label carries both
/// channels: the number is the shape, its accent the hue (P3), so a separate
/// dot would only repeat it. `None` when the card has nothing to put in a
/// header, and so no header.
fn header(notif: &Notification, count: usize) -> Option<(gtk4::Box, gtk4::Label)> {
    let header = crate::ui::hbox(2);
    // Urgency leads the header, because a critical card is the one card that
    // never expires (timeout_for) and the word is what says so. It used to be
    // a red ring around the whole card instead, which is an error dialog's
    // language rather than this panel's and sat on top of the material rather
    // than in it. The chip carries both channels on one label the way the
    // task number does: the fill is the shape, its red the hue (P3), and the
    // word survives a grayscale filter on its own. Still, not pulsing: a chip
    // that says "URGENT" in the header does not need motion to be found, and
    // it costs nothing to leave on screen for as long as a card that never
    // expires stays there, which an infinite animation does (P7).
    if notif.urgency == Urgency::Critical {
        let urgent = crate::ui::badge("URGENT", crate::ui::BadgeTone::Alert);
        crate::ui::overline::adopt(&urgent);
        urgent.set_valign(gtk4::Align::Center);
        header.append(&urgent);
    }
    // Task attribution: the number is the shape channel and its categorical
    // colour the hue (P3), so one label carries both and a separate dot would
    // only repeat it. Tasks 1–4 take the first four slots, as the bar does.
    if let Some(task) = notif.task {
        let num = crate::ui::overline(&format!("T{task}"), crate::ui::Tone::Fg);
        if (1..=4).contains(&task) {
            crate::ui::set_category(&num, usize::from(task));
        }
        header.append(&num);
    }
    // The app name carries the rail's colour, so the header says who sent it
    // in the same channel the edge does.
    if !notif.app_name.is_empty() {
        let app_label = crate::ui::overline(&notif.app_name, crate::ui::Tone::Fg);
        app_label.set_halign(gtk4::Align::Fill);
        app_label.set_hexpand(true);
        app_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        app_label.set_max_width_chars(1);
        crate::ui::set_category(&app_label, usize::from(accent_for(&notif.app_name)));
        header.append(&app_label);
    }

    // How many notifications this sender's card stands for. Says "there is
    // more of this" without spending a slot per copy.
    if let Some(words) = describe(count, &notif.app_name) {
        let badge = crate::ui::badge(&count.to_string(), crate::ui::BadgeTone::Neutral);
        badge.set_valign(gtk4::Align::Center);
        badge.set_tooltip_text(Some(&words));
        header.append(&badge);
    }

    // Age. Built for every card that has a header, empty until the card is a
    // minute old, so the minute tick has something to write into.
    // Metadata, closing the header opposite the app name, never competing
    // with the summary.
    let age = crate::ui::text(
        &age_label(notif.timestamp, std::time::SystemTime::now()).unwrap_or_default(),
        crate::ui::Text::Caption,
        crate::ui::Tone::Faint,
    );
    age.set_halign(gtk4::Align::End);
    if header.first_child().is_some() {
        header.append(&age);
        Some((header, age))
    } else {
        None
    }
}

/// The body text, capped at three lines, and the button that lifts the cap
/// when the body is long enough to need it.
fn body_text(vbox: &gtk4::Box, notif: &Notification, st: &Rc<RefCell<State>>) {
    if !notif.body.is_empty() {
        let markup = super::markup::sanitize(&notif.body);
        let body = gtk4::Label::builder()
            .label(&markup)
            .use_markup(true)
            .halign(gtk4::Align::Fill)
            .xalign(0.0)
            .wrap(true)
            .wrap_mode(gtk4::pango::WrapMode::WordChar)
            .max_width_chars(1)
            .lines(3)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .build();
        // The body is what the card is for, so it reads at full ink: muted
        // text over a bright backdrop is a card you skip rather than read.
        crate::ui::set_text_style(&body, crate::ui::Text::Body, crate::ui::Tone::Fg);
        vbox.append(&body);

        // The rest of a long body is one click away rather than only in the
        // centre. A button, not a hover reveal (P8).
        if body_is_truncated(&notif.body) {
            let more = crate::ui::button_with(
                crate::ui::Face::Label("more"),
                crate::ui::Kind::Flat,
                crate::ui::Size::Small,
            );
            more.set_halign(gtk4::Align::Start);
            let body_c = body.clone();
            let st_c = st.clone();
            more.connect_clicked(move |btn| {
                let expanded = body_c.lines() < 0;
                // Both properties, together, or the button does the opposite
                // of what it says. The line cap only holds while the label
                // can ellipsize, and an ellipsizing label with no cap does
                // not wrap at all: `lines(-1)` on its own left the whole body
                // on one ellipsized line, so "more" showed *less* than the
                // three lines it replaced (55 px of body down to 19).
                let (lines, ellipsize) = if expanded {
                    (3, gtk4::pango::EllipsizeMode::End)
                } else {
                    (-1, gtk4::pango::EllipsizeMode::None)
                };
                body_c.set_lines(lines);
                body_c.set_ellipsize(ellipsize);
                btn.set_label(if expanded { "more" } else { "less" });
                // The card's height changed, so the stack has to re-measure
                // and re-stack around it.
                reflow(&st_c);
            });
            vbox.append(&more);
        }
    }
}

/// The inline reply field. The sender offers an `inline-reply` action whose
/// label is the placeholder; the text comes back on `NotificationReplied`
/// rather than as an action, because it is content, not a button press.
fn reply_field(
    notif: &Notification,
    store: &Rc<RefCell<NotificationStore>>,
    st: &Rc<RefCell<State>>,
) -> Option<gtk4::Entry> {
    let (_, placeholder) = notif
        .actions
        .iter()
        .find(|(key, _)| key == INLINE_REPLY_KEY)?;
    let entry = gtk4::Entry::builder()
        .placeholder_text(placeholder)
        .hexpand(true)
        .build();
    crate::ui::entry::adopt(&entry, crate::ui::FieldSize::Normal);
    entry.add_css_class("notification-reply");
    let id = notif.id;
    let store_c = store.clone();
    let resident = notif.resident;
    entry.connect_activate(move |e| {
        let text = e.text();
        if text.is_empty() {
            return;
        }
        store::store_reply(&store_c, id, &text);
        e.set_text("");
        if !resident {
            store::store_close(&store_c, id, CloseReason::Dismissed);
        }
    });
    // The surface is already on-demand by the time a card with this field
    // is on screen (see sync_keyboard_mode); the click only has to move
    // GTK's own focus onto the entry.
    let click = gtk4::GestureClick::new();
    let entry_c = entry.clone();
    click.connect_pressed(move |_, _, _, _| {
        entry_c.grab_focus();
    });
    entry.add_controller(click);

    // Escape hands the keyboard straight back, so the surface can never
    // be left holding focus with no obvious way out.
    let keys = gtk4::EventControllerKey::new();
    let entry_c = entry.clone();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key == gtk4::gdk::Key::Escape {
            // Give the keyboard back by dropping the field's focus. The
            // surface stays on-demand until the card itself goes, which
            // costs nothing: on-demand never takes focus unasked.
            entry_c.set_text("");
            if let Some(root) = entry_c.root() {
                root.set_focus(None::<&gtk4::Widget>);
            }
            return gtk4::glib::Propagation::Stop;
        }
        gtk4::glib::Propagation::Proceed
    });
    entry.add_controller(keys);

    // Typing must not race the auto-dismiss timer out from under the
    // sentence being written.
    let st_c = st.clone();
    let focus = gtk4::EventControllerFocus::new();
    focus.connect_enter(move |_| pause_timers(&st_c));
    let st_c = st.clone();
    focus.connect_leave(move |_| resume_timers(&st_c));
    entry.add_controller(focus);
    Some(entry)
}

/// The action buttons, one per action the sender offers other than the
/// default (the card itself) and the inline reply (its own field). `None`
/// when the sender offers none.
fn action_buttons(
    notif: &Notification,
    store: &Rc<RefCell<NotificationStore>>,
) -> Option<gtk4::Box> {
    if notif.actions.is_empty() {
        return None;
    }
    let actions_box = crate::ui::hbox(2);
    actions_box.add_css_class("notification-actions");

    for (key, label) in &notif.actions {
        if key == "default" {
            continue; // default action is handled by clicking the popup body
        }
        if key == INLINE_REPLY_KEY {
            continue; // rendered as the text field above
        }
        let btn = gtk4::Button::builder().build();
        // With `action-icons` the key names an icon rather than being an
        // opaque token, and the label is the tooltip instead.
        if notif.action_icons
            && let Some(display) = gtk4::gdk::Display::default()
            && gtk4::IconTheme::for_display(&display).has_icon(key)
        {
            btn.set_child(Some(&gtk4::Image::from_icon_name(key)));
            btn.set_tooltip_text(Some(label));
        } else {
            btn.set_label(label);
        }
        crate::ui::button::adopt(&btn, crate::ui::Kind::Secondary, crate::ui::Size::Small);

        let id = notif.id;
        let store_c = store.clone();
        let key_c = key.clone();
        // `resident` is the spec's way of saying an action does not
        // dismiss. Without honouring it every button is terminal, so a
        // "snooze" or a "mark read" could never be offered by a sender.
        let resident = notif.resident;
        btn.connect_clicked(move |_| {
            log::info!("Action invoked: notification {id}, action {key_c}");
            store::store_action_invoked(&store_c, id, &key_c);
            if !resident {
                store::store_close(&store_c, id, CloseReason::Dismissed);
            }
        });
        actions_box.append(&btn);
    }

    Some(actions_box)
}

/// Snooze and mute live behind one visible button rather than only behind
/// a right-click, so they exist for someone who never dwells (P8). It is
/// the card's only trailing control: closing has three ways in already
/// (right-click, middle-click, drag) and none of them costs a widget.
fn menu_button(
    notif: &Notification,
    store: &Rc<RefCell<NotificationStore>>,
    st: &Rc<RefCell<State>>,
) -> gtk4::Button {
    let more_btn = crate::ui::button_with(
        crate::ui::Face::Glyph {
            glyph: "⋯",
            tooltip: "Snooze, mute, dismiss all",
        },
        crate::ui::Kind::Flat,
        crate::ui::Size::Small,
    );
    more_btn.set_valign(gtk4::Align::Start);
    let menu = card_menu(notif, store, st);
    menu.set_parent(&more_btn);
    // GTK4 hands a popover's parent no ownership, so one that outlives
    // its button is a leak the toolkit complains about by name
    // ("Finalizing GtkButton, but it still has children left"). Cards are
    // rebuilt on every replaces_id update, so this is per notification.
    let menu_c = menu.clone();
    more_btn.connect_destroy(move |_| menu_c.unparent());
    more_btn.connect_clicked(move |_| menu.popup());
    more_btn
}

/// Drag the card aside to dismiss it. Once the pointer has clearly
/// committed, the drag claims the event sequence so the click gesture
/// below never fires — otherwise flicking a card away would also focus
/// the app it came from.
fn drag_to_dismiss(
    notif: &Notification,
    store: &Rc<RefCell<NotificationStore>>,
    st: &Rc<RefCell<State>>,
) -> gtk4::GestureDrag {
    let drag = gtk4::GestureDrag::new();
    {
        let st_c = st.clone();
        let id = notif.id;
        drag.connect_drag_update(move |g, dx, _| {
            if dx.abs() > DRAG_CLAIM_PX {
                g.set_state(gtk4::EventSequenceState::Claimed);
            }
            // Only outward: dragging a right-anchored card leftward would
            // pull it across the screen rather than off it.
            let dx = dx.max(0.0);
            // Fade with distance, so the card reads as leaving rather than
            // sliding. The surface carries both, so the frost goes with it.
            let alpha = (1.0 - dx / (DRAG_DISMISS_PX * 1.6)).clamp(0.0, 1.0);
            if let Some(card) = st_c
                .borrow()
                .cards
                .iter()
                .find(|c| c.id == id && !c.exiting)
            {
                card.surface.drag_to(dx, alpha);
            }
        });
    }
    {
        let st_c = st.clone();
        let store_c = store.clone();
        let id = notif.id;
        drag.connect_drag_end(move |_, dx, _| {
            if dx >= DRAG_DISMISS_PX {
                store::store_close(&store_c, id, CloseReason::Dismissed);
            } else if let Some(card) = st_c
                .borrow()
                .cards
                .iter()
                .find(|c| c.id == id && !c.exiting)
            {
                card.surface.settle_back(anim::duration(anim::MOVE_MS));
            }
        });
    }
    drag
}

/// Left click goes where the notification came from; right click makes it
/// go away. Middle click keeps doing what right click now does, because
/// the muscle memory costs nothing to honour.
fn click_gesture(
    notif: &Notification,
    store: &Rc<RefCell<NotificationStore>>,
) -> gtk4::GestureClick {
    let gesture = gtk4::GestureClick::new();
    gesture.set_button(0);
    let id = notif.id;
    let store_c = store.clone();
    let names = Rc::new(window_names(notif));
    let claude_pid = notif.claude_pid;
    let has_default = notif.actions.iter().any(|(key, _)| key == "default");
    let resident = notif.resident;
    gesture.connect_released(move |g, _, _, _| match g.current_button() {
        gtk4::gdk::BUTTON_MIDDLE | gtk4::gdk::BUTTON_SECONDARY => {
            store::store_close(&store_c, id, CloseReason::Dismissed);
        }
        gtk4::gdk::BUTTON_PRIMARY => {
            let store_c = store_c.clone();
            crate::sway::tree::focus_source(
                names.to_vec(),
                claude_pid,
                crate::services::task_state::parent_pid,
                move |focused| {
                    // Nowhere to jump: the sender's own default action is the
                    // next best answer to "take me to this", and dismissing is
                    // the answer when it offered none.
                    if !focused && has_default {
                        log::info!("Action invoked: notification {id}, action default");
                        store::store_action_invoked(&store_c, id, "default");
                        if resident {
                            return;
                        }
                    }
                    store::store_close(&store_c, id, CloseReason::Dismissed);
                },
            );
        }
        _ => {}
    });
    gesture
}

/// What the notification says about itself that a window could be named
/// after, best evidence first.
///
/// The `desktop-entry` hint is the sender naming its own `.desktop` file,
/// which for a Wayland client is the same string sway reports as `app_id`.
/// `app_name` is free text but usually the program. A themed icon name is
/// what is left when a sender sets neither.
fn window_names(notif: &Notification) -> Vec<String> {
    let icon = match &notif.icon {
        Some(ImageSource::Named(name)) => Some(name.clone()),
        _ => None,
    };
    let mut names: Vec<String> = Vec::new();
    for name in [
        notif.desktop_entry.clone(),
        Some(notif.app_name.clone()),
        icon,
    ]
    .into_iter()
    .flatten()
    {
        let name = name.trim().trim_end_matches(".desktop").to_lowercase();
        if !name.is_empty() && !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_sender_keeps_one_accent_and_senders_spread_across_them() {
        // Stable: the same name is the same colour every run, which is the
        // whole reason this is not DefaultHasher.
        assert_eq!(accent_for("Backup"), accent_for("Backup"));
        // Case is not a second sender, and the header uppercases anyway.
        assert_eq!(accent_for("Backup"), accent_for("BACKUP"));
        assert_eq!(accent_for("Möte"), accent_for("möte"));

        for app in ["", "Chat", "Backup", "Kalender", "Disk", "Firefox"] {
            let a = accent_for(app);
            assert!((1..=ACCENTS as u8).contains(&a), "{app} got {a}");
        }

        // Every accent is reachable, so none of the six is dead CSS.
        let mut seen: Vec<u8> = (0..400).map(|i| accent_for(&format!("app{i}"))).collect();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen, (1..=ACCENTS as u8).collect::<Vec<u8>>());
    }

    #[test]
    fn age_reads_as_metadata_not_a_countdown() {
        let base = std::time::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let at = |secs| age_label(base, base + Duration::from_secs(secs));
        // Under a minute has nothing worth saying, and saying it every second
        // would be a loop the cadence budget does not pay for.
        assert_eq!(at(0), None);
        assert_eq!(at(59), None);
        assert_eq!(at(60).as_deref(), Some("1m"));
        assert_eq!(at(59 * 60).as_deref(), Some("59m"));
        assert_eq!(at(3600).as_deref(), Some("1h"));
        assert_eq!(at(86_400).as_deref(), Some("1d"));
        // A clock that went backwards is not an age.
        assert_eq!(age_label(base, base - Duration::from_secs(10)), None);
    }

    #[test]
    fn only_a_long_body_earns_an_expander() {
        assert!(!body_is_truncated("one line"));
        assert!(!body_is_truncated("a\nb\nc"));
        assert!(body_is_truncated("a\nb\nc\nd"));
        assert!(body_is_truncated(&"x".repeat(141)));
        assert!(!body_is_truncated(&"x".repeat(140)));
    }

    #[test]
    fn the_compact_template_is_for_toasts_with_nothing_to_act_on() {
        let toast = Notification {
            transient: true,
            summary: "Volume 40%".into(),
            ..Default::default()
        };
        assert!(is_compact(&toast));

        // A body or an action means there is something to read or press, and
        // the one-line template has room for neither.
        assert!(!is_compact(&Notification {
            body: "something".into(),
            ..toast.clone()
        }));
        assert!(!is_compact(&Notification {
            actions: vec![("ok".into(), "OK".into())],
            ..toast.clone()
        }));
        // Anything kept in history is not a toast...
        assert!(!is_compact(&Notification {
            transient: false,
            ..toast.clone()
        }));
        // ...unless it says what it is.
        assert!(is_compact(&Notification {
            transient: false,
            category: Some("device.added".into()),
            ..toast.clone()
        }));
        assert!(is_compact(&Notification {
            transient: false,
            category: Some("device".into()),
            ..toast.clone()
        }));
        // A prefix match must not swallow an unrelated category that merely
        // starts with the same letters.
        assert!(!is_compact(&Notification {
            transient: false,
            category: Some("devicemanager.thing".into()),
            ..toast
        }));
    }

    #[test]
    fn the_names_a_card_offers_are_ordered_by_how_much_they_are_worth() {
        let notif = Notification {
            app_name: "Fractal".into(),
            desktop_entry: Some("org.gnome.Fractal.desktop".into()),
            icon: Some(ImageSource::Named("fractal".into())),
            ..Default::default()
        };
        // The hint first, lowercased and stripped of its suffix; the icon
        // name is already the entry's tail, so it is not repeated.
        assert_eq!(window_names(&notif), ["org.gnome.fractal", "fractal"]);
        // A sender that says nothing about itself gets no lookup at all.
        assert!(window_names(&Notification::default()).is_empty());
    }
}
