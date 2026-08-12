//! Every window at once, with its own pixels on it.
//!
//! sway has no switcher. The workspace map on the bar is the closest thing,
//! and it is deliberately abstract: it says *task 2 has two screens*, never
//! *which of these four terminals is the one running the migration*. Titles
//! answer that badly — nine of the eleven windows open on this machine are a
//! browser or a terminal, and the words in their title bars are the least
//! memorable thing about them. Pixels answer it immediately.
//!
//! Two protocols meet here, joined by a string. `ext-foreign-toplevel-list-v1`
//! gives every window a stable `identifier`, and sway puts that same
//! identifier in its tree — so the switcher takes its *model* from sway IPC,
//! which knows about workspaces, focus order and how to focus something, and
//! its *pixels* from `ext-image-copy-capture-v1`, which knows nothing else.
//! Neither has to guess at the other by matching on titles.
//!
//! Thumbnails arrive one at a time and the grid is drawn before any of them
//! do. A window on a workspace nobody is looking at is still rendered by
//! swayfx and still captures, but the protocol allows the compositor to wait
//! indefinitely for content that never changes, so each capture is bounded
//! (`screenshot::capture::CAPTURE_TIMEOUT`) and one that does not arrive
//! simply leaves its card showing the app icon.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4::gdk;
use gtk4::prelude::*;
use gtk4_layer_shell::{KeyboardMode, Layer};

use crate::layer_shell::{self, LayerShellConfig};
use crate::screenshot::capture;

/// One window, as the switcher needs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Window {
    /// sway's container id — what focusing it actually needs.
    pub con_id: i64,
    pub app_id: String,
    pub title: String,
    pub workspace: String,
    /// The join key to the Wayland side. Absent for a window the compositor
    /// does not list (xwayland without a toplevel handle), which then simply
    /// has no thumbnail.
    pub identifier: Option<String>,
}

/// Thumbnails are drawn at this size; captures are boxed down to it before a
/// texture is made, because eleven full-resolution windows is 150 MB of RGBA
/// held for as long as the switcher is open.
const THUMB_W: u32 = 260;
const THUMB_H: u32 = 156;

/// Below this a thumbnail stops being recognisable. A session with more
/// windows than the screen holds at this size overflows rather than
/// shrinking further: typing narrows the grid in one keystroke, and forty
/// unreadable squares help nobody.
const MIN_THUMB_W: i32 = 150;

/// Cards per row, at most. Four keeps eleven windows to three rows on a
/// 16:10 panel; a wider output gets more only up to the point where the eye
/// still finds a card by position.
const MAX_COLUMNS: i32 = 6;

/// What a card costs around its thumbnail: `.switcher-item` padding and
/// border on both sides, and under it the title and workspace lines with
/// the box's spacing between them. Approximate on purpose — the fit leaves
/// a margin wide enough to absorb a font that measures differently.
const CARD_CHROME_W: i32 = 20;
const CARD_CHROME_H: i32 = 64;

/// Grid spacing and `.switcher-card` padding, mirrored from the builder and
/// the stylesheet so the fit can subtract what it cannot measure yet.
const GAP: i32 = 10;
const CARD_PADDING: i32 = 18;

/// The output to size against when no monitor can be resolved. Small enough
/// that the grid fits anything real.
const FALLBACK_OUTPUT: (i32, i32) = (1280, 800);

/// How much of the output the grid may use. The rest is the breathing room
/// a centred overlay needs to read as an overlay.
const FILL_W: f64 = 0.94;
const FILL_H: f64 = 0.90;

/// The grid's shape for one opening: how many columns, and how big a
/// thumbnail may be so that every card fits on the output at once.
///
/// The switcher shows *every* window, so its size is the session's, not the
/// designer's — eleven windows on a 1440x900 laptop and twenty-six on a
/// 2560x1440 desktop are both normal. A layer surface larger than its
/// output is not scrolled or scaled by the compositor, it is simply cut off
/// at the edge, so the fit happens here or not at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Fit {
    columns: i32,
    thumb_w: i32,
    thumb_h: i32,
}

