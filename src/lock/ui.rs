//! Lock screen surfaces — one per monitor, all mirroring the same state.
//!
//! `SurfaceSet` owns the per-monitor widget handles and broadcasts every
//! state change (status text, verifying, shake, fingerprint pill) to all of
//! them, so whichever screen the user looks at tells the same story. Windows
//! deregister themselves on destroy (the compositor unmaps/destroys lock
//! surfaces when a monitor is unplugged or the session unlocks).

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk4::prelude::*;

use crate::avatar::avatar;
use crate::switch_user;

/// Data for one greeter user chip. Sourced from `SWAYPPLET_SWITCH_USER_CMD
/// --list` when available (avatar + presence), otherwise just a name.
#[derive(Clone)]
pub struct UserChip {
    pub user: String,
    pub logged_in: bool,
    pub icon: Option<String>,
}

impl UserChip {
    /// A bare-name chip (no avatar image, no presence) for the
    /// SWAYPPLET_GREET_USERS fallback.
    pub fn plain(user: &str) -> Self {
        Self {
            user: user.to_string(),
            logged_in: false,
            icon: None,
        }
    }
}

/// Avatar diameter for lock/greeter chips.
const CHIP_AVATAR_SIZE: i32 = 36;

/// Runs `SWAYPPLET_LOCK_WAKE_CMD` (throttled) on any key or pointer activity.
/// The lock script blanks outputs after locking, and swayidle resume events
/// only fire for timeouts that already expired — right after a manual lock
/// none has, so without this the first keypress can't re-power the screen.
struct WakeCmd {
    cmd: Option<String>,
    last: Cell<Option<Instant>>,
}

impl Default for WakeCmd {
    fn default() -> Self {
        Self {
            cmd: std::env::var("SWAYPPLET_LOCK_WAKE_CMD")
                .ok()
                .filter(|c| !c.is_empty()),
            last: Cell::new(None),
        }
    }
}

impl WakeCmd {
    fn poke(&self) {
        let Some(cmd) = &self.cmd else { return };
        if self
            .last
            .get()
            .is_some_and(|t| t.elapsed() < Duration::from_secs(2))
        {
            return;
        }
        self.last.set(Some(Instant::now()));
        let cmd = cmd.clone();
        crate::spawn::spawn_work(
            move || std::process::Command::new("sh").args(["-c", &cmd]).status(),
            |result| {
                if !matches!(&result, Ok(s) if s.success()) {
                    log::warn!("wake command failed: {result:?}");
                }
            },
        );
    }
}

pub enum StatusKind {
    Info,
    Error,
}

struct Surface {
    window: gtk4::Window,
    card: gtk4::Box,
    user_entry: Option<gtk4::Entry>,
    /// Greeter chip row container, kept so `set_greet_chips` can refill it
    /// once the async `--list` (avatars, presence, enrollment) resolves.
    chip_row: Option<gtk4::Box>,
    user_chips: Vec<(String, gtk4::Button)>,
    entry: gtk4::PasswordEntry,
    status: gtk4::Label,
    fp_pill: gtk4::Box,
    fp_label: gtk4::Label,
    caps: gtk4::Label,
    clock: gtk4::Label,
    date: gtk4::Label,
}

#[derive(Clone, Default)]
pub struct SurfaceSet {
    inner: Rc<RefCell<Vec<Surface>>>,
    wake: Rc<WakeCmd>,
    /// `Some(prefill)` adds an editable username row above the password
    /// entry (greeter mode); `None` is the lock's implicit current user.
    user_field: Rc<RefCell<Option<String>>>,
    /// Greeter mode: known users rendered as clickable chips above the
    /// username row (only when more than one).
    users: Rc<RefCell<Vec<UserChip>>>,
    on_user_select: Rc<RefCell<Option<Rc<dyn Fn(String)>>>>,
}

