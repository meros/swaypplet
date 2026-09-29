//! Pilot's Helm: centered command cockpit and optical HUD fusing an instant
//! app launcher with glanceable telemetry pills, deep sub-sheet utility decks,
//! and flight action switches.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;

use crate::anim;
use crate::launcher::{LauncherView, Ran};
use crate::services::notifications::store::NotificationStore;
use crate::settings::Open;
use crate::shell::Surface;
use crate::ui::icons;
use crate::ui::{self, Kind, Text, Tone};
use crate::widgets::backup::BackupSection;
use crate::widgets::{
    audio::AudioSection,
    bluetooth::BluetoothSection,
    brightness::BrightnessSection,
    clipboard::ClipboardSection,
    display::DisplaySection,
    media::MediaSection,
    network::NetworkSection,
    notifications::NotificationsSection,
    power::{self, PowerSection},
    tiles,
    users::UserSection,
};

/// What the Helm card asks for on a screen with room for it: a launcher's
/// width, room for the four switches at their natural widths on one line,
/// and the width it renders at, since no row wants more. The results list
/// is read at one glance and the card stands on the screen rather than
/// across it (macOS Control Center, Raycast). One strip of every control
/// on a card about 1300 px wide was the other answer the 2026-09
/// alternatives zoo rendered (docs/alternatives-zoo.html); the owner chose
/// this one. No height: the sections decide how tall the card is, and the
/// deck stack has no sensible fixed height to fall back on.
const HELM_CARD_SIZE: crate::shell::fit::CardSize = crate::shell::fit::CardSize {
    width: 800,
    height: None,
};

/// How tall a sub-sheet's body stands on a screen with room for it.
const SUBSHEET_HEIGHT: i32 = 340;

/// Omnibox prefixes and the deck page each one opens. Settings' panes
/// carry their own (`settings::PANES`) and open settings' own surface, on
/// Enter rather than as they are typed, since that closes the Helm; `:set`
/// alone opens it as it was. A bare `:` lists all of them on the
/// `prefixes` page, which is built from this table so the list cannot go
/// stale.
const ROUTES: &[(&[&str], &str, &str)] = &[
    (
        &[
            ":wifi",
            ":net",
            ":vpn",
            ":ovpn",
            ":openvpn",
            ":wg",
            ":wireguard",
        ],
        "wifi",
        "Wi-Fi Networks",
    ),
    (&[":bt", ":blue"], "bluetooth", "Bluetooth Devices"),
    (&[":disp", ":screen"], "displays", "Display & Monitors"),
    (&[":clip", ":cb"], "clipboard", "Clipboard History"),
    (&[":power", ":sys"], "power", "System State & Power"),
    (&[":notif"], "notifications", "Notifications Center"),
    (&[":media", ":music"], "media", "Media Player"),
    (
        &[":audio", ":vol", ":sound", ":sink", ":mic"],
        "audio",
        "Audio & Sound Devices",
    ),
];

/// The deck page a typed prefix opens.
fn route(prefix: &str) -> Option<&'static str> {
    ROUTES
        .iter()
        .find(|(prefixes, _, _)| prefixes.iter().any(|p| prefix.starts_with(p)))
        .map(|(_, page, _)| *page)
}

/// Opens settings' own surface; the app hides the Helm first.
pub type OnSettings = Rc<dyn Fn(Open)>;

/// What the card's lists shrink to on an output too short for the card at
/// full density. They still scroll, so this costs visible rows and nothing
/// else. Everything below them in the card, the action deck above all, stays
/// on screen instead of being pushed off the bottom edge.
const COMPACT_LIST_HEIGHT: i32 = 80;

// ── Sections bundle ───────────────────────────────────────────────────────────

