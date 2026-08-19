//! App launcher powered by the elephant search daemon.
//!
//! [`LauncherView`] is an embeddable widget (search entry + results list +
//! search/keyboard wiring) with no window of its own. Both the standalone
//! full-screen [`Launcher`] and the start-menu popup mount the same view.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4_layer_shell::Edge;

use crate::anim;
use crate::elephant::{self, SearchResult};
use crate::layer_shell::{self, LayerShellConfig};

const MAX_VISIBLE_RESULTS: usize = 10;
const DEBOUNCE_MS: u64 = 100;

/// How tall the results list stands on a screen with room for it.
const RESULTS_HEIGHT: i32 = 360;

/// What the standalone launcher card asks for on a screen with room for it.
const LAUNCHER_CARD_SIZE: CardSize = CardSize {
    width: 560,
    height: Some(520),
};

static LAUNCHER_CONFIG: LayerShellConfig = LayerShellConfig {
    namespace: "swaypplet-launcher",
    layer: gtk4_layer_shell::Layer::Overlay,
    exclusive: false,
    default_width: None,
    default_height: None,
    anchors: &[
        (Edge::Top, true),
        (Edge::Bottom, true),
        (Edge::Left, true),
        (Edge::Right, true),
    ],
    margins: &[],
    keyboard_mode: gtk4_layer_shell::KeyboardMode::Exclusive,
};

// Default providers matching the walker config
const DEFAULT_PROVIDERS: &[&str] = &[
    "desktopapplications",
    "calc",
    "runner",
    "windows",
    "clipboard",
    "providerlist",
    "menus",
    "websearch",
];

struct LauncherState {
    results: Vec<SearchResult>,
    selected: usize,
    query_generation: u64,
}

// ── Embeddable launcher view ────────────────────────────────────────────────

/// Search entry + scrolled results list, wired to elephant. Mountable inside
/// any container. Calls the registered `on_activate` callback (if any) right
/// after firing the activation, so a host popup can hide itself.
pub struct LauncherView {
    root: gtk4::Box,
    entry: gtk4::SearchEntry,
    results_box: gtk4::Box,
    /// The results list. Held because it is the one part of the view a short
    /// output is allowed to shrink (see `install_monitor_fit`).
    scroller: gtk4::ScrolledWindow,
    state: Rc<RefCell<LauncherState>>,
    on_activate: Rc<RefCell<Option<Box<dyn Fn()>>>>,
}

impl LauncherView {
    pub fn new() -> Self {
        let root = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(0)
            .build();
        root.add_css_class("launcher-view");

        let entry = gtk4::SearchEntry::builder()
            .placeholder_text("Search")
            .hexpand(true)
            .build();
        entry.add_css_class("launcher-entry");

        let results_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(0)
            .build();
        results_box.add_css_class("launcher-results");

        let scroller = gtk4::ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .vexpand(true)
            .child(&results_box)
            .build();
        scroller.add_css_class("launcher-scroller");
        // The floor is a property rather than a CSS `min-height` because GTK
        // takes the larger of the two, so a rule here would override any host
        // that has to shrink the list to fit a short output.
        scroller.set_min_content_height(RESULTS_HEIGHT);

        root.append(&entry);
        root.append(&scroller);

        let view = LauncherView {
            root,
            entry,
            results_box,
            scroller,
            state: Rc::new(RefCell::new(LauncherState {
                results: Vec::new(),
                selected: 0,
                query_generation: 0,
            })),
            on_activate: Rc::new(RefCell::new(None)),
        };

        view.wire_search();
        view
    }

    pub fn entry(&self) -> &gtk4::SearchEntry {
        &self.entry
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    /// The results list, for a host that has to shrink it to fit its output.
    pub fn scroller(&self) -> &gtk4::ScrolledWindow {
        &self.scroller
    }

    /// Register a callback invoked right after an item is activated (used by
    /// the start menu to hide itself).
    pub fn set_on_activate<F: Fn() + 'static>(&self, f: F) {
        *self.on_activate.borrow_mut() = Some(Box::new(f));
    }

