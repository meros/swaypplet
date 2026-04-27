use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;

use crate::notifications::store::NotificationStore;
use crate::widgets::{
    audio::AudioSection,
    bluetooth::BluetoothSection,
    brightness::BrightnessSection,
    clipboard::ClipboardSection,
    display::DisplaySection,
    header::HeaderSection,
    media::MediaSection,
    network::NetworkSection,
    notifications::NotificationsSection,
    page_frame::PageFrame,
    power::PowerSection,
    quickinfo::QuickInfoStrip,
    screenshot::ScreenshotSection,
};

// ── Lane metadata ─────────────────────────────────────────────────────────────

struct LaneMeta {
    name: &'static str,
    icon: &'static str,
    _accent: &'static str,
}

// Lane order: logical (urgency-first reordering at runtime is a TODO)
// TODO: implement runtime urgency reordering (battery critical/low, notification critical)
const LANES: &[LaneMeta] = &[
    LaneMeta { name: "overview",       icon: "󰋜", _accent: "accent" },
    LaneMeta { name: "audio",          icon: "󰕾", _accent: "accent_tertiary" },
    LaneMeta { name: "brightness",     icon: "󰃞", _accent: "accent_secondary" },
    LaneMeta { name: "network",        icon: "󰖩", _accent: "accent_tertiary" },
    LaneMeta { name: "bluetooth",      icon: "󰂯", _accent: "accent_tertiary" },
    LaneMeta { name: "display",        icon: "󰍹", _accent: "accent_quaternary" },
    LaneMeta { name: "media",          icon: "󰎈", _accent: "accent" },
    LaneMeta { name: "notifications",  icon: "󰂚", _accent: "accent_quinary" },
    LaneMeta { name: "clipboard",      icon: "󰅍", _accent: "accent" },
    LaneMeta { name: "screenshot",     icon: "󰄀", _accent: "accent_quinary" },
    LaneMeta { name: "power",          icon: "󱐋", _accent: "accent" },
];

// ── Sections bundle ───────────────────────────────────────────────────────────

struct Sections {
    header:        HeaderSection,
    audio:         AudioSection,
    brightness:    BrightnessSection,
    network:       NetworkSection,
    bluetooth:     Rc<BluetoothSection>,
    display:       DisplaySection,
    media:         MediaSection,
    notifications: NotificationsSection,
    clipboard:     ClipboardSection,
    screenshot:    ScreenshotSection,
    power:         PowerSection,
    quickinfo:     QuickInfoStrip,
    store:         Rc<RefCell<NotificationStore>>,
    /// PageFrame wrappers — must stay alive as long as the panel.
    /// GTK holds strong refs to widgets once attached, but the Rust structs
    /// themselves need to remain to avoid drop-before-attach races.
    _page_frames:  Vec<PageFrame>,
}

impl Sections {
    fn refresh(&self) {
        self.quickinfo.refresh();
        self.header.refresh();
        self.audio.refresh();
        self.brightness.refresh();
        self.network.refresh();
        self.bluetooth.schedule_refresh();
        self.display.refresh();
        self.media.refresh();
        self.notifications.refresh();
        self.clipboard.refresh();
        self.screenshot.refresh();
        self.power.refresh();
    }
}

// ── Panel ─────────────────────────────────────────────────────────────────────

pub struct Panel {
    pub window: gtk4::Window,
    sections: Rc<Sections>,
    lane_strip: gtk4::ListBox,
}

