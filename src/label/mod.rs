//! Workspace labels: a name for the workspace you are on, in a glass pill at
//! the top middle of the screen.
//!
//! The label is free text, kept per workspace name. Three modes say when the
//! pill shows without being asked:
//!
//! | mode     | the pill                                                        |
//! |----------|-----------------------------------------------------------------|
//! | `hidden` | never on its own; the name is only seen when you summon it      |
//! | `auto`   | for a moment after you switch to a workspace that has a name    |
//! | `always` | stays up on every output whose workspace has a name             |
//!
//! A workspace with no name never shows on its own, in any mode: an empty
//! pill on every switch would be noise, and the name is what the pill is for.
//!
//! Two doors summon it. `swaypplet label` (a keybind) opens the pill on the
//! focused output with the text field focused, and a second press saves and
//! closes it. The mouse rests in a thin strip at the top middle of the output
//! (`Edge`) for [`DWELL_MS`] and the pill appears; a click on it starts the
//! edit. The mode and a remove button live in the pill while it is hovered or
//! edited, so the pill alone is enough to change everything it does.
//!
//! One pill per output, each showing the workspace visible on that output
//! (a layer surface is bound to its output, `shell::PerMonitor`). The edit
//! belongs to one workspace, fixed when it starts: if sway moves that output
//! to another workspace mid-edit, the text is saved to the first one and the
//! edit ends.
//!
//! The text field takes the keyboard exclusively while it edits, which steals
//! every key, so an edit that sees no typing for [`EDIT_IDLE_MS`] saves and
//! closes itself. Enter saves, Escape drops the change.
//!
//! The strip is hidden while a window is fullscreen on the focused output:
//! a four-pixel band over a video's top edge would still catch the pointer.
//!
//! State lives in `$XDG_STATE_HOME/swaypplet/workspace-labels.json`, written
//! only by this process (the CLI is a request to it, as `jump` is). It is
//! state and not a setting: a person fills it in all day, and nothing in it
//! has a Nix default.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::time::Duration;

use gtk4::gdk;
use gtk4::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, LayerShell as _};
use serde_json::{Value, json};

use crate::shell::{Namespace, PerMonitor, Surface};
use crate::sway::ipc::SwayService;
use crate::ui;

/// How long the pill stays after a workspace switch in `auto`.
const AUTO_MS: u64 = 2500;
/// How long it stays after the pointer leaves it.
const LEAVE_MS: u64 = 1200;
/// How long it stays after the strip summons it or a mode changes, if the
/// pointer never reaches it.
const PEEK_MS: u64 = 3000;
/// How long the pointer rests in the strip before the pill comes.
const DWELL_MS: u64 = 120;
/// An edit with no typing for this long saves and closes (module docs).
const EDIT_IDLE_MS: u64 = 20_000;
/// The strip the pointer rests in: wide enough to find, thin enough that a
/// window's top edge underneath it stays reachable.
const EDGE_WIDTH: i32 = 480;
const EDGE_HEIGHT: i32 = 8;
const PLACEHOLDER: &str = "Name this workspace";

// ── The saved state ─────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Mode {
    Hidden,
    #[default]
    Auto,
    Always,
}

impl Mode {
    const ALL: [Mode; 3] = [Mode::Hidden, Mode::Auto, Mode::Always];

    fn as_str(self) -> &'static str {
        match self {
            Mode::Hidden => "hidden",
            Mode::Auto => "auto",
            Mode::Always => "always",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Mode::Hidden => "Hidden",
            Mode::Auto => "Auto",
            Mode::Always => "Always",
        }
    }

    fn parse(s: &str) -> Option<Mode> {
        Mode::ALL.into_iter().find(|m| m.as_str() == s)
    }

    /// The mode a key that cycles lands on.
    fn next(self) -> Mode {
        match self {
            Mode::Hidden => Mode::Auto,
            Mode::Auto => Mode::Always,
            Mode::Always => Mode::Hidden,
        }
    }
}