impl SurfaceSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a lock window for one monitor and register it in the set.
    /// `on_submit` receives the entry text on Enter.
    pub fn build_surface(&self, on_submit: Rc<dyn Fn(String)>) -> gtk4::Window {
        let window = gtk4::Window::new();
        window.add_css_class("lock");
        let content = self.build_content(&window, on_submit);
        window.set_child(Some(&content));
        window
    }

    /// The full-screen lock content. Public so `--preview lock` can host it
    /// in a plain window for visual iteration.
    pub fn build_content(
        &self,
        window: &gtk4::Window,
        on_submit: Rc<dyn Fn(String)>,
    ) -> gtk4::Widget {
        let overlay = gtk4::Overlay::new();

        // Wallpaper (optional, crisp) + scrim for contrast; solid palette bg
        // otherwise. Only the card region reads frosted — the GlassPane
        // around the card re-draws this texture blurred behind it (swayfx
        // blur covers neither ext-session-lock surfaces nor own content).
        let backdrop = gtk4::Box::builder().hexpand(true).vexpand(true).build();
        backdrop.add_css_class("lock-backdrop");
        let wallpaper = wallpaper_path().and_then(|p| gdk4::Texture::from_filename(p).ok());
        if let Some(ref texture) = wallpaper {
            let picture = gtk4::Picture::for_paintable(texture);
            picture.set_content_fit(gtk4::ContentFit::Cover);
            picture.set_hexpand(true);
            picture.set_vexpand(true);
            overlay.set_child(Some(&picture));
            overlay.add_overlay(&backdrop);
            backdrop.add_css_class("lock-scrim");
        } else {
            overlay.set_child(Some(&backdrop));
        }

        // ── Centered column: clock, date, card ───────────────────────
        let column = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .halign(gtk4::Align::Center)
            .valign(gtk4::Align::Center)
            .spacing(0)
            .build();

        let clock = gtk4::Label::builder().label("").build();
        clock.add_css_class("lock-clock");
        let date = gtk4::Label::builder().label("").build();
        date.add_css_class("lock-date");

        let card = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(14)
            .width_request(360)
            .build();
        card.add_css_class("lock-card");

        // Frosted pane hugging the card exactly (margin lives on the pane so
        // the glass doesn't extend into the gap below the clock).
        let pane = super::glass::GlassPane::new();
        pane.set_margin_top(48);
        pane.set_child(&card);
        pane.set_texture(wallpaper.clone());

        let greet_mode = self.user_field.borrow().is_some();

        // User chips (greeter mode with several known users) — the kid clicks
        // an avatar instead of typing a name. Data (avatar, presence) comes
        // from the host `--list` when the greeter has it, else bare names.
        let users = self.users.borrow().clone();
        // Present with however many users we have now; the greeter refills
        // this row via `set_greet_chips` when the async `--list` resolves.
        // The row exists whenever greeter mode has any known users, so a
        // late `--list` upgrade never has to create a row that isn't there.
        let mut user_chips: Vec<(String, gtk4::Button)> = Vec::new();
        let chip_row = (greet_mode && !users.is_empty()).then(|| {
            let row = gtk4::Box::builder()
                .orientation(gtk4::Orientation::Horizontal)
                .spacing(10)
                .halign(gtk4::Align::Center)
                .build();
            row.add_css_class("lock-user-row");
            let active = self.user_field.borrow().clone().unwrap_or_default();
            user_chips = fill_chip_row(&row, &users, &active, &self.on_user_select);
            row
        });

        // Lock mode: a horizontal strip of user chips replaces the lone
        // "Switch user" button. Filled asynchronously so locking never blocks
        // on the `--list` subprocess; `--list` failure falls back to the
        // single legacy button (cycle).
        let lock_strip = (!greet_mode)
            .then(switch_user::cmd)
            .flatten()
            .map(|cmd| build_lock_strip(cmd));

        // Username row (greeter mode only) — the lock authenticates the
        // session user implicitly and never shows it.
        let user_entry = self.user_field.borrow().as_ref().map(|prefill| {
            let ue = gtk4::Entry::builder()
                .placeholder_text("Username")
                .text(prefill)
                .hexpand(true)
                .build();
            ue.add_css_class("lock-entry");
            ue
        });

        let entry = gtk4::PasswordEntry::builder()
            .show_peek_icon(false)
            .placeholder_text("Password")
            .hexpand(true)
            .build();
        entry.add_css_class("lock-entry");

        // Fingerprint pill — hidden until the reader is claimed.
        let fp_pill = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(10)
            .halign(gtk4::Align::Center)
            .visible(false)
            .build();
        fp_pill.add_css_class("lock-fp-pill");
        let fp_glyph = gtk4::Label::builder().label("\u{f0237}").build();
        fp_glyph.add_css_class("lock-fp-glyph");
        let fp_label = gtk4::Label::builder()
            .label("Touch fingerprint reader")
            .build();
        fp_label.add_css_class("lock-fp-label");
        fp_pill.append(&fp_glyph);
        fp_pill.append(&fp_label);

        let caps = gtk4::Label::builder()
            .label("\u{f0632}  Caps Lock is on")
            .halign(gtk4::Align::Center)
            .visible(false)
            .build();
        caps.add_css_class("lock-caps");

        let status = gtk4::Label::builder()
            .halign(gtk4::Align::Center)
            .wrap(true)
            .max_width_chars(40)
            .visible(false)
            .build();
        status.add_css_class("lock-status");

        if let Some(row) = &chip_row {
            card.append(row);
        }
        if let Some(ue) = &user_entry {
            card.append(ue);
            let pw = entry.clone();
            ue.connect_activate(move |_| {
                pw.grab_focus();
            });
        }
        card.append(&entry);
        card.append(&fp_pill);
        card.append(&caps);
        card.append(&status);
        if let Some(row) = &lock_strip {
            card.append(row);
        }

        column.append(&clock);
        column.append(&date);
        column.append(&pane);
        overlay.add_overlay(&column);

        entry.connect_activate(move |e| on_submit(e.text().to_string()));

        // Keyboard focus lands in the entry as soon as the surface maps;
        // a click anywhere pulls it back (e.g. after unplugging a monitor).
        let e = entry.clone();
        window.connect_map(move |_| {
            e.grab_focus();
        });
        let click = gtk4::GestureClick::new();
        let e = entry.clone();
        click.connect_pressed(move |_, _, _, _| {
            e.grab_focus();
        });
        window.add_controller(click);

        // Any input re-powers blanked outputs (capture phase so the entry
        // still receives the key afterwards).
        let key = gtk4::EventControllerKey::new();
        key.set_propagation_phase(gtk4::PropagationPhase::Capture);
        let wake = self.wake.clone();
        key.connect_key_pressed(move |_, _, _, _| {
            wake.poke();
            glib::Propagation::Proceed
        });
        window.add_controller(key);
        let motion = gtk4::EventControllerMotion::new();
        let wake = self.wake.clone();
        motion.connect_motion(move |_, _, _| wake.poke());
        window.add_controller(motion);

        let surface = Surface {
            window: window.clone(),
            card,
            user_entry,
            chip_row,
            user_chips,
            entry,
            status,
            fp_pill,
            fp_label,
            caps,
            clock,
            date,
        };
        self.update_surface_clock(&surface);
        self.inner.borrow_mut().push(surface);

        // Deregister when the compositor destroys the surface (monitor
        // unplug / unlock) so broadcasts don't touch dead widgets.
        let set = self.inner.clone();
        let win = window.clone();
        window.connect_destroy(move |_| {
            set.borrow_mut().retain(|s| s.window != win);
        });

        overlay.upcast()
    }

    /// Tick: clock, date, and caps-lock indicator on every surface.
    pub fn tick(&self) {
        let caps_on = caps_lock_state();
        for s in self.inner.borrow().iter() {
            self.update_surface_clock(s);
            s.caps.set_visible(caps_on);
        }
    }

    fn update_surface_clock(&self, s: &Surface) {
        if let Ok(now) = glib::DateTime::now_local() {
            if let Ok(t) = now.format("%H:%M") {
                s.clock.set_label(&t);
            }
            if let Ok(d) = now.format("%A %e %B") {
                s.date.set_label(d.trim());
            }
        }
    }

    /// Greeter mode: show an editable username row (prefilled) above the
    /// password entry. Call before any `build_surface`.
    pub fn enable_user_field(&self, prefill: &str) {
        *self.user_field.borrow_mut() = Some(prefill.to_string());
    }

    /// Greeter mode: show clickable user chips above the username row.
    /// Call before any `build_surface`, alongside `enable_user_field`.
    pub fn enable_user_chips(&self, users: &[UserChip], on_select: Rc<dyn Fn(String)>) {
        *self.users.borrow_mut() = users.to_vec();
        *self.on_user_select.borrow_mut() = Some(on_select);
    }

    /// Refill the greeter chip rows after the async `--list` resolves —
    /// upgrading the initial env-name chips to avatars + presence without
    /// having blocked window presentation on the subprocess. No-op for
    /// surfaces built without a chip row.
    pub fn set_greet_chips(&self, users: &[UserChip]) {
        *self.users.borrow_mut() = users.to_vec();
        let active = self.user_field.borrow().clone().unwrap_or_default();
        for s in self.inner.borrow_mut().iter_mut() {
            let Some(row) = s.chip_row.clone() else {
                continue;
            };
            s.user_chips = fill_chip_row(&row, users, &active, &self.on_user_select);
        }
    }

    /// Greeter mode: switch every surface to `user` — entry text, chip
    /// highlight, the prefill new surfaces start from — and put focus in
    /// the password entry.
    pub fn set_username(&self, user: &str) {
        *self.user_field.borrow_mut() = Some(user.to_string());
        for s in self.inner.borrow().iter() {
            if let Some(ue) = &s.user_entry {
                if ue.text() != user {
                    ue.set_text(user);
                }
            }
            for (name, chip) in &s.user_chips {
                if name == user {
                    chip.add_css_class("active");
                } else {
                    chip.remove_css_class("active");
                }
            }
            s.entry.grab_focus();
        }
    }

    /// Current username text (greeter mode). All surfaces mirror state, but
    /// the username is typed on one — read whichever is non-default first.
    pub fn username(&self) -> Option<String> {
        let surfaces = self.inner.borrow();
        surfaces
            .iter()
            .filter_map(|s| s.user_entry.as_ref())
            .map(|ue| ue.text().trim().to_string())
            .find(|t| !t.is_empty())
    }

    pub fn set_status(&self, text: &str, kind: StatusKind) {
        for s in self.inner.borrow().iter() {
            s.status.set_visible(!text.is_empty());
            s.status.set_label(text);
            s.status.remove_css_class("lock-status-error");
            s.status.remove_css_class("lock-status-info");
            s.status.add_css_class(match kind {
                StatusKind::Info => "lock-status-info",
                StatusKind::Error => "lock-status-error",
            });
        }
    }

    /// Grey out input while PAM is working; re-enable (and clear) after.
    pub fn set_verifying(&self, verifying: bool) {
        for s in self.inner.borrow().iter() {
            s.entry.set_sensitive(!verifying);
            if let Some(ue) = &s.user_entry {
                ue.set_sensitive(!verifying);
            }
            if verifying {
                s.card.add_css_class("lock-verifying");
            } else {
                s.card.remove_css_class("lock-verifying");
                s.entry.set_text("");
            }
        }
        if !verifying {
            self.focus_active_entry();
        }
    }

    /// Wrong password: shake every card (CSS keyframe re-trigger).
    pub fn shake(&self) {
        for s in self.inner.borrow().iter() {
            let card = s.card.clone();
            card.remove_css_class("lock-shake");
            glib::idle_add_local_once(move || {
                card.add_css_class("lock-shake");
            });
        }
    }

    /// Auth accepted — green flash while the unlock request goes out.
    pub fn flash_success(&self) {
        for s in self.inner.borrow().iter() {
            s.card.add_css_class("lock-success");
            s.entry.set_sensitive(false);
        }
    }

    pub fn show_fp(&self, visible: bool, label: &str) {
        for s in self.inner.borrow().iter() {
            s.fp_pill.set_visible(visible);
            if visible {
                s.fp_label.set_label(label);
            }
        }
    }

    fn focus_active_entry(&self) {
        let surfaces = self.inner.borrow();
        // Prefer the window that actually has compositor focus.
        for s in surfaces.iter() {
            if s.window.is_active() {
                s.entry.grab_focus();
                return;
            }
        }
        if let Some(s) = surfaces.first() {
            s.entry.grab_focus();
        }
    }
}

