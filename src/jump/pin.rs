//! Pins: a workspace kept in sight, live, in a corner of the screen.
//!
//! `swaypplet pin` pins the workspace you are on, or unpins it; `p` in the
//! Super+Tab card does the same for the one selected. Then you go elsewhere,
//! and the pin keeps showing it: a build scrolling, a Claude session
//! thinking, a video.
//!
//! A pin is a workspace view (`view.rs`, the same card the bar's peek shows)
//! on its own small layer surface, at the view's frame cap. A pin hides while its workspace is on a screen, since
//! then the real thing is in sight, and its capture stops with it.
//!
//! That rule made pinning silent: you pin the workspace you are on, which is
//! exactly when its pin hides. So every change says so. Pinning and
//! unpinning show the OSD card on every screen, a new pin shows itself in
//! its corner for a moment before it steps aside, it slides in and out
//! instead of popping, and the bar and the Super+Tab card mark every pinned
//! workspace ([`is_pinned`]). On the pin itself, pointing shows what a click
//! does and a × to unpin; right or middle click also unpins.
//!
//! A pin follows focus: it stands on the screen you are working on, and moves
//! when you move. A layer surface belongs to one output, so moving is
//! rebuilding the surface there.
//!
//! The bar is where pins live when they are not floating (`bar/pins.rs`): a
//! mark while any exist, a popover with every pin live, and a tuck that takes
//! the floating cards away without unpinning anything.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk4::prelude::*;
use gtk4::{gdk, glib};
use gtk4_layer_shell::{Edge, LayerShell as _};

use super::live;
use super::scene::{self, Scene};
use super::view::{self, PIN_GLYPH, UNPIN_GLYPH, View};
use crate::shell::{Namespace, Surface, layer};
use crate::sway::ipc::SwayService;

/// From the screen's corner, clear of the bar.
const MARGIN_RIGHT: i32 = crate::tokens::space(5);
const MARGIN_BOTTOM: i32 = 2 * crate::tokens::space(7);
/// Between stacked pins.
const GAP: i32 = crate::tokens::space(4);
/// A pin's full height on screen: the picture, the footer, the padding.
const PIN_STEP: i32 = view::H + 32 + 16 + GAP;
/// How long a new pin shows itself before its workspace's own screen hides
/// it: long enough to see where it lives.
const INTRODUCE: Duration = Duration::from_millis(1600);
/// How long a workspace the switcher committed to counts as on screen
/// before sway confirms it. The switch lands in tens of milliseconds; the
/// limit is for a switch that never comes (something else moved you first,
/// and the switcher put the row away instead).
const ARRIVAL: Duration = Duration::from_millis(1000);
/// A pin slides in from, and out to, the screen's edge by this much.
const SLIDE_PX: f64 = 48.0;

// ── Who is pinned, for the bar and the Super+Tab card ───────────────────

thread_local! {
    static PINNED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static TUCKED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static LISTENERS: RefCell<Vec<Box<dyn Fn()>>> = const { RefCell::new(Vec::new()) };
    /// The one set of pins, for the bar to act on.
    static HANDLE: RefCell<Option<Pins>> = const { RefCell::new(None) };
}

/// Every pinned workspace, oldest first.
pub fn pinned() -> Vec<String> {
    PINNED.with(|p| p.borrow().clone())
}

/// Whether the floating pins are tucked into the bar.
pub fn tucked() -> bool {
    TUCKED.with(std::cell::Cell::get)
}

/// The process's pins, once the app has made them.
pub fn handle() -> Option<Pins> {
    HANDLE.with(|h| h.borrow().clone())
}

fn notify() {
    LISTENERS.with(|l| {
        for f in l.borrow().iter() {
            f();
        }
    });
}

/// Whether `workspace` is pinned right now.
pub fn is_pinned(workspace: &str) -> bool {
    PINNED.with(|p| p.borrow().iter().any(|w| w == workspace))
}

/// Call `f` whenever a workspace is pinned or unpinned.
pub fn connect_changed(f: impl Fn() + 'static) {
    LISTENERS.with(|l| l.borrow_mut().push(Box::new(f)));
}

fn publish(names: Vec<String>) {
    let changed = PINNED.with(|p| {
        let mut p = p.borrow_mut();
        let changed = *p != names;
        *p = names;
        changed
    });
    if changed {
        notify();
    }
}

