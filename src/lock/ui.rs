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

use crate::anim::animations_enabled;
use crate::avatar::avatar;
use crate::icons;
use crate::switch_user;

/// Data for one user chip. Sourced from [`crate::switch_user::list`] when
/// available (avatar + presence), otherwise just a name.
///
/// The same chip appears in both modes, which is the whole point: whether
/// you are looking at a greeter or at someone's lock screen, picking a face
/// takes you to that person's session. Only the plumbing behind the click
/// differs (greetd start vs. `loginctl activate`), and the person tapping
/// the screen should never have to know which one they are on.
#[derive(Clone, PartialEq, Eq)]
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

/// How long [`SurfaceSet::begin_handoff`] runs before the caller actually
/// switches. Long enough to read as a deliberate handoff, short enough that
/// the machine still feels instant — Apple's HIG puts that band at
/// 100–500ms and M3's emphasized-exit durations sit right about here.
pub const HANDOFF: Duration = Duration::from_millis(180);

/// How long after the switch fires before [`SurfaceSet::end_handoff`] puts the
/// card back. Both surfaces that play the handoff outlive it: the greeter is
/// the machine's one idle greeter, handed back to whoever returns to that VT,
/// and the locker keeps running behind the session it just locked. Neither can
/// be left as a blurred pane with no card in it. Generous enough that a slow
/// `loginctl activate` still cuts away first.
const HANDOFF_RECOVER: Duration = Duration::from_secs(2);

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
    /// This surface is on the built-in panel, the one with the camera above
    /// it. The face indicator appears here and nowhere else.
    internal: bool,
    /// The one pixel that gives this surface something to commit.
    commit_pixel: gtk4::DrawingArea,
    /// Wrapper carrying the pill's entrance animation, kept off the pill so
    /// state changes cannot replay it.
    face_wrap: gtk4::Box,
    window: gtk4::Window,
    card: gtk4::Box,
    /// The client-side glass behind the card; its sigma is animatable
    /// (materialize on enter, dissolve on handoff — see glass.rs).
    pane: super::glass::GlassPane,
    user_entry: Option<gtk4::Entry>,
    /// Chip row container, kept so `set_user_chips` can refill it once the
    /// async session/enrollment query resolves. In lock mode it starts
    /// empty and hidden.
    chip_row: Option<gtk4::Box>,
    user_chips: Vec<(String, gtk4::Button)>,
    /// Lock mode: the "Switch user" fallback, retired once real chips land.
    switch_btn: Option<gtk4::Button>,
    entry: gtk4::PasswordEntry,
    status: gtk4::Label,
    fp_pill: gtk4::Box,
    fp_label: gtk4::Label,
    /// Face unlock indicator. Pinned to the top of the screen rather than
    /// placed in the card, so it sits under the camera and does not move
    /// between the lock screen and the elevate prompt. A fixed position is
    /// the point: the eye learns one place to look, and looking there aims
    /// the face on-axis to the lens, which is worth real match accuracy on a
    /// sensor with no depth channel.
    face_pill: gtk4::Box,
    face_ring: gtk4::Box,
    face_label: gtk4::Label,
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
    /// Whose chip reads as selected. The greeter moves it with the username
    /// field; the lock pins it to the session owner and never moves it.
    active_user: Rc<RefCell<String>>,
    /// Known users rendered as clickable chips above the username row (only
    /// when more than one).
    users: Rc<RefCell<Vec<UserChip>>>,
    on_user_select: Rc<RefCell<Option<Rc<dyn Fn(String)>>>>,
    /// The compositor is cross-fading the whole surface, so the surfaces must
    /// not also animate themselves in.
    crossfade: Rc<Cell<bool>>,
    /// Which of the two colours every surface's commit pixel is drawing.
    pulse: Rc<Cell<u32>>,
}