impl Panel {
    pub fn new(window: gtk4::Window, store: Rc<RefCell<NotificationStore>>) -> Self {
        // ── Full-screen backdrop ─────────────────────────────────────────────
        let backdrop = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .halign(gtk4::Align::Fill)
            .valign(gtk4::Align::Fill)
            .hexpand(true)
            .vexpand(true)
            .build();
        backdrop.add_css_class("panel-backdrop");

        // ── Centered 960×720 container ───────────────────────────────────────
        let container = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .halign(gtk4::Align::Center)
            .valign(gtk4::Align::Center)
            .width_request(960)
            .height_request(720)
            .build();
        container.add_css_class("panel-container");
        container.add_css_class("dispatch-shell");

        // ── QuickInfo top strip ──────────────────────────────────────────────
        let quickinfo = QuickInfoStrip::new();
        container.append(quickinfo.widget());

        // ── Body: lane strip + stack ─────────────────────────────────────────
        let body = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .hexpand(true)
            .vexpand(true)
            .spacing(0)
            .build();
        body.add_css_class("dispatch-body");

        // Lane strip (ListBox, selection-mode Single, 64px wide)
        let lane_strip = gtk4::ListBox::builder()
            .selection_mode(gtk4::SelectionMode::Single)
            .width_request(64)
            .build();
        lane_strip.add_css_class("dispatch-lanes");

        // Content stack
        let stack = gtk4::Stack::builder()
            .transition_type(gtk4::StackTransitionType::SlideLeftRight)
            .hhomogeneous(false)
            .vhomogeneous(false)
            .hexpand(true)
            .vexpand(true)
            .build();
        stack.add_css_class("dispatch-stack");

        // ── Build sections ───────────────────────────────────────────────────
        let header        = HeaderSection::new(store.clone());
        let audio         = AudioSection::new();
        let brightness    = BrightnessSection::new();
        let network       = NetworkSection::new();
        let bluetooth     = BluetoothSection::new();
        let display       = DisplaySection::new();
        let media         = MediaSection::new();
        let notifications = NotificationsSection::new(store.clone());
        let clipboard     = ClipboardSection::new();
        let screenshot    = ScreenshotSection::new();
        let power         = PowerSection::new();

        // ── Expand sections for page mode ────────────────────────────────────
        audio.expand_for_page();
        brightness.expand_for_page();
        network.expand_for_page();
        bluetooth.expand_for_page();
        display.expand_for_page();
        notifications.expand_for_page();
        clipboard.expand_for_page();
        screenshot.expand_for_page();
        power.expand_for_page();

        // ── Wrap each section in a PageFrame + add to stack ──────────────────
        let page_overview      = PageFrame::new("Quick toggles", None,                            header.widget());
        let page_audio         = PageFrame::new("Audio",          Some("Output and input devices"), audio.widget());
        let page_brightness    = PageFrame::new("Display brightness", None,                       brightness.widget());
        let page_network       = PageFrame::new("Network",        None,                            network.widget());
        let page_bluetooth     = PageFrame::new("Bluetooth",      None,                            bluetooth.widget());
        let page_display       = PageFrame::new("Displays",       None,                            display.widget());
        let page_media         = PageFrame::new("Now playing",    None,                            media.widget());
        let page_notifications = PageFrame::new("Notifications",  None,                            notifications.widget());
        let page_clipboard     = PageFrame::new("Clipboard",      None,                            clipboard.widget());
        let page_screenshot    = PageFrame::new("Screenshot & record", None,                       screenshot.widget());
        let page_power         = PageFrame::new("Power",          None,                            power.widget());

        stack.add_titled(page_overview.widget(),      Some("overview"),      "Overview");
        stack.add_titled(page_audio.widget(),         Some("audio"),         "Audio");
        stack.add_titled(page_brightness.widget(),    Some("brightness"),    "Brightness");
        stack.add_titled(page_network.widget(),       Some("network"),       "Network");
        stack.add_titled(page_bluetooth.widget(),     Some("bluetooth"),     "Bluetooth");
        stack.add_titled(page_display.widget(),       Some("display"),       "Display");
        stack.add_titled(page_media.widget(),         Some("media"),         "Media");
        stack.add_titled(page_notifications.widget(), Some("notifications"), "Notifications");
        stack.add_titled(page_clipboard.widget(),     Some("clipboard"),     "Clipboard");
        stack.add_titled(page_screenshot.widget(),    Some("screenshot"),    "Screenshot");
        stack.add_titled(page_power.widget(),         Some("power"),         "Power");

        let page_frames = vec![
            page_overview,
            page_audio,
            page_brightness,
            page_network,
            page_bluetooth,
            page_display,
            page_media,
            page_notifications,
            page_clipboard,
            page_screenshot,
            page_power,
        ];

        // ── Build lane rows ──────────────────────────────────────────────────
        for lane in LANES {
            let row = gtk4::ListBoxRow::new();
            row.set_widget_name(lane.name);
            row.add_css_class("dispatch-lane");

            let inner = gtk4::Box::builder()
                .orientation(gtk4::Orientation::Vertical)
                .halign(gtk4::Align::Center)
                .spacing(2)
                .build();

            let icon_lbl = gtk4::Label::builder()
                .label(lane.icon)
                .build();
            icon_lbl.add_css_class("dispatch-lane-icon");

            let dot = gtk4::Box::builder()
                .width_request(6)
                .height_request(6)
                .halign(gtk4::Align::Center)
                .build();
            dot.add_css_class("dispatch-lane-dot");

            inner.append(&icon_lbl);
            inner.append(&dot);
            row.set_child(Some(&inner));
            lane_strip.append(&row);
        }

        // ── Lane selection → stack page change ──────────────────────────────
        {
            let stack_clone = stack.clone();
            lane_strip.connect_row_selected(move |_, row| {
                if let Some(row) = row {
                    let name = row.widget_name();
                    stack_clone.set_visible_child_name(&name);
                }
            });
        }

        body.append(&lane_strip);
        body.append(&stack);
        container.append(&body);
        backdrop.append(&container);
        window.set_child(Some(&backdrop));

        // ── Sections bundle ──────────────────────────────────────────────────
        let sections = Rc::new(Sections {
            header,
            audio,
            brightness,
            network,
            bluetooth,
            display,
            media,
            notifications,
            clipboard,
            screenshot,
            power,
            quickinfo,
            store: store.clone(),
            _page_frames: page_frames,
        });

        // ── Keyboard controller (capture phase) ──────────────────────────────
        let key_controller = gtk4::EventControllerKey::new();
        key_controller.set_propagation_phase(gtk4::PropagationPhase::Capture);
        {
            let window_clone  = window.clone();
            let lane_strip_c  = lane_strip.clone();
            let stack_c       = stack.clone();
            let sections_c    = sections.clone();

            key_controller.connect_key_pressed(move |_, key, _, modifiers| {
                use gtk4::gdk::Key;
                use gtk4::gdk::ModifierType;

                let search_active = sections_c.quickinfo.widget()
                    .has_css_class("qi-search-active");

                if search_active {
                    match key {
                        Key::Escape => {
                            sections_c.quickinfo.exit_search_mode();
                            // Refocus current lane row
                            if let Some(row) = lane_strip_c.selected_row() {
                                row.grab_focus();
                            }
                            return glib::Propagation::Stop;
                        }
                        _ => return glib::Propagation::Proceed,
                    }
                }

                match key {
                    // Dismiss panel
                    Key::Escape => {
                        window_clone.set_visible(false);
                        glib::Propagation::Stop
                    }

                    // Enter search mode
                    Key::slash => {
                        sections_c.quickinfo.enter_search_mode();
                        glib::Propagation::Stop
                    }

                    // Focus into stack content (→, Enter, Tab without Shift)
                    Key::Right | Key::Return | Key::KP_Enter => {
                        if let Some(child) = stack_c.visible_child() {
                            child.grab_focus();
                        }
                        glib::Propagation::Stop
                    }

                    // Tab (without Shift) — forward into stack
                    Key::Tab if !modifiers.contains(ModifierType::SHIFT_MASK) => {
                        if let Some(child) = stack_c.visible_child() {
                            child.grab_focus();
                        }
                        glib::Propagation::Stop
                    }

                    // ← or Shift+Tab — back to lane strip
                    Key::Left => {
                        if let Some(row) = lane_strip_c.selected_row() {
                            row.grab_focus();
                        }
                        glib::Propagation::Stop
                    }

                    Key::Tab if modifiers.contains(ModifierType::SHIFT_MASK) => {
                        if let Some(row) = lane_strip_c.selected_row() {
                            row.grab_focus();
                        }
                        glib::Propagation::Stop
                    }

                    // Digit shortcuts: 1..9 = lanes 0..8, 0 = lane 10 (power)
                    Key::_1 => { select_lane(&lane_strip_c, 0); glib::Propagation::Stop }
                    Key::_2 => { select_lane(&lane_strip_c, 1); glib::Propagation::Stop }
                    Key::_3 => { select_lane(&lane_strip_c, 2); glib::Propagation::Stop }
                    Key::_4 => { select_lane(&lane_strip_c, 3); glib::Propagation::Stop }
                    Key::_5 => { select_lane(&lane_strip_c, 4); glib::Propagation::Stop }
                    Key::_6 => { select_lane(&lane_strip_c, 5); glib::Propagation::Stop }
                    Key::_7 => { select_lane(&lane_strip_c, 6); glib::Propagation::Stop }
                    Key::_8 => { select_lane(&lane_strip_c, 7); glib::Propagation::Stop }
                    Key::_9 => { select_lane(&lane_strip_c, 8); glib::Propagation::Stop }
                    Key::_0 => { select_lane(&lane_strip_c, 10); glib::Propagation::Stop }

                    _ => glib::Propagation::Proceed,
                }
            });
        }
        window.add_controller(key_controller);

        // ── Search lane filtering ────────────────────────────────────────────
        {
            let lane_strip_c = lane_strip.clone();
            let quickinfo_c  = sections.quickinfo.widget().clone();

            sections.quickinfo.search_entry().connect_search_changed(move |entry: &gtk4::SearchEntry| {
                let query = entry.text().to_lowercase();
                filter_lanes(&lane_strip_c, &query);
                let _ = quickinfo_c; // keep alive
            });
        }
        {
            let lane_strip_c  = lane_strip.clone();
            let sections_c    = sections.clone();
            sections.quickinfo.search_entry().connect_activate(move |_| {
                // Select first non-dimmed lane
                select_first_undimmed(&lane_strip_c);
                sections_c.quickinfo.exit_search_mode();
                if let Some(row) = lane_strip_c.selected_row() {
                    row.grab_focus();
                }
            });
        }

        // ── Backdrop click → dismiss ─────────────────────────────────────────
        let backdrop_gesture = gtk4::GestureClick::new();
        backdrop_gesture.set_propagation_phase(gtk4::PropagationPhase::Bubble);
        {
            let window_clone = window.clone();
            backdrop_gesture.connect_released(move |_, _, _, _| {
                window_clone.set_visible(false);
            });
        }
        backdrop.add_controller(backdrop_gesture);

        // No-op gesture on container stops clicks from bubbling to backdrop
        let container_gesture = gtk4::GestureClick::new();
        container_gesture.set_propagation_phase(gtk4::PropagationPhase::Bubble);
        container_gesture.connect_released(|gesture, _, _, _| {
            gesture.set_state(gtk4::EventSequenceState::Claimed);
        });
        container.add_controller(container_gesture);

        Self { window, sections, lane_strip }
    }

