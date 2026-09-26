//! Pilot's Helm: centered command cockpit and optical HUD fusing an instant
//! app launcher with glanceable telemetry pills, deep sub-sheet utility decks,
//! and flight action switches.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;

use crate::anim;
use crate::launcher::LauncherView;
use crate::notifications::store::NotificationStore;
use crate::settings::SettingsSection;
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

/// What the Helm card asks for on a screen with room for it. No height: the
/// sections decide how tall the card is, and the deck stack has no sensible
/// fixed height to fall back on.
///
/// The width is a floor rather than the width the card renders at. Laid out
/// with room to spare it takes 1033 logical px, the width its widest row
/// wants; squeezed onto a laptop panel that row wraps and the card comes down
/// to around 620. This number only sets how far `install_monitor_fit` is
/// allowed to clamp it before the rows start giving up space.
const HELM_CARD_SIZE: crate::launcher::CardSize = crate::launcher::CardSize {
    width: 740,
    height: None,
};

/// How tall a sub-sheet's body stands on a screen with room for it.
const SUBSHEET_HEIGHT: i32 = 340;

/// Omnibox prefixes and the deck page each one opens. The settings tabs
/// carry their own (`settings::TABS`); `:set` alone opens the pane on
/// whatever tab it was last on. A bare `:` lists all of them on the
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

/// The page a typed prefix opens, and the settings tab when it names one.
/// The settings tabs are tried before `:set`, so `:sleep` is not `:set`.
fn route(prefix: &str) -> Option<(&'static str, Option<&'static str>)> {
    if let Some((_, page, _)) = ROUTES
        .iter()
        .find(|(prefixes, _, _)| prefixes.iter().any(|p| prefix.starts_with(p)))
    {
        return Some((page, None));
    }
    if let Some(tab) = crate::settings::tab_for_prefix(prefix) {
        return Some(("settings", Some(tab)));
    }
    if prefix.starts_with(":set") || prefix.starts_with(":pref") {
        return Some(("settings", None));
    }
    None
}

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
    settings: Rc<SettingsSection>,
    /// Quick-strip toggle tiles (Night Light, Caffeine, etc.)
    tiles: RefCell<Vec<(gtk4::ToggleButton, tiles::TileSpec)>>,
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
        self.settings.refresh();
        for (btn, spec) in self.tiles.borrow().iter() {
            tiles::init_tile_state(btn, spec);
        }
    }
}

// ── Panel (Pilot's Helm HUD) ──────────────────────────────────────────────────

pub struct Panel {
    pub window: gtk4::Window,
    sections: Rc<Sections>,
    launcher: Rc<LauncherView>,
    deck_stack: gtk4::Stack,
    /// Enter/exit transition for the glass HUD: fast pane tint, full-length
    /// content fade, short [`anim::SlideBin`] settle (motion on glass,
    /// anim.rs). `is_shown()` is the intent flag; the window unmaps when the
    /// exit finishes.
    reveal: anim::Reveal,
}