/// Go to `workspace`: what a click on its picture does, pinned or peeked.
pub fn go(workspace: &str) {
    crate::sway::ipc::run_command(&format!(
        "workspace \"{}\"",
        workspace.replace('\\', "\\\\").replace('"', "\\\"")
    ));
}

// ── The pins ────────────────────────────────────────────────────────────

/// What a region pin follows: one window, and the piece of it.
#[derive(Clone)]
struct RegionPin {
    id: String,
    con_id: i64,
    crop: live::Crop,
}

struct Pin {
    /// Who the pin is: the workspace's name, or `window:<identifier>` for a
    /// region pin.
    key: String,
    /// The workspace it shows, or the one its window is on; while that is on
    /// a screen, the pin hides.
    workspace: String,
    /// The footer's label.
    label: String,
    /// `Some` for a piece of one window rather than a whole workspace.
    region: Option<RegionPin>,
    surface: Surface,
    /// The picture and its footer; it rebuilds itself when the workspace
    /// changes shape.
    view: View,
    /// Until then the pin shows even though its workspace is on a screen.
    introduce_until: Option<Instant>,
    /// The output its surface is on.
    output: Option<String>,
}

impl Drop for Pin {
    fn drop(&mut self) {
        // The capture before the surface it draws into.
        self.view.stop();
    }
}

type Notice = Box<dyn Fn(&str, &str)>;

#[derive(Default)]
struct Inner {
    app: Option<gtk4::Application>,
    sway: Option<Rc<SwayService>>,
    notice: Option<Notice>,
    pins: Vec<Pin>,
    tucked: bool,
    /// Held out of sight while the Super+Tab switcher is up. Not the same
    /// as tucked: nothing changes in the bar, and they come back on their
    /// own when it closes.
    held: bool,
    /// The last tree read asked for, and the newest one applied. Reads run
    /// on threads of their own and can land out of order; an older one
    /// landing last put back the windows of a moment before, and a window
    /// opened in between stayed out of the pin until sway next said
    /// something.
    reads: Reads,
    /// The workspace the switcher has just committed to, counted as on
    /// screen until sway says it is or [`ARRIVAL`] runs out. See
    /// [`Pins::expect_arrival`].
    arriving: Option<(String, Instant)>,
}

#[derive(Clone, Default)]
pub struct Pins {
    inner: Rc<RefCell<Inner>>,
}

impl Pins {
    pub fn new(app: &gtk4::Application) -> Pins {
        let pins = Pins::default();
        pins.inner.borrow_mut().app = Some(app.clone());
        HANDLE.with(|h| h.replace(Some(pins.clone())));
        pins
    }

    /// Take the floating pins away, or bring them back. Nothing is unpinned:
    /// the bar keeps showing every pin while they are tucked.
    pub fn set_tucked(&self, tucked: bool) {
        self.inner.borrow_mut().tucked = tucked;
        TUCKED.with(|t| t.set(tucked));
        notify();
        self.refresh();
    }

    /// Hide every pin while the Super+Tab switcher is up, and bring them
    /// back after. The switcher's workspaces and its ring are under the
    /// pins' layer, and a pin over them hides part of what you choose from.
    pub fn set_held(&self, held: bool) {
        if self.inner.borrow().held == held {
            return;
        }
        self.inner.borrow_mut().held = held;
        self.refresh();
    }

    /// The switcher committed to `workspace`: its pin stays hidden through
    /// the switch.
    ///
    /// The switcher lets go of the pins as it closes, and the switch it
    /// commits reaches sway a moment later. In between, a pin of the
    /// workspace being switched to saw it off screen, slid in, and slid out
    /// again once the switch landed: a sub-second flash of exactly the pin
    /// the switch makes pointless. Call this before [`Self::set_held`]
    /// `(false)`.
    pub fn expect_arrival(&self, workspace: &str) {
        self.inner.borrow_mut().arriving = Some((workspace.to_string(), Instant::now() + ARRIVAL));
        // Look again once the wait is over, in case no sway event comes.
        let this = self.clone();
        glib::timeout_add_local_once(ARRIVAL + Duration::from_millis(20), move || {
            this.refresh();
        });
    }

    /// Go to a pinned workspace.
    pub fn go(&self, workspace: &str) {
        go(workspace);
    }

