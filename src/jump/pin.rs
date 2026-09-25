//! Pins: a workspace kept in sight, live, in a corner of the screen.
//!
//! `swaypplet pin` pins the workspace you are on, or unpins it; `p` in the
//! Super+Tab card does the same for the one selected. Then you go elsewhere,
//! and the pin keeps showing it: a build scrolling, a Claude session
//! thinking, a video.
//!
//! A pin is the Super+Tab picture (`card::preview`) on its own small layer
//! surface, with no frame cap: every frame the compositor renders for a
//! window reaches it. A pin hides while its workspace is on a screen, since
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

use super::card::{self, Live};
use super::live;
use super::scene::{self, Scene};
use crate::layer_shell::{self, LayerShellConfig};
use crate::sway_ipc::SwayService;

/// The picture, 16:10.
const PIN_W: i32 = 400;
const PIN_H: i32 = 250;
/// From the screen's corner, clear of the bar.
const MARGIN_RIGHT: i32 = 16;
const MARGIN_BOTTOM: i32 = 64;
/// Between stacked pins.
const GAP: i32 = 12;
/// A pin's full height on screen: the picture, the footer, the padding.
const PIN_STEP: i32 = PIN_H + 32 + 16 + GAP;
/// How long a new pin shows itself before its workspace's own screen hides
/// it: long enough to see where it lives.
const INTRODUCE: Duration = Duration::from_millis(1600);
/// A pin slides in from, and out to, the screen's edge by this much.
const SLIDE_PX: f64 = 48.0;

const PIN_GLYPH: &str = "\u{f0403}";
const UNPIN_GLYPH: &str = "\u{f0404}";

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

// ── The pins ────────────────────────────────────────────────────────────

/// What a region pin follows: one window, and the piece of it.
#[derive(Clone)]
struct RegionPin {
    id: String,
    con_id: i64,
    crop: live::Crop,
    /// The window's size at the last rebuild, so a resize rebuilds.
    size: (i32, i32),
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
    window: gtk4::Window,
    reveal: crate::anim::Reveal,
    /// Where the picture goes; rebuilt when the workspace changes shape.
    holder: gtk4::Box,
    live: Rc<RefCell<Live>>,
    stream: Option<live::Stream>,
    scene: Option<Scene>,
    /// Until then the pin shows even though its workspace is on a screen.
    introduce_until: Option<Instant>,
    /// The output its surface is on.
    output: Option<String>,
}