fn fit(count: usize, output_w: i32, output_h: i32) -> Fit {
    let count = (count as i32).max(1);
    let avail_w = (output_w as f64 * FILL_W) as i32 - 2 * CARD_PADDING;
    let avail_h = (output_h as f64 * FILL_H) as i32 - 2 * CARD_PADDING;

    let columns = ((avail_w + GAP) / (THUMB_W as i32 + CARD_CHROME_W + GAP))
        .clamp(1, MAX_COLUMNS)
        .min(count);
    let rows = (count + columns - 1) / columns;

    // What one card may occupy, minus its own chrome, is what its thumbnail
    // may be. Width and height both bind; the tighter one wins and the
    // aspect ratio follows from it.
    let per_w = (avail_w - (columns - 1) * GAP) / columns - CARD_CHROME_W;
    let per_h = (avail_h - (rows - 1) * GAP) / rows - CARD_CHROME_H;
    let thumb_w = (THUMB_W as i32)
        .min(per_w)
        .min(per_h * THUMB_W as i32 / THUMB_H as i32)
        .max(MIN_THUMB_W);

    Fit {
        columns,
        thumb_w,
        thumb_h: thumb_w * THUMB_H as i32 / THUMB_W as i32,
    }
}

/// The output the switcher will map on, as GDK knows it.
///
/// The compositor would pick one for us, but the grid has to be sized before
/// it maps and a grid sized to the wrong screen is a grid cut off at the
/// edge of the smaller one. sway names its focused output; GDK names the
/// same connector.
fn monitor_named(connector: &str) -> Option<gdk::Monitor> {
    let monitors = gdk::Display::default()?.monitors();
    (0..monitors.n_items())
        .filter_map(|i| monitors.item(i))
        .filter_map(|obj| obj.downcast::<gdk::Monitor>().ok())
        .find(|monitor| monitor.connector().as_deref() == Some(connector))
}

pub struct Switcher {
    window: gtk4::Window,
    grid: gtk4::Grid,
    reveal: crate::anim::Reveal,
    /// The windows currently drawn, in the order their cards are laid out.
    entries: RefCell<Vec<Window>>,
    cards: RefCell<Vec<Card>>,
    selected: Cell<usize>,
    /// Typeahead. Cleared on every open, because the filter that helped last
    /// time is never the one that helps now.
    query: RefCell<String>,
    filter_label: gtk4::Label,
    loading: Cell<bool>,
    /// The shape the current opening was laid out to. Recomputed on every
    /// show, because both the window count and the focused output change
    /// between one Super+Tab and the next.
    fit: Cell<Fit>,
}

/// The widgets of one card that can change after it is built.
struct Card {
    root: gtk4::Widget,
    picture: gtk4::Picture,
    icon: gtk4::Image,
}

impl Switcher {
    pub fn new(app: &gtk4::Application) -> Rc<Self> {
        static CONFIG: LayerShellConfig = LayerShellConfig {
            namespace: "swaypplet-switcher",
            layer: Layer::Overlay,
            exclusive: false,
            default_width: None,
            default_height: None,
            anchors: &[],
            margins: &[],
            // The switcher is typed into: arrows, typeahead, Enter.
            keyboard_mode: KeyboardMode::Exclusive,
        };

        let window = layer_shell::create_layer_window(app, &CONFIG);
        window.set_decorated(false);

        let wrapper = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .halign(gtk4::Align::Center)
            .valign(gtk4::Align::Center)
            .build();

        let card = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .build();
        card.add_css_class("glass-card");
        card.add_css_class("switcher-card");

        let content = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(10)
            .build();

        let filter_label = gtk4::Label::builder().xalign(0.0).build();
        filter_label.add_css_class("switcher-filter");
        filter_label.set_visible(false);

        let grid = gtk4::Grid::builder()
            .row_spacing(10)
            .column_spacing(10)
            .build();
        grid.add_css_class("switcher-grid");

        content.append(&filter_label);
        content.append(&grid);
        card.append(&content);
        wrapper.append(&card);
        window.set_child(Some(&wrapper));

        let reveal = crate::anim::Reveal::new(&window, &card).content(&content);

        let switcher = Rc::new(Switcher {
            window,
            grid,
            reveal,
            entries: RefCell::new(Vec::new()),
            cards: RefCell::new(Vec::new()),
            selected: Cell::new(0),
            query: RefCell::new(String::new()),
            filter_label,
            loading: Cell::new(false),
            fit: Cell::new(fit(1, FALLBACK_OUTPUT.0, FALLBACK_OUTPUT.1)),
        });
        switcher.wire();
        switcher
    }