impl Panel {
    pub fn new(
        window: gtk4::Window,
        store: Rc<RefCell<NotificationStore>>,
        audio_service: Rc<crate::audio::AudioService>,
    ) -> Self {
        // ── Backdrop (full-screen transparent click-catcher) ────────────────
        let backdrop = ui::vbox(0);
        backdrop.set_halign(gtk4::Align::Fill);
        backdrop.set_valign(gtk4::Align::Fill);
        backdrop.set_hexpand(true);
        backdrop.set_vexpand(true);
        // The design system's base type and colour. On the window's child,
        // not the window: the GTK theme's `window.background` outranks a
        // class on the window node and would keep its own text colour.
        ui::surface(&backdrop);

        // ── Top spacer (positions Helm at the optical foveal sweet spot ~25-28%) ──
        // Height and the card's width both come from
        // crate::launcher::install_monitor_fit below, against the output the
        // window lands on.
        let top_spacer = ui::vbox(0);

        // ── Root container (the floating glass Helm card) ─────────────────────
        let root = ui::vbox(0);
        root.set_halign(gtk4::Align::Center);
        root.set_valign(gtk4::Align::Start);
        ui::card(&root, ui::Card::Floating);
        root.add_css_class("helm-card");

        // ── Build sections ───────────────────────────────────────────────────
        let audio = AudioSection::new(audio_service.clone());
        let brightness = BrightnessSection::new();
        let network = NetworkSection::new();
        let bluetooth = BluetoothSection::new();
        let display = DisplaySection::new();
        let media = MediaSection::new();
        let notifications = NotificationsSection::new(store.clone());
        let clipboard = ClipboardSection::new();
        let power = PowerSection::new();
        let users = UserSection::new();
        // Its own watcher: the panel outlives no bar process in particular,
        // and the status directory is two small files.
        let backup = BackupSection::new(&crate::backup::BackupStatusService::start());
        // Shared with the omnibox router below, which picks a tab by prefix.
        let settings = Rc::new(SettingsSection::new());

        audio.expand_for_page();
        network.expand_for_page();
        bluetooth.expand_for_page();
        display.expand_for_page();
        clipboard.expand_for_page();
        power.expand_for_page();
        notifications.expand_for_page();

        let launcher = Rc::new(LauncherView::new());

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

        // Page 10: Settings. Five tabs (src/settings/), and the reason it is
        // a page in this card rather than a window of its own is the Glass
        // one: the card is the material, so every slider changes the surface
        // it is drawn on, live.
        {
            let ret = return_to_search.clone();
            let settings_sheet = build_subsheet_with_tabs(
                "Settings",
                "󰒓",
                Some(settings.tabs_widget()),
                settings.widget(),
                move || ret(),
            );
            deck_stack.add_named(&settings_sheet, Some("settings"));
        }

        // Page 11: the prefixes, for a bare `:`.
        {
            let ret = return_to_search.clone();
            let sheet = build_subsheet("Prefixes", "󰘳", &build_prefix_list(), move || ret());
            deck_stack.add_named(&sheet, Some("prefixes"));
        }

        // ── Top Telemetry Ribbon ─────────────────────────────────────────────
        let telemetry_ribbon =
            build_telemetry_ribbon(&deck_stack, &audio, &brightness, &store, &network);

        // ── Bottom Action Flight Deck ─────────────────────────────────────────
        // Each deck tile is kept beside its spec so `refresh` can re-read it
        // when the menu opens.
        let mut tile_pairs: Vec<(gtk4::ToggleButton, tiles::TileSpec)> = Vec::new();
        let flight_deck = build_flight_deck(&window, &store, &mut tile_pairs, &deck_stack);

        // ── Assemble Content ─────────────────────────────────────────────────
        let content = ui::vbox(0);
        content.append(&telemetry_ribbon);
        content.append(&deck_stack);
        content.append(&flight_deck);
        root.append(&content);

        let slide = anim::SlideBin::new();
        slide.set_child(&root);
        slide.jump_to(anim::SLIDE_PX);
        backdrop.append(&top_spacer);
        backdrop.append(&slide);
        window.set_child(Some(&backdrop));

        // `root`, not the slide bin around it, is what carries the size
        // request: the bin lays its child out with a BinLayout, so a request
        // on the bin would be overridden by the child's own.
        //
        // The lists are what the card gives up when the output is short, and
        // the only thing it gives up: every control keeps its place and its
        // size, the lists just show fewer rows before scrolling.
        let lists = elastic_lists(&launcher, &deck_stack);
        crate::launcher::install_monitor_fit(
            &window,
            &top_spacer,
            &root,
            HELM_CARD_SIZE,
            Some(Rc::new(move |compact| {
                for (list, full) in &lists {
                    list.set_min_content_height(if compact { COMPACT_LIST_HEIGHT } else { *full });
                }
            })),
        );

        // Enter/exit transition for the glass menu
        let reveal = anim::Reveal::new(&window, &root)
            .content(&content)
            .slide(&slide, anim::SLIDE_PX);

        // Shared dismiss path: fade the menu out, then unmap.
        let hide_menu = {
            let reveal_c = reveal.clone();
            Rc::new(move || reveal_c.hide())
        };

        // ── Prefix routing from Omnibox ──────────────────────────────────────
        {
            let deck_stack_c = deck_stack.clone();
            let settings_c = settings.clone();
            let network_c = network.clone();
            launcher.entry().connect_search_changed(move |entry| {
                let text = entry.text().to_string();
                let lower = text.to_lowercase();
                let prefix = lower.trim();
                if prefix == ":" {
                    deck_stack_c.set_visible_child_name("prefixes");
                } else if let Some((page, tab)) = route(prefix) {
                    if let Some(tab) = tab {
                        settings_c.show(tab);
                    }
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

        // ── Backdrop click → dismiss; clicks on the root card are claimed ────
        let backdrop_gesture = gtk4::GestureClick::new();
        backdrop_gesture.set_propagation_phase(gtk4::PropagationPhase::Bubble);
        {
            let hide = hide_menu.clone();
            backdrop_gesture.connect_released(move |_, _, _, _| {
                hide();
            });
        }
        backdrop.add_controller(backdrop_gesture);

        let root_gesture = gtk4::GestureClick::new();
        root_gesture.set_propagation_phase(gtk4::PropagationPhase::Bubble);
        root_gesture.connect_released(|gesture, _, _, _| {
            gesture.set_state(gtk4::EventSequenceState::Claimed);
        });
        root.add_controller(root_gesture);

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
            settings,
            tiles: RefCell::new(tile_pairs),
        });

        Self {
            window,
            sections,
            launcher,
            deck_stack,
            reveal,
        }
    }

    pub fn toggle(&self) {
        if self.reveal.is_shown() && self.window.is_visible() {
            self.reveal.hide();
        } else {
            self.launcher.reset();
            self.deck_stack.set_visible_child_name("launcher");
            self.reveal.show();
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

/// Every prefix the omnibox knows, one row per page, from the same tables
/// the router reads.
fn build_prefix_list() -> gtk4::Box {
    let list = ui::vbox(1);
    list.add_css_class("prefix-list");

    let hint = ui::text(
        "Type one of these after the colon to open its page. Anything else searches.",
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
            "Settings · last tab".to_string(),
        )))
        .collect();
    for (prefixes, title) in rows {
        let row = ui::hbox(4);
        row.add_css_class("prefix-row");
        // What you type, in the mono keys are set in; the page it opens
        // beside it, a level quieter.
        let keys = ui::text(&prefixes, Text::Label, Tone::Fg);
        keys.add_css_class("ui-mono");
        keys.set_width_chars(26);
        keys.set_max_width_chars(26);
        keys.set_wrap(true);
        let page = ui::text(&title, Text::Body, Tone::Muted);
        page.set_hexpand(true);
        row.append(&keys);
        row.append(&page);
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

    let back_btn = ui::small_button("← Back (Esc)", Kind::Secondary);
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
    ui::glyph(&glyph, Text::TitleSm, Tone::Muted);
    line.append(&glyph);
    line.append(&ui::text(label, Text::Label, Tone::Fg));
    let pill = gtk4::Button::builder().child(&line).build();
    ui::make_button(&pill, Kind::Secondary);
    pill.add_css_class("pill");
    let stack = stack.clone();
    pill.connect_clicked(move |_| {
        flip(&stack, page);
    });
    pill
}

/// A ribbon pill holding a slider, and the glyph button in front of it that
/// flips the deck to the slider's page.
fn ribbon_slider(
    icon: &str,
    tooltip: &str,
    scale: &gtk4::Scale,
    stack: &gtk4::Stack,
    page: &'static str,
) -> gtk4::Box {
    let pill = ui::pill_group(2);
    let btn = ui::glyph_button(icon, tooltip, Kind::Flat);
    btn.add_css_class("pill");
    btn.set_valign(gtk4::Align::Center);
    {
        let stack = stack.clone();
        btn.connect_clicked(move |_| {
            flip(&stack, page);
        });
    }
    pill.append(&btn);

    scale.set_draw_value(false);
    scale.set_hexpand(true);
    // A floor, not a size: the scale expands to fill the ribbon wherever
    // there is room, so lowering it only decides how much the ribbon can give
    // up on a narrow output. A slider is the right thing to squeeze first,
    // because it stays usable at any width while a label has to be cut.
    scale.set_width_request(44);
    if scale.parent().is_some() {
        scale.unparent();
    }
    pill.append(scale);
    pill
}

fn build_telemetry_ribbon(
    deck_stack: &gtk4::Stack,
    audio: &AudioSection,
    brightness: &BrightnessSection,
    store: &Rc<RefCell<NotificationStore>>,
    network: &NetworkSection,
) -> gtk4::Box {
    let ribbon = ui::hbox(2);
    ribbon.set_hexpand(true);
    ribbon.add_css_class("helm-telemetry-ribbon");

    // 1. Power / Battery pill (dynamic)
    let (icon_str, label_str) = if let Some(path) = power::find_battery_path() {
        if let Some(bat) = power::read_battery(&path) {
            let icon = power::battery_icon(bat.capacity, bat.charging);
            let state_suffix = if bat.charging { " 󱐋" } else { "" };
            (icon, format!("{}%{}", bat.capacity, state_suffix))
        } else {
            ("󰁹", "Power".to_string())
        }
    } else {
        ("󰁹", "AC".to_string())
    };
    ribbon.append(&ribbon_pill(icon_str, &label_str, deck_stack, "power"));

    // 2. Audio Volume scrubber pill (click icon to open audio devices & mixer)
    ribbon.append(&ribbon_slider(
        icons::SPEAKER_HIGH,
        "Open Audio Devices & Mixer (:audio)",
        &audio.output_volume_scale(),
        deck_stack,
        "audio",
    ));

    // 3. Brightness scrubber pill (click icon to open display settings)
    ribbon.append(&ribbon_slider(
        icons::BRIGHTNESS,
        "Open Display & Monitors (:disp)",
        &brightness.brightness_scale(),
        deck_stack,
        "displays",
    ));

    // 4. Wi-Fi Pill: opening the page also starts a scan.
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

    // 5. Bluetooth Pill
    ribbon.append(&ribbon_pill("󰂯", "Bluetooth", deck_stack, "bluetooth"));

    // 6. Displays Pill
    ribbon.append(&ribbon_pill("󰍹", "Displays", deck_stack, "displays"));

    // 7. Notifications Pill
    let notif_count = store.borrow().all().len();
    ribbon.append(&ribbon_pill(
        "󰂚",
        &notif_count.to_string(),
        deck_stack,
        "notifications",
    ));

    ribbon
}

fn build_flight_deck(
    window: &gtk4::Window,
    store: &Rc<RefCell<NotificationStore>>,
    tile_pairs: &mut Vec<(gtk4::ToggleButton, tiles::TileSpec)>,
    deck_stack: &gtk4::Stack,
) -> gtk4::Box {
    let deck = ui::hbox(3);
    deck.set_hexpand(true);
    deck.add_css_class("helm-action-deck");

    // Flight switches (Left group).
    //
    // A FlowBox rather than a Box because this strip is what made the card
    // 1033 logical px wide at minimum: nine switches in a row that could not
    // break. A FlowBox's minimum is its widest single child, so the strip can
    // fold onto a second row on a laptop panel with every switch still there
    // and still clickable.
    //
    // `max_children_per_line` is set from the switch count once they are all
    // in, further down. It has to be exactly that count: it is how many slots
    // wide the flow box asks to be, so leaving it at the default of 7 wraps
    // the row on a screen that has room for it, and setting it higher makes
    // the strip claim width for slots that do not exist and widens the card.
    let left_group = gtk4::FlowBox::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .selection_mode(gtk4::SelectionMode::None)
        .min_children_per_line(1)
        .row_spacing(crate::tokens::space(3) as u32)
        .column_spacing(crate::tokens::space(3) as u32)
        .halign(gtk4::Align::Start)
        .build();
    left_group.add_css_class("deck-switches");

    // Night Light + the session inhibitors, in the order tiles.rs gives them.
    for spec in tiles::deck_specs() {
        let btn = tiles::build_tile(&spec);
        tiles::init_tile_state(&btn, &spec);
        btn.add_css_class("deck-tile-btn");
        tile_pairs.push((btn.clone(), spec));
        left_group.append(&btn);
    }

    // DND
    let dnd_btn = tiles::build_dnd_tile(store.clone());
    dnd_btn.add_css_class("deck-tile-btn");
    left_group.append(&dnd_btn);

    // Screenshot Region
    left_group.append(&rail_action("󰄀", "Screenshot region", window, {
        let window = window.clone();
        let store = store.clone();
        move || shot(&window, &store, crate::screenshot::Shot::Region)
    }));

    // Color Picker Loupe
    left_group.append(&rail_action("󰏘", "Color picker loupe", window, {
        let window = window.clone();
        let store = store.clone();
        move || shot(&window, &store, crate::screenshot::Shot::Pick)
    }));

    // Record Screen
    left_group.append(&rail_action("󰑋", "Record screen", window, {
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
    left_group.append(&clip_btn);

    // Settings
    let settings_btn = deck_button("󰒓", "Settings");
    {
        let stack_c = deck_stack.clone();
        settings_btn.connect_clicked(move |_| {
            flip(&stack_c, "settings");
        });
    }
    left_group.append(&settings_btn);

    // One slot per switch, counted rather than written down, so adding a
    // switch here does not silently start wrapping the row on every screen.
    let switches = std::iter::successors(left_group.first_child(), |child| child.next_sibling());
    left_group.set_max_children_per_line(switches.count() as u32);

    deck.append(&left_group);

    // Spacer
    let spacer = ui::hbox(0);
    spacer.set_hexpand(true);
    deck.append(&spacer);

    // Session cluster (Right group). Centred rather than filling, because
    // once the switches beside it wrap to a second row the deck is twice as
    // tall and these buttons would stretch to match it.
    let session = power::build_session_row();
    session.set_valign(gtk4::Align::Center);
    deck.append(&session);

    deck
}

/// A deck button: a glyph at title size on a component button.
fn deck_button(icon: &str, tooltip: &str) -> gtk4::Button {
    let glyph = gtk4::Label::new(Some(icon));
    ui::glyph(&glyph, Text::Title, Tone::Fg);
    let btn = gtk4::Button::builder()
        .child(&glyph)
        .tooltip_text(tooltip)
        .build();
    ui::make_button(&btn, Kind::Secondary);
    btn.add_css_class("icon");
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

// ── Footer action implementations ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes_route_to_their_page_and_the_settings_tabs_before_set() {
        assert_eq!(route(":wifi"), Some(("wifi", None)));
        assert_eq!(route(":vpn"), Some(("wifi", None)));
        assert_eq!(route(":mic"), Some(("audio", None)));
        assert_eq!(route(":notif"), Some(("notifications", None)));
        // A settings tab, and one whose prefix starts with `:s` like `:set`.
        assert_eq!(route(":glass"), Some(("settings", Some("glass"))));
        assert_eq!(route(":sleep"), Some(("settings", Some("idle"))));
        assert_eq!(route(":settings"), Some(("settings", None)));
        assert_eq!(route(":pref"), Some(("settings", None)));
        assert_eq!(route(":nothing"), None);
        assert_eq!(route("firefox"), None);
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