    /// Follow sway: rebuild a pin whose workspace changed shape, and hide
    /// the ones whose workspace is on a screen.
    ///
    /// On every window change too, and not only the ones the bar's model
    /// sees: that model leaves out where windows are and which they are, so
    /// a window opened or closed beside another of the same process, moved
    /// within a workspace, resized, or mapped again under a new identifier
    /// changed nothing in it, and the pin kept capturing the windows it had.
    pub fn set_sway(&self, sway: Rc<SwayService>) {
        let this = self.clone();
        sway.connect_windows(move || this.refresh());
        self.inner.borrow_mut().sway = Some(sway);
        self.refresh();
    }

    /// How a pin or an unpin is announced: an icon and a line, which the app
    /// shows on the OSD card.
    pub fn set_notice(&self, notice: impl Fn(&str, &str) + 'static) {
        self.inner.borrow_mut().notice = Some(Box::new(notice));
    }

    /// Pin the focused workspace, or unpin it when it already is.
    pub fn toggle_focused(&self) {
        let this = self.clone();
        crate::spawn::spawn_work(
            || {
                let mut conn = crate::sway::ipc::connect().ok()?;
                let ws = conn
                    .get_workspaces()
                    .ok()?
                    .into_iter()
                    .find(|w| w.focused)?;
                Some((ws.name, ws.output))
            },
            move |found| {
                let Some((name, output)) = found else { return };
                this.toggle_on(name, Some(output));
            },
        );
    }

    /// Pin `workspace` on the focused output, or unpin it. Returns whether
    /// it is pinned afterwards.
    pub fn toggle(&self, workspace: String) -> bool {
        let sway = self.inner.borrow().sway.clone();
        self.toggle_on(
            workspace,
            crate::sway::ipc::focused_output_from(sway.as_deref()),
        )
    }

    fn toggle_on(&self, workspace: String, output: Option<String>) -> bool {
        if self.is_pinned(&workspace) {
            self.unpin(&workspace);
            false
        } else {
            self.pin_on(workspace, output);
            true
        }
    }

    fn is_pinned(&self, workspace: &str) -> bool {
        self.inner.borrow().pins.iter().any(|p| p.key == workspace)
    }

    fn announce(&self, icon: &str, verb: &str, label: &str) {
        if let Some(notice) = &self.inner.borrow().notice {
            notice(icon, &format!("{verb} {label}"));
        }
    }

    fn publish(&self) {
        publish(
            self.inner
                .borrow()
                .pins
                .iter()
                .filter(|p| p.region.is_none())
                .map(|p| p.workspace.clone())
                .collect(),
        );
    }

    fn pin_on(&self, workspace: String, output: Option<String>) {
        let Some(app) = self.inner.borrow().app.clone() else {
            return;
        };
        let monitor = output.as_deref().and_then(layer::monitor_by_connector);
        let label = crate::sway::workspace::label_for_name(&workspace);
        let parts = build_window(&app, monitor.as_ref(), &label);

        self.wire(&parts, &workspace);

        self.inner.borrow_mut().pins.push(Pin {
            key: workspace.clone(),
            workspace: workspace.clone(),
            label: label.clone(),
            region: None,
            surface: parts.surface,
            view: parts.view,
            introduce_until: Some(Instant::now() + INTRODUCE),
            output: output.clone(),
        });
        // A new pin is meant to be seen, and only the new one: see introduce.
        self.introduce();
        self.announce(PIN_GLYPH, "PINNED", &label);
    }

    /// Pin a piece of one window, on `output`.
    pub fn pin_region(&self, region: scene::Region, output: Option<String>) {
        let Some(id) = region.window.id.clone() else {
            return;
        };
        let key = format!("window:{id}");
        if self.is_pinned(&key) {
            self.unpin(&key);
        }
        let Some(app) = self.inner.borrow().app.clone() else {
            return;
        };
        let monitor = output.as_deref().and_then(layer::monitor_by_connector);
        let label = region_label(&region.window.app, &region.workspace);
        let parts = build_window(&app, monitor.as_ref(), &label);
        self.wire(&parts, &key);
        self.inner.borrow_mut().pins.push(Pin {
            key,
            workspace: region.workspace.clone(),
            label: label.clone(),
            region: Some(RegionPin {
                id,
                con_id: region.window.con_id,
                crop: region.frac,
            }),
            surface: parts.surface,
            view: parts.view,
            introduce_until: Some(Instant::now() + INTRODUCE),
            output,
        });
        self.introduce();
        self.announce(PIN_GLYPH, "PINNED", &label);
    }