    /// Reset to the empty-query state (cleared input + default app list).
    pub fn reset(&self) {
        self.entry.set_text("");
        {
            let mut s = self.state.borrow_mut();
            s.results.clear();
            s.selected = 0;
        }
        // Empty query shows the default desktop-application list.
        run_search(
            String::new(),
            bump_generation(&self.state),
            self.state.clone(),
            self.results_box.clone(),
            self.on_activate.clone(),
        );
    }

    pub fn focus_entry(&self) {
        self.entry.grab_focus();
    }

    /// Attach a Capture-phase key controller to `widget` so Enter/Escape/arrows
    /// reach the launcher before the SearchEntry consumes them. `on_escape` is
    /// called when Escape is pressed (e.g. to hide the host popup).
    pub fn install_key_controller<E: Fn() + 'static>(
        &self,
        widget: &impl IsA<gtk4::Widget>,
        on_escape: E,
    ) {
        let key_controller = gtk4::EventControllerKey::new();
        key_controller.set_propagation_phase(gtk4::PropagationPhase::Capture);

        let view_state = self.state.clone();
        let results_box = self.results_box.clone();
        let entry = self.entry.clone();
        let on_activate = self.on_activate.clone();

        key_controller.connect_key_pressed(move |_, key, _, _| match key {
            gtk4::gdk::Key::Escape => {
                on_escape();
                glib::Propagation::Stop
            }
            gtk4::gdk::Key::Down => {
                move_selection_state(&view_state, &results_box, 1);
                glib::Propagation::Stop
            }
            gtk4::gdk::Key::Up => {
                move_selection_state(&view_state, &results_box, -1);
                glib::Propagation::Stop
            }
            gtk4::gdk::Key::Return | gtk4::gdk::Key::KP_Enter => {
                let s = view_state.borrow();
                if let Some(item) = s.results.get(s.selected) {
                    let provider = item.provider.clone();
                    let identifier = item.identifier.clone();
                    let action = default_action(item);
                    let query = entry.text().to_string();
                    drop(s);
                    activate_async(provider, identifier, action, query);
                    if let Some(cb) = on_activate.borrow().as_ref() {
                        cb();
                    }
                }
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        });
        widget.add_controller(key_controller);
    }

    fn wire_search(&self) {
        let results_box = self.results_box.clone();
        let state = self.state.clone();
        let entry = self.entry.clone();
        let on_activate = self.on_activate.clone();

        let debounce_id: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));

        entry.connect_search_changed(move |entry| {
            let query = entry.text().to_string();

            if let Some(id) = debounce_id.borrow_mut().take() {
                crate::spawn::remove_source(id);
            }

            let results_box_c = results_box.clone();
            let state_c = state.clone();
            let on_activate_c = on_activate.clone();

            let generation = bump_generation(&state_c);

            let debounce_id_c = debounce_id.clone();
            let id = glib::timeout_add_local_once(
                std::time::Duration::from_millis(DEBOUNCE_MS),
                move || {
                    *debounce_id_c.borrow_mut() = None;
                    run_search(query, generation, state_c, results_box_c, on_activate_c);
                },
            );
            *debounce_id.borrow_mut() = Some(id);
        });
    }
}

impl Default for LauncherView {
    fn default() -> Self {
        Self::new()
    }
}

// ── Standalone full-screen launcher window ──────────────────────────────────

pub struct Launcher {
    window: gtk4::Window,
    view: LauncherView,
    reveal: anim::Reveal,
}