struct Sections {
    audio: AudioSection,
    brightness: BrightnessSection,
    network: NetworkSection,
    bluetooth: Rc<BluetoothSection>,
    display: DisplaySection,
    media: MediaSection,
    notifications: NotificationsSection,
    clipboard: ClipboardSection,
    power: PowerSection,
    users: UserSection,
    backup: BackupSection,
    /// Quick-strip toggle tiles (Night Light, Caffeine, etc.)
    tiles: RefCell<Vec<TileEntry>>,
    /// A typed restart or shut down waiting for its second Enter, and
    /// until when (`CONFIRM_FOR`).
    armed: std::cell::Cell<Option<(&'static str, std::time::Instant)>>,
    /// The DND tile and the store it shows, for its status on open.
    dnd: RefCell<Option<(ui::SplitTile, Rc<RefCell<NotificationStore>>)>>,
}

impl Sections {
    fn refresh(&self) {
        self.audio.refresh();
        self.brightness.refresh();
        self.network.refresh();
        self.bluetooth.refresh();
        self.display.refresh();
        self.media.refresh();
        self.notifications.refresh();
        self.clipboard.refresh();
        self.power.refresh();
        self.users.refresh();
        self.backup.refresh();
        for (btn, spec, status) in self.tiles.borrow().iter() {
            tiles::init_tile_state(btn, spec);
            if let Some(status) = status {
                tiles::refresh_status(status, spec);
            }
        }
        if let Some((dnd, store)) = self.dnd.borrow().as_ref() {
            let (on, text) = tiles::dnd_view(&store.borrow());
            dnd.toggle.set_active(on);
            ui::set_tile_status(dnd, &text);
        }
    }
}

// ── Panel (Pilot's Helm HUD) ──────────────────────────────────────────────────

pub struct Panel {
    /// The layer surface, built by the caller (`app::panel_surface`): its
    /// root is the full-screen backdrop, its card the Helm.
    surface: Surface,
    sections: Rc<Sections>,
    launcher: Rc<LauncherView>,
    deck_stack: gtk4::Stack,
    on_settings: OnSettings,
}

impl Panel {
    pub fn new(
        surface: Surface,
        store: Rc<RefCell<NotificationStore>>,
        audio_service: Rc<crate::services::audio::AudioService>,
        on_settings: OnSettings,
    ) -> Self {
        let window = surface.window().clone();

        // ── Backdrop (full-screen transparent click-catcher) ────────────────
        // The Surface's root, which carries the design system's base type
        // and colour (on the window's child, not the window: the GTK theme's
        // `window.background` outranks a class on the window node).
        let backdrop = surface.root().clone();
        backdrop.set_hexpand(true);
        backdrop.set_vexpand(true);

        // ── Top spacer (positions Helm at the optical foveal sweet spot ~25-28%) ──
        // Height and the card's width both come from
        // crate::shell::fit::install_monitor_fit below, against the output the
        // window lands on.
        let top_spacer = ui::vbox(0);
        backdrop.prepend(&top_spacer);

        // ── Root container (the floating glass Helm card) ─────────────────────
        let root = surface.card().clone();
        root.set_halign(gtk4::Align::Center);
        root.set_valign(gtk4::Align::Start);
        root.add_css_class("helm-card");

        // ── Build sections ───────────────────────────────────────────────────
        let audio = AudioSection::new(audio_service.clone());
        let brightness = BrightnessSection::new();
        let network = NetworkSection::new();
        let bluetooth =
            BluetoothSection::new(crate::services::bluetooth::BluetoothService::start());
        let display = DisplaySection::new();
        let media = MediaSection::new();
        let notifications = NotificationsSection::new(store.clone());
        let clipboard = ClipboardSection::new();
        let power = PowerSection::new();
        let users = UserSection::new();
        // Its own watcher: the panel outlives no bar process in particular,
        // and the status directory is two small files.
        let backup = BackupSection::new(&crate::services::backup::BackupStatusService::start());

        audio.expand_for_page();
        network.expand_for_page();
        bluetooth.expand_for_page();
        display.expand_for_page();
        clipboard.expand_for_page();
        power.expand_for_page();
        notifications.expand_for_page();

        let launcher = Rc::new(LauncherView::new());
        // The pages this card routes to, for the launcher to offer by name.
        launcher.set_pages(
            ROUTES
                .iter()
                .map(|(prefixes, _, title)| crate::launcher::Page::new(title, prefixes))
                .chain(crate::settings::prefixes().map(|(prefixes, title)| {
                    crate::launcher::Page::new(&title, prefixes)
                }))
                .collect(),
        );

        // ── Deck stack (Launcher stage ↔ In-place utility sub-sheets) ─────────
        let deck_stack = ui::page_stack(
            gtk4::StackTransitionType::Crossfade,
            crate::tokens::motion::EXPAND,
        );
        deck_stack.set_vexpand(true);
        deck_stack.add_css_class("helm-deck-stack");

        // Page 1: Default Elephant launcher stage
        let launcher_box = ui::vbox(0);
        launcher_box.set_vexpand(true);
        launcher_box.add_css_class("helm-launcher-stage");
        launcher_box.append(launcher.widget());
        deck_stack.add_named(&launcher_box, Some("launcher"));

        // Helper for returning from sub-sheets to search
        let return_to_search = {
            let deck_stack_c = deck_stack.clone();
            let launcher_c = launcher.clone();
            Rc::new(move || {
                deck_stack_c.set_visible_child_name("launcher");
                launcher_c.entry().set_text("");
                launcher_c.focus_entry();
            })
        };

        // Page 2: Wi-Fi Networks
        {
            let ret = return_to_search.clone();
            let wifi_sheet = build_subsheet("Wi-Fi Networks", "󰤨", network.widget(), move || ret());
            deck_stack.add_named(&wifi_sheet, Some("wifi"));
        }

        // Page 3: Bluetooth Devices
        {
            let ret = return_to_search.clone();
            let bt_sheet =
                build_subsheet("Bluetooth Devices", "󰂯", bluetooth.widget(), move || {
                    ret()
                });
            deck_stack.add_named(&bt_sheet, Some("bluetooth"));
        }

        // Page 4: Displays & Output configuration
        {
            let ret = return_to_search.clone();
            let disp_sheet =
                build_subsheet("Display & Monitors", "󰍹", display.widget(), move || {
                    ret()
                });
            deck_stack.add_named(&disp_sheet, Some("displays"));
        }

        // Page 5: Clipboard History
        {
            let ret = return_to_search.clone();
            let clip_sheet =
                build_subsheet("Clipboard History", "󰅍", clipboard.widget(), move || {
                    ret()
                });
            deck_stack.add_named(&clip_sheet, Some("clipboard"));
        }

        // Page 6: System Power Diagnostics & Users
        {
            let ret = return_to_search.clone();
            let power_container = ui::vbox(4);
            power_container.append(power.widget());
            power_container.append(users.widget());
            power_container.append(backup.widget());
            let power_sheet =
                build_subsheet("System State & Power", "󰁹", &power_container, move || ret());
            deck_stack.add_named(&power_sheet, Some("power"));
        }

        // Page 7: Notifications Center
        {
            let ret = return_to_search.clone();
            let notif_sheet = build_subsheet(
                "Notifications Center",
                "󰂚",
                notifications.widget(),
                move || ret(),
            );
            deck_stack.add_named(&notif_sheet, Some("notifications"));
        }

        // Page 8: Media Player
        {
            let ret = return_to_search.clone();
            let media_sheet = build_subsheet("Media Player", "󰝚", media.widget(), move || ret());
            deck_stack.add_named(&media_sheet, Some("media"));
        }

        // Page 9: Audio & Sound Devices
        {
            let ret = return_to_search.clone();
            let audio_sheet =
                build_subsheet("Audio & Sound Devices", "󰕾", audio.widget(), move || {
                    ret()
                });
            deck_stack.add_named(&audio_sheet, Some("audio"));
        }

        // Page 11: the prefixes, for a bare `:`.
        {
            let ret = return_to_search.clone();
            let sheet = build_subsheet("Prefixes", "󰘳", &build_prefix_list(), move || ret());
            deck_stack.add_named(&sheet, Some("prefixes"));
        }

        // ── Top Telemetry Ribbon ─────────────────────────────────────────────
        let telemetry_ribbon = build_telemetry_ribbon(&deck_stack, &audio, &brightness, &network);

        // ── Bottom Action Flight Deck ─────────────────────────────────────────
        // Each deck tile is kept beside its spec so `refresh` can re-read it
        // when the menu opens.
        let mut tile_pairs: Vec<TileEntry> = Vec::new();
        let mut dnd_tile = None;
        let flight_deck = build_flight_deck(
            &window,
            &store,
            &mut tile_pairs,
            &mut dnd_tile,
            &deck_stack,
            &on_settings,
        );

        // ── Assemble Content ─────────────────────────────────────────────────
        let content = ui::vbox(0);
        content.append(&telemetry_ribbon);
        content.append(&deck_stack);
        content.append(&flight_deck);
        root.append(&content);

        // `root`, not the slide bin around it, is what carries the size
        // request: the bin lays its child out with a BinLayout, so a request
        // on the bin would be overridden by the child's own.
        //
        // The lists are what the card gives up when the output is short, and
        // the only thing it gives up: every control keeps its place and its
        // size, the lists just show fewer rows before scrolling.
        let lists = elastic_lists(&launcher, &deck_stack);
        crate::shell::fit::install_monitor_fit(
            &window,
            &top_spacer,
            &root,
            HELM_CARD_SIZE,
            Some(Rc::new(move |compact| {
                for (list, full) in &lists {
                    list.set_min_content_height(if compact { COMPACT_LIST_HEIGHT } else { *full });
                }
            })),
            None,
        );

        // Enter/exit transition for the glass menu: fast pane tint,
        // full-length content fade, the short settle the Surface was built
        // with (motion on glass, anim.rs).
        surface.set_content(&content);

        // Shared dismiss path: fade the menu out, then unmap.
        let hide_menu = {
            let surface = surface.clone();
            Rc::new(move || surface.hide())
        };
        // A notification clicked through to its sender takes the panel away.
        notifications.set_on_activate(hide_menu.clone());

        // ── Prefix routing from Omnibox ──────────────────────────────────────
        {
            let deck_stack_c = deck_stack.clone();
            let network_c = network.clone();
            launcher.entry().connect_search_changed(move |entry| {
                let text = entry.text().to_string();
                let lower = text.to_lowercase();
                let prefix = lower.trim();
                if prefix == ":" {
                    deck_stack_c.set_visible_child_name("prefixes");
                } else if let Some(page) = route(prefix) {
                    if page == "wifi" {
                        network_c.trigger_scan();
                    }
                    deck_stack_c.set_visible_child_name(page);
                } else if !prefix.starts_with(':')
                    && deck_stack_c.visible_child_name().as_deref() != Some("launcher")
                {
                    deck_stack_c.set_visible_child_name("launcher");
                }
            });
        }

        // ── A settings prefix, on Enter ──────────────────────────────────────
        // Taken before the selected row, which for `:capture` would be the
        // region shot its synonym matches, and for `:set` an app.
        {
            let on_settings = on_settings.clone();
            launcher.set_on_enter(move |text| match Open::for_prefix(&text.to_lowercase()) {
                Some(open) => {
                    on_settings(open);
                    true
                }
                None => false,
            });
        }

        // ── Arrange displays… on the displays page ───────────────────────────
        {
            let on_settings = on_settings.clone();
            display.set_on_arrange(move || on_settings(Open::Pane("displays")));
        }

        // ── Settings rows and pages from the launcher ────────────────────────
        {
            let on_settings = on_settings.clone();
            let entry = launcher.entry().clone();
            launcher.set_on_setting(move |id| open_setting(&on_settings, &entry, id));
        }

        // ── Launcher activation + Esc hide the menu / return to search ───────
        {
            let hide = hide_menu.clone();
            launcher.set_on_activate(move || hide());
        }
        {
            let hide = hide_menu.clone();
            let deck_stack_c = deck_stack.clone();
            let launcher_c = launcher.clone();
            launcher.install_key_controller(&window, move || {
                if deck_stack_c.visible_child_name().as_deref() != Some("launcher") {
                    deck_stack_c.set_visible_child_name("launcher");
                    launcher_c.entry().set_text("");
                    launcher_c.focus_entry();
                } else {
                    hide();
                }
            });
        }

        // ── Backdrop click → dismiss ─────────────────────────────────────────
        // A hit test rather than a claiming gesture on the card, which can
        // starve the controls inside it (see Surface::connect_backdrop_click).
        {
            let hide = hide_menu.clone();
            surface.connect_backdrop_click(move || hide());
        }

        // ── Sections bundle ──────────────────────────────────────────────────
        let sections = Rc::new(Sections {
            audio,
            brightness,
            network,
            bluetooth,
            display,
            media,
            notifications,
            clipboard,
            power,
            users,
            backup,
            tiles: RefCell::new(tile_pairs),
            armed: std::cell::Cell::new(None),
            dnd: RefCell::new(dnd_tile.map(|t| (t, store.clone()))),
        });

        // ── What the deck does, by name: "no sleep", "dnd", "lock" ───────────
        {
            let list = {
                let weak = Rc::downgrade(&sections);
                move || weak.upgrade().map(|s| deck_actions(&s)).unwrap_or_default()
            };
            let run = {
                let weak = Rc::downgrade(&sections);
                let launcher = Rc::downgrade(&launcher);
                let window = window.clone();
                let store = store.clone();
                move |id: &str| match (weak.upgrade(), launcher.upgrade()) {
                    (Some(s), Some(l)) => run_deck_action(&s, &window, &store, &l, id),
                    _ => Ran::Hide,
                }
            };
            launcher.set_actions(list, run);
        }

        Self {
            surface,
            sections,
            launcher,
            deck_stack,
            on_settings,
        }
    }

