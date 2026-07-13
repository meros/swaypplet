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

        // Wallpaper (optional, frosted) + scrim for contrast; solid palette
        // bg otherwise. The blur is baked into the texture — swayfx blur only
        // covers layer-shell surfaces, not ext-session-lock ones.
        let backdrop = gtk4::Box::builder().hexpand(true).vexpand(true).build();
        backdrop.add_css_class("lock-backdrop");
        if let Some(path) = wallpaper_path() {
            let picture = match blurred_wallpaper(&path) {
                Some(texture) => gtk4::Picture::for_paintable(&texture),
                None => gtk4::Picture::for_filename(&path),
            };
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
            .margin_top(48)
            .build();
        card.add_css_class("lock-card");

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

        card.append(&entry);
        card.append(&fp_pill);
        card.append(&caps);
        card.append(&status);

        column.append(&clock);
        column.append(&date);
        column.append(&card);
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

/// Frosted-glass backdrop: decode straight to a thumbnail, then repeatedly
/// double it back up with bilinear passes — a gaussian-pyramid expansion that
/// reads as a wide blur at a fraction of a real convolution's cost.
fn blurred_wallpaper(path: &str) -> Option<gdk4::Texture> {
    use gtk4::gdk_pixbuf::{InterpType, Pixbuf};
    let mut img = Pixbuf::from_file_at_scale(path, 64, 64, true).ok()?;
    while img.width() < 512 {
        img = img.scale_simple(img.width() * 2, img.height() * 2, InterpType::Bilinear)?;
    }
    Some(gdk4::Texture::for_pixbuf(&img))
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