impl Launcher {
    pub fn new(app: &gtk4::Application) -> Self {
        let window = layer_shell::create_layer_window(app, &LAUNCHER_CONFIG);
        window.add_css_class("launcher");

        let backdrop = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .halign(gtk4::Align::Fill)
            .valign(gtk4::Align::Fill)
            .hexpand(true)
            .vexpand(true)
            .build();
        backdrop.add_css_class("launcher-backdrop");

        let top_spacer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

        // Size requests come from install_monitor_fit below, which clamps
        // them to the output the launcher opens on.
        let container = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(0)
            .halign(gtk4::Align::Center)
            .build();
        container.add_css_class("glass-card");
        container.add_css_class("launcher-container");

        let view = LauncherView::new();
        container.append(view.widget());

        backdrop.append(&top_spacer);
        backdrop.append(&container);
        window.set_child(Some(&backdrop));

        install_monitor_fit(&window, &top_spacer, &container, LAUNCHER_CARD_SIZE, None);

        // Enter/exit transition (motion on glass, anim.rs): the container is
        // the pane, the launcher view the content. Pure crossfade.
        let reveal = anim::Reveal::new(&window, &container).content(view.widget());

        // Hide the window after a result is activated.
        {
            let reveal_c = reveal.clone();
            view.set_on_activate(move || reveal_c.hide());
        }

        // Esc / arrows / Enter handled on the window in capture phase.
        {
            let reveal_c = reveal.clone();
            view.install_key_controller(&window, move || reveal_c.hide());
        }

        // Backdrop click → dismiss.
        let gesture = gtk4::GestureClick::new();
        {
            let reveal_c = reveal.clone();
            gesture.connect_released(move |_, _, _, _| {
                reveal_c.hide();
            });
        }
        window.add_controller(gesture);

        Launcher {
            window,
            view,
            reveal,
        }
    }

    pub fn toggle(&self) {
        if self.reveal.is_shown() && self.window.is_visible() {
            self.reveal.hide();
        } else {
            self.view.reset();
            self.reveal.show();
            self.view.focus_entry();
        }
    }
}

// ── Shared helpers ──────────────────────────────────────────────────────────

fn bump_generation(state: &Rc<RefCell<LauncherState>>) -> u64 {
    let mut s = state.borrow_mut();
    s.query_generation += 1;
    s.query_generation
}

fn move_selection_state(state: &Rc<RefCell<LauncherState>>, results_box: &gtk4::Box, delta: i32) {
    let mut s = state.borrow_mut();
    if s.results.is_empty() {
        return;
    }
    let old = s.selected;
    let len = s.results.len();
    let new = if delta < 0 {
        old.saturating_sub((-delta) as usize)
    } else {
        (old + delta as usize).min(len - 1)
    };
    if new != old {
        s.selected = new;
        drop(s);
        update_selection(results_box, old, new);
    }
}

fn activate_async(provider: String, identifier: String, action: String, query: String) {
    std::thread::spawn(move || {
        if let Err(e) = elephant::activate(&provider, &identifier, &action, &query) {
            log::warn!("Elephant activate failed: {}", e);
        }
    });
}

fn clear_results_box(results_box: &gtk4::Box) {
    while let Some(child) = results_box.first_child() {
        results_box.remove(&child);
    }
}

fn run_search(
    query: String,
    generation: u64,
    state: Rc<RefCell<LauncherState>>,
    results_box: gtk4::Box,
    on_activate: Rc<RefCell<Option<Box<dyn Fn()>>>>,
) {
    // Empty query → default desktop-application list only.
    let providers: Vec<&str> = if query.is_empty() {
        vec!["desktopapplications"]
    } else {
        DEFAULT_PROVIDERS.to_vec()
    };

    let query_c = query.clone();
    crate::spawn::spawn_work(
        move || match elephant::query(&query_c, &providers, MAX_VISIBLE_RESULTS as i32) {
            Ok(results) => results,
            Err(e) => {
                log::warn!("Elephant query failed: {}", e);
                Vec::new()
            }
        },
        move |results| {
            // A newer query superseded this one while it ran — drop the results.
            if generation != state.borrow().query_generation {
                return;
            }
            {
                let mut s = state.borrow_mut();
                s.results = results;
                s.selected = 0;
            }
            rebuild_results_ui(&results_box, &state, &query, &on_activate);
        },
    );
}

fn rebuild_results_ui(
    results_box: &gtk4::Box,
    state: &Rc<RefCell<LauncherState>>,
    query: &str,
    on_activate: &Rc<RefCell<Option<Box<dyn Fn()>>>>,
) {
    clear_results_box(results_box);

    let s = state.borrow();
    let selected = s.selected;

    for (i, result) in s.results.iter().enumerate() {
        let row = build_result_row(result, i == selected, query, on_activate);
        results_box.append(&row);
    }
}