    pub fn window(&self) -> &gtk4::Window {
        self.surface.window()
    }

    pub fn toggle(&self) {
        if self.surface.is_shown() && self.surface.window().is_visible() {
            self.surface.hide();
        } else {
            self.launcher.reset();
            self.deck_stack.set_visible_child_name("launcher");
            self.surface.show();
            self.launcher.focus_entry();
            // Harness hook: the nested session in dev/render.sh has no
            // keyboard, so `SWAYPPLET_PANEL_QUERY` types an omnibox prefix
            // (":wifi") on open, for a shot of that sub-sheet.
            if let Ok(query) = std::env::var("SWAYPPLET_PANEL_QUERY")
                && !query.is_empty()
            {
                let entry = self.launcher.entry().clone();
                glib::timeout_add_local_once(std::time::Duration::from_millis(300), move || {
                    entry.set_text(&query)
                });
                // `SWAYPPLET_PANEL_ACTIVATE=1` then presses Enter 1.5 s
                // later, or that many milliseconds after opening when it is
                // a larger number: a result opened, a settings row among
                // them, or settings itself for a settings prefix, timed to
                // land between the harness's shots.
                if let Ok(v) = std::env::var("SWAYPPLET_PANEL_ACTIVATE")
                    && !v.is_empty()
                {
                    let ms = v.parse::<u64>().ok().filter(|ms| *ms > 1).unwrap_or(1800);
                    // Through Enter's own path, so a shot of it is a test of
                    // it: the hook once opened settings by itself, and a
                    // settings prefix that Enter never reached looked fine.
                    let launcher = self.launcher.clone();
                    glib::timeout_add_local_once(std::time::Duration::from_millis(ms), move || {
                        launcher.press_enter()
                    });
                }
            }
            // The section reads land as widget churn (sysfs, clipboard rows,
            // wallpaper rescans, eight worker threads); measured on open they
            // cost 2-11 ms on the main thread right where the fade needs the
            // frame clock, and the elephant rebuild collides with them. Run
            // them once the enter transition is over.
            let sections = self.sections.clone();
            glib::timeout_add_local_once(
                std::time::Duration::from_millis((anim::duration(anim::ENTER_MS) + 60.0) as u64),
                move || sections.refresh(),
            );
        }
    }