impl Drop for Pin {
    fn drop(&mut self) {
        self.stream = None;
        // The alpha handle goes before its `wl_surface`.
        self.reveal.release_alpha();
        crate::layer_shell::destroy_window(&self.window);
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

    /// Go to a pinned workspace.
    pub fn go(&self, workspace: &str) {
        crate::sway_ipc::run_command(&format!(
            "workspace \"{}\"",
            workspace.replace('\\', "\\\\").replace('"', "\\\"")
        ));
    }

    /// Follow sway: rebuild a pin whose workspace changed shape, and hide
    /// the ones whose workspace is on a screen.
    pub fn set_sway(&self, sway: Rc<SwayService>) {
        let this = self.clone();
        sway.connect_change(move || this.refresh());
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
                let mut conn = crate::sway_ipc::connect().ok()?;
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
        self.toggle_on(workspace, crate::sway_ipc::focused_output())
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
        let monitor = output
            .as_deref()
            .and_then(layer_shell::monitor_by_connector);
        let label = pin_label(&workspace);
        let parts = build_window(&app, monitor.as_ref(), &label);

        self.wire(&parts, &workspace);

        self.inner.borrow_mut().pins.push(Pin {
            key: workspace.clone(),
            workspace: workspace.clone(),
            label: label.clone(),
            region: None,
            window: parts.window,
            reveal: parts.reveal,
            holder: parts.holder,
            live: Rc::default(),
            stream: None,
            scene: None,
            introduce_until: Some(Instant::now() + INTRODUCE),
            output: output.clone(),
        });
        // A new pin is meant to be seen: pinning brings the tucked ones back.
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
        let monitor = output
            .as_deref()
            .and_then(layer_shell::monitor_by_connector);
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
                size: (0, 0),
            }),
            window: parts.window,
            reveal: parts.reveal,
            holder: parts.holder,
            live: Rc::default(),
            stream: None,
            scene: None,
            introduce_until: Some(Instant::now() + INTRODUCE),
            output,
        });
        self.introduce();
        self.announce(PIN_GLYPH, "PINNED", &label);
    }

    /// After a pin is added: untuck, restack, publish, draw, and look again
    /// once the introduction is over.
    fn introduce(&self) {
        if self.inner.borrow().tucked {
            self.inner.borrow_mut().tucked = false;
            TUCKED.with(|t| t.set(false));
        }
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
        if pin.reveal.is_shown() {
            let reveal = pin.reveal.clone();
            let slot = RefCell::new(Some(pin));
            reveal.connect_hidden(move || drop(slot.borrow_mut().take()));
            reveal.hide();
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
                crate::sway_ipc::run_command(&format!("[con_id={con_id}] focus"));
            }
            Some((workspace, None)) => self.go(&workspace),
            None => {}
        }
    }

    /// A click on the picture goes there; right or middle click, or the ×,
    /// unpins.
    fn wire(&self, parts: &Parts, workspace: &str) {
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
        parts.holder.add_controller(click);
        {
            let this = self.clone();
            let workspace = workspace.clone();
            parts.close.connect_clicked(move |_| this.unpin(&workspace));
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
        let monitor = layer_shell::monitor_by_connector(output);
        for i in moving {
            let (key, label) = {
                let inner = self.inner.borrow();
                (inner.pins[i].key.clone(), inner.pins[i].label.clone())
            };
            let parts = build_window(&app, monitor.as_ref(), &label);
            self.wire(&parts, &key);
            let mut inner = self.inner.borrow_mut();
            let pin = &mut inner.pins[i];
            // The old surface goes as the new one comes: Drop order on the
            // replaced fields is not Drop of the pin, so release by hand.
            pin.stream = None;
            pin.reveal.release_alpha();
            crate::layer_shell::destroy_window(&pin.window);
            pin.window = parts.window;
            pin.reveal = parts.reveal;
            pin.holder = parts.holder;
            pin.scene = None;
            if let Some(region) = &mut pin.region {
                region.size = (0, 0);
            }
            pin.output = Some(output.to_string());
        }
        self.stack();
    }

    /// Stack the pins up from the corner, oldest lowest.
    fn stack(&self) {
        for (i, pin) in self.inner.borrow().pins.iter().enumerate() {
            pin.window
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
        self.follow(focused_output.as_deref());
        let wanted: Vec<(String, Option<String>)> = self
            .inner
            .borrow()
            .pins
            .iter()
            .map(|p| (p.key.clone(), p.region.as_ref().map(|r| r.id.clone())))
            .collect();
        let this = self.clone();
        crate::spawn::spawn_work(
            move || {
                let tree = crate::sway_ipc::connect().ok()?.get_tree().ok()?;
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
                let tucked = inner.tucked;
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
                    let show = !tucked && (introducing || !on_screen.contains(&pin.workspace));
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

/// What a refresh found for a pin.
#[derive(Clone)]
enum Found {
    Workspace(Scene),
    Window(scene::Window, String),
}

/// Hide a pin whose workspace is on a screen.
fn hide(pin: &mut Pin) {
    pin.stream = None;
    // Only a pin that is shown has anything to hide. A surface `follow` has
    // just rebuilt on another output was never shown, so it was never
    // realized, and Reveal's exit path on it reached for a GDK surface that
    // did not exist: the process went down, and every pin with it. That was
    // "switching to a pinned workspace unpins it", whenever the switch also
    // moved focus to the other screen.
    if pin.reveal.is_shown() {
        pin.reveal.hide();
    }
}

/// Show or hide a region pin, and rebuild its picture when its window
/// changed size.
fn update_region(pin: &mut Pin, window: &scene::Window, show: bool) {
    if !show {
        hide(pin);
        return;
    }
    let Some(region) = pin.region.as_mut() else {
        return;
    };
    let size = (window.w, window.h);
    if region.size != size || pin.stream.is_none() {
        region.size = size;
        while let Some(child) = pin.holder.first_child() {
            pin.holder.remove(&child);
        }
        // The piece's own shape, fitted into the pin's box.
        let (_, _, fw, fh) = region.crop;
        let (pw, ph) = (f64::from(window.w) * fw, f64::from(window.h) * fh);
        let (s, _, _) = scene::fit(pw.round() as i32, ph.round() as i32, PIN_W, PIN_H);
        let picture = card::LivePicture::new();
        picture.set_size_request(
            ((pw * s).round() as i32).max(8),
            ((ph * s).round() as i32).max(8),
        );
        picture.set_halign(gtk4::Align::Center);
        pin.holder.append(&picture);
        *pin.live.borrow_mut() = Live::default();
        pin.live.borrow_mut().add(region.id.clone(), picture);

        let (tx, rx) = async_channel::unbounded::<live::Frame>();
        let live = pin.live.clone();
        glib::spawn_future_local(async move {
            while let Ok(frame) = rx.recv().await {
                live.borrow().frame(frame);
            }
        });
        pin.stream = Some(live::Stream::start_region(
            region.id.clone(),
            region.crop,
            (PIN_W * 2) as u32,
            0,
            tx,
        ));
    }
    if !pin.reveal.is_shown() {
        pin.reveal.show();
    }
}

/// Show or hide a pin, and rebuild its picture when its scene changed.
fn update(pin: &mut Pin, scene: Option<Scene>, show: bool) {
    if !show {
        hide(pin);
        return;
    }
    let changed = pin.scene != scene;
    if changed || pin.stream.is_none() {
        while let Some(child) = pin.holder.first_child() {
            pin.holder.remove(&child);
        }
        *pin.live.borrow_mut() = Live::default();
        let picture = card::preview(scene.as_ref(), PIN_W, PIN_H, &mut pin.live.borrow_mut());
        pin.holder.append(&picture);
        pin.scene = scene;

        let ids = pin.live.borrow().window_ids();
        pin.stream = None;
        if !ids.is_empty() {
            let (tx, rx) = async_channel::unbounded::<live::Frame>();
            let live = pin.live.clone();
            glib::spawn_future_local(async move {
                while let Ok(frame) = rx.recv().await {
                    live.borrow().frame(frame);
                }
            });
            // No frame cap: a pin is watched, and is small enough to afford
            // every frame.
            pin.stream = Some(live::Stream::start(ids, (PIN_W * 2) as u32, 0, tx));
        }
    }
    if !pin.reveal.is_shown() {
        pin.reveal.show();
    }
}

/// A region pin's label: the app, and where its window is.
fn region_label(app: &str, workspace: &str) -> String {
    format!("{app} \u{00b7} {}", pin_label(workspace))
}

/// The Super+Tab tile's label for the workspace, so a pin and its tile
/// agree.
fn pin_label(workspace: &str) -> String {
    let num: i32 = workspace
        .split(':')
        .next()
        .and_then(|n| n.parse().ok())
        .unwrap_or(-1);
    super::rows::label_for(&super::place::Place {
        num,
        name: workspace.to_string(),
        output: String::new(),
    })
}

struct Parts {
    window: gtk4::Window,
    reveal: crate::anim::Reveal,
    holder: gtk4::Box,
    close: gtk4::Button,
}

fn build_window(app: &gtk4::Application, monitor: Option<&gdk::Monitor>, label: &str) -> Parts {
    static CONFIG: LayerShellConfig = LayerShellConfig {
        namespace: "swaypplet-pin",
        // Top, not Overlay: a fullscreen window covers a pin, the way it
        // covers the bar.
        layer: gtk4_layer_shell::Layer::Top,
        exclusive: false,
        default_width: None,
        default_height: None,
        anchors: &[(Edge::Bottom, true), (Edge::Right, true)],
        // No right margin on the surface: it reaches the screen's edge, and
        // the gap to the card is the card's own (below). The entrance slides
        // the card toward that edge and back, and a card moving inside its
        // surface is what the compositor's glass follows (the pin namespace's
        // glass entry masks it to the card's pixels, as the notifications'
        // does); a surface that ended at the card clipped it instead, and its
        // glass stood still.
        margins: &[(Edge::Bottom, MARGIN_BOTTOM)],
        keyboard_mode: gtk4_layer_shell::KeyboardMode::None,
    };
    let window = layer_shell::create_layer_window_on(app, &CONFIG, monitor);
    window.set_resizable(false);
    window.set_decorated(false);

    let frame = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(6)
        .build();
    frame.add_css_class("glass-card");
    frame.add_css_class("jump-pin");

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 6);

    // The picture: a click goes there, and the pointer says so.
    let holder = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    holder.add_css_class("jump-pin-picture");
    holder.set_cursor_from_name(Some("pointer"));
    content.append(&holder);

    // The footer: the workspace, what a click does, and the way out. The
    // hint and the × are quiet until the pointer is on the pin.
    let footer = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    footer.add_css_class("jump-pin-footer");
    let mark = gtk4::Label::new(Some(PIN_GLYPH));
    mark.add_css_class("jump-pin-mark");
    footer.append(&mark);
    let label = gtk4::Label::builder().label(label).xalign(0.0).build();
    label.add_css_class("jump-pin-label");
    footer.append(&label);
    let hint = gtk4::Label::builder()
        .label("click to go · right-click to unpin")
        .xalign(1.0)
        .hexpand(true)
        .build();
    hint.add_css_class("jump-pin-hint");
    footer.append(&hint);
    let close = gtk4::Button::builder()
        .label("\u{00d7}")
        .tooltip_text("Unpin")
        .build();
    close.add_css_class("flat");
    close.add_css_class("jump-pin-close");
    footer.append(&close);
    content.append(&footer);

    frame.append(&content);

    // Slides in from the screen's edge and fades up, and leaves the same
    // way (anim::Reveal, the shell's one entrance).
    let slide = crate::anim::SlideBin::horizontal();
    slide.set_child(&frame);
    slide.set_margin_end(MARGIN_RIGHT);
    window.set_child(Some(&slide));
    let reveal = crate::anim::Reveal::new(&window, &frame)
        .content(&content)
        .slide(&slide, SLIDE_PX);

    Parts {
        window,
        reveal,
        holder,
        close,
    }
}