#[derive(Default, Debug, PartialEq, Eq)]
struct Data {
    mode: Mode,
    /// Workspace name → label. Never holds an empty label.
    labels: BTreeMap<String, String>,
}

impl Data {
    fn parse(raw: &[u8]) -> Data {
        let Ok(value) = serde_json::from_slice::<Value>(raw) else {
            return Data::default();
        };
        Data {
            mode: value["mode"]
                .as_str()
                .and_then(Mode::parse)
                .unwrap_or_default(),
            labels: value["labels"]
                .as_object()
                .into_iter()
                .flatten()
                .filter_map(|(ws, text)| {
                    let text = text.as_str()?.trim();
                    (!text.is_empty()).then(|| (ws.clone(), text.to_string()))
                })
                .collect(),
        }
    }

    fn to_json(&self) -> String {
        let text = json!({ "mode": self.mode.as_str(), "labels": self.labels });
        serde_json::to_string_pretty(&text).unwrap_or_default()
    }

    fn path() -> PathBuf {
        let dir = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
            .unwrap_or_else(|| PathBuf::from("."));
        dir.join("swaypplet").join("workspace-labels.json")
    }

    fn load() -> Data {
        std::fs::read(Self::path())
            .map(|raw| Data::parse(&raw))
            .unwrap_or_default()
    }

    /// Written whole through a rename, so a crash mid-write leaves the old
    /// file and never half of the new one.
    fn save(&self) {
        let path = Self::path();
        let write = || -> std::io::Result<()> {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let tmp = path.with_extension("json.tmp");
            std::fs::write(&tmp, self.to_json())?;
            std::fs::rename(&tmp, &path)
        };
        if let Err(e) = write() {
            log::warn!("label: could not save {}: {e}", path.display());
        }
    }
}

// ── One pill ────────────────────────────────────────────────────────────

/// The pill on one output.
struct Pill {
    connector: String,
    surface: Surface,
    stack: gtk4::Stack,
    shown: gtk4::Label,
    entry: gtk4::Entry,
    controls: gtk4::Box,
    modes: Vec<(Mode, gtk4::Button)>,
    clear: gtk4::Button,
}

