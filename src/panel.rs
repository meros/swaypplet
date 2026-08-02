//! Start-menu popup: bottom-left anchored surface fusing an app launcher
//! (left column) with a compact quick-settings control center (right column).

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

// ── Panel (start menu) ─────────────────────────────────────────────────────────

pub struct Panel {
    pub window: gtk4::Window,
    sections: Rc<Sections>,
    launcher: Rc<LauncherView>,
    /// Enter/exit transition for the glass menu: fast pane tint, full-length
    /// content fade, short [`anim::SlideBin`] settle (motion on glass,
    /// anim.rs). `is_shown()` is the intent flag; the window unmaps when the
    /// exit finishes.
    reveal: anim::Reveal,
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
        root.add_css_class("glass-card");
        root.add_css_class("startmenu-root");

        // The menu fades in while settling up SLIDE_PX from below the
        // window's bottom edge (4px above the bar), and fades out sinking
        // back — pane tint fast, content over the full fade, plus the slide
        // (motion on glass, anim.rs). The window is a fixed 780x700 layer
        // surface, so only internal layout animates; the surface itself
        // never resizes, and the settle overshoot is clipped at its bottom
        // edge.

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

        // RIGHT column — quick settings (~40%). propagate_natural_height lets
        // the window grow to fit the whole column (its natural height would
        // otherwise be near-zero, squeezing it to the left column's height and
        // slicing the last card at the viewport edge); the scrollbar only
        // appears when the column outgrows the monitor.
        let right_scroller = gtk4::ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .vexpand(true)
            .propagate_natural_height(true)
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
        let users = UserSection::new();

        // 1. Volume slider (hoisted from AudioSection).
        right.append(&slider_row(
            icons::SPEAKER_HIGH,
            audio.output_volume_scale(),
        ));
        // 2. Brightness slider (hoisted from BrightnessSection).
        right.append(&slider_row(
            icons::BRIGHTNESS,
            brightness.brightness_scale(),
        ));

        // 3. Toggle tiles — declarative spec → one factory.
        let specs = tiles::tile_specs(); // [wifi, bluetooth, night, caffeine]
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
        let wifi_tile = tile_with_reveal(&specs[0], network.widget(), &right, &mut tile_pairs);
        grid_place(&grid, &wifi_tile, &mut col, &mut row_i);

        // Bluetooth (with inline device-list reveal).
        let bt_tile = tile_with_reveal(&specs[1], bluetooth.widget(), &right, &mut tile_pairs);
        grid_place(&grid, &bt_tile, &mut col, &mut row_i);

        // DND (store-backed).
        let dnd_tile = tiles::build_dnd_tile(store.clone());
        grid_place(&grid, &dnd_tile, &mut col, &mut row_i);

        // Night Light.
        let night_btn = tiles::build_tile(&specs[2]);
        tiles::init_tile_state(&night_btn, &specs[2]);
        grid_place(&grid, &night_btn, &mut col, &mut row_i);
        tile_pairs.push((night_btn, copy_spec(&specs[2])));

        // Caffeine.
        let caffeine_btn = tiles::build_tile(&specs[3]);
        tiles::init_tile_state(&caffeine_btn, &specs[3]);
        grid_place(&grid, &caffeine_btn, &mut col, &mut row_i);
        tile_pairs.push((caffeine_btn, copy_spec(&specs[3])));

        // Display — reveal-only tile (no radio); chevron + button reveal the
        // output/monitor controls inline, matching the Wi-Fi/Bluetooth pattern.
        // Expand the section so its output list shows as soon as we reveal it.
        display.expand_for_page();
        let display_tile = reveal_only_tile(icons::DISPLAY, "Display", display.widget(), &right);
        grid_place(&grid, &display_tile, &mut col, &mut row_i);

        right.append(&grid);

        // 4. Media mini-card (MediaSection hides itself when nothing plays).
        right.append(media.widget());

        // 5. Notifications — compact, height-limited via the section's own
        //    scroller (a second wrapping ScrolledWindow would nest three
        //    vertical scrollers and clip the cards below).
        notifications.expand_for_page();
        notifications.set_list_max_height(200);
        notifications
            .widget()
            .add_css_class("startmenu-notifications");
        right.append(notifications.widget());

        // 6. Power status card (battery + governor). Session *actions* now live
        //    in the left rail; this card is the at-a-glance status readout.
        power.expand_for_page();
        right.append(power.widget());

        // 7. Session-aware user switcher (avatars + presence). Hidden unless the
        //    host exposes SWAYPPLET_SWITCH_USER_CMD; rows are filled on refresh.
        right.append(users.widget());

        // ── Full-width clipboard reveal (beneath the body) ───────────────────
        // Clipboard rows are wide, so the ClipboardSection (cliphist history
        // with click-to-copy) lives in a full-width Revealer rather than the
        // narrow right column. Toggled by the rail Clipboard button.
        clipboard.expand_for_page();
        let clipboard_revealer = gtk4::Revealer::builder()
            .transition_type(gtk4::RevealerTransitionType::SlideDown)
            .transition_duration(200)
            .reveal_child(false)
            .build();
        clipboard_revealer.add_css_class("startmenu-clipboard");
        clipboard_revealer.set_child(Some(clipboard.widget()));

        // ── Left rail: utilities (top) + session actions (bottom) ────────────
        // The rail absorbs the formerly-orphaned utility buttons and the power
        // session actions into a single far-left zone, leaving the centre stage
        // purely for launching and the right column purely for quick settings.
        let rail = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(6)
            .build();
        rail.add_css_class("startmenu-rail");

        rail.append(&rail_action(
            "󰄀",
            "Screenshot region",
            &window,
            screenshot_region,
        ));