    /// Open it, reading the window list fresh.
    ///
    /// Always fresh: the whole value of the surface is that it describes the
    /// session as it is right now, and a cached list is a switcher that sends
    /// you to a window that closed.
    pub fn show(self: &Rc<Self>) {
        if self.loading.replace(true) {
            return;
        }
        self.query.borrow_mut().clear();

        let this = self.clone();
        crate::spawn::spawn_work(read_session, move |session| {
            this.loading.set(false);
            match session {
                Ok((windows, output)) if !windows.is_empty() => {
                    this.build(windows, output.as_deref());
                    this.reveal.show();
                    this.load_thumbnails();
                }
                Ok(_) => log::debug!("switcher: no windows"),
                Err(e) => log::warn!("switcher: {e}"),
            }
        });
    }

    pub fn hide(&self) {
        self.reveal.hide();
    }

    pub fn toggle(self: &Rc<Self>) {
        if self.reveal.is_shown() {
            self.hide();
        } else {
            self.show();
        }
    }

    fn wire(self: &Rc<Self>) {
        let keys = gtk4::EventControllerKey::new();
        let this = self.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            match key {
                gdk::Key::Escape => this.hide(),
                gdk::Key::Return | gdk::Key::KP_Enter => this.activate(),
                gdk::Key::Left => this.step(-1),
                gdk::Key::Right | gdk::Key::Tab => this.step(1),
                gdk::Key::Up => this.step(-(this.fit.get().columns as isize)),
                gdk::Key::Down => this.step(this.fit.get().columns as isize),
                gdk::Key::BackSpace => {
                    this.query.borrow_mut().pop();
                    this.refilter();
                }
                other => {
                    // Typeahead: anything that produces a character narrows
                    // the grid, so eleven windows become one without the
                    // hand leaving the home row.
                    match other.to_unicode().filter(|c| !c.is_control()) {
                        Some(c) => {
                            this.query.borrow_mut().push(c);
                            this.refilter();
                        }
                        None => return glib::Propagation::Proceed,
                    }
                }
            }
            glib::Propagation::Stop
        });
        self.window.add_controller(keys);
    }

    /// Lay out a card per window, on the output the switcher is about to
    /// appear on and at the size that output has room for.
    fn build(self: &Rc<Self>, windows: Vec<Window>, output: Option<&str>) {
        while let Some(child) = self.grid.first_child() {
            self.grid.remove(&child);
        }
        self.cards.borrow_mut().clear();

        let monitor = output.and_then(monitor_named);
        // Setting it while unmapped is the only time it takes; `show` maps
        // the surface right after this returns.
        if let Some(monitor) = &monitor {
            use gtk4_layer_shell::LayerShell;
            self.window.set_monitor(Some(monitor));
        }
        let (output_w, output_h) = monitor
            .map(|m| (m.geometry().width(), m.geometry().height()))
            .unwrap_or(FALLBACK_OUTPUT);
        let shape = fit(windows.len(), output_w, output_h);
        self.fit.set(shape);

        for (index, window) in windows.iter().enumerate() {
            let card = self.card_for(window, index);
            self.grid.attach(
                &card.root,
                index as i32 % shape.columns,
                index as i32 / shape.columns,
                1,
                1,
            );
            self.cards.borrow_mut().push(card);
        }

        *self.entries.borrow_mut() = windows;
        // The focused window is card 0 (see `read_windows`), so the first
        // step lands on the one before it — which is what a switcher is for.
        self.selected.set(if self.entries.borrow().len() > 1 {
            1
        } else {
            0
        });
        self.repaint_selection();
        self.refilter();
    }

    fn card_for(self: &Rc<Self>, window: &Window, index: usize) -> Card {
        let root = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(6)
            .width_request(self.fit.get().thumb_w)
            .build();
        root.add_css_class("switcher-item");

        let picture = gtk4::Picture::builder()
            .content_fit(gtk4::ContentFit::Cover)
            .hexpand(true)
            .vexpand(true)
            .build();

        // The icon stands in until (or instead of) a thumbnail, so a card is
        // never an empty rectangle.
        let icon = gtk4::Image::from_icon_name(&icon_name(&window.app_id));
        icon.set_pixel_size(48);
        icon.set_halign(gtk4::Align::Center);
        icon.set_valign(gtk4::Align::Center);
        icon.add_css_class("switcher-icon");

        // The *frame* holds the thumbnail's size, and the picture is the
        // overlay's main child. An overlay measures only its main child, so a
        // picture that requests 260x156 from the overlay side is drawn at that
        // size inside a card the grid sized to a 48 px icon — every thumbnail
        // spilling over its neighbours. Overflow::Hidden then keeps what
        // `Cover` crops inside the rounded corners.
        let stack = gtk4::Overlay::new();
        stack.set_size_request(self.fit.get().thumb_w, self.fit.get().thumb_h);
        stack.set_overflow(gtk4::Overflow::Hidden);
        stack.add_css_class("switcher-thumb");
        stack.set_child(Some(&picture));
        stack.add_overlay(&icon);

        // `max_width_chars(1)` with hexpand is how a label ellipsizes to the
        // width it is given instead of asking for the width of its text: a
        // 28-char title measures wider than a 260 px thumbnail and would set
        // the column width itself.
        let title = gtk4::Label::builder()
            .label(&window.title)
            .xalign(0.0)
            .max_width_chars(1)
            .hexpand(true)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .build();
        title.add_css_class("switcher-title");

        let workspace = gtk4::Label::builder()
            .label(&window.workspace)
            .xalign(0.0)
            .build();
        workspace.add_css_class("switcher-workspace");

        root.append(&stack);
        root.append(&title);
        root.append(&workspace);

        let click = gtk4::GestureClick::new();
        let this = self.clone();
        click.connect_released(move |_, _, _, _| {
            this.selected.set(index);
            this.activate();
        });
        root.add_controller(click);

        Card {
            root: root.upcast(),
            picture,
            icon,
        }
    }

    /// Ask for every thumbnail at once and fill each card as it lands.
    fn load_thumbnails(self: &Rc<Self>) {
        for (index, window) in self.entries.borrow().iter().enumerate() {
            let Some(identifier) = window.identifier.clone() else {
                continue;
            };
            let this = self.clone();
            let (thumb_w, thumb_h) = (self.fit.get().thumb_w as u32, self.fit.get().thumb_h as u32);
            crate::spawn::spawn_work(
                move || capture::toplevel(&identifier).map(|i| i.boxed_down(thumb_w, thumb_h)),
                move |result| match result {
                    Ok(image) => this.set_thumbnail(index, &image),
                    // Expected for a window the compositor declines to render;
                    // the card keeps its icon.
                    Err(e) => log::debug!("switcher: thumbnail {index}: {e}"),
                },
            );
        }
    }

    fn set_thumbnail(&self, index: usize, image: &capture::Image) {
        let cards = self.cards.borrow();
        let Some(card) = cards.get(index) else {
            return;
        };
        if image.width == 0 || image.height == 0 {
            return;
        }
        let texture = gdk::MemoryTexture::new(
            image.width as i32,
            image.height as i32,
            gdk::MemoryFormat::R8g8b8a8,
            &glib::Bytes::from(&image.pixels),
            (image.width * 4) as usize,
        );
        card.picture.set_paintable(Some(&texture));
        card.icon.set_visible(false);
    }

    /// Move the selection through the cards that survive the filter.
    fn step(&self, by: isize) {
        let visible = self.visible_indices();
        if visible.is_empty() {
            return;
        }
        let current = visible
            .iter()
            .position(|i| *i == self.selected.get())
            .unwrap_or(0) as isize;
        let next = (current + by).rem_euclid(visible.len() as isize) as usize;
        self.selected.set(visible[next]);
        self.repaint_selection();
    }

    fn visible_indices(&self) -> Vec<usize> {
        let query = self.query.borrow().to_lowercase();
        self.entries
            .borrow()
            .iter()
            .enumerate()
            .filter(|(_, w)| matches(w, &query))
            .map(|(i, _)| i)
            .collect()
    }

    fn refilter(&self) {
        let visible = self.visible_indices();
        for (index, card) in self.cards.borrow().iter().enumerate() {
            card.root.set_visible(visible.contains(&index));
        }

        let query = self.query.borrow();
        self.filter_label.set_visible(!query.is_empty());
        self.filter_label.set_label(&format!(
            "{query}  ·  {} of {}",
            visible.len(),
            self.entries.borrow().len()
        ));

        // A filter that excludes the selection moves it rather than leaving a
        // highlight on a hidden card and an Enter that goes somewhere unseen.
        if !visible.contains(&self.selected.get())
            && let Some(first) = visible.first()
        {
            self.selected.set(*first);
        }
        self.repaint_selection();
    }

    fn repaint_selection(&self) {
        let selected = self.selected.get();
        for (index, card) in self.cards.borrow().iter().enumerate() {
            if index == selected {
                card.root.add_css_class("selected");
            } else {
                card.root.remove_css_class("selected");
            }
        }
    }

    /// Focus the selected window and get out of the way.
    fn activate(&self) {
        let entries = self.entries.borrow();
        let Some(window) = entries.get(self.selected.get()) else {
            return;
        };
        crate::sway_ipc::run_command(&format!("[con_id={}] focus", window.con_id));
        drop(entries);
        self.hide();
    }
}