impl Pill {
    fn new(app: &gtk4::Application, monitor: &gdk::Monitor, owner: Weak<Inner>) -> Pill {
        let connector = monitor
            .connector()
            .map(|c| c.to_string())
            .unwrap_or_default();
        let surface = Surface::builder(app, Namespace::Label)
            .monitor(Some(monitor))
            .anchor(&[Edge::Top])
            .card(ui::Card::Thin)
            .build();

        // The transparent apron the glass clips inside (see the OSD).
        let wrapper = surface.root();
        wrapper.set_halign(gtk4::Align::Center);
        wrapper.set_valign(gtk4::Align::Start);
        wrapper.add_css_class("label-wrapper");

        let outer = surface.card();
        outer.set_halign(gtk4::Align::Center);
        outer.add_css_class("label-pill");

        let shown = gtk4::Label::builder()
            .xalign(0.5)
            .hexpand(true)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .max_width_chars(40)
            .build();
        ui::set_text_style(&shown, ui::Text::Title, ui::Tone::Fg);

        let entry = gtk4::Entry::builder()
            .placeholder_text(PLACEHOLDER)
            .max_length(80)
            .width_chars(28)
            .hexpand(true)
            .build();
        ui::entry::adopt(&entry, ui::FieldSize::Normal);

        let stack = gtk4::Stack::new();
        stack.add_named(&shown, Some("read"));
        stack.add_named(&entry, Some("edit"));
        stack.set_visible_child_name("read");
        stack.set_hexpand(true);

        // Mode and remove: in the pill, so the pill alone changes everything
        // it does. Shown while the pill is hovered or edited.
        let track = ui::segmented();
        let mut modes = Vec::new();
        for mode in Mode::ALL {
            let button = gtk4::Button::with_label(mode.title());
            ui::segment::adopt(&button, true);
            track.append(&button);
            let owner = owner.clone();
            button.connect_clicked(move |_| {
                if let Some(owner) = owner.upgrade() {
                    Labels { inner: owner }.set_mode(mode);
                }
            });
            modes.push((mode, button));
        }
        let clear = ui::button_with(
            ui::Face::Glyph {
                glyph: ui::icons::CLOSE,
                tooltip: "Remove the name",
            },
            ui::Kind::Flat,
            ui::Size::Normal,
        );
        let controls = ui::hbox(3);
        controls.append(&track);
        controls.append(&clear);
        controls.set_visible(false);

        let content = ui::hbox(4);
        content.append(&stack);
        content.append(&controls);
        outer.append(&content);
        surface.set_content(&content);

        // Click on the name: edit it.
        let click = gtk4::GestureClick::new();
        {
            let owner = owner.clone();
            let connector = connector.clone();
            click.connect_released(move |_, _, _, _| {
                if let Some(owner) = owner.upgrade() {
                    Labels { inner: owner }.begin_edit(&connector);
                }
            });
        }
        shown.add_controller(click);

        // The pointer on the pill keeps it up; leaving starts the countdown.
        let motion = gtk4::EventControllerMotion::new();
        {
            let owner = owner.clone();
            let connector = connector.clone();
            motion.connect_enter(move |_, _, _| {
                if let Some(owner) = owner.upgrade() {
                    Labels { inner: owner }.pointer(&connector, true);
                }
            });
        }
        {
            let owner = owner.clone();
            let connector = connector.clone();
            motion.connect_leave(move |_| {
                if let Some(owner) = owner.upgrade() {
                    Labels { inner: owner }.pointer(&connector, false);
                }
            });
        }
        outer.add_controller(motion);

        // Enter saves (the entry's own signal), Escape drops.
        let keys = gtk4::EventControllerKey::new();
        {
            let owner = owner.clone();
            keys.connect_key_pressed(move |_, key, _, _| {
                if key != gdk::Key::Escape {
                    return glib::Propagation::Proceed;
                }
                if let Some(owner) = owner.upgrade() {
                    Labels { inner: owner }.finish_edit(false);
                }
                glib::Propagation::Stop
            });
        }
        entry.add_controller(keys);
        {
            let owner = owner.clone();
            entry.connect_activate(move |_| {
                if let Some(owner) = owner.upgrade() {
                    Labels { inner: owner }.finish_edit(true);
                }
            });
        }
        {
            let owner = owner.clone();
            entry.connect_changed(move |_| {
                if let Some(owner) = owner.upgrade() {
                    Labels { inner: owner }.arm_edit_idle();
                }
            });
        }
        {
            let connector = connector.clone();
            clear.connect_clicked(move |_| {
                if let Some(owner) = owner.upgrade() {
                    Labels { inner: owner }.clear(&connector);
                }
            });
        }

        Pill {
            connector,
            surface,
            stack,
            shown,
            entry,
            controls,
            modes,
            clear,
        }
    }

    /// Draw `text` (empty: the prompt), `mode`, and whether the controls show.
    fn draw(&self, text: &str, mode: Mode, controls: bool) {
        if text.is_empty() {
            self.shown.set_label(PLACEHOLDER);
            ui::set_text_style(&self.shown, ui::Text::Title, ui::Tone::Faint);
        } else {
            self.shown.set_label(text);
            ui::set_text_style(&self.shown, ui::Text::Title, ui::Tone::Fg);
        }
        self.controls.set_visible(controls);
        self.clear.set_visible(!text.is_empty());
        for (m, button) in &self.modes {
            let state = if *m == mode {
                ui::Selection::Current
            } else {
                ui::Selection::Idle
            };
            ui::set_selection(button, state);
        }
    }
}

/// The strip at the top middle of an output that summons the pill.
struct EdgeZone {
    surface: Surface,
}