        // Clipboard — toggles the inline full-width history reveal (does NOT
        // hide the menu).
        let clip_btn = gtk4::Button::builder()
            .child(&gtk4::Label::new(Some("󰅍")))
            .build();
        clip_btn.add_css_class("rail-btn");
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
        rail.append(&clip_btn);

        rail.append(&rail_action("󰏘", "Color picker", &window, color_pick));

        // Spacer pushes the session actions to the bottom of the rail.
        let rail_spacer = gtk4::Box::builder().vexpand(true).build();
        rail.append(&rail_spacer);

        let rail_divider = gtk4::Separator::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .build();
        rail_divider.add_css_class("startmenu-rail-divider");
        rail.append(&rail_divider);

        rail.append(&power::build_session_rail());

        // ── Assemble body: rail | launcher (stage) | quick settings ──────────
        body.append(&rail);
        body.append(&left);
        body.append(&right_scroller);

        // Content sits on the glass (`root` carries the frosted background)
        // and fades over the full duration while the pane tint arrives fast.
        let content = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(0)
            .build();
        content.append(&body);
        content.append(&clipboard_revealer);
        root.append(&content);

        let slide = anim::SlideBin::new();
        slide.set_child(&root);
        slide.jump_to(anim::SLIDE_PX);
        backdrop.append(&slide);
        window.set_child(Some(&backdrop));

        // Every open/close path (bar toggle, backdrop click, Esc, rail
        // actions) goes through this one Reveal, which also drives the
        // SlideBin settle and parks the surface once the exit finishes.
        // Premap warms the compositor blur pipeline (see Reveal::premap).
        let reveal = anim::Reveal::new(&window, &root)
            .content(&content)
            .slide(&slide, anim::SLIDE_PX);
        reveal.premap();

        // Shared dismiss path: fade the menu out, then unmap.
        let hide_menu = {
            let reveal_c = reveal.clone();
            Rc::new(move || reveal_c.hide())
        };

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

        // ── Launcher activation + Esc hide the menu ──────────────────────────
        {
            let hide = hide_menu.clone();
            launcher.set_on_activate(move || hide());
        }
        {
            let hide = hide_menu.clone();
            launcher.install_key_controller(&window, move || hide());
        }

        // ── Backdrop click → dismiss; clicks on the menu are claimed ─────────
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

        Self {
            window,
            sections,
            launcher,
            reveal,
        }
    }

    pub fn toggle(&self) {
        if self.reveal.is_shown() && self.window.is_visible() {
            self.reveal.hide();
        } else {
            // Instant-hide paths (rail actions, power session actions) unmap
            // the window without going through hide(); show() on an unmapped
            // window restarts from the fully hidden pose, so the enter
            // fade+settle plays instead of skipping.
            self.launcher.reset();
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

/// Attach `child` to the 2-column grid, advancing the cursor.
fn grid_place(grid: &gtk4::Grid, child: &impl IsA<gtk4::Widget>, col: &mut i32, row_i: &mut i32) {
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

    // The scale is hoisted from its owning section, where it already has a
    // parent. GTK4 does not auto-reparent — append would hit the
    // `gtk_widget_get_parent == NULL` assertion and silently drop it (the
    // slider would never show). Detach it from its old parent first.
    if scale.parent().is_some() {
        scale.unparent();
    }

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
) -> gtk4::Overlay {
    let btn = tiles::build_tile(spec);
    tiles::init_tile_state(&btn, spec);
    tile_pairs.push((btn.clone(), copy_spec(spec)));

    let revealer = gtk4::Revealer::builder()
        .transition_type(gtk4::RevealerTransitionType::SlideDown)
        .transition_duration(200)
        .reveal_child(false)
        .build();
    revealer.set_child(Some(section_widget));
    right_column.append(&revealer);

    // Chevron floats at the tile's right edge; tapping the tile body toggles the
    // radio, tapping the chevron reveals the device list inline below.
    let (cell, chevron) = tiles::wrap_with_chevron(&btn);
    {
        let revealer_c = revealer.clone();
        chevron.connect_clicked(move |b| {
            let open = !revealer_c.reveals_child();
            revealer_c.set_reveal_child(open);
            tiles::set_chevron_open(b, open);
        });
    }

    cell
}

/// A reveal-only tile: a non-radio toggle + chevron that both reveal the given
/// section widget inline (in a Revealer appended into the right column). Used
/// for Display, which has no on/off radio — only an inline panel to expand.
fn reveal_only_tile(
    icon: &str,
    label: &str,
    section_widget: &gtk4::Box,
    right_column: &gtk4::Box,
) -> gtk4::Overlay {
    let btn = tiles::build_reveal_tile(icon, label);

    let revealer = gtk4::Revealer::builder()
        .transition_type(gtk4::RevealerTransitionType::SlideDown)
        .transition_duration(200)
        .reveal_child(false)
        .build();
    revealer.set_child(Some(section_widget));
    right_column.append(&revealer);

    let (cell, chevron) = tiles::wrap_with_chevron(&btn);

    // Reveal-only (no radio): both the toggle body and the chevron drive the
    // same reveal. The button's toggled handler manages its `.active` class.
    let sync = {
        let revealer_c = revealer.clone();
        let btn_c = btn.clone();
        let chevron_c = chevron.clone();
        move |open: bool| {
            revealer_c.set_reveal_child(open);
            btn_c.set_active(open);
            tiles::set_chevron_open(&chevron_c, open);
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

    cell
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
        on_state: spec.on_state,
    }
}

/// A rail icon button that hides the menu instantly (no exit wipe — the
/// action may capture the screen), then runs `action`. The stale reveal
/// state this leaves behind is healed by `Panel::toggle`.
fn rail_action(icon: &str, tooltip: &str, window: &gtk4::Window, action: fn()) -> gtk4::Button {
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