impl SurfaceSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Call before any `build_surface`. With the compositor fading the whole
    /// surface, the card's own `auth-card-enter` would multiply into it (the
    /// card's opacity becomes css x surface, quadratic) and the card would
    /// visibly lag the wallpaper it sits on. Same for the blur ramp: one
    /// motion, not three.
    pub fn set_crossfade(&self, on: bool) {
        self.crossfade.set(on);
    }

    /// Build a lock window for one monitor and register it in the set.
    /// `on_submit` receives the entry text on Enter.
    ///
    /// `monitor` is the output this surface will be assigned to, needed only
    /// so the face indicator can tell the built-in panel from an external
    /// one — the camera is over exactly one of them.
    pub fn build_surface(
        &self,
        on_submit: Rc<dyn Fn(String)>,
        monitor: Option<&gdk4::Monitor>,
    ) -> gtk4::Window {
        let window = gtk4::Window::new();
        window.add_css_class("lock");
        let internal = monitor.is_some_and(crate::layer_shell::is_internal);
        let content = self.build_content(&window, on_submit, internal);
        window.set_child(Some(&content));
        window
    }

    /// The full-screen lock content. Public so `--preview lock` can host it
    /// in a plain window for visual iteration.
    pub fn build_content(
        &self,
        window: &gtk4::Window,
        on_submit: Rc<dyn Fn(String)>,
        internal: bool,
    ) -> gtk4::Widget {
        let overlay = gtk4::Overlay::new();

        // Wallpaper (optional, crisp) + scrim for contrast; solid palette bg
        // otherwise. Only the card region reads frosted — the GlassPane
        // around the card re-draws this texture blurred behind it (swayfx
        // blur covers neither ext-session-lock surfaces nor own content).
        let backdrop = gtk4::Box::builder().hexpand(true).vexpand(true).build();
        backdrop.add_css_class("lock-backdrop");
        super::stage("wallpaper decode start");
        let wallpaper = wallpaper_texture();
        super::stage("wallpaper decode done");
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

        // One pixel that changes on demand, so the surface has something to
        // commit. See `pulse` for why a lock screen needs that.
        let commit_pixel = gtk4::DrawingArea::new();
        commit_pixel.set_content_width(1);
        commit_pixel.set_content_height(1);
        commit_pixel.set_halign(gtk4::Align::Start);
        commit_pixel.set_valign(gtk4::Align::Start);
        commit_pixel.set_can_target(false);
        commit_pixel.set_can_focus(false);
        {
            let phase = self.pulse.clone();
            commit_pixel.set_draw_func(move |_, cr, _, _| {
                // Two alphas one 255th apart: different enough that GSK sees
                // a changed node and damages it, far too close to be seen.
                let a = f64::from(1 + phase.get() % 2) / 255.0;
                cr.set_source_rgba(0.0, 0.0, 0.0, a);
                let _ = cr.paint();
            });
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
        card.add_css_class("glass-card");
        card.add_css_class("lock-card");

        // Frosted pane hugging the card exactly (margin lives on the pane so
        // the glass doesn't extend into the gap below the clock).
        let pane = super::glass::GlassPane::new();
        pane.set_margin_top(48);
        pane.set_child(&card);
        pane.set_texture(wallpaper.clone());
        // Materialize: sigma ramps 0 → full with the card's enter fade
        // (auth-card-enter, 300 ms = anim::ENTER_MS). This is client-side
        // GSK blur, the one glass in swaypplet with a real radius to
        // animate; ramp_blur_to jumps under reduced motion.
        if self.crossfade.get() {
            window.add_css_class("lock-crossfade");
            pane.set_blur_radius(super::glass::BLUR_RADIUS);
        } else {
            pane.set_blur_radius(0.0);
            pane.ramp_blur_to(super::glass::BLUR_RADIUS, crate::anim::ENTER_MS);
        }

        let greet_mode = self.user_field.borrow().is_some();

        // The user picker — same row, same chips, both modes. Click a face
        // and you land in that person's session; the greeter starts one, the
        // lock screen jumps to a running one. Nobody has to work out which
        // screen they are standing in front of.
        //
        // The row is created whenever it *could* be filled, so the async
        // refill (`set_user_chips`) never has to invent one that isn't
        // there. It stays hidden until there is more than one face to pick.
        let users = self.users.borrow().clone();
        let mut user_chips: Vec<(String, gtk4::Button)> = Vec::new();
        let chip_row = (greet_mode || switch_user::available()).then(|| {
            let row = gtk4::Box::builder()
                .orientation(gtk4::Orientation::Horizontal)
                .spacing(10)
                .halign(gtk4::Align::Center)
                .build();
            let active = self.active_user.borrow().clone();
            user_chips = fill_chip_row(&row, &users, &active, &self.on_user_select);
            row
        });

        // Lock mode fallback for when the chip query comes back empty or
        // fails: one button to a greeter, which can pick for itself.
        // `set_user_chips` hides it as soon as real chips arrive.
        let lock_switch = (!greet_mode && switch_user::available()).then(|| {
            let btn = build_switch_button();
            btn.set_visible(users.len() < 2);
            btn
        });

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
        let fp_glyph = gtk4::Label::builder().label(icons::FINGERPRINT).build();
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
        if let Some(btn) = &lock_switch {
            card.append(btn);
        }

        column.append(&clock);
        column.append(&date);
        column.append(&pane);
        overlay.add_overlay(&commit_pixel);

        // Face unlock indicator, pinned top-centre under the camera.
        let face_pill = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(12)
            .halign(gtk4::Align::Center)
            .valign(gtk4::Align::Start)
            .visible(false)
            .build();
        face_pill.add_css_class("face-pill");
        // The ring is drawn in CSS, not set as a glyph: it has to sweep while
        // looking, lock when a face is found, complete on a match and break
        // on a failure, and a font glyph can do none of that.
        let face_ring = gtk4::Box::builder()
            .width_request(22)
            .height_request(22)
            .build();
        face_ring.add_css_class("face-ring");
        let face_label = gtk4::Label::builder().label("").build();
        face_label.add_css_class("face-pill-label");
        face_pill.append(&face_ring);
        face_pill.append(&face_label);

        // The entrance rides on a wrapper rather than on the pill. `animation`
        // is a single property, so an entrance on .face-pill would be rewritten
        // by every state class -- and every looking -> face edge would replay
        // the arrival, dropping the pill mid-check. Two nodes, two independent
        // animations.
        let face_wrap = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .halign(gtk4::Align::Center)
            .valign(gtk4::Align::Start)
            .build();
        face_wrap.set_margin_top(56);
        face_wrap.append(&face_pill);

        overlay.add_overlay(&column);
        overlay.add_overlay(&face_wrap);

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
            internal,
            commit_pixel,
            face_wrap,
            window: window.clone(),
            card,
            pane,
            user_entry,
            chip_row,
            user_chips,
            switch_btn: lock_switch,
            entry,
            status,
            fp_pill,
            fp_label,
            face_pill,
            face_ring,
            face_label,
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

    /// Repaint one pixel on every surface, so the frame GTK is about to
    /// draw carries a `wl_surface.commit`.
    ///
    /// The cross-fade lives in the surface's `wp_alpha_modifier` multiplier,
    /// which is double-buffered state: it reaches the compositor only on the
    /// surface's next commit. A lock surface may not be committed by hand —
    /// `null_buffer` and `dimensions_mismatch` are both fatal — so
    /// `lock/fade.rs` asks GTK for the commit with `queue_draw`, and GTK,
    /// finding a render node tree identical to the last one, produces no
    /// frame at all. Measured over one entrance: 38 multiplier values set,
    /// 11 commits, and a 185 ms stretch in the middle where the value went
    /// 0.22 to 0.97 with nothing reaching the compositor. That is a cross-
    /// fade that holds at a fifth opacity and then snaps to full.
    ///
    /// The exit never showed it because an unlock always follows an auth
    /// success, and the success flash repaints the card on every frame.
    pub fn pulse(&self) {
        self.pulse.set(self.pulse.get().wrapping_add(1));
        for s in self.inner.borrow().iter() {
            s.commit_pixel.queue_draw();
        }
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
        *self.active_user.borrow_mut() = prefill.to_string();
    }

    /// Lock mode: mark whose session this is, so their chip reads as
    /// selected the way the greeter's target user does. There is no username
    /// field to move it — the lock only ever authenticates this one person.
    pub fn set_current_user(&self, user: &str) {
        *self.active_user.borrow_mut() = user.to_string();
    }

    /// Show clickable user chips above the username row. Call before any
    /// `build_surface`; `set_user_chips` handles later arrivals.
    pub fn enable_user_chips(&self, users: &[UserChip], on_select: Rc<dyn Fn(String)>) {
        *self.users.borrow_mut() = users.to_vec();
        *self.on_user_select.borrow_mut() = Some(on_select);
    }

    /// Refill the chip rows once the session/enrollment query resolves —
    /// in the greeter, upgrading env-name chips to avatars + presence; on
    /// the lock screen, populating the row for the first time. Either way
    /// the surface was on screen long before this landed. No-op for
    /// surfaces built without a chip row.
    pub fn set_user_chips(&self, users: &[UserChip]) {
        if *self.users.borrow() == users {
            return;
        }
        *self.users.borrow_mut() = users.to_vec();
        let active = self.active_user.borrow().clone();
        for s in self.inner.borrow_mut().iter_mut() {
            let Some(row) = s.chip_row.clone() else {
                continue;
            };
            s.user_chips = fill_chip_row(&row, users, &active, &self.on_user_select);
            // Real faces beat a generic button; the greeter the button
            // would have opened is one chip click away anyway.
            if let Some(btn) = &s.switch_btn {
                btn.set_visible(users.len() < 2);
            }
        }
    }

    /// Greeter mode: switch every surface to `user` — entry text, chip
    /// highlight, the prefill new surfaces start from — and put focus in
    /// the password entry.
    pub fn set_username(&self, user: &str) {
        *self.user_field.borrow_mut() = Some(user.to_string());
        *self.active_user.borrow_mut() = user.to_string();
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
            self.focus_entry();
        }
    }

    /// The beat between picking a face and landing in that session: the
    /// picked chip blooms, everything else dissolves back to bare wallpaper.
    /// The surface on the far side of the VT fades its card up from that
    /// same frame, so the cut happens inside one continuous move instead of
    /// between two unrelated screens.
    ///
    /// Returns the delay the caller should wait before switching. Zero when
    /// the user has animations off — then nothing is drawn and nothing is
    /// worth waiting for.
    pub fn begin_handoff(&self, user: &str) -> Duration {
        if !animations_enabled() {
            return Duration::ZERO;
        }
        for s in self.inner.borrow().iter() {
            s.card.add_css_class("lock-handoff");
            // The glass dissolves with the card (matches the 240ms+120ms
            // card fade in style.css), so the wallpaper is bare when the
            // VT cut lands.
            s.pane.ramp_blur_to(0.0, 360.0);
            for (name, chip) in &s.user_chips {
                chip.add_css_class(if name == user { "picked" } else { "dropped" });
            }
            // Nothing typed from here on lands anywhere useful.
            s.entry.set_sensitive(false);
            if let Some(ue) = &s.user_entry {
                ue.set_sensitive(false);
            }
        }
        // The switch is fire-and-forget: it can fail, and even when it works
        // this surface is still here afterwards. Arm the way back now, while
        // we know we faded something out.
        let restore = self.clone();
        glib::timeout_add_local_once(HANDOFF + HANDOFF_RECOVER, move || restore.end_handoff());
        HANDOFF
    }

    /// Undo [`begin_handoff`]: card back, chips back, typing allowed again.
    /// Fires on a timer rather than on the switch failing, because the common
    /// case isn't failure — it's the surface being returned to later.
    pub fn end_handoff(&self) {
        for s in self.inner.borrow().iter() {
            s.card.remove_css_class("lock-handoff");
            s.pane
                .ramp_blur_to(super::glass::BLUR_RADIUS, crate::anim::ENTER_MS);
            for (_, chip) in &s.user_chips {
                chip.remove_css_class("picked");
                chip.remove_css_class("dropped");
            }
            s.entry.set_sensitive(true);
            if let Some(ue) = &s.user_entry {
                ue.set_sensitive(true);
            }
        }
        self.set_status("", StatusKind::Info);
        // Desensitizing dropped the caret; nothing would take typing otherwise.
        self.focus_entry();
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

    /// Drive the face indicator on every surface.
    ///
    /// `state` is a CSS class rather than an enum of drawing instructions, so
    /// the whole visual vocabulary lives in the stylesheet and this stays a
    /// state broadcast.
    /// Shown on the built-in panel only. The camera is above that screen and
    /// no other, so a pill on an external monitor asks the user to look away
    /// from the sensor reading their face. On a machine with no internal
    /// panel there is nothing to prefer, so every surface gets it.
    pub fn show_face(&self, visible: bool, state: &str, label: &str) {
        let surfaces = self.inner.borrow();
        let has_internal = surfaces.iter().any(|s| s.internal);
        for s in surfaces.iter() {
            if has_internal && !s.internal {
                s.face_pill.set_visible(false);
                continue;
            }
            let arriving = visible && !s.face_pill.is_visible();
            s.face_pill.set_visible(visible);
            if !visible {
                s.face_wrap.remove_css_class("face-pill-enter");
                continue;
            }
            if arriving {
                // Re-added on the next main-loop turn so the style actually
                // recomputes between removal and addition; adding it back in
                // the same frame would not restart the animation.
                s.face_wrap.remove_css_class("face-pill-enter");
                let wrap = s.face_wrap.clone();
                glib::idle_add_local_once(move || {
                    wrap.add_css_class("face-pill-enter");
                });
            }
            s.face_label.set_label(label);
            crate::face_ring::apply(&s.face_ring, Some(&s.face_pill), state);
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

    /// Put the caret back in the password entry — on the surface the
    /// compositor considers focused, if any.
    pub fn focus_entry(&self) {
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
    // One face is no choice at all — the password entry already says who
    // you are. Only a real picker earns the vertical space.
    row.set_visible(users.len() > 1);
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
    content.append(&name);

    let chip = gtk4::Button::builder().child(&content).build();
    chip.add_css_class("lock-user-chip");
    if active {
        chip.add_css_class("active");
    }
    chip
}

/// Lock-mode "Switch user" button — jumps to a greeter instead of offering
/// direct user targets.
fn build_switch_button() -> gtk4::Button {
    let btn = gtk4::Button::with_label(&format!("{}  Switch user", icons::SWITCH_USER));
    btn.add_css_class("lock-switch-user");
    btn.set_halign(gtk4::Align::Center);
    btn.connect_clicked(|_| switch_user::to_greeter());
    btn
}

/// Decode the wallpaper once, ahead of the lock request.
///
/// It used to be decoded inside `build_content`, i.e. once per monitor,
/// inside the `connect_monitor` burst, inside `instance.lock()` — precisely
/// the interval the compositor holds the live desktop on screen for.
pub fn preload_wallpaper() {
    let _ = wallpaper_texture();
}

/// Decoded once per process and shared by every surface.
fn wallpaper_texture() -> Option<gdk4::Texture> {
    thread_local! {
        static TEXTURE: std::cell::OnceCell<Option<gdk4::Texture>> =
            const { std::cell::OnceCell::new() };
    }
    TEXTURE.with(|cell| {
        cell.get_or_init(|| wallpaper_path().and_then(|p| gdk4::Texture::from_filename(p).ok()))
            .clone()
    })
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