fn build_result_row(
    result: &SearchResult,
    selected: bool,
    query: &str,
    on_activate: &Rc<RefCell<Option<Box<dyn Fn()>>>>,
) -> gtk4::Box {
    let row = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(12)
        .build();
    row.add_css_class("launcher-result");
    if selected {
        row.add_css_class("selected");
    }

    let icon_label = gtk4::Label::builder()
        .label(provider_icon(&result.provider))
        .halign(gtk4::Align::Center)
        .valign(gtk4::Align::Center)
        .build();
    icon_label.add_css_class("launcher-result-icon");

    let mut used_themed_icon = false;
    if !result.icon.is_empty() && !result.icon.contains('/') {
        if let Some(display) = gtk4::gdk::Display::default() {
            let theme = gtk4::IconTheme::for_display(&display);
            if theme.has_icon(&result.icon) {
                let image = gtk4::Image::builder()
                    .icon_name(&result.icon)
                    .pixel_size(24)
                    .build();
                image.add_css_class("launcher-result-icon-img");
                row.append(&image);
                used_themed_icon = true;
            }
        }
    }
    if !used_themed_icon {
        row.append(&icon_label);
    }

    let text_box = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .valign(gtk4::Align::Center)
        .build();

    let name_label = gtk4::Label::builder()
        .label(&result.text)
        .halign(gtk4::Align::Start)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .build();
    name_label.add_css_class("launcher-result-name");
    text_box.append(&name_label);

    if !result.subtext.is_empty() {
        let sub_label = gtk4::Label::builder()
            .label(&result.subtext)
            .halign(gtk4::Align::Start)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .build();
        sub_label.add_css_class("launcher-result-sub");
        text_box.append(&sub_label);
    }

    row.append(&text_box);

    // Only badge non-default providers (websearch, calc, …). The dominant
    // "desktopapplications" source is implied by the surface, so badging every
    // row with it is pure visual noise.
    if result.provider != "desktopapplications" {
        let badge = gtk4::Label::builder()
            .label(&result.provider)
            .halign(gtk4::Align::End)
            .valign(gtk4::Align::Center)
            .build();
        badge.add_css_class("launcher-result-badge");
        row.append(&badge);
    }

    // Click to activate.
    let gesture = gtk4::GestureClick::new();
    let provider = result.provider.clone();
    let identifier = result.identifier.clone();
    let action = default_action(result);
    let query_str = query.to_string();
    let on_activate = on_activate.clone();
    gesture.connect_released(move |_, _, _, _| {
        activate_async(
            provider.clone(),
            identifier.clone(),
            action.clone(),
            query_str.clone(),
        );
        if let Some(cb) = on_activate.borrow().as_ref() {
            cb();
        }
    });
    row.add_controller(gesture);

    row
}

fn update_selection(results_box: &gtk4::Box, old: usize, new: usize) {
    let mut child = results_box.first_child();
    let mut i = 0;
    while let Some(widget) = child {
        if i == old {
            widget.remove_css_class("selected");
        }
        if i == new {
            widget.add_css_class("selected");
        }
        child = widget.next_sibling();
        i += 1;
    }
}

/// Default action for a result — the first elephant action, or "start".
fn default_action(result: &SearchResult) -> String {
    result
        .actions
        .first()
        .cloned()
        .unwrap_or_else(|| "start".to_string())
}

/// Breathing room kept between the card and the screen edges when the card's
/// preferred size does not fit the output it opened on.
const CARD_SIDE_MARGIN: i32 = 16;
const CARD_BOTTOM_MARGIN: i32 = 24;

/// Marks a card that had to give up vertical density to fit its output.
/// data/style.css answers it by dropping the fixed minimum heights that would
/// otherwise hold the card taller than the screen it is on.
const COMPACT_CLASS: &str = "card-compact";

/// Offset used until the compositor tells us which output the surface landed
/// on. A quarter of 1080p, so the card is roughly in place on the first frame
/// of a display we have not measured yet.
const FALLBACK_TOP_OFFSET: i32 = 270;