    /// After a pin is added: restack, publish, draw, and look again once the
    /// introduction is over.
    ///
    /// The tucked pins stay tucked. Pinning used to untuck them all, and
    /// pinning the workspace you are on then left exactly the wrong thing on
    /// screen: the new pin hides once its introduction ends, because its
    /// workspace is the one in front of you, while the others, brought back,
    /// stay. You pinned b and were left looking at a pin of something else.
    /// The new pin shows through its introduction whether the rest are
    /// tucked or not (see `refresh`).
    fn introduce(&self) {
        self.stack();
        self.publish();
        self.refresh();
        // The introduction ends by itself: look again once it has.
        let this = self.clone();
        glib::timeout_add_local_once(INTRODUCE + Duration::from_millis(50), move || {
            this.refresh();
        });
    }

    /// Unpin by key: a workspace's name, or a region pin's `window:<id>`.
    pub fn unpin(&self, key: &str) {
        let pin = {
            let mut inner = self.inner.borrow_mut();
            let Some(i) = inner.pins.iter().position(|p| p.key == key) else {
                return;
            };
            inner.pins.remove(i)
        };
        self.stack();
        self.publish();
        self.announce(UNPIN_GLYPH, "UNPINNED", &pin.label);
        // Slide out, then go. The pin lives in the hook until the exit is
        // over; a pin already hidden goes at once.
        if pin.surface.is_shown() {
            let surface = pin.surface.clone();
            let slot = RefCell::new(Some(pin));
            surface.connect_hidden(move || drop(slot.borrow_mut().take()));
            surface.hide();
        }
    }

    /// Go to what a pin shows: its workspace, or the window a region pin
    /// follows (focusing it switches to its workspace).
    fn go_pin(&self, key: &str) {
        let target = self
            .inner
            .borrow()
            .pins
            .iter()
            .find(|p| p.key == key)
            .map(|p| (p.workspace.clone(), p.region.as_ref().map(|r| r.con_id)));
        match target {
            Some((_, Some(con_id))) => {
                crate::sway::ipc::run_command(&format!("[con_id={con_id}] focus"));
            }
            Some((workspace, None)) => self.go(&workspace),
            None => {}
        }
    }

    /// A click on the picture goes there; right or middle click, or the ×,
    /// unpins.
    fn wire(&self, parts: &Parts, workspace: &str) {
        let close = parts.view.action("\u{00d7}", "Unpin");
        let workspace = workspace.to_string();
        let click = gtk4::GestureClick::new();
        click.set_button(0);
        {
            let this = self.clone();
            let workspace = workspace.clone();
            click.connect_released(move |g, _, _, _| match g.current_button() {
                gdk::BUTTON_PRIMARY => this.go_pin(&workspace),
                gdk::BUTTON_SECONDARY | gdk::BUTTON_MIDDLE => this.unpin(&workspace),
                _ => {}
            });
        }
        parts.view.picture().add_controller(click);
        {
            let this = self.clone();
            let workspace = workspace.clone();
            close.connect_clicked(move |_| this.unpin(&workspace));
        }
    }

    /// Move every pin to `output`, the one with focus: rebuild its surface
    /// there, and let the next update draw its picture on it.
    fn follow(&self, output: Option<&str>) {
        let Some(output) = output else { return };
        let Some(app) = self.inner.borrow().app.clone() else {
            return;
        };
        let moving: Vec<usize> = self
            .inner
            .borrow()
            .pins
            .iter()
            .enumerate()
            .filter(|(_, p)| p.output.as_deref() != Some(output))
            .map(|(i, _)| i)
            .collect();
        if moving.is_empty() {
            return;
        }
        let monitor = layer::monitor_by_connector(output);
        for i in moving {
            let (key, label) = {
                let inner = self.inner.borrow();
                (inner.pins[i].key.clone(), inner.pins[i].label.clone())
            };
            let parts = build_window(&app, monitor.as_ref(), &label);
            self.wire(&parts, &key);
            let mut inner = self.inner.borrow_mut();
            let pin = &mut inner.pins[i];
            // The old surface goes as the new one comes, its capture first.
            pin.view.stop();
            pin.surface = parts.surface;
            pin.view = parts.view;
            pin.output = Some(output.to_string());
        }
        self.stack();
    }

    /// Stack the pins up from the corner, oldest lowest.
    fn stack(&self) {
        for (i, pin) in self.inner.borrow().pins.iter().enumerate() {
            pin.surface
                .window()
                .set_margin(Edge::Bottom, MARGIN_BOTTOM + i as i32 * PIN_STEP);
        }
    }