impl EdgeZone {
    fn new(app: &gtk4::Application, monitor: &gdk::Monitor, owner: Weak<Inner>) -> EdgeZone {
        let connector = monitor
            .connector()
            .map(|c| c.to_string())
            .unwrap_or_default();
        // As small as its content: a layer surface with no default size is
        // 200 by 200, which would catch the pointer over a stretch of every
        // window's title bar. A default of 1 by 1 is gtk4-layer-shell's
        // documented way to shrink to the content, which is the strip.
        let surface = Surface::builder(app, Namespace::LabelEdge)
            .monitor(Some(monitor))
            .anchor(&[Edge::Top])
            .width(1)
            .height(1)
            .resizable(false)
            .no_card()
            .build();
        // Painted, if barely: a surface with nothing to draw never got its
        // second size request, and stayed at gtk4-layer-shell's initial
        // 200 by 200. A component's hairline at 1% does it.
        let strip = ui::separator(gtk4::Orientation::Horizontal);
        strip.set_opacity(0.01);
        strip.set_size_request(EDGE_WIDTH, EDGE_HEIGHT);
        surface.root().append(&strip);

        // Rest here for DWELL_MS and the pill comes; passing through does not.
        let dwell: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
        let motion = gtk4::EventControllerMotion::new();
        {
            let dwell = dwell.clone();
            motion.connect_enter(move |_, _, _| {
                log::debug!("label: pointer in the strip");
                let owner = owner.clone();
                let connector = connector.clone();
                let slot = dwell.clone();
                let id = glib::timeout_add_local_once(Duration::from_millis(DWELL_MS), move || {
                    slot.borrow_mut().take();
                    log::debug!("label: dwell over");
                    if let Some(owner) = owner.upgrade() {
                        Labels { inner: owner }.peek(&connector, true, PEEK_MS);
                    }
                });
                if let Some(old) = dwell.borrow_mut().replace(id) {
                    crate::spawn::remove_source(old);
                }
            });
        }
        motion.connect_leave(move |_| {
            log::debug!("label: pointer left the strip");
            if let Some(id) = dwell.borrow_mut().take() {
                crate::spawn::remove_source(id);
            }
        });
        // On the window, not the root: the controller then sees the pointer
        // wherever it is on the surface, with or without a child under it.
        surface.window().add_controller(motion);
        surface.show();

        EdgeZone { surface }
    }
}

// ── The service ─────────────────────────────────────────────────────────

struct Inner {
    sway: Rc<SwayService>,
    pills: Rc<PerMonitor<Pill>>,
    edges: Rc<PerMonitor<EdgeZone>>,
    data: RefCell<Data>,
    /// Output → the workspace visible on it at the last sway event.
    visible: RefCell<HashMap<String, String>>,
    /// The output being edited and the workspace the edit belongs to.
    editing: RefCell<Option<(String, String)>>,
    /// Outputs whose pill has the pointer on it.
    hover: RefCell<Vec<String>>,
    /// Outputs showing for a moment, and whether with the controls.
    peeking: RefCell<HashMap<String, bool>>,
    /// Ends every peek.
    peek_timer: RefCell<Option<glib::SourceId>>,
    edit_timer: RefCell<Option<glib::SourceId>>,
    /// Set while a change is being drawn, so a draw never starts another.
    drawing: Cell<bool>,
}

/// Cloning shares the service.
#[derive(Clone)]
pub struct Labels {
    inner: Rc<Inner>,
}