/// The size a card asks for when the screen has room, and the sizes it is
/// willing to shrink to when the screen has not.
///
/// `height` is `None` for a card that sizes itself to its content, which is
/// then never given a height request at all. The panel is that case: its
/// sections decide how tall it is.
pub(crate) struct CardSize {
    pub width: i32,
    pub height: Option<i32>,
}

/// Everything the fit needs about one card: its preferred size, and how to
/// ask it to give up vertical density when the output is too short for it.
///
/// The density hook is a callback rather than a CSS class because the fit
/// measures the card immediately after asking it to thin out, and GTK
/// validates style lazily. A class added here does not reach `measure` until
/// a later frame, so the fit would go on placing a card of the height it used
/// to have. Widget properties change the measurement on the spot, so the
/// caller sets those and the fit stays the only thing that reads geometry.
struct Fit {
    size: CardSize,
    on_compact: Option<Rc<dyn Fn(bool)>>,
    /// The output size the card was last fitted to.
    ///
    /// Deciding whether the card has to thin out means asking it for full
    /// density and measuring, which queues a resize, which brings us straight
    /// back here through the `layout` hook. GDK cuts that off with "layout
    /// continuously requested, giving up after 4 tries" and the panel never
    /// appears. Repeating the fit for an output whose size has not changed
    /// cannot reach a different answer, so it is skipped.
    fitted_to: std::cell::Cell<Option<(i32, i32)>>,
}

impl Fit {
    fn set_compact(&self, compact: bool) {
        if let Some(on_compact) = &self.on_compact {
            on_compact(compact);
        }
    }
}

/// Fit `card` to the monitor the window is actually on: clamp its size
/// request to what the output can show, then size `top_spacer` so the card
/// sits at the optical foveal sweet spot, a quarter of the way down.
///
/// Every part of this fixes the same class of bug, a fixed number that
/// silently assumes one screen's geometry. The offset came from
/// `monitors().item(0)`, whatever GDK happened to enumerate first, so a
/// 2560x1440 primary handed a 360 px spacer to a card opening on a 1440x900
/// secondary and pushed its lower half past the bottom edge. The sizes came
/// from literals in the builders and from CSS minimums taller than the screen
/// the card had to fit, with no way for anyone to scroll or drag the card
/// back into view.
///
/// Everything is recomputed on map, whenever the surface enters another
/// output, whenever the compositor sends a new size, and whenever the
/// output's geometry changes.
///
/// The card is not re-fitted while it stays mapped and its content grows.
/// Doing that would mean queueing a resize from inside layout, and both cards
/// settle their content before they are shown, so map time is late enough.
pub(crate) fn install_monitor_fit(
    window: &gtk4::Window,
    top_spacer: &gtk4::Box,
    card: &impl IsA<gtk4::Widget>,
    size: CardSize,
    on_compact: Option<Rc<dyn Fn(bool)>>,
) {
    let card = card.as_ref().clone();
    let fit = Rc::new(Fit {
        size,
        on_compact,
        fitted_to: std::cell::Cell::new(None),
    });

    // The preferred size and offset stand in until an output is known, so the
    // card looks exactly as designed on a screen big enough to hold it and
    // never flashes at some placeholder size on the way there.
    card.set_width_request(fit.size.width);
    if let Some(height) = fit.size.height {
        card.set_height_request(height);
    }
    top_spacer.set_height_request(FALLBACK_TOP_OFFSET);

    // The geometry watch has to move with the surface: a mode change on an
    // output we already left must not resize this card.
    let tracked: Rc<RefCell<Option<(gtk4::gdk::Monitor, glib::SignalHandlerId)>>> =
        Rc::new(RefCell::new(None));

    // Wayland delivers wl_surface.enter after the surface is mapped, so the
    // map pass usually has no output to measure yet and the enter below is
    // what places the card. Both paths are wired because a remap onto the
    // same output emits no enter.
    window.connect_map({
        let top_spacer = top_spacer.clone();
        let card = card.clone();
        let fit = fit.clone();
        let tracked = tracked.clone();
        move |window| refresh_monitor_fit(window, &top_spacer, &card, &fit, &tracked)
    });

    window.connect_realize({
        let top_spacer = top_spacer.clone();
        let card = card.clone();
        let fit = fit.clone();
        let tracked = tracked.clone();
        move |window| {
            let Some(surface) = window.surface() else {
                return;
            };
            surface.connect_enter_monitor({
                let window = window.downgrade();
                let top_spacer = top_spacer.clone();
                let card = card.clone();
                let fit = fit.clone();
                let tracked = tracked.clone();
                move |_, _| {
                    if let Some(window) = window.upgrade() {
                        refresh_monitor_fit(&window, &top_spacer, &card, &fit, &tracked);
                    }
                }
            });

            // The compositor's configure is the only place the surface's real
            // height comes from, and it arrives after both the map and the
            // enter that precede it. Without this the fit runs against a
            // height that is either uninitialised or left over from the
            // output the surface just left. `Fit::fitted_to` is what keeps
            // this from becoming a resize loop.
            surface.connect_layout({
                let window = window.downgrade();
                let top_spacer = top_spacer.clone();
                let card = card.clone();
                let fit = fit.clone();
                let tracked = tracked.clone();
                move |_, _, _| {
                    if let Some(window) = window.upgrade() {
                        refresh_monitor_fit(&window, &top_spacer, &card, &fit, &tracked);
                    }
                }
            });
        }
    });
}

