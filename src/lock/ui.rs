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

/// Stored, not just passed through: the message line re-renders whenever
/// Caps Lock changes, and it has to remember how the last status was meant.
#[derive(Clone, Copy, Default)]
pub enum StatusKind {
    #[default]
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
    entry: gtk4::PasswordEntry,
    /// The fingerprint pill's slot, or `None` on a card built without one.
    /// Reserved from the first frame when it exists (see [`crate::slot`]):
    /// fprintd answers hundreds of milliseconds after the surface maps, and
    /// that is exactly the window the entrance is playing in.
    fp_pill: Option<gtk4::Box>,
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
    /// One reserved line for everything the card has to say: the status of
    /// the last attempt, or the Caps Lock warning when there is no status.
    /// They used to be two rows that came and went independently, which is
    /// two ways for the card to resize while someone is reading it.
    msg: gtk4::Label,
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
    /// Whether to lay out a fingerprint slot at all. Answered before the
    /// first surface is built (`crate::fp::self_enrolled_blocking` during the
    /// lock's warm-up); a card built without one can never grow a pill, and a
    /// card built with one can never be surprised by it.
    fp_slot: Rc<Cell<bool>>,
    /// Message-line state, kept here rather than read back off the labels:
    /// both inputs can change independently and the line renders from both.
    caps: Rc<Cell<bool>>,
    status: Rc<RefCell<(String, StatusKind)>>,
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