    /// Read the tree once, and bring every pin up to date with it.
    fn refresh(&self) {
        if self.inner.borrow().pins.is_empty() {
            return;
        }
        let (on_screen, focused_output): (Vec<String>, Option<String>) =
            match &self.inner.borrow().sway {
                Some(sway) => {
                    let snap = sway.snapshot();
                    (
                        snap.workspaces
                            .iter()
                            .filter(|w| w.visible)
                            .map(|w| w.name.clone())
                            .collect(),
                        snap.workspaces
                            .iter()
                            .find(|w| w.focused)
                            .map(|w| w.output.clone()),
                    )
                }
                None => (Vec::new(), None),
            };
        let on_screen = with_arrival(
            on_screen,
            &mut self.inner.borrow_mut().arriving,
            Instant::now(),
        );
        self.follow(focused_output.as_deref());
        let wanted: Vec<(String, Option<String>)> = self
            .inner
            .borrow()
            .pins
            .iter()
            .map(|p| (p.key.clone(), p.region.as_ref().map(|r| r.id.clone())))
            .collect();
        let this = self.clone();
        let read = self.inner.borrow_mut().reads.ask();
        crate::spawn::spawn_work(
            move || {
                let tree = crate::sway::ipc::connect().ok()?.get_tree().ok()?;
                Some(
                    wanted
                        .into_iter()
                        .map(|(key, window)| {
                            let found = match window {
                                None => scene::scene(&tree, &key).map(Found::Workspace),
                                Some(id) => scene::find_window(&tree, &id)
                                    .map(|(w, ws)| Found::Window(w, ws)),
                            };
                            (key, found)
                        })
                        .collect::<Vec<_>>(),
                )
            },
            move |found| {
                let Some(found) = found else { return };
                let mut inner = this.inner.borrow_mut();
                if !inner.reads.land(read) {
                    return;
                }
                let tucked = inner.tucked;
                let held = inner.held;
                // A workspace or a window that is gone takes its pin with it.
                let before = inner.pins.len();
                inner
                    .pins
                    .retain(|p| found.iter().any(|(k, f)| *k == p.key && f.is_some()));
                let gone = inner.pins.len() != before;
                let now = Instant::now();
                for pin in &mut inner.pins {
                    let Some(f) = found
                        .iter()
                        .find(|(k, _)| *k == pin.key)
                        .and_then(|(_, f)| f.clone())
                    else {
                        continue;
                    };
                    if let Found::Window(_, ws) = &f {
                        pin.workspace = ws.clone();
                    }
                    let introducing = pin.introduce_until.is_some_and(|t| now < t);
                    if !introducing {
                        pin.introduce_until = None;
                    }
                    let show =
                        !held && (introducing || (!tucked && !on_screen.contains(&pin.workspace)));
                    match f {
                        Found::Workspace(scene) => update(pin, Some(scene), show),
                        Found::Window(window, _) => update_region(pin, &window, show),
                    }
                }
                drop(inner);
                this.stack();
                if gone {
                    this.publish();
                }
            },
        );
    }
}

/// The workspaces on screen, `visible` plus one the switcher is switching
/// to, while it is still on its way: until `visible` has it, or its time is
/// up, after which it is forgotten.
fn with_arrival(
    mut visible: Vec<String>,
    arriving: &mut Option<(String, Instant)>,
    now: Instant,
) -> Vec<String> {
    if let Some((name, until)) = arriving.as_ref() {
        if visible.contains(name) || now >= *until {
            *arriving = None;
        } else {
            visible.push(name.clone());
        }
    }
    visible
}

/// Tree reads in the order they were asked for.
#[derive(Default)]
struct Reads {
    asked: u64,
    applied: u64,
}

impl Reads {
    /// A new read's number.
    fn ask(&mut self) -> u64 {
        self.asked += 1;
        self.asked
    }

    /// Whether read `n`, just landed, is newer than every one applied; if
    /// so it counts as applied.
    fn land(&mut self, n: u64) -> bool {
        if n <= self.applied {
            return false;
        }
        self.applied = n;
        true
    }
}

/// What a refresh found for a pin.
#[derive(Clone)]
enum Found {
    Workspace(Scene),
    Window(scene::Window, String),
}