/// One pill-shaped user chip: round avatar (with presence dot) + name. The
/// caller wires the click; `active` marks the current user (accent ring via
/// the `.lock-user-chip.active` CSS descendant selector).
/// Clear `row` and (re)build one greeter chip per user, wiring each to the
/// shared select callback. Returns the (name, button) handles so the surface
/// can toggle the active class on username changes. Shared by the initial
/// `build_content` and the async `set_greet_chips` refill.
fn fill_chip_row(
    row: &gtk4::Box,
    users: &[UserChip],
    active: &str,
    on_select: &Rc<RefCell<Option<Rc<dyn Fn(String)>>>>,
) -> Vec<(String, gtk4::Button)> {
    while let Some(child) = row.first_child() {
        row.remove(&child);
    }
    let mut chips = Vec::with_capacity(users.len());
    for u in users {
        let chip = avatar_chip(&u.user, u.icon.as_deref(), u.logged_in, u.user == active);
        let cb = on_select.clone();
        let name = u.user.clone();
        chip.connect_clicked(move |_| {
            if let Some(cb) = cb.borrow().clone() {
                cb(name.clone());
            }
        });
        row.append(&chip);
        chips.push((u.user.clone(), chip));
    }
    chips
}

fn avatar_chip(user: &str, icon: Option<&str>, logged_in: bool, active: bool) -> gtk4::Button {
    let content = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(10)
        .valign(gtk4::Align::Center)
        .build();
    content.append(&avatar(user, icon, CHIP_AVATAR_SIZE, logged_in));

    let name = gtk4::Label::new(Some(user));
    name.add_css_class("lock-user-name");
    content.append(&name);

    let chip = gtk4::Button::builder().child(&content).build();
    chip.add_css_class("lock-user-chip");
    if active {
        chip.add_css_class("active");
    }
    chip
}