impl Labels {
    pub fn new(app: &gtk4::Application, sway: Rc<SwayService>) -> Labels {
        let inner = Rc::new(Inner {
            sway: sway.clone(),
            pills: PerMonitor::new(),
            edges: PerMonitor::new(),
            data: RefCell::new(Data::load()),
            visible: RefCell::new(HashMap::new()),
            editing: RefCell::new(None),
            hover: RefCell::new(Vec::new()),
            peeking: RefCell::new(HashMap::new()),
            peek_timer: RefCell::new(None),
            edit_timer: RefCell::new(None),
            drawing: Cell::new(false),
        });
        let labels = Labels { inner };
        {
            let app = app.clone();
            let weak = Rc::downgrade(&labels.inner);
            labels.inner.pills.after_change({
                let weak = weak.clone();
                move || {
                    if let Some(inner) = weak.upgrade() {
                        Labels { inner }.apply();
                    }
                }
            });
            labels
                .inner
                .pills
                .watch(move |monitor| Pill::new(&app, monitor, weak.clone()));
        }
        {
            let app = app.clone();
            let weak = Rc::downgrade(&labels.inner);
            labels
                .inner
                .edges
                .watch(move |monitor| EdgeZone::new(&app, monitor, weak.clone()));
        }
        {
            let weak = Rc::downgrade(&labels.inner);
            sway.connect_change(move || {
                if let Some(inner) = weak.upgrade() {
                    Labels { inner }.on_sway();
                }
            });
        }
        labels.on_sway();
        labels
    }

    /// What the CLI asks for: `edit` (the default, also `toggle`), `mode
    /// <hidden|auto|always|next>`, `clear`.
    pub fn command(&self, args: &[&str]) {
        match args {
            [] | ["edit"] | ["toggle"] => self.summon(),
            ["mode", name] => {
                let mode = if *name == "next" {
                    Some(self.inner.data.borrow().mode.next())
                } else {
                    Mode::parse(name)
                };
                match mode {
                    Some(mode) => self.set_mode(mode),
                    None => log::warn!("label: unknown mode `{name}`"),
                }
            }
            ["clear"] => {
                if let Some(output) = self.inner.sway.focused_output() {
                    self.clear(&output);
                }
            }
            other => log::warn!("label: unknown command {other:?}"),
        }
    }

    // ── What is shown ───────────────────────────────────────────────────

    fn workspace_on(&self, output: &str) -> Option<String> {
        self.inner.visible.borrow().get(output).cloned()
    }

    fn text_on(&self, output: &str) -> String {
        self.workspace_on(output)
            .and_then(|ws| self.inner.data.borrow().labels.get(&ws).cloned())
            .unwrap_or_default()
    }

    /// In `always`, a named workspace keeps its pill up.
    fn resting(&self, output: &str) -> bool {
        self.inner.data.borrow().mode == Mode::Always && !self.text_on(output).is_empty()
    }

    /// Bring every pill to what the state says: shown or hidden, and drawn.
    fn apply(&self) {
        if self.inner.drawing.replace(true) {
            return;
        }
        let mode = self.inner.data.borrow().mode;
        let editing = self.inner.editing.borrow().as_ref().map(|(o, _)| o.clone());
        self.inner.pills.for_each(|pill| {
            let out = pill.connector.as_str();
            let editing_here = editing.as_deref() == Some(out);
            let hovered = self.inner.hover.iter_has(out);
            let peek = self.inner.peeking.borrow().get(out).copied();
            let wanted = editing_here || hovered || peek.is_some() || self.resting(out);
            if wanted {
                pill.draw(
                    &self.text_on(out),
                    mode,
                    editing_here || hovered || peek == Some(true),
                );
                pill.surface.show();
            } else {
                pill.surface.hide();
            }
        });
        self.inner.drawing.set(false);
    }

    /// Show `output`'s pill, then let it go after `ms` unless something holds
    /// it. Replaces a countdown already running.
    fn peek(&self, output: &str, controls: bool, ms: u64) {
        log::debug!("label: peek {output} (workspace {:?})", self.workspace_on(output));
        if self.workspace_on(output).is_none() {
            return;
        }
        {
            let mut peeking = self.inner.peeking.borrow_mut();
            let held = peeking.get(output).copied().unwrap_or(false);
            peeking.insert(output.to_string(), controls || held);
        }
        self.arm_peek(ms);
        self.apply();
    }