/// The app's own icon where a theme ships one, a generic window otherwise.
/// `Image::from_icon_name` on an unknown name draws a broken-image
/// placeholder, and an app id is only sometimes an icon name — `Alacritty`
/// and `chrome-…-Default` are not.
fn icon_name(app_id: &str) -> String {
    let lower = app_id.to_lowercase();
    let known = gdk::Display::default()
        .map(|display| gtk4::IconTheme::for_display(&display).has_icon(&lower))
        .unwrap_or(false);
    if known {
        lower
    } else {
        "application-x-executable".to_string()
    }
}

/// Does this window match what has been typed?
///
/// App id and title both, because "chrome" and the half-remembered word from
/// a tab title are equally likely to be what comes to mind.
pub(crate) fn matches(window: &Window, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    window.app_id.to_lowercase().contains(query) || window.title.to_lowercase().contains(query)
}

/// Read every window out of sway's tree, most recently focused first, and
/// the output the switcher will be drawn on.
///
/// Order comes from sway's own `focus` arrays: each container lists its
/// children in the order they were last focused, so a depth-first walk that
/// follows them produces the session's MRU order without swaypplet having to
/// keep a history of its own. The focused output rides along because it is
/// needed before the surface maps, and one connection can answer both.
fn read_session() -> Result<(Vec<Window>, Option<String>), String> {
    let mut conn = swayipc::Connection::new().map_err(|e| format!("sway ipc: {e}"))?;
    let tree = conn.get_tree().map_err(|e| format!("sway ipc: {e}"))?;
    let focused = conn
        .get_outputs()
        .map_err(|e| format!("sway ipc: {e}"))?
        .into_iter()
        .find(|o| o.focused)
        .map(|o| o.name);
    Ok((collect(&tree), focused))
}