/// Lock-mode user strip container, populated asynchronously from `--list`.
/// Clicking another user's chip switches to their session (the host command
/// locks this one first). The current user is shown but inert.
fn build_lock_strip(cmd: String) -> gtk4::Box {
    let row = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(10)
        .halign(gtk4::Align::Center)
        .build();
    row.add_css_class("lock-user-row");

    let row_c = row.clone();
    crate::spawn::spawn_work(switch_user::list, move |users| {
        while let Some(child) = row_c.first_child() {
            row_c.remove(&child);
        }
        match users {
            Some(users) if !users.is_empty() => {
                for u in &users {
                    let chip = avatar_chip(&u.user, u.icon.as_deref(), u.logged_in, u.current);
                    if !u.current {
                        let cmd = cmd.clone();
                        let user = u.user.clone();
                        chip.connect_clicked(move |_| switch_user::switch_to(&cmd, &user));
                    }
                    // Current user: shown, marked, no action.
                    row_c.append(&chip);
                }
            }
            _ => {
                let btn = gtk4::Button::with_label("󰓤  Switch user");
                btn.add_css_class("lock-switch-user");
                btn.set_halign(gtk4::Align::Center);
                let cmd = cmd.clone();
                btn.connect_clicked(move |_| switch_user::cycle(&cmd));
                row_c.append(&btn);
            }
        }
    });

    row
}

fn wallpaper_path() -> Option<String> {
    let path = std::env::var("SWAYPPLET_LOCK_WALLPAPER").ok()?;
    if !path.is_empty() && std::path::Path::new(&path).is_file() {
        Some(path)
    } else {
        None
    }
}

fn caps_lock_state() -> bool {
    gdk4::Display::default()
        .and_then(|d| d.default_seat())
        .and_then(|seat| seat.keyboard())
        .map(|kb| kb.is_caps_locked())
        .unwrap_or(false)
}