/// Resolve the current output, re-hang the geometry watch if it changed, then
/// re-fit. Does nothing while the surface has no output yet, leaving whatever
/// size and offset were set last.
fn refresh_monitor_fit(
    window: &gtk4::Window,
    top_spacer: &gtk4::Box,
    card: &gtk4::Widget,
    fit: &Rc<Fit>,
    tracked: &Rc<RefCell<Option<(gtk4::gdk::Monitor, glib::SignalHandlerId)>>>,
) {
    let Some(monitor) = surface_monitor(window) else {
        return;
    };

    let unchanged = tracked
        .borrow()
        .as_ref()
        .is_some_and(|(watched, _)| watched == &monitor);
    if !unchanged {
        if let Some((previous, handler)) = tracked.borrow_mut().take() {
            previous.disconnect(handler);
        }
        // Weak on the widgets, because the monitor outlives the window and a
        // strong capture would keep a closed launcher alive for the session.
        let handler = monitor.connect_geometry_notify({
            let window = window.downgrade();
            let top_spacer = top_spacer.downgrade();
            let card = card.downgrade();
            let fit = fit.clone();
            let tracked = Rc::downgrade(tracked);
            move |_| {
                if let (Some(window), Some(top_spacer), Some(card), Some(tracked)) = (
                    window.upgrade(),
                    top_spacer.upgrade(),
                    card.upgrade(),
                    tracked.upgrade(),
                ) {
                    refresh_monitor_fit(&window, &top_spacer, &card, &fit, &tracked);
                }
            }
        });
        *tracked.borrow_mut() = Some((monitor.clone(), handler));
    }

    fit_card(window, &monitor, top_spacer, card, fit);
}

/// The height the card actually has to play with.
///
/// The monitor is the wrong number on an output carrying a bar. The panel's
/// layer surface is anchored to all four edges, so the compositor hands it
/// the output minus the bar's exclusive zone, measured here at 61 logical px,
/// and a card fitted to the full monitor puts its bottom row behind the bar.
///
/// Only the surface knows the real figure, and it knows it late: before its
/// first configure it reports a height of 1, and just after it moves output
/// it still reports the size the previous output gave it.
///
/// So it is believed only when it is at least half the output, which no bar
/// eats into, and capped at the output, which no configure can exceed. A
/// height of 1 taken at face value would fit the card to nothing and squeeze
/// its rows below their minimum for a frame, which GTK complains about by the
/// screenful. Whatever this returns early is corrected by the `layout` hook
/// in [`install_monitor_fit`] as soon as the real size arrives.
fn usable_height(window: &gtk4::Window, monitor: &gtk4::gdk::Monitor) -> i32 {
    let screen = monitor.geometry().height();
    window
        .surface()
        .map(|surface| surface.height())
        .filter(|height| height * 2 >= screen)
        .map_or(screen, |height| height.min(screen))
}