    pub fn toggle(&self) {
        if self.window.is_visible() {
            self.window.set_visible(false);
        } else {
            // Select overview lane before showing to avoid flicker
            if let Some(row) = self.lane_strip.row_at_index(0) {
                self.lane_strip.select_row(Some(&row));
            }
            self.window.set_visible(true);
            let sections = self.sections.clone();
            let lane_strip = self.lane_strip.clone();
            glib::idle_add_local_once(move || {
                sections.refresh();
                update_lane_states(&lane_strip, &sections);
            });
        }
    }

    pub fn refresh(&self) {
        self.sections.refresh();
        update_lane_states(&self.lane_strip, &self.sections);
    }

    pub fn refresh_audio(&self) {
        self.sections.audio.refresh();
    }

    pub fn refresh_brightness(&self) {
        self.sections.brightness.refresh();
    }
}

// ── Lane helpers ──────────────────────────────────────────────────────────────

fn select_lane(lane_strip: &gtk4::ListBox, idx: i32) {
    if let Some(row) = lane_strip.row_at_index(idx) {
        lane_strip.select_row(Some(&row));
        row.grab_focus();
    }
}

fn filter_lanes(lane_strip: &gtk4::ListBox, query: &str) {
    let mut idx = 0i32;
    while let Some(row) = lane_strip.row_at_index(idx) {
        let name = row.widget_name().to_lowercase();
        if query.is_empty() || name.contains(query) {
            row.remove_css_class("dim");
        } else {
            row.add_css_class("dim");
        }
        idx += 1;
    }
}

