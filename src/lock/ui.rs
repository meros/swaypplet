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
use crate::services::status::{Severity, StatusItem};
use crate::auth_field::{AuthField, Caption, Tone};
use crate::switch_user;
use crate::ui;
use crate::ui::icons;

/// Data for one user chip. Sourced from [`crate::switch_user::list`] when
/// available (avatar + presence), otherwise just a name.
///
/// Greeter-only. The chip is an answer to "who are you", and only the greeter
/// asks that; the lock screen already knows, and its one switching affordance
/// is the button that hands the seat to a greeter so the question can be asked
/// where it can be answered.
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

/// Avatar diameter in the greeter's user row.
const CHIP_AVATAR_SIZE: i32 = 24;

/// What the commit pixel paints when it only needs *a* commit, never a
/// visible pixel: drawn at 1–2/255 alpha so GSK sees a changed node and
/// nobody sees anything. Two 255ths of black is far under the compositor's
/// discard line, so it never becomes glass.
const INVISIBLE_INK: crate::tokens::Rgb = crate::tokens::Rgb::BLACK;

/// The shade behind the lock's clock, in logical pixels: wider than the
/// clock by a margin each side, so the fade happens on picture, not on the
/// digits. The card starts where the shade ends and never overlaps it: a
/// card over a shaded pixel composites off the glass key and draws a flat
/// tint instead of the material.
const SHADE_SIZE: (i32, i32) = (1000, 360);

/// The soft black oval behind the lock's clock and date: `SHADE_PEAK` at its
/// centre, falling to nothing at its rim along a smoothstep, so it has no
/// edge to see (`tokens::backdrop`, "The lock's clock on its shade").
fn clock_shade() -> gtk4::DrawingArea {
    let area = gtk4::DrawingArea::new();
    area.set_content_width(SHADE_SIZE.0);
    area.set_content_height(SHADE_SIZE.1);
    area.set_can_target(false);
    area.set_can_focus(false);
    area.set_draw_func(|_, cr, w, h| {
        let (w, h) = (f64::from(w), f64::from(h));
        let ink = crate::tokens::Rgb::BLACK;
        cr.translate(w / 2.0, h / 2.0);
        cr.scale(w / 2.0, h / 2.0);
        let shade = gtk4::cairo::RadialGradient::new(0.0, 0.0, 0.0, 0.0, 0.0, 1.0);
        for i in 0..=16 {
            let t = f64::from(i) / 16.0;
            let fall = 1.0 - t * t * (3.0 - 2.0 * t);
            shade.add_color_stop_rgba(t, ink.0, ink.1, ink.2, crate::tokens::SHADE_PEAK * fall);
        }
        let _ = cr.set_source(&shade);
        cr.arc(0.0, 0.0, 1.0, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
    });
    area
}

/// How long [`SurfaceSet::begin_handoff`] runs before the caller actually
/// switches: the picker's exit (`--motion-exit`, the same token the chips
/// and the card leave on in the stylesheet). Long enough to read as a
/// deliberate handoff, short enough that the machine still feels instant.
pub fn handoff() -> Duration {
    crate::anim::span(crate::tokens::motion::EXIT)
}

/// How long after the switch fires before [`SurfaceSet::end_handoff`] puts the
/// card back. Both surfaces that play the handoff outlive it: the greeter is
/// the machine's one idle greeter, handed back to whoever returns to that VT,
/// and the locker keeps running behind the session it just locked. Neither can
/// be left as a bare wallpaper with no card on it. Generous enough that a slow
/// `loginctl activate` still cuts away first.
const HANDOFF_RECOVER: Duration = Duration::from_secs(2);

/// What a user chip's click does, set once the surface knows.
type OnUserSelect = Rc<RefCell<Option<Rc<dyn Fn(String)>>>>;

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
    /// The greeter's user row, kept so `set_user_chips` can refill it once
    /// the async session/enrollment query resolves. `None` in lock mode,
    /// which has no picker at all.
    chip_row: Option<gtk4::Box>,
    user_chips: Vec<(String, gtk4::Button)>,
    entry: gtk4::PasswordEntry,
    /// The password field, chrome and marks included. See
    /// [`crate::auth_field`] and docs/AUTH_CARD.md: everything the card used
    /// to say in rows under the entry, it now says on or below this one box.
    field: AuthField,
    /// Face unlock indicator. Pinned to the top of the screen rather than
    /// placed in the card, so it sits under the camera and does not move
    /// between the lock screen and the elevate prompt. A fixed position is
    /// the point: the eye learns one place to look, and looking there aims
    /// the face on-axis to the lens, which is worth real match accuracy on a
    /// sensor with no depth channel.
    face_pill: gtk4::Box,
    face_ring: gtk4::Box,
    face_label: gtk4::Label,
    /// Two reserved lines that are never empty. The resting sentence is the
    /// floor; errors, the Caps Lock edge and fingerprint hints take the line
    /// in that order and hand it back.
    caption: Caption,
    clock: gtk4::Label,
    date: gtk4::Label,
    /// The Claude Code line under the date (`set_sessions`).
    sessions: SessionLine,
}

