//! Pilot's Helm: centered command cockpit and optical HUD fusing an instant
//! app launcher with glanceable telemetry pills, deep sub-sheet utility decks,
//! and flight action switches.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;

use crate::anim;
use crate::icons;
use crate::launcher::LauncherView;
use crate::notifications::store::NotificationStore;
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
        window.add_css_class("startmenu");
        window.add_css_class("helm-window");

        // ── Backdrop (full-screen transparent click-catcher) ────────────────
        let backdrop = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .halign(gtk4::Align::Fill)
            .valign(gtk4::Align::Fill)
            .hexpand(true)
            .vexpand(true)
            .build();
        backdrop.add_css_class("startmenu-backdrop");
        backdrop.add_css_class("helm-backdrop");

        // ── Top spacer (positions Helm at the optical foveal sweet spot ~25-28%) ──
        let top_offset = crate::launcher::monitor_top_offset();
        let top_spacer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        top_spacer.set_height_request(top_offset);

        // ── Root container (the floating glass Helm card) ─────────────────────
        let root = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .halign(gtk4::Align::Center)
            .valign(gtk4::Align::Start)
            .width_request(740)
            .build();
        root.add_css_class("glass-card");
        root.add_css_class("startmenu-root");
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

        audio.expand_for_page();
        network.expand_for_page();
        bluetooth.expand_for_page();
        display.expand_for_page();
        clipboard.expand_for_page();
        power.expand_for_page();
        notifications.expand_for_page();

        let launcher = Rc::new(LauncherView::new());

        // ── Deck stack (Launcher stage ↔ In-place utility sub-sheets) ─────────
        let deck_stack = gtk4::Stack::builder()
            .transition_type(gtk4::StackTransitionType::Crossfade)
            .transition_duration(150)
            .vexpand(true)
            .build();
        deck_stack.add_css_class("helm-deck-stack");

        // Page 1: Default Elephant launcher stage
        let launcher_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .vexpand(true)
            .build();
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
            let bt_sheet = build_subsheet("Bluetooth Devices", "󰂯", bluetooth.widget(), move || ret());
            deck_stack.add_named(&bt_sheet, Some("bluetooth"));
        }

        // Page 4: Displays & Output configuration
        {
            let ret = return_to_search.clone();
            let disp_sheet = build_subsheet("Display & Monitors", "󰍹", display.widget(), move || ret());
            deck_stack.add_named(&disp_sheet, Some("displays"));
        }

        // Page 5: Clipboard History
        {
            let ret = return_to_search.clone();
            let clip_sheet = build_subsheet("Clipboard History", "󰅍", clipboard.widget(), move || ret());
            deck_stack.add_named(&clip_sheet, Some("clipboard"));
        }

        // Page 6: System Power Diagnostics & Users
        {
            let ret = return_to_search.clone();
            let power_container = gtk4::Box::builder()
                .orientation(gtk4::Orientation::Vertical)
                .spacing(12)
                .build();
            power_container.append(power.widget());
            power_container.append(users.widget());
            let power_sheet = build_subsheet("System State & Power", "󰁹", &power_container, move || ret());
            deck_stack.add_named(&power_sheet, Some("power"));
        }

        // Page 7: Notifications Center
        {
            let ret = return_to_search.clone();
            let notif_sheet = build_subsheet("Notifications Center", "󰂚", notifications.widget(), move || ret());
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
            let audio_sheet = build_subsheet("Audio & Sound Devices", "󰕾", audio.widget(), move || ret());
            deck_stack.add_named(&audio_sheet, Some("audio"));
        }

        // ── Top Telemetry Ribbon ─────────────────────────────────────────────
        let telemetry_ribbon = build_telemetry_ribbon(&deck_stack, &audio, &brightness, &store);

        // ── Bottom Action Flight Deck ─────────────────────────────────────────
        // Each deck tile is kept beside its spec so `refresh` can re-read it
        // when the menu opens.
        let mut tile_pairs: Vec<(gtk4::ToggleButton, tiles::TileSpec)> = Vec::new();
        let flight_deck = build_flight_deck(&window, &store, &mut tile_pairs, &deck_stack);

        // ── Assemble Content ─────────────────────────────────────────────────
        let content = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(0)
            .build();
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
            launcher.entry().connect_search_changed(move |entry| {
                let text = entry.text().to_string();
                let lower = text.to_lowercase();
                let prefix = lower.trim();
                if prefix.starts_with(":wifi") || prefix.starts_with(":net") || prefix.starts_with(":vpn") || prefix.starts_with(":ovpn") || prefix.starts_with(":openvpn") || prefix.starts_with(":wg") || prefix.starts_with(":wireguard") {
                    deck_stack_c.set_visible_child_name("wifi");
                } else if prefix.starts_with(":bt") || prefix.starts_with(":blue") {
                    deck_stack_c.set_visible_child_name("bluetooth");
                } else if prefix.starts_with(":disp") || prefix.starts_with(":screen") {
                    deck_stack_c.set_visible_child_name("displays");
                } else if prefix.starts_with(":clip") || prefix.starts_with(":cb") {
                    deck_stack_c.set_visible_child_name("clipboard");
                } else if prefix.starts_with(":power") || prefix.starts_with(":sys") {
                    deck_stack_c.set_visible_child_name("power");
                } else if prefix.starts_with(":notif") {
                    deck_stack_c.set_visible_child_name("notifications");
                } else if prefix.starts_with(":media") || prefix.starts_with(":music") {
                    deck_stack_c.set_visible_child_name("media");
                } else if prefix.starts_with(":audio") || prefix.starts_with(":vol") || prefix.starts_with(":sound") || prefix.starts_with(":sink") || prefix.starts_with(":mic") {
                    deck_stack_c.set_visible_child_name("audio");
                } else if !prefix.starts_with(':') && deck_stack_c.visible_child_name().as_deref() != Some("launcher") {
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
            let sections = self.sections.clone();
            let launcher = self.launcher.clone();
            glib::idle_add_local_once(move || {
                sections.refresh();
                launcher.focus_entry();
            });
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

fn build_subsheet(
    title: &str,
    icon: &str,
    content: &impl IsA<gtk4::Widget>,
    on_back: impl Fn() + 'static,
) -> gtk4::Box {
    let container = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(8)
        .build();
    container.add_css_class("helm-subsheet");

    let header = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(10)
        .build();
    header.add_css_class("subsheet-header");

    let back_btn = gtk4::Button::builder()
        .label("← Back (Esc)")
        .build();
    back_btn.add_css_class("subsheet-back-btn");
    back_btn.connect_clicked(move |_| on_back());
    header.append(&back_btn);

    let title_lbl = gtk4::Label::builder()
        .label(&format!("{icon}  {title}"))
        .hexpand(true)
        .halign(gtk4::Align::Start)
        .build();
    title_lbl.add_css_class("subsheet-title");
    header.append(&title_lbl);

    container.append(&header);

    let scroller = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .vexpand(true)
        .min_content_height(340)
        .max_content_height(420)
        .child(content)
        .build();
    scroller.add_css_class("subsheet-scroller");
    container.append(&scroller);

    container
}

fn build_telemetry_ribbon(
    deck_stack: &gtk4::Stack,
    audio: &AudioSection,
    brightness: &BrightnessSection,
    store: &Rc<RefCell<NotificationStore>>,
) -> gtk4::Box {
    let ribbon = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(6)
        .hexpand(true)
        .build();
    ribbon.add_css_class("helm-telemetry-ribbon");

    // 1. Power / Battery pill (dynamic)
    let pill_power = gtk4::Button::builder().build();
    pill_power.add_css_class("telemetry-pill");
    let power_box = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(6)
        .build();
    
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

    let power_icon = gtk4::Label::builder().label(icon_str).build();
    power_icon.add_css_class("pill-icon-green");
    let power_lbl = gtk4::Label::builder().label(&label_str).build();
    power_box.append(&power_icon);
    power_box.append(&power_lbl);
    pill_power.set_child(Some(&power_box));
    {
        let stack_c = deck_stack.clone();
        pill_power.connect_clicked(move |_| {
            if stack_c.visible_child_name().as_deref() == Some("power") {
                stack_c.set_visible_child_name("launcher");
            } else {
                stack_c.set_visible_child_name("power");
            }
        });
    }
    ribbon.append(&pill_power);

    // 2. Audio Volume scrubber pill (click icon to open audio devices & mixer)
    let pill_audio = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(4)
        .build();
    pill_audio.add_css_class("telemetry-pill");
    pill_audio.add_css_class("telemetry-pill-interactive");
    
    let audio_btn = gtk4::Button::builder()
        .child(&gtk4::Label::new(Some(icons::SPEAKER_HIGH)))
        .tooltip_text("Open Audio Devices & Mixer (:audio)")
        .build();
    audio_btn.add_css_class("telemetry-icon-sub-btn");
    {
        let stack_c = deck_stack.clone();
        audio_btn.connect_clicked(move |_| {
            if stack_c.visible_child_name().as_deref() == Some("audio") {
                stack_c.set_visible_child_name("launcher");
            } else {
                stack_c.set_visible_child_name("audio");
            }
        });
    }
    pill_audio.append(&audio_btn);

    let scale = audio.output_volume_scale();
    scale.set_draw_value(false);
    scale.set_hexpand(true);
    scale.set_width_request(80);
    if scale.parent().is_some() {
        scale.unparent();
    }
    pill_audio.append(&scale);
    ribbon.append(&pill_audio);

    // 3. Brightness scrubber pill (click icon to open display settings)
    let pill_bright = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(4)
        .build();
    pill_bright.add_css_class("telemetry-pill");
    pill_bright.add_css_class("telemetry-pill-interactive");
    
    let bright_btn = gtk4::Button::builder()
        .child(&gtk4::Label::new(Some(icons::BRIGHTNESS)))
        .tooltip_text("Open Display & Monitors (:disp)")
        .build();
    bright_btn.add_css_class("telemetry-icon-sub-btn");
    {
        let stack_c = deck_stack.clone();
        bright_btn.connect_clicked(move |_| {
            if stack_c.visible_child_name().as_deref() == Some("displays") {
                stack_c.set_visible_child_name("launcher");
            } else {
                stack_c.set_visible_child_name("displays");
            }
        });
    }
    pill_bright.append(&bright_btn);

    let b_scale = brightness.brightness_scale();
    b_scale.set_draw_value(false);
    b_scale.set_hexpand(true);
    b_scale.set_width_request(70);
    if b_scale.parent().is_some() {
        b_scale.unparent();
    }
    pill_bright.append(&b_scale);
    ribbon.append(&pill_bright);

    // 4. Wi-Fi Pill
    let pill_wifi = gtk4::Button::builder().build();
    pill_wifi.add_css_class("telemetry-pill");
    let wifi_box = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(6)
        .build();
    let wifi_icon = gtk4::Label::builder().label("󰤨").build();
    wifi_icon.add_css_class("pill-icon-blue");
    let wifi_lbl = gtk4::Label::builder().label("Wi-Fi").build();
    wifi_box.append(&wifi_icon);
    wifi_box.append(&wifi_lbl);
    pill_wifi.set_child(Some(&wifi_box));
    {
        let stack_c = deck_stack.clone();
        pill_wifi.connect_clicked(move |_| {
            if stack_c.visible_child_name().as_deref() == Some("wifi") {
                stack_c.set_visible_child_name("launcher");
            } else {
                stack_c.set_visible_child_name("wifi");
            }
        });
    }
    ribbon.append(&pill_wifi);

    // 5. Bluetooth Pill
    let pill_bt = gtk4::Button::builder().build();
    pill_bt.add_css_class("telemetry-pill");
    let bt_box = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(6)
        .build();
    let bt_icon = gtk4::Label::builder().label("󰂯").build();
    bt_icon.add_css_class("pill-icon-blue");
    let bt_lbl = gtk4::Label::builder().label("Bluetooth").build();
    bt_box.append(&bt_icon);
    bt_box.append(&bt_lbl);
    pill_bt.set_child(Some(&bt_box));
    {
        let stack_c = deck_stack.clone();
        pill_bt.connect_clicked(move |_| {
            if stack_c.visible_child_name().as_deref() == Some("bluetooth") {
                stack_c.set_visible_child_name("launcher");
            } else {
                stack_c.set_visible_child_name("bluetooth");
            }
        });
    }
    ribbon.append(&pill_bt);

    // 6. Displays Pill
    let pill_disp = gtk4::Button::builder().build();
    pill_disp.add_css_class("telemetry-pill");
    let disp_box = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(6)
        .build();
    let disp_icon = gtk4::Label::builder().label("󰍹").build();
    disp_icon.add_css_class("pill-icon-yellow");
    let disp_lbl = gtk4::Label::builder().label("Displays").build();
    disp_box.append(&disp_icon);
    disp_box.append(&disp_lbl);
    pill_disp.set_child(Some(&disp_box));
    {
        let stack_c = deck_stack.clone();
        pill_disp.connect_clicked(move |_| {
            if stack_c.visible_child_name().as_deref() == Some("displays") {
                stack_c.set_visible_child_name("launcher");
            } else {
                stack_c.set_visible_child_name("displays");
            }
        });
    }
    ribbon.append(&pill_disp);

    // 7. Notifications Pill
    let pill_notif = gtk4::Button::builder().build();
    pill_notif.add_css_class("telemetry-pill");
    let notif_box = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(4)
        .build();
    let notif_icon = gtk4::Label::builder().label("󰂚").build();
    notif_icon.add_css_class("pill-icon-red");
    let notif_count = store.borrow().all().len();
    let notif_count_str = if notif_count > 0 { format!("{notif_count}") } else { "0".to_string() };
    let notif_lbl = gtk4::Label::builder()
        .label(&notif_count_str)
        .build();
    notif_box.append(&notif_icon);
    notif_box.append(&notif_lbl);
    pill_notif.set_child(Some(&notif_box));
    {
        let stack_c = deck_stack.clone();
        pill_notif.connect_clicked(move |_| {
            if stack_c.visible_child_name().as_deref() == Some("notifications") {
                stack_c.set_visible_child_name("launcher");
            } else {
                stack_c.set_visible_child_name("notifications");
            }
        });
    }
    ribbon.append(&pill_notif);

    ribbon
}

fn build_flight_deck(
    window: &gtk4::Window,
    store: &Rc<RefCell<NotificationStore>>,
    tile_pairs: &mut Vec<(gtk4::ToggleButton, tiles::TileSpec)>,
    deck_stack: &gtk4::Stack,
) -> gtk4::Box {
    let deck = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(8)
        .hexpand(true)
        .build();
    deck.add_css_class("helm-action-deck");

    // Flight switches (Left group)
    let left_group = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(6)
        .build();

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
    let clip_btn = gtk4::Button::builder()
        .child(&gtk4::Label::new(Some("󰅍")))
        .build();
    clip_btn.add_css_class("rail-btn");
    clip_btn.set_tooltip_text(Some("Clipboard history"));
    {
        let stack_c = deck_stack.clone();
        clip_btn.connect_clicked(move |_| {
            if stack_c.visible_child_name().as_deref() == Some("clipboard") {
                stack_c.set_visible_child_name("launcher");
            } else {
                stack_c.set_visible_child_name("clipboard");
            }
        });
    }
    left_group.append(&clip_btn);

    deck.append(&left_group);

    // Spacer
    let spacer = gtk4::Box::builder().hexpand(true).build();
    deck.append(&spacer);

    // Session cluster (Right group)
    deck.append(&power::build_session_row());

    deck
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
    let btn = gtk4::Button::builder()
        .child(&gtk4::Label::new(Some(icon)))
        .build();
    btn.add_css_class("rail-btn");
    btn.set_tooltip_text(Some(tooltip));
    let window_c = window.clone();
    btn.connect_clicked(move |_| {
        window_c.set_visible(false);
        action();
    });
    btn
}

// ── Footer action implementations ─────────────────────────────────────────────

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