    fn arm_peek(&self, ms: u64) {
        if let Some(id) = self.inner.peek_timer.borrow_mut().take() {
            crate::spawn::remove_source(id);
        }
        let weak = Rc::downgrade(&self.inner);
        let id = glib::timeout_add_local_once(Duration::from_millis(ms), move || {
            let Some(inner) = weak.upgrade() else { return };
            inner.peek_timer.borrow_mut().take();
            inner.peeking.borrow_mut().clear();
            Labels { inner }.apply();
        });
        *self.inner.peek_timer.borrow_mut() = Some(id);
    }

    /// The pointer came onto or left the pill on `output`.
    fn pointer(&self, output: &str, inside: bool) {
        {
            let mut hover = self.inner.hover.borrow_mut();
            hover.retain(|o| o != output);
            if inside {
                hover.push(output.to_string());
            }
        }
        if !inside {
            // Keep it up for a beat, so a slip off the edge is not a hide.
            self.peek(output, false, LEAVE_MS);
            return;
        }
        self.apply();
    }

    // ── Sway ────────────────────────────────────────────────────────────

    fn on_sway(&self) {
        let snapshot = self.inner.sway.snapshot();
        let mut switched = Vec::new();
        {
            let mut visible = self.inner.visible.borrow_mut();
            let now: HashMap<String, String> = snapshot
                .workspaces
                .iter()
                .filter(|w| w.visible)
                .map(|w| (w.output.clone(), w.name.clone()))
                .collect();
            for (output, name) in &now {
                // The first sight of an output is not a switch.
                if visible.get(output).is_some_and(|old| old != name) {
                    switched.push(output.clone());
                }
            }
            *visible = now;
        }

        // An edit belongs to the workspace it started on.
        let moved = self
            .inner
            .editing
            .borrow()
            .as_ref()
            .is_some_and(|(output, ws)| self.workspace_on(output).as_ref() != Some(ws));
        if moved {
            self.finish_edit(true);
        }

        // A band over a fullscreen window's top edge would catch the pointer.
        let fullscreen = snapshot.fullscreen_on_focused_output;
        self.inner
            .edges
            .for_each(|edge| edge.surface.window().set_visible(!fullscreen));

        // A pill showing for the workspace you left goes at once; it is not
        // given a countdown for one that has no name.
        for output in &switched {
            self.inner.peeking.borrow_mut().remove(output);
        }

        if self.inner.data.borrow().mode == Mode::Auto && !fullscreen {
            for output in switched {
                if !self.text_on(&output).is_empty() {
                    self.peek(&output, false, AUTO_MS);
                }
            }
        }
        self.apply();
    }

    // ── Editing ─────────────────────────────────────────────────────────

    /// The keybind: edit on the focused output, or save and close.
    fn summon(&self) {
        if self.inner.editing.borrow().is_some() {
            self.finish_edit(true);
            return;
        }
        if let Some(output) = self.inner.sway.focused_output() {
            self.begin_edit(&output);
        }
    }

    fn begin_edit(&self, output: &str) {
        if self.inner.editing.borrow().is_some() {
            return;
        }
        let Some(ws) = self.workspace_on(output) else {
            return;
        };
        let text = self.text_on(output);
        *self.inner.editing.borrow_mut() = Some((output.to_string(), ws));
        let mut entry = None;
        self.inner.pills.for_each(|pill| {
            if pill.connector == output {
                pill.entry.set_text(&text);
                pill.stack.set_visible_child_name("edit");
                pill.surface
                    .window()
                    .set_keyboard_mode(KeyboardMode::Exclusive);
                entry = Some(pill.entry.clone());
            }
        });
        self.apply();
        if let Some(entry) = entry {
            // After the map: a widget in an unmapped window takes no focus.
            glib::idle_add_local_once(move || {
                entry.grab_focus();
                entry.select_region(0, -1);
            });
        }
        self.arm_edit_idle();
    }