/// "1 waiting for you · 2 working" under the date: how many Claude Code
/// sessions want the owner and how many work, never what they are called.
/// Anyone at the machine can read the lock screen, and a task label can name
/// a customer or an issue.
///
/// The line keeps its height when there is nothing to say, so the clock does
/// not move when a session starts or ends: the stamp is centred on the shade.
#[derive(Clone)]
struct SessionLine {
    row: gtk4::Box,
    dot: gtk4::Label,
    text: gtk4::Label,
}

impl SessionLine {
    fn new() -> Self {
        let row = ui::hbox(2);
        row.set_halign(gtk4::Align::Center);
        row.add_css_class("lock-sessions");
        // The dot is the one colour on the shade, and only for a session
        // that cannot go on without the owner.
        let dot = ui::text("\u{25CF}", ui::Text::Body, ui::Tone::Warning);
        let text = ui::text("", ui::Text::Body, ui::Tone::Fg);
        ui::on_shade::adopt(&text);
        row.append(&dot);
        row.append(&text);
        let line = Self { row, dot, text };
        line.show(None);
        line
    }

    fn show(&self, line: Option<&(bool, String)>) {
        match line {
            Some((attention, words)) => {
                self.dot.set_visible(*attention);
                self.text.set_label(words);
                self.row.set_opacity(1.0);
            }
            None => {
                // A space, so the empty line still has a line's height.
                self.text.set_label(" ");
                self.dot.set_visible(false);
                self.row.set_opacity(0.0);
            }
        }
    }
}

/// What the session line says for `items`, or `None` when no session wants
/// the owner or works. Idle sessions are not counted: they ask for nothing.
pub fn session_line(items: &[StatusItem]) -> Option<(bool, String)> {
    let count = |severity| items.iter().filter(|i| i.severity == severity).count();
    let (waiting, working) = (count(Severity::Attention), count(Severity::Active));
    let parts: Vec<String> = [
        (waiting > 0).then(|| format!("{waiting} waiting for you")),
        (working > 0).then(|| format!("{working} working")),
    ]
    .into_iter()
    .flatten()
    .collect();
    (!parts.is_empty()).then(|| (waiting > 0, parts.join(" \u{00B7} ")))
}