/// Hide a pin whose workspace is on a screen.
fn hide(pin: &mut Pin) {
    pin.view.stop();
    // Only a pin that is shown has anything to hide. A surface `follow` has
    // just rebuilt on another output was never shown, so it was never
    // realized, and Reveal's exit path on it reached for a GDK surface that
    // did not exist: the process went down, and every pin with it. That was
    // "switching to a pinned workspace unpins it", whenever the switch also
    // moved focus to the other screen.
    if pin.surface.is_shown() {
        pin.surface.hide();
    }
}

/// Show or hide a region pin; its view rebuilds when the window resized.
fn update_region(pin: &mut Pin, window: &scene::Window, show: bool) {
    if !show {
        hide(pin);
        return;
    }
    let Some(region) = pin.region.as_ref() else {
        return;
    };
    pin.view
        .show_region(&region.id, region.crop, (window.w, window.h), view::FPS);
    if !pin.surface.is_shown() {
        pin.surface.show();
    }
}

/// Show or hide a pin; its view rebuilds when the scene changed.
fn update(pin: &mut Pin, scene: Option<Scene>, show: bool) {
    if !show {
        hide(pin);
        return;
    }
    pin.view.show_scene(scene, view::FPS);
    if !pin.surface.is_shown() {
        pin.surface.show();
    }
}

/// A region pin's label: the app, and where its window is.
fn region_label(app: &str, workspace: &str) -> String {
    format!(
        "{app} \u{00b7} {}",
        crate::sway::workspace::label_for_name(workspace)
    )
}

struct Parts {
    surface: Surface,
    view: View,
}

fn build_window(app: &gtk4::Application, monitor: Option<&gdk::Monitor>, label: &str) -> Parts {
    let surface = Surface::builder(app, Namespace::Pin)
        .monitor(monitor)
        // Top, not Overlay: a fullscreen window covers a pin, the way it
        // covers the bar.
        .layer(gtk4_layer_shell::Layer::Top)
        .anchor(&[Edge::Bottom, Edge::Right])
        // No right margin on the surface: it reaches the screen's edge, and
        // the gap to the card is the card's own (below). The entrance slides
        // the card toward that edge and back, and a card moving inside its
        // surface is what the compositor's glass follows (the pin
        // namespace's glass entry masks it to the card's pixels, as the
        // notifications' does); a surface that ended at the card clipped it
        // instead, and its glass stood still.
        .margin(Edge::Bottom, MARGIN_BOTTOM)
        .card(crate::ui::Card::Floating)
        // Slides in from the screen's edge and fades up, and leaves the same
        // way (anim::Reveal, the shell's one entrance).
        .slide(gtk4::Orientation::Horizontal, SLIDE_PX)
        .build();
    let frame = surface.card();
    frame.add_css_class("jump-pin");
    if let Some(slide) = surface.slide() {
        slide.set_margin_end(MARGIN_RIGHT);
    }

    let mut view = View::new(label, "click to go \u{00b7} right-click to unpin");
    view.set_scale(f64::from(monitor.map_or(1, |m| m.scale_factor())));
    frame.append(view.widget());
    surface.set_content(view.widget());

    Parts { surface, view }
}

#[cfg(test)]
mod tests {
    use super::{Reads, with_arrival};
    use std::time::{Duration, Instant};

    #[test]
    fn a_workspace_on_its_way_counts_as_on_screen_until_it_lands() {
        let now = Instant::now();
        let later = now + Duration::from_millis(1000);
        let mut arriving = Some(("7".to_string(), later));
        // Switch not landed yet: 7 counts as shown, so its pin stays hidden.
        let shown = with_arrival(vec!["1".into()], &mut arriving, now);
        assert_eq!(shown, ["1", "7"]);
        assert!(arriving.is_some());
        // Landed: sway reports it, and the expectation is spent.
        let shown = with_arrival(vec!["7".into()], &mut arriving, now);
        assert_eq!(shown, ["7"]);
        assert!(arriving.is_none());
        // Never landed: forgotten once its time is up.
        let mut arriving = Some(("7".to_string(), later));
        let shown = with_arrival(vec!["1".into()], &mut arriving, later);
        assert_eq!(shown, ["1"]);
        assert!(arriving.is_none());
    }

    #[test]
    fn a_read_that_lands_after_a_newer_one_is_dropped() {
        let mut reads = Reads::default();
        let (a, b, c) = (reads.ask(), reads.ask(), reads.ask());
        assert!(reads.land(b));
        assert!(!reads.land(a), "older than one applied");
        assert!(reads.land(c));
        assert!(!reads.land(c), "applied once");
    }
}