    /// Call before any `build_surface`: does this machine have a fingerprint
    /// reader worth leaving room for?
    ///
    /// The answer decides the card's height, so it has to be known before the
    /// card exists. `true` lays out a pill-sized slot that stays blank until
    /// the reader reports in (and goes blank again if it drops out) without
    /// ever moving anything; `false` builds a card with no slot, which is
    /// then equally incapable of growing one.
    pub fn set_fp_expected(&self, expected: bool) {
        self.fp_slot.set(expected);
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
            // One face is no choice at all — the password entry already says
            // who you are. Only a real picker earns the vertical space, and
            // this is the only moment that judgement may be made: from here
            // on the card's height is fixed.
            row.set_visible(users.len() > 1);
            row
        });

        // Lock mode fallback for when the warm-up found nobody to switch to:
        // one button to a greeter, which can pick for itself. Shown or not
        // shown for the life of the card, like everything else in it.
        let lock_switch =
            (!greet_mode && switch_user::available() && users.len() < 2).then(build_switch_button);

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

        // Fingerprint pill. Built only when a reader is expected, and then
        // laid out for good — blank until fprintd claims the device, blank
        // again if it drops out, never absent. The label is capped rather
        // than wrapped: a hint long enough to wrap would push the pill past
        // the card and widen it, which is the same jump sideways.
        // Hints replace each other in place: the glyph is pinned to the left
        // edge and the label grows rightwards into space the pill already
        // owns. Sized to the row above it rather than to its own text, so
        // "Touch fingerprint reader" and "Try again" are the same object.
        let fp_label = gtk4::Label::builder()
            .label("Touch fingerprint reader")
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .xalign(0.0)
            .hexpand(true)
            .build();
        fp_label.add_css_class("lock-fp-label");
        let fp_pill = self.fp_slot.get().then(|| {
            let pill = gtk4::Box::builder()
                .orientation(gtk4::Orientation::Horizontal)
                .spacing(12)
                .halign(gtk4::Align::Fill)
                .build();
            pill.add_css_class("lock-fp-pill");
            let fp_glyph = gtk4::Label::builder().label(icons::FINGERPRINT).build();
            fp_glyph.add_css_class("lock-fp-glyph");
            pill.append(&fp_glyph);
            pill.append(&fp_label);
            crate::slot::reserve(&pill);
            pill
        });

        // The card's one message line. Two lines' worth of height, always,
        // whether it is showing a two-line PAM error, "Wrong password", the
        // Caps Lock warning or nothing at all. Capped at two lines so no
        // message can ever claim a third.
        let msg = gtk4::Label::builder()
            .halign(gtk4::Align::Center)
            .justify(gtk4::Justification::Center)
            .wrap(true)
            .wrap_mode(gtk4::pango::WrapMode::WordChar)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .lines(2)
            .max_width_chars(40)
            .valign(gtk4::Align::Center)
            .build();
        msg.add_css_class("lock-message");
        crate::slot::reserve(&msg);

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
        if let Some(pill) = &fp_pill {
            card.append(pill);
        }
        card.append(&msg);
        if let Some(btn) = &lock_switch {
            card.append(btn);
        }

        column.append(&clock);
        column.append(&date);
        column.append(&pane);
        overlay.add_overlay(&commit_pixel);

        // Face unlock indicator, pinned top-centre under the camera.
        // Built like the fingerprint pill and for the same reason: the ring
        // is what the eye tracks, so it is pinned to the left edge and the
        // wording changes underneath a shape that does not. `.face-pill`
        // carries a fixed width in the stylesheet, shared with the elevate
        // cue, so "Looking for you" and "Didn't recognise you" occupy exactly
        // the same box.
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
        let face_label = gtk4::Label::builder()
            .label("")
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .xalign(0.0)
            .hexpand(true)
            .build();
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
            entry,
            fp_pill,
            fp_label,
            face_pill,
            face_ring,
            face_label,
            msg,
            clock,
            date,
        };
        self.update_surface_clock(&surface);
        self.render_message(&surface);
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

    /// Tick: clock, date, and caps-lock state on every surface.
    pub fn tick(&self) {
        let caps_on = caps_lock_state();
        let caps_changed = self.caps.replace(caps_on) != caps_on;
        for s in self.inner.borrow().iter() {
            self.update_surface_clock(s);
            if caps_changed {
                self.render_message(s);
            }
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
    /// the lock screen, confirming what the warm-up already drew. Either way
    /// the surface was on screen long before this landed. No-op for surfaces
    /// built without a chip row.
    ///
    /// Contents only. Whether the row and the fallback button are on screen
    /// was settled when the card was built, and stays settled: a picker that
    /// materialised here would push the whole card down at a moment when the
    /// user is already typing into it. The cost is a card that was built
    /// without a picker keeping none for this one lock — the "Switch user"
    /// button it does have reaches the same place by way of a greeter.
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

    /// What the card has to say about the last attempt. Empty clears it.
    ///
    /// The line it lands on is already laid out, so this only ever changes
    /// what is painted there.
    pub fn set_status(&self, text: &str, kind: StatusKind) {
        *self.status.borrow_mut() = (text.to_string(), kind);
        for s in self.inner.borrow().iter() {
            self.render_message(s);
        }
    }

    /// Paint the message line from the two things that feed it.
    ///
    /// A status wins the line, because it answers something the user just
    /// did; Caps Lock gets it when there is nothing else to say. When both
    /// are true and the status is a failure they share it, since a rejected
    /// password with Caps Lock on is one fact, not two — splitting them
    /// across separate rows left the reader to join them up, and cost a
    /// resize for the privilege.
    fn render_message(&self, s: &Surface) {
        let (text, kind) = self.status.borrow().clone();
        let caps = self.caps.get();
        const CAPS: &str = "\u{f0632}  Caps Lock is on";
        // Markup, so the two facts can be weighted differently on one label:
        // the rejection in the status colour, the Caps Lock note under it in
        // a quieter voice. PAM and greetd write some of these strings, so
        // escape before doing so.
        let (markup, class) = if !text.is_empty() {
            let text = glib::markup_escape_text(&text);
            let markup = match kind {
                StatusKind::Error if caps => {
                    format!("{text}\n<span size=\"smaller\" alpha=\"75%\">{CAPS}</span>")
                }
                _ => text.to_string(),
            };
            (
                markup,
                match kind {
                    StatusKind::Info => "lock-status-info",
                    StatusKind::Error => "lock-status-error",
                },
            )
        } else if caps {
            (CAPS.to_string(), "lock-status-caps")
        } else {
            (String::new(), "lock-status-info")
        };

        for c in ["lock-status-info", "lock-status-error", "lock-status-caps"] {
            if c != class {
                s.msg.remove_css_class(c);
            }
        }
        s.msg.add_css_class(class);
        // Keep the old words while the line fades out; replacing them first
        // would flash the new message at zero opacity and back.
        if !markup.is_empty() {
            s.msg.set_markup(&markup);
        }
        crate::slot::show(&s.msg, !markup.is_empty());
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

    /// Fade the fingerprint pill in or out inside its reserved slot.
    ///
    /// No-op on a card built without one — `set_fp_expected(false)` said
    /// there would be no reader, and a card that grew one anyway would be
    /// the jump this whole arrangement exists to prevent.
    pub fn show_fp(&self, visible: bool, label: &str) {
        for s in self.inner.borrow().iter() {
            let Some(pill) = &s.fp_pill else { continue };
            if visible {
                s.fp_label.set_label(label);
            }
            crate::slot::show(pill, visible);
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
///
/// Whether the row is on screen is decided once, by `build_content`, and is
/// deliberately not this function's business — see `set_user_chips`.
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
    // Dev hook, same family as SWAYPPLET_PREVIEW_AVATAR: the headless render
    // harness has no keyboard to latch, and the Caps Lock line is one of the
    // states whose arrival has to be shown not to move the card.
    if let Ok(v) = std::env::var("SWAYPPLET_PREVIEW_CAPS") {
        return v == "1";
    }
    gdk4::Display::default()
        .and_then(|d| d.default_seat())
        .and_then(|seat| seat.keyboard())
        .map(|kb| kb.is_caps_locked())
        .unwrap_or(false)
}