#[derive(Clone, Default)]
pub struct SurfaceSet {
    inner: Rc<RefCell<Vec<Surface>>>,
    wake: Rc<WakeCmd>,
    /// Greeter mode: the card asks for `active_user`'s password and the user
    /// row stands under it. Off, the card is the lock's, for the session's
    /// own user, who is never named.
    greeter: Rc<Cell<bool>>,
    /// The user the greeter's card is for, whose button reads as selected.
    active_user: Rc<RefCell<String>>,
    /// Known users, one button each under the card (only when more than
    /// one).
    users: Rc<RefCell<Vec<UserChip>>>,
    on_user_select: OnUserSelect,
    /// The compositor is cross-fading the whole surface, so the surfaces must
    /// not also animate themselves in.
    crossfade: Rc<Cell<bool>>,
    /// Whether the reader is armed right now. The caption's resting sentence
    /// is computed from this and nothing else, which is the honesty rule:
    /// the card names a method only while that method is accepting input.
    fp_armed: Rc<Cell<bool>>,
    /// Caps Lock, so a rejection can compose the warning onto its own line
    /// rather than needing a second row for it.
    caps: Rc<Cell<bool>>,
    /// Which of the two colours every surface's commit pixel is drawing.
    pulse: Rc<Cell<u32>>,
    /// The session line as last set, for a surface built after it.
    sessions: Rc<RefCell<Option<(bool, String)>>>,
}