    /// Open on the notification centre: the quiet summary card's click
    /// (`services::notifications::context`).
    pub fn show_notifications(&self) {
        if !(self.surface.is_shown() && self.surface.window().is_visible()) {
            self.toggle();
        }
        self.deck_stack.set_visible_child_name("notifications");
    }

    /// Open what a launcher result names (`settings::search`): a settings
    /// row in settings, or a Helm page, opening the Helm first.
    pub fn open_setting(&self, id: &str) {
        if !id.starts_with(':') || Open::for_prefix(id).is_some() {
            open_setting(&self.on_settings, self.launcher.entry(), id);
            return;
        }
        self.open_page(id);
    }

    /// Show the Helm on the page `prefix` opens (`:wifi`).
    pub fn open_page(&self, prefix: &str) {
        if !self.is_shown() {
            self.toggle();
        }
        let entry = self.launcher.entry();
        entry.set_text(prefix);
        entry.set_position(-1);
    }

    pub fn is_shown(&self) -> bool {
        self.surface.is_shown() && self.surface.window().is_visible()
    }

    pub fn hide(&self) {
        if self.is_shown() {
            self.surface.hide();
        }
    }

    pub fn refresh_audio(&self) {
        self.sections.audio.refresh();
    }

    pub fn refresh_brightness(&self) {
        self.sections.brightness.refresh();
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Every list in the card whose height may be given up on a short output: the
/// launcher's results, and the body of each sub-sheet.
///
/// Collected by walking the built pages rather than threaded through
/// `build_subsheet`, which nine call sites would have had to pass along. Each
/// sub-sheet keeps its scroller as a direct child, so one level is enough.
/// Each list is paired with the height it was built with, so full density is
/// whatever the widget already asked for and no number is written twice.
fn elastic_lists(
    launcher: &LauncherView,
    deck_stack: &gtk4::Stack,
) -> Vec<(gtk4::ScrolledWindow, i32)> {
    let mut lists = vec![launcher.scroller().clone()];
    let mut page = deck_stack.first_child();
    while let Some(current) = page {
        let mut child = current.first_child();
        while let Some(widget) = child {
            if let Ok(list) = widget.clone().downcast::<gtk4::ScrolledWindow>() {
                lists.push(list);
            }
            child = widget.next_sibling();
        }
        page = current.next_sibling();
    }
    lists
        .into_iter()
        .map(|list| {
            let full = list.min_content_height();
            (list, full)
        })
        .collect()
}

/// A launcher result that opens something: a settings pane or row, in
/// settings; a Helm page by its prefix, which the omnibox routes like a
/// typed one.
fn open_setting(on_settings: &OnSettings, entry: &gtk4::SearchEntry, id: &str) {
    if let Some(open) = Open::for_prefix(id) {
        on_settings(open);
    } else if id.starts_with(':') {
        entry.set_text(id);
        entry.set_position(-1);
    } else {
        on_settings(Open::Row(id.to_string()));
    }
}

/// Every prefix the omnibox knows, one row per page, from the same tables
/// the router reads.
fn build_prefix_list() -> gtk4::Box {
    let list = ui::vbox(1);
    list.add_css_class("prefix-list");

    let hint = ui::text(
        "Type one of these after the colon to open its page; a settings one opens on Enter. Anything else searches.",
        Text::Caption,
        Tone::Faint,
    );
    hint.set_wrap(true);
    hint.add_css_class("prefix-hint");
    list.append(&hint);

    let rows: Vec<(String, String)> = ROUTES
        .iter()
        .map(|(prefixes, _, title)| (prefixes.join("  "), title.to_string()))
        .chain(crate::settings::prefixes().map(|(prefixes, title)| (prefixes.join("  "), title)))
        .chain(std::iter::once((
            ":set  :pref".to_string(),
            "Settings, as it was".to_string(),
        )))
        .collect();
    for (prefixes, title) in rows {
        let row = ui::hbox(4);
        row.add_css_class("prefix-row");
        // The page first, in a fixed gutter, because that is what the list
        // is scanned by; what you type for it beside it, in the mono keys
        // are set in, a level quieter, wrapping where a page has many.
        let page = ui::text(&title, Text::Body, Tone::Fg);
        page.set_width_chars(22);
        page.set_xalign(0.0);
        page.set_valign(gtk4::Align::Start);
        let keys = ui::text(&prefixes, Text::Label, Tone::Muted);
        crate::ui::set_mono(&keys, true);
        keys.set_hexpand(true);
        keys.set_wrap(true);
        keys.set_valign(gtk4::Align::Start);
        row.append(&page);
        row.append(&keys);
        list.append(&row);
    }
    list
}

fn build_subsheet(
    title: &str,
    icon: &str,
    content: &impl IsA<gtk4::Widget>,
    on_back: impl Fn() + 'static,
) -> gtk4::Box {
    build_subsheet_with_tabs(title, icon, None::<&gtk4::Widget>, content, on_back)
}

fn build_subsheet_with_tabs(
    title: &str,
    icon: &str,
    tabs: Option<&impl IsA<gtk4::Widget>>,
    content: &impl IsA<gtk4::Widget>,
    on_back: impl Fn() + 'static,
) -> gtk4::Box {
    let container = ui::vbox(3);
    container.add_css_class("helm-subsheet");

    let header = if tabs.is_some() {
        ui::vbox(3)
    } else {
        ui::hbox(3)
    };
    header.add_css_class("subsheet-header");

    let top_row = if tabs.is_some() {
        let row = ui::hbox(3);
        header.append(&row);
        row
    } else {
        header.clone()
    };

    let back_btn = ui::button_with(
        ui::Face::Label("← Back (Esc)"),
        Kind::Secondary,
        ui::Size::Small,
    );
    back_btn.set_valign(gtk4::Align::Center);
    back_btn.connect_clicked(move |_| on_back());
    top_row.append(&back_btn);

    let title_lbl = ui::text(&format!("{icon}  {title}"), Text::TitleSm, Tone::Fg);
    title_lbl.set_hexpand(true);
    title_lbl.set_halign(gtk4::Align::Start);
    top_row.append(&title_lbl);

    if let Some(tabs) = tabs {
        header.append(tabs.as_ref());
    }

    container.append(&header);

    // The floor is a property rather than a CSS `min-height`, because GTK
    // takes the larger of the two and a rule would then outrank the fit when
    // it has to shrink this list to keep the card on a short screen.
    let scroller = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .propagate_natural_width(false)
        .vexpand(true)
        .min_content_height(SUBSHEET_HEIGHT)
        .max_content_height(420)
        .child(content)
        .build();
    container.append(&scroller);

    container
}

/// Flip the deck between `page` and the launcher, and say whether it is now
/// on `page`.
/// One deck tile the panel re-reads on open: its toggle, its spec, and its
/// status line when it has one.
type TileEntry = (gtk4::ToggleButton, tiles::TileSpec, Option<gtk4::Label>);

/// The inhibitor a deck spec switches, if it is one.
fn inhibitor_of(spec: &tiles::TileSpec) -> Option<crate::services::inhibit::Inhibitor> {
    crate::services::inhibit::Inhibitor::ALL
        .into_iter()
        .find(|w| w.label() == spec.label)
}

/// Put a built split tile on the deck (through `add`, the strip's or the
/// grid's own append) and in the refresh list.
fn push_tile(
    tile: ui::SplitTile,
    spec: tiles::TileSpec,
    tile_pairs: &mut Vec<TileEntry>,
    add: &dyn Fn(&gtk4::Box),
) {
    tiles::init_tile_state(&tile.toggle, &spec);
    tiles::refresh_status(&tile.status, &spec);
    tile.root.add_css_class("deck-tile-btn");
    add(&tile.root);
    tile_pairs.push((tile.toggle, spec, Some(tile.status)));
}

fn flip(stack: &gtk4::Stack, page: &str) -> bool {
    if stack.visible_child_name().as_deref() == Some(page) {
        stack.set_visible_child_name("launcher");
        false
    } else {
        stack.set_visible_child_name(page);
        true
    }
}

/// A ribbon pill: a pill button with a glyph and a word, which flips the
/// deck to `page`. The glyph is muted, not coloured: colour on the ribbon
/// would claim a status none of these pills reports.
fn ribbon_pill(icon: &str, label: &str, stack: &gtk4::Stack, page: &'static str) -> gtk4::Button {
    let line = ui::hbox(2);
    let glyph = gtk4::Label::new(Some(icon));
    ui::glyph::adopt(&glyph, Text::TitleSm, Tone::Muted);
    line.append(&glyph);
    line.append(&ui::text(label, Text::Label, Tone::Fg));
    let pill = ui::button_with(
        ui::Face::Child(line.upcast_ref()),
        Kind::Secondary,
        ui::Size::Normal,
    );
    pill.add_css_class("pill");
    let stack = stack.clone();
    pill.connect_clicked(move |_| {
        flip(&stack, page);
    });
    pill
}

/// A ribbon pill holding a slider, and the glyph button in front of it that
/// flips the deck to the slider's page.
///
/// The ribbon's slider is its own, on the section's adjustment, rather than
/// the section's slider moved up here: that one carries the page's marks
/// (100 % on the volume rail), which GTK draws under the track, and in a
/// pill one control tall they pushed the track against the pill's top edge.
/// Sharing the adjustment keeps the two in step both ways, and the section's
/// own slider stays on its page.
fn ribbon_slider(
    icon: &str,
    tooltip: &str,
    adjustment: &gtk4::Adjustment,
    stack: &gtk4::Stack,
    page: &'static str,
) -> gtk4::Box {
    let pill = ui::pill_group(2);
    let btn = ui::button_with(
        ui::Face::Glyph {
            glyph: icon,
            tooltip,
        },
        Kind::Flat,
        ui::Size::Normal,
    );
    btn.add_css_class("pill");
    btn.set_valign(gtk4::Align::Center);
    {
        let stack = stack.clone();
        btn.connect_clicked(move |_| {
            flip(&stack, page);
        });
    }
    pill.append(&btn);

    let scale = gtk4::Scale::new(gtk4::Orientation::Horizontal, Some(adjustment));
    ui::slider::adopt(&scale, ui::Density::Normal);
    scale.set_draw_value(false);
    scale.set_hexpand(true);
    scale.set_valign(gtk4::Align::Center);
    // A floor, not a size: the scale expands to fill the ribbon wherever
    // there is room, so lowering it only decides how much the ribbon can give
    // up on a narrow output. A slider is the right thing to squeeze first,
    // because it stays usable at any width while a label has to be cut.
    scale.set_width_request(44);
    pill.append(&scale);
    pill
}

/// The ribbon: the two sliders you reach for, and the two radios you
/// switch. The bar already shows the battery and the notifications, so the
/// ribbon does not repeat them; the power and notification pages open by
/// name (`:power`, `:notif`) and from the DND tile's chevron.
fn build_telemetry_ribbon(
    deck_stack: &gtk4::Stack,
    audio: &AudioSection,
    brightness: &BrightnessSection,
    network: &NetworkSection,
) -> gtk4::Box {
    let ribbon = ui::hbox(2);
    ribbon.set_hexpand(true);
    ribbon.add_css_class("helm-telemetry-ribbon");

    // Volume: the glyph opens the devices and the mixer.
    ribbon.append(&ribbon_slider(
        icons::SPEAKER_HIGH,
        "Audio devices and mixer (:audio)",
        &audio.output_volume_scale().adjustment(),
        deck_stack,
        "audio",
    ));

    // Brightness: the glyph opens the displays page, the one door to it.
    ribbon.append(&ribbon_slider(
        icons::BRIGHTNESS,
        "Displays (:disp)",
        &brightness.brightness_scale().adjustment(),
        deck_stack,
        "displays",
    ));

    // Wi-Fi: opening the page also starts a scan.
    let pill_wifi = ribbon_pill("󰤨", "Wi-Fi", deck_stack, "wifi");
    {
        let stack_c = deck_stack.clone();
        let net_c = network.clone();
        pill_wifi.connect_clicked(move |_| {
            if stack_c.visible_child_name().as_deref() == Some("wifi") {
                net_c.trigger_scan();
            }
        });
    }
    ribbon.append(&pill_wifi);

    ribbon.append(&ribbon_pill("󰂯", "Bluetooth", deck_stack, "bluetooth"));

    ribbon
}

fn build_flight_deck(
    window: &gtk4::Window,
    store: &Rc<RefCell<NotificationStore>>,
    tile_pairs: &mut Vec<TileEntry>,
    dnd_tile: &mut Option<ui::SplitTile>,
    deck_stack: &gtk4::Stack,
    on_settings: &OnSettings,
) -> gtk4::Box {
    // The switches: one line of the grid, at equal widths; the actions and
    // the session take the next.
    let switches = ui::hbox(3);
    switches.set_homogeneous(true);
    switches.set_hexpand(true);
    switches.add_css_class("deck-switches");
    let add_switch: Box<dyn Fn(&gtk4::Box)> = {
        let switches = switches.clone();
        Box::new(move |w| switches.append(w))
    };

    // The durations for the timed switches fold out under the deck; one
    // fold, shared by both (`tiles::DurationFold`).
    let fold = tiles::DurationFold::new();
    {
        // Folded away whenever the panel closes, so it never reopens stale.
        let fold = fold.clone();
        window.connect_unmap(move |_| fold.close());
    }

    // Night Light + the session inhibitors, in the order tiles.rs gives
    // them, as split tiles: the body toggles, the chevron opens the detail
    // (the display page; a duration for the inhibitors), and the line under
    // the name says why the tile is in its state.
    for spec in tiles::deck_specs() {
        let detail: Box<dyn Fn(&gtk4::Button)> = match inhibitor_of(&spec) {
            Some(which) => {
                let entry: Rc<RefCell<Option<(gtk4::ToggleButton, gtk4::Label)>>> = Rc::default();
                let tile = {
                    let entry = entry.clone();
                    let spec_c = spec.clone();
                    let fold = fold.clone();
                    tiles::build_split(&spec, move |anchor| {
                        let entry = entry.clone();
                        let spec_c = spec_c.clone();
                        fold.toggle(anchor, which, move |ok| {
                            if let (true, Some((toggle, status))) = (ok, entry.borrow().as_ref()) {
                                toggle.set_active(true);
                                tiles::refresh_status(status, &spec_c);
                            }
                        });
                    })
                };
                *entry.borrow_mut() = Some((tile.toggle.clone(), tile.status.clone()));
                // Harness hook: `SWAYPPLET_PANEL_FOLD=no-sleep` opens that
                // switch's durations once the panel is up, for a shot of the
                // fold (the nested session has no pointer).
                // "No Sleep" or "no-sleep": the dash form keeps a space out
                // of dev/render-all.sh's env field.
                let asked = std::env::var("SWAYPPLET_PANEL_FOLD")
                    .map(|v| v.replace('-', " "))
                    .unwrap_or_default();
                if asked.eq_ignore_ascii_case(which.label()) {
                    let detail = tile.detail.clone();
                    glib::timeout_add_local_once(std::time::Duration::from_millis(600), move || {
                        detail.emit_clicked();
                    });
                }
                push_tile(tile, spec, tile_pairs, &add_switch);
                continue;
            }
            // Night Light: the chevron is how warm, which is a setting; the
            // body is on or off.
            None => {
                let on_settings = on_settings.clone();
                let warmth =
                    crate::settings::search::Target::Row("look", "Night light", "Night warmth")
                        .id();
                Box::new(move |_: &gtk4::Button| on_settings(Open::Row(warmth.clone())))
            }
        };
        let tile = tiles::build_split(&spec, detail);
        push_tile(tile, spec, tile_pairs, &add_switch);
    }

    // DND: the chevron opens the notification centre.
    let dnd = {
        let stack_c = deck_stack.clone();
        tiles::build_dnd_split(store.clone(), move |_| {
            flip(&stack_c, "notifications");
        })
    };
    dnd.root.add_css_class("deck-tile-btn");
    add_switch(&dnd.root);

    // The one-click actions, as one group on the deck's second row.
    let actions = ui::hbox(3);
    *dnd_tile = Some(dnd);

    // Screenshot Region
    actions.append(&rail_action("󰄀", "Screenshot region", window, {
        let window = window.clone();
        let store = store.clone();
        move || shot(&window, &store, crate::screenshot::Shot::Region)
    }));

    // Color Picker Loupe
    actions.append(&rail_action("󰏘", "Color picker loupe", window, {
        let window = window.clone();
        let store = store.clone();
        move || shot(&window, &store, crate::screenshot::Shot::Pick)
    }));

    // Record Screen
    actions.append(&rail_action("󰑋", "Record screen", window, {
        let window = window.clone();
        let store = store.clone();
        move || shot(&window, &store, crate::screenshot::Shot::Record)
    }));

    // Clipboard Drawer
    let clip_btn = deck_button("󰅍", "Clipboard history");
    {
        let stack_c = deck_stack.clone();
        clip_btn.connect_clicked(move |_| {
            flip(&stack_c, "clipboard");
        });
    }
    actions.append(&clip_btn);

    // Settings
    let settings_btn = deck_button("󰒓", "Settings (:set)");
    {
        let on_settings = on_settings.clone();
        settings_btn.connect_clicked(move |_| on_settings(Open::Last));
    }
    actions.append(&settings_btn);

    // Session cluster, at the second row's far end. Centred rather than
    // filling, so the buttons keep their own height.
    let session = power::build_session_row();
    session.set_valign(gtk4::Align::Center);

    let spacer = ui::hbox(0);
    spacer.set_hexpand(true);

    // Two rows: the switches, then the actions left and the session right.
    let deck = ui::vbox(3);
    deck.append(&switches);
    let second = ui::hbox(3);
    second.append(&actions);
    second.append(&spacer);
    second.append(&session);
    deck.append(&second);
    deck.set_hexpand(true);
    deck.add_css_class("helm-action-deck");

    // The deck and, under it, the fold its timed switches open.
    let column = ui::vbox(2);
    column.set_hexpand(true);
    column.append(&deck);
    column.append(&fold.root);
    column
}

/// A deck button: a glyph at title size on a component button.
fn deck_button(icon: &str, tooltip: &str) -> gtk4::Button {
    let glyph = gtk4::Label::new(Some(icon));
    ui::glyph::adopt(&glyph, Text::Title, Tone::Fg);
    let btn = ui::button_with(
        ui::Face::Icon {
            child: glyph.upcast_ref(),
            tooltip,
        },
        Kind::Secondary,
        ui::Size::Normal,
    );
    btn.add_css_class("deck-btn");
    btn
}

/// A rail icon button that hides the menu instantly (no exit wipe — the
/// action may capture the screen), then runs `action`. The stale reveal
/// state this leaves behind is healed by `Panel::toggle`.
fn rail_action(
    icon: &str,
    tooltip: &str,
    window: &gtk4::Window,
    action: impl Fn() + 'static,
) -> gtk4::Button {
    let btn = deck_button(icon, tooltip);
    let window_c = window.clone();
    btn.connect_clicked(move |_| {
        window_c.set_visible(false);
        action();
    });
    btn
}

/// Words a deck switch also answers to, beyond its name.
fn synonyms(label: &str) -> &'static [&'static str] {
    match label {
        "Night Light" => &["night shift", "warm", "blue light", "gamma"],
        "No Sleep" => &["caffeine", "keep awake", "awake", "inhibit"],
        "No Lock" => &["presentation", "keep unlocked", "inhibit"],
        _ => &[],
    }
}

/// A switch's state for its row: on or off, and the tile's reason.
fn state_line(on: bool, status: Option<&gtk4::Label>) -> String {
    let state = if on { "On" } else { "Off" };
    match status.map(|l| l.text()).filter(|t| !t.is_empty()) {
        Some(why) => format!("{state} · {why}"),
        None => state.to_string(),
    }
}

/// Everything the deck does, as launcher rows: its switches in the state
/// they are in now, the timed ones for a while, its shots, and the
/// session. The deck stays for the pointer; this is the same set for the
/// keyboard, run by the same code (`run_deck_action`).
fn deck_actions(sections: &Sections) -> Vec<crate::launcher::Action> {
    use crate::launcher::Action;
    let mut out = Vec::new();
    for (toggle, spec, status) in sections.tiles.borrow().iter() {
        out.push(Action::new(
            &format!("tile:{}", spec.label),
            spec.label,
            &state_line(toggle.is_active(), status.as_ref()),
            synonyms(spec.label),
        ));
        if inhibitor_of(spec).is_some() {
            for (label, minutes) in tiles::DURATIONS {
                let Some(m) = minutes else { continue };
                out.push(Action::new(
                    &format!("for:{}:{m}", spec.label),
                    &format!("{} for {label}", spec.label),
                    "Then off by itself",
                    synonyms(spec.label),
                ));
            }
        }
    }
    if let Some((dnd, _)) = sections.dnd.borrow().as_ref() {
        out.push(Action::new(
            "dnd",
            "Do not disturb",
            &state_line(dnd.toggle.is_active(), Some(&dnd.status)),
            &["dnd", "quiet", "silence", "notifications"],
        ));
    }
    out.extend([
        Action::new(
            "shot:region",
            "Screenshot a region",
            "Drag a rectangle; a click takes the screen",
            &["capture", "snip", "print"],
        ),
        Action::new(
            "shot:window",
            "Screenshot a window",
            "Pick one from a live grid",
            &["capture", "print"],
        ),
        Action::new(
            "shot:pick",
            "Pick a colour",
            "The colour under the pointer",
            &["color", "picker", "eyedropper", "loupe"],
        ),
        Action::new(
            "shot:record",
            "Record the screen",
            "A region, as video",
            &["screencast", "video", "capture"],
        ),
        Action::new(
            "session:lock",
            "Lock",
            "Lock the screen now",
            &["lock screen"],
        ),
        Action::new(
            "session:suspend",
            "Suspend",
            "Sleep now; the session stays",
            &["sleep"],
        ),
        Action::new(
            "session:logout",
            "Log out",
            "End the session now",
            &["logout", "sign out", "exit"],
        )
        .whole_words(),
        Action::new(
            "session:reboot",
            "Restart",
            confirm_line(sections, "session:reboot", "restart"),
            &["reboot"],
        )
        .whole_words(),
        Action::new(
            "session:poweroff",
            "Shut down",
            confirm_line(sections, "session:poweroff", "shut down"),
            &["shutdown", "power off", "poweroff"],
        )
        .whole_words(),
    ]);
    out
}

/// How long a typed restart or shut down waits for its second Enter, as
/// the rail's buttons wait for their second click (`power::wire_confirm`).
const CONFIRM_FOR: std::time::Duration = std::time::Duration::from_secs(3);

/// Whether `id` waits for its second Enter now.
fn is_armed(sections: &Sections, id: &str) -> bool {
    sections
        .armed
        .get()
        .is_some_and(|(armed, until)| armed == id && std::time::Instant::now() < until)
}

/// The row's line for a confirmed action: what the first Enter does, or,
/// once armed, what the second will.
fn confirm_line(sections: &Sections, id: &str, verb: &str) -> &'static str {
    match (is_armed(sections, id), verb) {
        (true, "restart") => "Press Enter again to restart now",
        (true, _) => "Press Enter again to shut down now",
        (false, _) => "Enter, then Enter again to confirm",
    }
}

/// Run a deck action by its row's identifier (`deck_actions`), and say
/// what is left: hide, nothing (a shot hides the Helm at once, so the
/// capture does not hold it), or stay (a restart waiting for its second
/// Enter).
fn run_deck_action(
    sections: &Sections,
    window: &gtk4::Window,
    store: &Rc<RefCell<NotificationStore>>,
    launcher: &Rc<LauncherView>,
    id: &str,
) -> Ran {
    use crate::widgets::power::Session;
    log::debug!("helm: action {id}");
    if let Some(label) = id.strip_prefix("tile:") {
        // The tile's own click: its action, its state, its status line.
        if let Some((toggle, _, _)) = sections
            .tiles
            .borrow()
            .iter()
            .find(|(_, s, _)| s.label == label)
        {
            toggle.emit_clicked();
        }
        return Ran::Hide;
    }
    if let Some((label, minutes)) = id.strip_prefix("for:").and_then(|r| r.rsplit_once(':')) {
        let which = crate::services::inhibit::Inhibitor::ALL
            .into_iter()
            .find(|w| w.label() == label);
        if let (Some(which), Ok(m)) = (which, minutes.parse::<u32>()) {
            crate::spawn::spawn_work(move || which.arm_for(true, Some(m)), |_| {});
        }
        return Ran::Hide;
    }
    let shot_of = |kind| {
        shot(window, store, kind);
        Ran::Away
    };
    // Restart and shut down act on the second Enter within `CONFIRM_FOR`;
    // the first arms the row, which says so, and the arm lapses by itself.
    let confirmed = |id: &'static str, session: Session| {
        if is_armed(sections, id) {
            sections.armed.set(None);
            session.run();
            return Ran::Hide;
        }
        sections
            .armed
            .set(Some((id, std::time::Instant::now() + CONFIRM_FOR)));
        let entry = launcher.entry().clone();
        glib::timeout_add_local_once(CONFIRM_FOR, move || {
            entry.emit_by_name::<()>("search-changed", &[]);
        });
        Ran::Stay
    };
    match id {
        "dnd" => {
            if let Some((dnd, _)) = sections.dnd.borrow().as_ref() {
                dnd.toggle.emit_clicked();
            }
            Ran::Hide
        }
        "shot:region" => shot_of(crate::screenshot::Shot::Region),
        "shot:window" => shot_of(crate::screenshot::Shot::Window),
        "shot:pick" => shot_of(crate::screenshot::Shot::Pick),
        "shot:record" => shot_of(crate::screenshot::Shot::Record),
        "session:lock" => {
            Session::Lock.run();
            Ran::Hide
        }
        "session:suspend" => {
            Session::Suspend.run();
            Ran::Hide
        }
        "session:logout" => {
            Session::Logout.run();
            Ran::Hide
        }
        "session:reboot" => confirmed("session:reboot", Session::Reboot),
        "session:poweroff" => confirmed("session:poweroff", Session::Poweroff),
        _ => {
            log::warn!("helm: no action {id}");
            Ran::Hide
        }
    }
}