pub(crate) fn collect(root: &swayipc::Node) -> Vec<Window> {
    let mut out = Vec::new();
    walk(root, "", &mut out);
    out
}

fn walk(node: &swayipc::Node, workspace: &str, out: &mut Vec<Window>) {
    let workspace = match node.node_type {
        swayipc::NodeType::Workspace => node.name.as_deref().unwrap_or(workspace),
        _ => workspace,
    };

    // A node with an app id and no children is a window. Checking for
    // children rather than for a node type keeps xwayland containers, which
    // wrap their window in another node, from being counted twice.
    let is_window = node.nodes.is_empty()
        && node.floating_nodes.is_empty()
        && (node.app_id.is_some() || node.window_properties.is_some());

    if is_window && node.node_type != swayipc::NodeType::Workspace {
        out.push(Window {
            con_id: node.id,
            app_id: node
                .app_id
                .clone()
                .or_else(|| {
                    node.window_properties
                        .as_ref()
                        .and_then(|p| p.class.clone())
                })
                .unwrap_or_default(),
            title: node.name.clone().unwrap_or_default(),
            workspace: workspace.to_string(),
            identifier: node.foreign_toplevel_identifier.clone(),
        });
        return;
    }

    // `focus` first, in order, then anything it did not mention — a container
    // that has never been focused has an empty focus list and still has
    // children worth showing.
    let children: Vec<&swayipc::Node> = node
        .nodes
        .iter()
        .chain(node.floating_nodes.iter())
        .collect();

    for id in &node.focus {
        if let Some(child) = children.iter().find(|c| c.id == *id) {
            walk(child, workspace, out);
        }
    }
    for child in &children {
        if !node.focus.contains(&child.id) {
            walk(child, workspace, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(app_id: &str, title: &str) -> Window {
        Window {
            con_id: 1,
            app_id: app_id.into(),
            title: title.into(),
            workspace: "1:t1a".into(),
            identifier: None,
        }
    }

    #[test]
    fn an_empty_query_matches_everything() {
        assert!(matches(&window("Alacritty", "vim"), ""));
    }

    #[test]
    fn the_query_reaches_both_the_app_and_the_title() {
        let w = window("google-chrome", "Create Version · Frontend");
        assert!(matches(&w, "chrome"));
        assert!(matches(&w, "frontend"));
        assert!(!matches(&w, "slack"));
    }

    /// The size a card takes on screen, thumbnail plus chrome, which is what
    /// the fit actually has to keep inside the output.
    fn grid_size(count: usize, f: Fit) -> (i32, i32) {
        let rows = (count as i32 + f.columns - 1) / f.columns;
        (
            f.columns * (f.thumb_w + CARD_CHROME_W) + (f.columns - 1) * GAP + 2 * CARD_PADDING,
            rows * (f.thumb_h + CARD_CHROME_H) + (rows - 1) * GAP + 2 * CARD_PADDING,
        )
    }

    #[test]
    fn a_normal_session_gets_full_size_thumbnails() {
        // Twelve windows on the laptop's 1440x900 logical output.
        let f = fit(12, 1440, 900);
        assert_eq!(f.thumb_w, THUMB_W as i32);
        assert_eq!(f.columns, 4);
    }

    #[test]
    fn the_grid_stays_inside_the_output_it_is_drawn_on() {
        for (count, w, h) in [
            (12, 1440, 900),
            (26, 1440, 900),
            (26, 2560, 1440),
            (7, 1024, 768),
            (40, 2560, 1440),
        ] {
            let f = fit(count, w, h);
            let (gw, gh) = grid_size(count, f);
            assert!(gw <= w, "{count} windows on {w}x{h}: {gw} px wide");
            assert!(
                gh <= h || f.thumb_w == MIN_THUMB_W,
                "{count} windows on {w}x{h}: {gh} px tall"
            );
        }
    }

    #[test]
    fn thumbnails_never_shrink_past_recognisable() {
        // Forty windows on a small screen: the floor holds and the grid is
        // allowed to overflow rather than drawing forty unreadable squares.
        let f = fit(40, 1024, 768);
        assert_eq!(f.thumb_w, MIN_THUMB_W);
        assert!(f.thumb_h > 0);
    }

    #[test]
    fn one_window_is_one_column() {
        assert_eq!(fit(1, 2560, 1440).columns, 1);
        // And none at all still produces a usable shape rather than a
        // division by zero.
        assert!(fit(0, 2560, 1440).columns >= 1);
    }

    #[test]
    fn matching_ignores_case_on_the_window_side() {
        // The query is lowercased by the caller; the window's own strings
        // are not, and `Alacritty` is capitalised in every sway tree here.
        assert!(matches(&window("Alacritty", "leaves create"), "alacritty"));
    }
}

#[cfg(test)]
mod live {
    /// Reads the real session and captures a thumbnail for every window it
    /// finds. Ignored: needs a compositor.
    #[test]
    #[ignore]
    fn every_window_joins_to_its_pixels() {
        let (windows, _output) = super::read_session().expect("tree");
        assert!(!windows.is_empty(), "no windows");

        let (mut ok, mut missing, mut failed) = (0, 0, 0);
        for w in &windows {
            match &w.identifier {
                None => {
                    missing += 1;
                    println!("no identifier: {} {}", w.app_id, w.title);
                }
                Some(id) => match crate::screenshot::capture::toplevel(id) {
                    Ok(image) => {
                        let t = image.boxed_down(super::THUMB_W, super::THUMB_H);
                        ok += 1;
                        println!(
                            "{:<26} {}x{} -> {}x{}  [{}] {}",
                            w.app_id,
                            image.width,
                            image.height,
                            t.width,
                            t.height,
                            w.workspace,
                            w.title
                        );
                    }
                    Err(e) => {
                        failed += 1;
                        println!("FAILED {} {}: {e}", w.app_id, w.title);
                    }
                },
            }
        }
        println!("\n{ok} captured, {failed} failed, {missing} without an identifier");
        assert!(ok > 0, "nothing captured");
    }
}
