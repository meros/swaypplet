//! Start-menu popup: bottom-left anchored surface fusing an app launcher
//! (left column) with a compact quick-settings control center (right column).

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;

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
    power::PowerSection,
    tiles,
};

// Quick-strip glyphs.
const ICON_SPEAKER: &str = "󰕾";
const ICON_BRIGHTNESS: &str = "󰃞";
const ICON_DISPLAY: &str = "󰍹";

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
    /// Quick-strip toggle tiles (Wi-Fi, Bluetooth, Night Light, Idle) paired
    /// with their spec so state can be re-read on refresh. DND is store-backed
    /// and refreshed separately.
    tiles: RefCell<Vec<(gtk4::ToggleButton, tiles::TileSpec)>>,
}

impl Sections {
    fn refresh(&self) {
        self.audio.refresh();
        self.brightness.refresh();
        self.network.refresh();
        self.bluetooth.schedule_refresh();
        self.display.refresh();
        self.media.refresh();
        self.notifications.refresh();
        self.clipboard.refresh();
        self.power.refresh();
        for (btn, spec) in self.tiles.borrow().iter() {
            tiles::init_tile_state(btn, spec);
        }
    }
}

// ── Panel (start menu) ─────────────────────────────────────────────────────────

pub struct Panel {
    pub window: gtk4::Window,
    sections: Rc<Sections>,
    launcher: Rc<LauncherView>,
}