/// The output this window is displayed on.
///
/// `monitor_at_surface` answers from the output the compositor sent
/// wl_surface.enter for, which is also the right answer when a layer surface
/// was pinned to an output explicitly. It is `None` until that enter arrives.
fn surface_monitor(window: &gtk4::Window) -> Option<gtk4::gdk::Monitor> {
    let surface = window.surface()?;
    gtk4::gdk::Display::default()?.monitor_at_surface(&surface)
}

/// Clamp the card's size request to the output, thin it out if it still does
/// not fit, then place it.
///
/// The order matters. Width is settled first because the card's minimum
/// height is a function of its width: rows wrap and grow taller as the card
/// narrows, and an offset computed against the wide measurement would put the
/// card back off the bottom of the screen.
fn fit_card(
    window: &gtk4::Window,
    monitor: &gtk4::gdk::Monitor,
    top_spacer: &gtk4::Box,
    card: &gtk4::Widget,
    fit: &Fit,
) {
    // Monitor geometry is in logical pixels, the same space size requests and
    // the spacer's height are in, so a scaled output needs no conversion.
    let screen_width = monitor.geometry().width();
    let screen_height = usable_height(window, monitor);

    if fit.fitted_to.get() == Some((screen_width, screen_height)) {
        return;
    }
    fit.fitted_to.set(Some((screen_width, screen_height)));

    // The clamp lowers the request, which is a floor, so it cannot pull a
    // card below the minimum width its own content demands: GTK allocates the
    // larger of the two. Rows that wrap are what let the content minimum fall
    // far enough for this to bite.
    let usable_width = (screen_width - 2 * CARD_SIDE_MARGIN).max(0);
    card.set_width_request(fit.size.width.min(usable_width));

    // Ask the card what it will really be given rather than assuming it got
    // the request: for a card whose content is wider than the clamp, the two
    // differ, and measuring a height for a width the card has already refused
    // is both an over-estimate and a GTK warning.
    let (width, _, _, _) = card.measure(gtk4::Orientation::Horizontal, -1);

    let room = (screen_height - CARD_BOTTOM_MARGIN).max(0);
    if let Some(height) = fit.size.height {
        // A fixed-height card flush against the top is the tallest it can
        // ever be here, so that is the ceiling. The offset below then decides
        // where in the remaining room it actually sits.
        card.set_height_request(height.min(room));
    }

    // A short output needs the card thinned before it is placed, because the
    // lists inside it hold a floor tall enough on its own to outgrow the whole
    // screen, and no offset can rescue a card that does not fit at any offset.
    //
    // Full density is asked for first every time, so a card that visits a
    // small output once does not stay thin for the rest of the session.
    fit.set_compact(false);
    card.remove_css_class(COMPACT_CLASS);
    let mut card_min = card.measure(gtk4::Orientation::Vertical, width).0;
    if card_min > room {
        fit.set_compact(true);
        // The class only carries padding and margin trims, which the callback
        // above cannot express. It lands a frame late, and that is harmless
        // here: everything it changes makes the card shorter, so measuring
        // without it errs towards leaving the card more room than it needs.
        card.add_css_class(COMPACT_CLASS);
        card_min = card.measure(gtk4::Orientation::Vertical, width).0;
    }

    // A card taller than three quarters of the screen has to start above the
    // sweet spot, or its bottom rows, the launcher's last results among them,
    // sit off-screen where nothing can reach them.
    let limit = (room - card_min).max(0);
    top_spacer.set_height_request((screen_height / 4).min(limit));
}

fn provider_icon(provider: &str) -> &'static str {
    match provider {
        "desktopapplications" => "󰀻",
        "runner" => "",
        "windows" => "󰖯",
        "clipboard" => "󰅌",
        "calc" | "calculator" => "󰃬",
        "websearch" => "󰖟",
        "files" => "󰈔",
        "menus" => "󰍜",
        "bookmarks" => "󰃃",
        _ => "󰍉",
    }
}