/// Hand a shot to `screenshot`, which owns the whole flow.
///
/// The panel is dismissed first, and not because it would be untidy in the
/// picture: the selector freezes the screen, so a panel still mapped would be
/// frozen into it and then covered by the selector showing that frozen copy.
fn shot(
    window: &gtk4::Window,
    store: &Rc<RefCell<NotificationStore>>,
    shot: crate::screenshot::Shot,
) {
    let Some(app) = window.application() else {
        return;
    };
    window.set_visible(false);
    let store = store.clone();
    // One frame for the unmap to reach the compositor before the capture
    // does. Anything shorter races the surface the capture must not contain.
    glib::timeout_add_local_once(std::time::Duration::from_millis(120), move || {
        crate::screenshot::take(&app, &store, shot);
    });
}

// ── Footer action implementations ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes_route_to_their_page_and_settings_is_not_a_page() {
        assert_eq!(route(":wifi"), Some("wifi"));
        assert_eq!(route(":vpn"), Some("wifi"));
        assert_eq!(route(":mic"), Some("audio"));
        assert_eq!(route(":notif"), Some("notifications"));
        // Settings' prefixes open its own surface, not a deck page.
        assert_eq!(route(":glass"), None);
        assert_eq!(route(":settings"), None);
        assert_eq!(route(":nothing"), None);
        assert_eq!(route("firefox"), None);
    }

    #[test]
    fn every_quick_search_result_opens_a_page() {
        for q in crate::settings::search::QUICK {
            assert!(route(q.prefix).is_some(), "{} routes nowhere", q.prefix);
        }
    }

    #[test]
    fn no_prefix_is_claimed_by_two_pages() {
        let mut seen = std::collections::HashSet::new();
        for (prefixes, _, _) in ROUTES {
            for p in *prefixes {
                assert!(seen.insert(*p), "{p} is claimed twice");
            }
        }
        for (prefixes, _) in crate::settings::prefixes() {
            for p in prefixes {
                assert!(seen.insert(*p), "{p} is claimed by a page and a tab");
            }
        }
    }
}