impl SurfaceSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Call before any `build_surface`. With the compositor fading the whole
    /// surface, the card's own `auth-card-enter` would multiply into it (the
    /// card's opacity becomes css x surface, quadratic) and the card would
    /// visibly lag the wallpaper it sits on. One motion, not two.
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
        // Transparent, like every other swaypplet window, on the lock and on
        // the greeter alike: the compositor draws the wallpaper and puts the
        // glass on it, so an opaque fill here would hide both.
        let window = gtk4::Window::new();
        let internal = monitor.is_some_and(crate::shell::layer::is_internal);
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
        ui::surface::adopt(&overlay);

        // Nothing full-screen: the wallpaper stands as it is. On both
        // surfaces this builds, the wallpaper and the glass behind the cards
        // are the compositor's: the lock reads `layer_effects "session-lock"`
        // and draws the wallpaper into the lock's own scene tree, and the
        // greeter gets the same material keyed on its layer-shell namespace,
        // `swaypplet-greeter`, over its compositor's `output * bg`. A picture
        // here would cover the very pixels the material refracts.
        //
        // There used to be a 0.20 black scrim here, and on the lock the
        // compositor also blurred and dimmed the wallpaper under it. Both
        // tinted the whole picture to make the clock readable; the clock sits
        // on its own glass plate now (below), so the wallpaper stays sharp
        // and at full brightness everywhere else, and every card paints the
        // plain key.
        let ground = ui::vbox(0);
        ground.set_hexpand(true);
        ground.set_vexpand(true);
        overlay.set_child(Some(&ground));

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
                let ink = INVISIBLE_INK;
                cr.set_source_rgba(ink.0, ink.1, ink.2, a);
                let _ = cr.paint();
            });
        }

        // ── Centered column: clock, date, card ───────────────────────
        let column = ui::vbox(0);
        column.set_halign(gtk4::Align::Center);
        column.set_valign(gtk4::Align::Center);

        // The clock and the date stand on the wallpaper, large, over a soft
        // shade: a black oval that fades out with no edge, darkest behind
        // the digits (`clock_shade`). It darkens only what is behind the
        // clock, so the rest of the picture stays as it is, and it keeps
        // the clock readable over a bright, busy region where no outline
        // around the glyphs does. A glass plate here read as a second card.
        //
        // What still stands on bare wallpaper, the switch-user button and
        // the greeter's users, keeps the halo that follows the wallpaper
        // behind it (`ui::on_wallpaper`).
        let greet_mode = self.greeter.get();
        let clock = ui::text("", ui::Text::Hero, ui::Tone::Fg);
        clock.set_xalign(0.5);
        crate::ui::set_numeric(&clock, true);
        ui::on_shade::adopt(&clock);
        let date = ui::text("", ui::Text::Title, ui::Tone::Fg);
        date.set_xalign(0.5);
        date.add_css_class("lock-date");
        ui::on_shade::adopt(&date);
        let stamp = ui::vbox(0);
        stamp.set_halign(gtk4::Align::Center);
        stamp.set_valign(gtk4::Align::Center);
        stamp.append(&clock);
        stamp.append(&date);
        let sessions = SessionLine::new();
        sessions.show(self.sessions.borrow().as_ref());
        stamp.append(&sessions.row);
        let plate = gtk4::Overlay::new();
        plate.add_css_class("lock-stamp");
        plate.set_child(Some(&clock_shade()));
        plate.add_overlay(&stamp);

        // spacing 0: every gap below is an explicit margin in 10-lock.css,
        // because the gaps are deliberately unequal and a GtkBox has exactly
        // one spacing to give.
        let card = ui::vbox(0);
        card.set_width_request(360);
        // The card is what makes the glass: it paints the key, above the compositor's mask threshold, so the material is
        // stencilled to exactly this box. The card used to be hosted in a
        // GlassPane that drew a blurred copy of the wallpaper behind it and
        // ramped its sigma in; the compositor owns that now, and it has the
        // material up before this surface's first frame rather than a few
        // frames after it.
        ui::card::adopt(&card, ui::Card::Floating);
        card.add_css_class("lock-card");
        // The shade is wider than the card, and the column is as wide as
        // its widest child.
        card.set_halign(gtk4::Align::Center);

        if self.crossfade.get() {
            window.add_css_class("lock-crossfade");
        }

        // The user picker, greeter only, and outside the card: the card is
        // the lock's, field and caption and nothing else, on both surfaces.
        // A greeter is asked "who are you", so every account it knows is a
        // button under the card, standing on the wallpaper in the slot the
        // lock gives its "Switch user" button, and built the same way. A lock
        // screen gets no row: the session behind it belongs to one person.
        //
        // The row is created whenever it *could* be filled, so the async
        // refill (`set_user_chips`) never has to invent one that isn't
        // there. It stays hidden until there is more than one face to pick.
        let users = self.users.borrow().clone();
        let mut user_chips: Vec<(String, gtk4::Button)> = Vec::new();
        let stand: Rc<dyn Fn(&gtk4::Widget)> = Rc::new(ui::on_wallpaper::adopt);
        let chip_row = greet_mode.then(|| {
            let row = ui::hbox(3);
            row.set_halign(gtk4::Align::Center);
            row.add_css_class("lock-user-row");
            let active = self.active_user.borrow().clone();
            user_chips = fill_chip_row(&row, &users, &active, &self.on_user_select, &stand);
            // One face is no choice at all. Only a real picker earns the
            // vertical space, and this is the only moment that judgement may
            // be made: from here on the column's height is fixed.
            row.set_visible(users.len() > 1);
            row
        });

        // The lock's whole switching affordance: one button to a greeter,
        // which does the picking. Shown or not shown for the life of the card,
        // like everything else in it — and it needs nothing asynchronous to
        // decide, since `available()` is a file on disk rather than a session
        // query.
        let lock_switch = (!greet_mode && switch_user::available())
            .then(|| build_switch_button(self, stand.as_ref()));

        let entry = gtk4::PasswordEntry::builder()
            .show_peek_icon(false)
            .placeholder_text(placeholder(greet_mode, &self.active_user.borrow()))
            .hexpand(true)
            .build();
        let field = AuthField::new(&entry);

        let caption = Caption::new(40);

        card.append(field.widget());
        card.append(caption.widget());

        column.append(&plate);
        column.append(&card);
        // Outside the card, deliberately. Inside it, the button bounded the
        // caption's reserved second line on both sides and turned it back
        // into the hole this design exists to remove; below the glass it
        // leaves the caption as the card's last element, so that slack falls
        // against the bottom padding and reads as margin.
        //
        // It is also the card's only non-authentication action, and a link on
        // the wallpaper is what GNOME does with "Not listed?" for the same
        // reason.
        if let Some(btn) = &lock_switch {
            column.append(btn);
        }
        if let Some(row) = &chip_row {
            column.append(row);
        }
        overlay.add_overlay(&commit_pixel);

        // Face unlock indicator, pinned top-centre under the camera.
        // Built like the fingerprint pill and for the same reason: the ring
        // is what the eye tracks, so it is pinned to the left edge and the
        // wording changes underneath a shape that does not. `.face-pill`
        // carries a fixed width in the stylesheet, shared with the elevate
        // cue, so "Looking for you" and "Didn't recognise you" occupy exactly
        // the same box. The face in the ring (`ui::face_ring`) glances while
        // looking, stills when found, smiles on a match and frowns on a miss.
        //
        // The entrance rides on the wrapper rather than on the pill.
        // `animation` is a single property, so an entrance on the pill would
        // be rewritten by every state class -- and every looking -> face edge
        // would replay the arrival, dropping the pill mid-check. Two nodes,
        // two independent animations.
        //
        // Frosted like everything else, and by the same route as the card: it
        // paints the plain key, and the compositor draws the material there.
        // Before that it carried a client-side glass pane.
        let ui::FacePill {
            wrap: face_wrap,
            pill: face_pill,
            ring: face_ring,
            label: face_label,
        } = ui::face_pill(22);
        ui::card::adopt(&face_pill, ui::Card::Floating);
        face_pill.add_css_class("face-pill");
        face_pill.set_visible(false);

        overlay.add_overlay(&column);
        overlay.add_overlay(&face_wrap);

        // An error holds until the next keystroke. A keystroke means the user
        // has chosen the password path and no longer needs telling what went
        // wrong with the last one; holding it longer would sit on top of the
        // resting sentence for no reason.
        {
            let caption = caption.clone();
            entry.connect_changed(move |_| caption.clear_status());
        }
        // The secret leaves the widget the moment Enter takes it. Clearing it
        // later, on the reject path in `set_verifying(false)`, left it sitting
        // in the field on every path that never gets there: a greeter jump to
        // a running session, whose surface comes back re-enabled with the
        // password still in it, and a parked submit waiting on greetd.
        entry.connect_activate(move |e| {
            let secret = e.text().to_string();
            e.set_text("");
            on_submit(secret);
        });

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
            chip_row,
            user_chips,
            entry,
            field,
            face_pill,
            face_ring,
            face_label,
            caption,
            clock,
            date,
            sessions,
        };
        self.update_surface_clock(&surface);
        surface.caption.set_resting(self.resting_text());
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

    /// The Claude Code line under the date, on every surface.
    pub fn set_sessions(&self, items: &[StatusItem]) {
        let line = session_line(items);
        for s in self.inner.borrow().iter() {
            s.sessions.show(line.as_ref());
        }
        *self.sessions.borrow_mut() = line;
    }

    /// Tick: clock, date, and caps-lock state on every surface.
    pub fn tick(&self) {
        let caps_on = caps_lock_state();
        let caps_changed = self.caps.replace(caps_on) != caps_on;
        for s in self.inner.borrow().iter() {
            self.update_surface_clock(s);
            // The words mark the edge and then give the line back: a
            // permanent sentence about a shift key would sit on top of
            // everything else the caption has to say for as long as the key
            // is latched. The standing warning is composed onto the rejection
            // instead, which is the moment it changes what the user does.
            if caps_changed && caps_on {
                s.caption.caps_edge();
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

    /// Greeter mode: the card asks for `user`'s password, and the user row
    /// stands under it. Call before any `build_surface`.
    pub fn enable_greeter(&self, user: &str) {
        self.greeter.set(true);
        *self.active_user.borrow_mut() = user.to_string();
    }

    /// Show one button per user under the card. Call before any
    /// `build_surface`; `set_user_chips` handles later arrivals.
    pub fn enable_user_chips(&self, users: &[UserChip], on_select: Rc<dyn Fn(String)>) {
        *self.users.borrow_mut() = users.to_vec();
        *self.on_user_select.borrow_mut() = Some(on_select);
    }

    /// Refill the chip rows once the session/enrollment query resolves,
    /// upgrading the greeter's env-name chips to avatars + presence. The
    /// surface was on screen long before this landed. No-op for surfaces
    /// built without a chip row, which is every lock surface.
    ///
    /// Contents only. Whether the row is on screen was settled when the card
    /// was built, and stays settled: a picker that materialised here would
    /// push the whole card down at a moment when the user is already typing
    /// into it.
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
            let stand: Rc<dyn Fn(&gtk4::Widget)> = Rc::new(ui::on_wallpaper::adopt);
            s.user_chips = fill_chip_row(&row, users, &active, &self.on_user_select, &stand);
        }
    }

    /// Greeter mode: switch every surface to `user` (the field's
    /// placeholder, the selected button, what new surfaces start from) and
    /// put focus in the password entry.
    pub fn set_username(&self, user: &str) {
        *self.active_user.borrow_mut() = user.to_string();
        let hint = placeholder(self.greeter.get(), user);
        for s in self.inner.borrow().iter() {
            s.entry.set_placeholder_text(Some(&hint));
            for (name, chip) in &s.user_chips {
                ui::set_selected(chip, name == user);
            }
            s.entry.grab_focus();
        }
    }

    /// The user the greeter's card is for; `None` on the lock.
    pub fn username(&self) -> Option<String> {
        let user = self.active_user.borrow();
        (self.greeter.get() && !user.is_empty()).then(|| user.clone())
    }

    /// What the card has to say about the last attempt. Empty clears it,
    /// dropping the caption back to its resting sentence.
    pub fn set_status(&self, text: &str, kind: StatusKind) {
        let tone = match kind {
            StatusKind::Info => Tone::Info,
            StatusKind::Error => Tone::Error,
        };
        let caps = self.caps.get();
        for s in self.inner.borrow().iter() {
            s.caption.status(text, tone, caps);
        }
    }

    /// The sentence under everything else: what the user may do right now.
    ///
    /// The honesty rule in one function. It names the reader only while the
    /// reader is armed — not while fprintd is enumerating, not while the
    /// device is claimed elsewhere, not on a machine that has none.
    fn resting_text(&self) -> &'static str {
        if self.fp_armed.get() {
            "Touch the reader or enter your password"
        } else {
            "Enter your password"
        }
    }

    fn refresh_resting(&self) {
        let text = self.resting_text();
        for s in self.inner.borrow().iter() {
            s.caption.set_resting(text);
        }
    }

    /// Grey out input while PAM is working; re-enable (and clear) after.
    pub fn set_verifying(&self, verifying: bool) {
        for s in self.inner.borrow().iter() {
            s.entry.set_sensitive(!verifying);
            if verifying {
                s.caption.status("Checking\u{2026}", Tone::Info, false);
            }
            s.field.set_busy(verifying);
            if !verifying {
                s.entry.set_text("");
            }
        }
        if !verifying {
            self.focus_entry();
        }
    }

    /// The beat between asking for another session and landing in it: the
    /// picked chip blooms, everything else dissolves back to bare wallpaper.
    /// The surface on the far side of the VT fades its card up from that
    /// same frame, so the cut happens inside one continuous move instead of
    /// between two unrelated screens.
    ///
    /// `picked` is the chip that blooms; `None` on the lock screen, which has
    /// no picker to bloom and only the card to dissolve.
    ///
    /// Returns the delay the caller should wait before switching. Zero when
    /// the user has animations off — then nothing is drawn and nothing is
    /// worth waiting for.
    pub fn begin_handoff(&self, picked: Option<&str>) -> Duration {
        if !animations_enabled() {
            return Duration::ZERO;
        }
        for s in self.inner.borrow().iter() {
            // The card's fade also dissolves the glass, with no second ramp to
            // keep in step: the material is stencilled to the card's alpha, so
            // as `.lock-handoff` fades the fill past the compositor's mask
            // threshold the material goes with it, and the wallpaper is bare
            // when the VT cut lands.
            s.card.add_css_class("lock-handoff");
            for (name, chip) in &s.user_chips {
                ui::set_handoff(
                    chip,
                    Some(if Some(name.as_str()) == picked {
                        ui::Handoff::Picked
                    } else {
                        ui::Handoff::Dropped
                    }),
                );
            }
            // Nothing typed from here on lands anywhere useful.
            s.entry.set_sensitive(false);
        }
        // The switch is fire-and-forget: it can fail, and even when it works
        // this surface is still here afterwards. Arm the way back now, while
        // we know we faded something out.
        let restore = self.clone();
        glib::timeout_add_local_once(handoff() + HANDOFF_RECOVER, move || restore.end_handoff());
        handoff()
    }

    /// Undo [`begin_handoff`]: card back, chips back, typing allowed again.
    /// Fires on a timer rather than on the switch failing, because the common
    /// case isn't failure — it's the surface being returned to later.
    pub fn end_handoff(&self) {
        for s in self.inner.borrow().iter() {
            s.card.remove_css_class("lock-handoff");
            for (_, chip) in &s.user_chips {
                ui::set_handoff(chip, None);
            }
            s.entry.set_sensitive(true);
        }
        self.set_status("", StatusKind::Info);
        // Desensitizing dropped the caret; nothing would take typing otherwise.
        self.focus_entry();
    }

    /// Wrong password: shake every card (CSS keyframe re-trigger).
    pub fn shake(&self) {
        for s in self.inner.borrow().iter() {
            s.field.flash_reject();
            ui::shake(&s.card);
        }
    }

    /// Auth accepted — green flash while the unlock request goes out.
    pub fn flash_success(&self) {
        for s in self.inner.borrow().iter() {
            ui::set_success(&s.card, true);
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
    pub fn show_face(&self, state: Option<ui::FaceState>, label: &str) {
        let visible = state.is_some();
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
                ui::set_face_enter(&s.face_wrap, false);
                continue;
            }
            if arriving {
                // Re-added on the next main-loop turn so the style actually
                // recomputes between removal and addition; adding it back in
                // the same frame would not restart the animation.
                ui::set_face_enter(&s.face_wrap, false);
                let wrap = s.face_wrap.clone();
                glib::idle_add_local_once(move || {
                    ui::set_face_enter(&wrap, true);
                });
            }
            s.face_label.set_label(label);
            ui::set_face_state(&s.face_ring, Some(&s.face_pill), state);
        }
    }

    /// The reader armed, or stood down.
    ///
    /// Lights the mark inside the field, starts the field's pulse, and swaps
    /// the caption's resting sentence for one that names the reader. Nothing
    /// here changes an allocation.
    pub fn set_fp_armed(&self, armed: bool) {
        if self.fp_armed.replace(armed) == armed {
            return;
        }
        for s in self.inner.borrow().iter() {
            s.field.set_fp_armed(armed);
        }
        self.refresh_resting();
    }

    /// Something the reader said about the last touch. Holds the caption for
    /// a beat, then the resting sentence comes back on its own.
    pub fn fp_hint(&self, text: &str) {
        for s in self.inner.borrow().iter() {
            s.caption.hint(text);
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

/// What the password field says while it is empty: the lock's "Password",
/// or on the greeter whose it is, since the card itself names nobody.
fn placeholder(greeter: bool, user: &str) -> String {
    if greeter && !user.is_empty() {
        format!("Password for {user}")
    } else {
        "Password".to_string()
    }
}

/// Clear `row` and (re)build one greeter button per user, wiring each to the
/// shared select callback. Returns the (name, button) handles so the surface
/// can move the selection. Shared by the initial `build_content` and the
/// async `set_user_chips` refill.
///
/// Whether the row is on screen is decided once, by `build_content`, and is
/// deliberately not this function's business — see `set_user_chips`.
fn fill_chip_row(
    row: &gtk4::Box,
    users: &[UserChip],
    active: &str,
    on_select: &OnUserSelect,
    stand: &Rc<dyn Fn(&gtk4::Widget)>,
) -> Vec<(String, gtk4::Button)> {
    while let Some(child) = row.first_child() {
        row.remove(&child);
    }
    let mut chips = Vec::with_capacity(users.len());
    for u in users {
        let chip = user_button(u, u.user == active, stand.as_ref());
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

/// One user under the greeter's card: a flat button with the avatar and the
/// name, built like the lock's "Switch user" and standing on the wallpaper
/// the same way. The avatar's presence dot says the user is signed in, so
/// the card will resume that session rather than start one.
fn user_button(u: &UserChip, active: bool, stand: &dyn Fn(&gtk4::Widget)) -> gtk4::Button {
    let content = ui::hbox(2);
    content.set_valign(gtk4::Align::Center);
    content.append(&ui::avatar(
        &u.user,
        u.icon.as_deref(),
        CHIP_AVATAR_SIZE,
        u.logged_in,
    ));
    content.append(&gtk4::Label::new(Some(&u.user)));
    let btn = ui::button_with(
        ui::Face::Child(content.upcast_ref()),
        ui::Kind::Flat,
        ui::Size::Normal,
    );
    stand(btn.upcast_ref());
    btn.add_css_class("lock-user");
    if u.logged_in {
        btn.set_tooltip_text(Some("Signed in: resumes the running session"));
    }
    ui::set_selected(&btn, active);
    btn
}

/// Lock-mode "Switch user" button — jumps to a greeter instead of offering
/// direct user targets.
///
/// It plays the same handoff beat the greeter's chips do. Without it the only
/// feedback for the press is the VT cut itself, which arrives two D-Bus round
/// trips later and reads as a screen that stopped responding; with it the card
/// is already gone when the cut lands.
///
/// The switch itself never touches this session's lock: `to_greeter` locks our
/// session (a no-op, it already is) and activates a *different* VT. Nothing
/// here can unlock, and the compositor keeps the session hidden throughout.
///
/// It hangs below the card with nothing behind it, and it is the lock
/// screen's whole switching affordance, so it stands on the wallpaper the way
/// the date does (`stand`): unobtrusive is a matter of weight (a flat
/// button), and being unreadable over a bright image is not one colour can
/// fix.
fn build_switch_button(set: &SurfaceSet, stand: &dyn Fn(&gtk4::Widget)) -> gtk4::Button {
    let btn = ui::button(
        &format!("{}  Switch user", icons::SWITCH_USER),
        ui::Kind::Flat,
    );
    stand(btn.upcast_ref());
    btn.add_css_class("lock-switch-user");
    btn.set_halign(gtk4::Align::Center);
    let set = set.clone();
    btn.connect_clicked(move |_| {
        set.set_status("Switching\u{2026}", StatusKind::Info);
        let delay = set.begin_handoff(None);
        glib::timeout_add_local_once(delay, switch_user::to_greeter);
    });
    btn
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

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, severity: Severity) -> StatusItem {
        StatusItem {
            source: "claude",
            id: id.into(),
            severity,
            title: "[norban] a customer's name".into(),
            detail: None,
            since: None,
        }
    }

    #[test]
    fn the_line_counts_and_never_names() {
        let items = [
            item("1", Severity::Attention),
            item("2", Severity::Active),
            item("3", Severity::Active),
            item("4", Severity::Info),
        ];
        assert_eq!(
            session_line(&items),
            Some((true, "1 waiting for you \u{00B7} 2 working".into()))
        );
    }

    #[test]
    fn working_alone_has_no_dot() {
        assert_eq!(
            session_line(&[item("1", Severity::Active)]),
            Some((false, "1 working".into()))
        );
    }

    #[test]
    fn idle_sessions_leave_the_line_empty() {
        assert_eq!(session_line(&[item("1", Severity::Info)]), None);
        assert_eq!(session_line(&[]), None);
    }
}