impl Panel {
    pub fn new(window: gtk4::Window, store: Rc<RefCell<NotificationStore>>) -> Self {
        window.add_css_class("startmenu");

        // ── Backdrop (click outside the menu closes it) ──────────────────────
        let backdrop = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .halign(gtk4::Align::Start)
            .valign(gtk4::Align::End)
            .hexpand(true)
            .vexpand(true)
            .build();
        backdrop.add_css_class("startmenu-backdrop");

        // ── Root container (the menu surface) ────────────────────────────────
        let root = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .halign(gtk4::Align::Start)
            .valign(gtk4::Align::End)
            .build();
        root.add_css_class("startmenu-root");

        // ── Body: two columns ─────────────────────────────────────────────────
        let body = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .hexpand(true)
            .vexpand(true)
            .spacing(0)
            .build();
        body.add_css_class("startmenu-body");

        // LEFT column — launcher (~60%).
        let launcher = Rc::new(LauncherView::new());
        let left = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .hexpand(true)
            .vexpand(true)
            .build();
        left.add_css_class("startmenu-launcher");
        left.append(launcher.widget());

        // RIGHT column — quick settings (~40%).
        let right_scroller = gtk4::ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .vexpand(true)
            .width_request(300)
            .build();
        let right = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(10)
            .build();
        right.add_css_class("startmenu-quick");
        right_scroller.set_child(Some(&right));

        // ── Build sections ───────────────────────────────────────────────────
        let audio = AudioSection::new();
        let brightness = BrightnessSection::new();
        let network = NetworkSection::new();
        let bluetooth = BluetoothSection::new();
        let display = DisplaySection::new();
        let media = MediaSection::new();
        let notifications = NotificationsSection::new(store.clone());
        let clipboard = ClipboardSection::new();
        let power = PowerSection::new();

        // 1. Volume slider (hoisted from AudioSection).
        right.append(&slider_row(ICON_SPEAKER, audio.output_volume_scale()));
        // 2. Brightness slider (hoisted from BrightnessSection).
        right.append(&slider_row(ICON_BRIGHTNESS, brightness.brightness_scale()));

        // 3. Toggle tiles — declarative spec → one factory.
        let specs = tiles::tile_specs(); // [wifi, bluetooth, night, idle]
        let grid = gtk4::Grid::builder()
            .row_spacing(8)
            .column_spacing(8)
            .column_homogeneous(true)
            .build();
        grid.add_css_class("startmenu-tile-grid");

        let mut tile_pairs: Vec<(gtk4::ToggleButton, tiles::TileSpec)> = Vec::new();
        let mut col = 0i32;
        let mut row_i = 0i32;

        // Wi-Fi (with inline device-list reveal).
        let (wifi_tile, _) = tile_with_reveal(&specs[0], network.widget(), &right, &mut tile_pairs);
        grid_place(&grid, &wifi_tile, &mut col, &mut row_i);

        // Bluetooth (with inline device-list reveal).
        let (bt_tile, _) = tile_with_reveal(&specs[1], bluetooth.widget(), &right, &mut tile_pairs);
        grid_place(&grid, &bt_tile, &mut col, &mut row_i);

        // DND (store-backed).
        let (dnd_tile, _) = tiles::build_dnd_tile(store.clone());
        grid_place(&grid, &dnd_tile, &mut col, &mut row_i);

        // Night Light.
        let (night_tile, night_btn) = tiles::build_tile(&specs[2]);
        tiles::init_tile_state(&night_btn, &specs[2]);
        tile_pairs.push((night_btn, copy_spec(&specs[2])));
        grid_place(&grid, &night_tile, &mut col, &mut row_i);

        // Idle.
        let (idle_tile, idle_btn) = tiles::build_tile(&specs[3]);
        tiles::init_tile_state(&idle_btn, &specs[3]);
        tile_pairs.push((idle_btn, copy_spec(&specs[3])));
        grid_place(&grid, &idle_tile, &mut col, &mut row_i);

        // Display — reveal-only tile (no radio); chevron + button reveal the
        // output/monitor controls inline, matching the Wi-Fi/Bluetooth pattern.
        // Expand the section so its output list shows as soon as we reveal it.
        display.expand_for_page();
        let display_tile = reveal_only_tile(ICON_DISPLAY, "Display", display.widget(), &right);
        grid_place(&grid, &display_tile, &mut col, &mut row_i);

        right.append(&grid);

        // 4. Media mini-card (MediaSection hides itself when nothing plays).
        right.append(media.widget());

        // 5. Notifications — compact, height-limited scroll.
        notifications.expand_for_page();
        let notif_scroller = gtk4::ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .max_content_height(200)
            .propagate_natural_height(true)
            .build();
        notif_scroller.add_css_class("startmenu-notifications");
        notif_scroller.set_child(Some(notifications.widget()));
        right.append(&notif_scroller);

        body.append(&left);
        body.append(&right_scroller);

        // ── Full-width clipboard reveal (beneath the body) ───────────────────
        // Clipboard rows are wide, so the ClipboardSection (cliphist history
        // with click-to-copy) lives in a full-width Revealer rather than the
        // narrow right column. Toggled by the footer Clipboard button.
        clipboard.expand_for_page();
        let clipboard_revealer = gtk4::Revealer::builder()
            .transition_type(gtk4::RevealerTransitionType::SlideDown)
            .transition_duration(200)
            .reveal_child(false)
            .build();
        clipboard_revealer.add_css_class("startmenu-clipboard");
        clipboard_revealer.set_child(Some(clipboard.widget()));

        // ── Footer ─────────────────────────────────────────────────────────
        let footer = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(8)
            .build();
        footer.add_css_class("startmenu-footer");

        let actions = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(6)
            .hexpand(true)
            .halign(gtk4::Align::Start)
            .build();
        actions.append(&footer_action("󰄀", "Screenshot region", &window, screenshot_region));

        // Clipboard — toggles the inline full-width history reveal (does NOT
        // hide the menu).
        let clip_btn = gtk4::Button::builder().label("󰅍").build();
        clip_btn.add_css_class("startmenu-footer-btn");
        clip_btn.set_tooltip_text(Some("Clipboard history"));
        {
            let revealer_c = clipboard_revealer.clone();
            clip_btn.connect_clicked(move |b| {
                let open = !revealer_c.reveals_child();
                revealer_c.set_reveal_child(open);
                if open {
                    b.add_css_class("active");
                } else {
                    b.remove_css_class("active");
                }
            });
        }
        actions.append(&clip_btn);

        actions.append(&footer_action("󰏘", "Color picker", &window, color_pick));
        footer.append(&actions);

        // Right: power/session actions (reuse PowerSection; expand its detail so
        // the lock/suspend/reboot/shutdown buttons show).
        power.expand_for_page();
        let power_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .halign(gtk4::Align::End)
            .build();
        power_box.add_css_class("startmenu-power");
        power_box.append(power.widget());
        footer.append(&power_box);

        root.append(&body);
        root.append(&clipboard_revealer);
        root.append(&footer);
        backdrop.append(&root);
        window.set_child(Some(&backdrop));

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
            tiles: RefCell::new(tile_pairs),
        });

        // ── Launcher activation + Esc hide the menu ──────────────────────────
        {
            let window_c = window.clone();
            launcher.set_on_activate(move || window_c.set_visible(false));
        }
        {
            let window_c = window.clone();
            launcher.install_key_controller(&window, move || window_c.set_visible(false));
        }

        // ── Backdrop click → dismiss; clicks on the menu are claimed ─────────
        let backdrop_gesture = gtk4::GestureClick::new();
        backdrop_gesture.set_propagation_phase(gtk4::PropagationPhase::Bubble);
        {
            let window_c = window.clone();
            backdrop_gesture.connect_released(move |_, _, _, _| {
                window_c.set_visible(false);
            });
        }
        backdrop.add_controller(backdrop_gesture);

        let root_gesture = gtk4::GestureClick::new();
        root_gesture.set_propagation_phase(gtk4::PropagationPhase::Bubble);
        root_gesture.connect_released(|gesture, _, _, _| {
            gesture.set_state(gtk4::EventSequenceState::Claimed);
        });
        root.add_controller(root_gesture);

        Self {
            window,
            sections,
            launcher,
        }
    }

    pub fn toggle(&self) {
        if self.window.is_visible() {
            self.window.set_visible(false);
        } else {
            self.launcher.reset();
            self.window.set_visible(true);
            let sections = self.sections.clone();
            let launcher = self.launcher.clone();
            glib::idle_add_local_once(move || {
                sections.refresh();
                launcher.focus_entry();
            });
        }
    }

    #[allow(dead_code)]
    pub fn refresh(&self) {
        self.sections.refresh();
    }

    pub fn refresh_audio(&self) {
        self.sections.audio.refresh();
    }

    pub fn refresh_brightness(&self) {
        self.sections.brightness.refresh();
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Attach `child` to the 2-column grid, advancing the cursor.
fn grid_place(grid: &gtk4::Grid, child: &gtk4::Box, col: &mut i32, row_i: &mut i32) {
    grid.attach(child, *col, *row_i, 1, 1);
    *col += 1;
    if *col >= 2 {
        *col = 0;
        *row_i += 1;
    }
}

/// A horizontal row: glyph icon + a hoisted scale. The scale already has its
/// value-changed handler wired by its owning section.
fn slider_row(icon: &str, scale: gtk4::Scale) -> gtk4::Box {
    let row = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(8)
        .build();
    row.add_css_class("startmenu-slider");

    let icon_lbl = gtk4::Label::builder()
        .label(icon)
        .halign(gtk4::Align::Center)
        .valign(gtk4::Align::Center)
        .build();
    icon_lbl.add_css_class("startmenu-slider-icon");

    scale.set_hexpand(true);
    scale.set_draw_value(false);

    row.append(&icon_lbl);
    row.append(&scale);
    row
}

/// Build a toggle tile whose chevron reveals the full device-list section
/// widget inline (in a Revealer appended into the right column). The toggle
/// button drives the radio on/off via the spec.
fn tile_with_reveal(
    spec: &tiles::TileSpec,
    section_widget: &gtk4::Box,
    right_column: &gtk4::Box,
    tile_pairs: &mut Vec<(gtk4::ToggleButton, tiles::TileSpec)>,
) -> (gtk4::Box, gtk4::ToggleButton) {
    let (vbox, btn) = tiles::build_tile(spec);
    tiles::init_tile_state(&btn, spec);
    tile_pairs.push((btn.clone(), copy_spec(spec)));

    let revealer = gtk4::Revealer::builder()
        .transition_type(gtk4::RevealerTransitionType::SlideDown)
        .transition_duration(200)
        .reveal_child(false)
        .build();
    revealer.set_child(Some(section_widget));
    right_column.append(&revealer);

    let chevron = gtk4::Button::builder().label("▸").build();
    chevron.add_css_class("startmenu-tile-chevron");
    {
        let revealer_c = revealer.clone();
        chevron.connect_clicked(move |b| {
            let open = !revealer_c.reveals_child();
            revealer_c.set_reveal_child(open);
            b.set_label(if open { "▾" } else { "▸" });
        });
    }
    vbox.append(&chevron);

    (vbox, btn)
}

/// A reveal-only tile: a non-radio toggle + chevron that both reveal the given
/// section widget inline (in a Revealer appended into the right column). Used
/// for Display, which has no on/off radio — only an inline panel to expand.
fn reveal_only_tile(
    icon: &str,
    label: &str,
    section_widget: &gtk4::Box,
    right_column: &gtk4::Box,
) -> gtk4::Box {
    let vbox = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .build();
    vbox.add_css_class("startmenu-tile");

    let btn = gtk4::ToggleButton::builder().label(icon).build();
    btn.add_css_class("toggle-btn");

    let label_w = gtk4::Label::builder().label(label).build();
    label_w.add_css_class("toggle-label");

    vbox.append(&btn);
    vbox.append(&label_w);

    let revealer = gtk4::Revealer::builder()
        .transition_type(gtk4::RevealerTransitionType::SlideDown)
        .transition_duration(200)
        .reveal_child(false)
        .build();
    revealer.set_child(Some(section_widget));
    right_column.append(&revealer);

    let chevron = gtk4::Button::builder().label("▸").build();
    chevron.add_css_class("startmenu-tile-chevron");

    // Both the toggle button and the chevron drive the same reveal.
    let sync = {
        let revealer_c = revealer.clone();
        let btn_c = btn.clone();
        let chevron_c = chevron.clone();
        move |open: bool| {
            revealer_c.set_reveal_child(open);
            btn_c.set_active(open);
            if open {
                btn_c.add_css_class("active");
            } else {
                btn_c.remove_css_class("active");
            }
            chevron_c.set_label(if open { "▾" } else { "▸" });
        }
    };
    {
        let revealer_c = revealer.clone();
        let sync_c = sync.clone();
        btn.connect_clicked(move |_| sync_c(!revealer_c.reveals_child()));
    }
    {
        let revealer_c = revealer.clone();
        chevron.connect_clicked(move |_| sync(!revealer_c.reveals_child()));
    }
    vbox.append(&chevron);

    vbox
}

/// `TileSpec` holds only `Copy` fields (str slices + fn pointers); duplicate one
/// cheaply to keep a copy alongside its button for refresh.
fn copy_spec(spec: &tiles::TileSpec) -> tiles::TileSpec {
    tiles::TileSpec {
        icon: spec.icon,
        label: spec.label,
        tooltip_on: spec.tooltip_on,
        tooltip_off: spec.tooltip_off,
        action: spec.action,
        read_state: spec.read_state,
    }
}

/// A footer icon button that hides the menu, then runs `action`.
fn footer_action(icon: &str, tooltip: &str, window: &gtk4::Window, action: fn()) -> gtk4::Button {
    let btn = gtk4::Button::builder().label(icon).build();
    btn.add_css_class("startmenu-footer-btn");
    btn.set_tooltip_text(Some(tooltip));
    let window_c = window.clone();
    btn.connect_clicked(move |_| {
        window_c.set_visible(false);
        action();
    });
    btn
}

// ── Footer action implementations ─────────────────────────────────────────────

fn screenshot_region() {
    use std::process::Command;
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    let dir = format!("{home}/Pictures/Screenshots");
    let _ = std::fs::create_dir_all(&dir);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = format!("{dir}/screenshot-{ts}.png");
    let cmd = format!("grim -g \"$(slurp)\" {path}");
    if let Err(e) = Command::new("sh").args(["-c", &cmd]).spawn() {
        log::error!("Failed to spawn grim/slurp region capture: {e}");
    }
}


fn color_pick() {
    use std::process::Command;
    if let Err(e) = Command::new("hyprpicker").arg("-a").spawn() {
        if e.kind() == std::io::ErrorKind::NotFound {
            log::warn!("hyprpicker not found");
        } else {
            log::warn!("hyprpicker -a failed: {e}");
        }
    }
}