    /// End the edit, saving the field when `save`.
    fn finish_edit(&self, save: bool) {
        let Some((output, ws)) = self.inner.editing.borrow_mut().take() else {
            return;
        };
        if let Some(id) = self.inner.edit_timer.borrow_mut().take() {
            crate::spawn::remove_source(id);
        }
        let mut typed = None;
        self.inner.pills.for_each(|pill| {
            if pill.connector == output {
                typed = Some(pill.entry.text().to_string());
                pill.stack.set_visible_child_name("read");
                pill.surface.window().set_keyboard_mode(KeyboardMode::None);
            }
        });
        if let (true, Some(text)) = (save, typed) {
            self.set_label(&ws, &text);
        }
        // Whatever the mode, the result is seen once.
        if self.text_on(&output).is_empty() {
            self.apply();
        } else {
            self.peek(&output, false, AUTO_MS);
        }
    }

    fn arm_edit_idle(&self) {
        if self.inner.editing.borrow().is_none() {
            return;
        }
        if let Some(id) = self.inner.edit_timer.borrow_mut().take() {
            crate::spawn::remove_source(id);
        }
        let weak = Rc::downgrade(&self.inner);
        let id = glib::timeout_add_local_once(Duration::from_millis(EDIT_IDLE_MS), move || {
            let Some(inner) = weak.upgrade() else { return };
            inner.edit_timer.borrow_mut().take();
            Labels { inner }.finish_edit(true);
        });
        *self.inner.edit_timer.borrow_mut() = Some(id);
    }

    fn set_label(&self, ws: &str, text: &str) {
        let text = text.trim();
        {
            let mut data = self.inner.data.borrow_mut();
            if text.is_empty() {
                data.labels.remove(ws);
            } else {
                data.labels.insert(ws.to_string(), text.to_string());
            }
            data.save();
        }
    }

    /// Remove the name of the workspace on `output`.
    fn clear(&self, output: &str) {
        if self.inner.editing.borrow().is_some() {
            self.finish_edit(false);
        }
        if let Some(ws) = self.workspace_on(output) {
            self.set_label(&ws, "");
        }
        self.apply();
    }

    fn set_mode(&self, mode: Mode) {
        {
            let mut data = self.inner.data.borrow_mut();
            if data.mode == mode {
                return;
            }
            data.mode = mode;
            data.save();
        }
        // The change is seen where it was made, with the controls up, even
        // when the new mode is `hidden`.
        match self.inner.sway.focused_output() {
            Some(output) => self.peek(&output, true, PEEK_MS),
            None => self.apply(),
        }
    }
}

trait HasOutput {
    fn iter_has(&self, output: &str) -> bool;
}

impl HasOutput for RefCell<Vec<String>> {
    fn iter_has(&self, output: &str) -> bool {
        self.borrow().iter().any(|o| o == output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_round_trips_and_keeps_swedish_letters() {
        let mut data = Data {
            mode: Mode::Always,
            ..Data::default()
        };
        data.labels.insert("3".into(), "Fakturering åäö".into());
        assert_eq!(Data::parse(data.to_json().as_bytes()), data);
    }

    #[test]
    fn a_bad_file_reads_as_the_default() {
        assert_eq!(Data::parse(b"not json"), Data::default());
        assert_eq!(Data::default().mode, Mode::Auto);
    }

    #[test]
    fn empty_and_wrong_typed_labels_are_dropped_and_an_unknown_mode_is_auto() {
        let data = Data::parse(br#"{"mode":"loud","labels":{"1":"  ","2":5,"3":" ok "}}"#);
        assert_eq!(data.mode, Mode::Auto);
        assert_eq!(data.labels.len(), 1);
        assert_eq!(data.labels["3"], "ok");
    }

    #[test]
    fn next_cycles_through_all_three_modes() {
        assert_eq!(Mode::Hidden.next(), Mode::Auto);
        assert_eq!(Mode::Auto.next(), Mode::Always);
        assert_eq!(Mode::Always.next(), Mode::Hidden);
        for mode in Mode::ALL {
            assert_eq!(Mode::parse(mode.as_str()), Some(mode));
        }
    }
}