fn select_first_undimmed(lane_strip: &gtk4::ListBox) {
    let mut idx = 0i32;
    while let Some(row) = lane_strip.row_at_index(idx) {
        if !row.has_css_class("dim") {
            lane_strip.select_row(Some(&row));
            return;
        }
        idx += 1;
    }
}

// ── Lane state heuristics ─────────────────────────────────────────────────────
//
// Reads section widget state + system data after each refresh, drives the
// `.warning` / `.alerting` / `.active` / `.media-active` / `.dim` classes on
// each lane row. Stays cheap — only fires on panel show / explicit refresh.

fn update_lane_states(lane_strip: &gtk4::ListBox, sections: &Sections) {
    let mut idx = 0i32;
    while let Some(row) = lane_strip.row_at_index(idx) {
        let name = row.widget_name().to_string();
        // Clear all dynamic state classes; keep `.dim` (driven by search) intact.
        for cls in &["warning", "alerting", "active", "media-active"] {
            row.remove_css_class(cls);
        }

        match name.as_str() {
            "media" => {
                let media_widget = sections.media.widget();
                if !media_widget.is_visible() {
                    row.add_css_class("dim");
                } else {
                    row.remove_css_class("dim");
                    if media_widget.has_css_class("playing") {
                        row.add_css_class("active");
                        row.add_css_class("media-active");
                    }
                }
            }
            "power" => {
                if let Some(pct) = read_battery_capacity() {
                    if pct < 10 {
                        row.add_css_class("alerting");
                    } else if pct < 20 {
                        row.add_css_class("warning");
                    }
                }
            }
            "notifications" => {
                if has_critical_notification(&sections.store) {
                    row.add_css_class("alerting");
                }
            }
            _ => {}
        }
        idx += 1;
    }
}

fn read_battery_capacity() -> Option<u32> {
    use std::fs;
    // Pick the first BAT* under /sys/class/power_supply.
    let entries = fs::read_dir("/sys/class/power_supply").ok()?;
    for e in entries.flatten() {
        let name = e.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("BAT") {
            let path = e.path().join("capacity");
            if let Ok(s) = fs::read_to_string(&path) {
                return s.trim().parse::<u32>().ok();
            }
        }
    }
    None
}

fn has_critical_notification(store: &Rc<RefCell<NotificationStore>>) -> bool {
    use crate::notifications::Urgency;
    store
        .borrow()
        .all()
        .iter()
        .any(|n| n.urgency == Urgency::Critical)
}
