use std::cell::{Cell, RefCell};
use std::fs;
use std::rc::Rc;
use std::sync::mpsc;

use gtk4::prelude::*;

// ---------------------------------------------------------------------------
// Sysfs helpers
// ---------------------------------------------------------------------------

const CPU_GOVERNOR: &str = "/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor";
const CPU_AVAIL_GOVERNORS: &str =
    "/sys/devices/system/cpu/cpu0/cpufreq/scaling_available_governors";

fn read_sysfs(path: &str) -> Option<String> {
    match fs::read_to_string(path) {
        Ok(s) => {
            let trimmed = s.trim().to_owned();
            if trimmed.is_empty() { None } else { Some(trimmed) }
        }
        Err(e) => {
            log::warn!("Failed to read {}: {}", path, e);
            None
        }
    }
}

/// Scan `/sys/class/power_supply/` for the first entry whose `type` file
/// contains "Battery" and return its path (e.g.
/// `/sys/class/power_supply/BAT0`). Returns `None` on desktops without a
/// battery.
fn find_battery_path() -> Option<String> {
    let dir = match fs::read_dir("/sys/class/power_supply") {
        Ok(d) => d,
        Err(e) => {
            log::warn!("Cannot read /sys/class/power_supply: {}", e);
            return None;
        }
    };

    let mut entries: Vec<_> = dir
        .filter_map(|e| e.ok())
        .filter(|e| {
            let type_path = e.path().join("type");
            fs::read_to_string(&type_path)
                .map(|t| t.trim().eq_ignore_ascii_case("Battery"))
                .unwrap_or(false)
        })
        .collect();

    // Sort for deterministic order (BAT0 before BAT1, etc.).
    entries.sort_by_key(|e| e.file_name());

    entries
        .first()
        .map(|e| e.path().to_string_lossy().into_owned())
}

// ---------------------------------------------------------------------------
// Domain types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct BatteryState {
    /// 0–100
    capacity: u8,
    charging: bool,
    /// Watts (power_now / 1_000_000)
    power_w: Option<f64>,
    /// Wh remaining
    energy_now_wh: Option<f64>,
    /// Wh at full
    energy_full_wh: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
enum GovernorProfile {
    Performance,
    Balanced,
    Powersave,
    Other(String),
}

impl GovernorProfile {
    fn from_sysfs(raw: &str) -> Self {
        match raw.trim() {
            "performance" => GovernorProfile::Performance,
            "schedutil" | "ondemand" | "conservative" => GovernorProfile::Balanced,
            "powersave" => GovernorProfile::Powersave,
            other => GovernorProfile::Other(other.to_owned()),
        }
    }

    fn sysfs_name(&self) -> &str {
        match self {
            GovernorProfile::Performance => "performance",
            GovernorProfile::Balanced => "schedutil",
            GovernorProfile::Powersave => "powersave",
            GovernorProfile::Other(s) => s.as_str(),
        }
    }
}

// ---------------------------------------------------------------------------
// Battery reading
// ---------------------------------------------------------------------------

fn read_battery(bat_path: &str) -> Option<BatteryState> {
    let capacity: u8 = read_sysfs(&format!("{}/capacity", bat_path))
        .and_then(|s| s.parse().ok())
        .or_else(|| {
            log::error!("Battery info unavailable: cannot read {}/capacity", bat_path);
            None
        })?;

    let status = read_sysfs(&format!("{}/status", bat_path)).unwrap_or_else(|| {
        log::warn!("Cannot read {}/status, assuming Discharging", bat_path);
        "Discharging".to_owned()
    });

    let charging =
        status.eq_ignore_ascii_case("Charging") || status.eq_ignore_ascii_case("Full");

    let power_w = read_sysfs(&format!("{}/power_now", bat_path))
        .and_then(|s| s.parse::<u64>().ok())
        .map(|uw| uw as f64 / 1_000_000.0);

    let energy_now_wh = read_sysfs(&format!("{}/energy_now", bat_path))
        .and_then(|s| s.parse::<u64>().ok())
        .map(|uwh| uwh as f64 / 1_000_000.0);

    let energy_full_wh = read_sysfs(&format!("{}/energy_full", bat_path))
        .and_then(|s| s.parse::<u64>().ok())
        .map(|uwh| uwh as f64 / 1_000_000.0);

    Some(BatteryState {
        capacity,
        charging,
        power_w,
        energy_now_wh,
        energy_full_wh,
    })
}

// ---------------------------------------------------------------------------
// Battery display helpers
// ---------------------------------------------------------------------------

/// Battery icon (Nerd Font) based on charge level and charging status.
fn battery_icon(capacity: u8, charging: bool) -> &'static str {
    if charging {
        return "󰂄";
    }
    match capacity {
        90..=100 => "󰁹",
        70..=89 => "󰂁",
        50..=69 => "󰁾",
        20..=49 => "󰁻",
        _ => "󰂃",
    }
}

/// Format a duration in hours as "Xh Ym".
///
/// Returns `None` when the estimate is unreliable (zero, negative, or NaN).
/// Returns `Some("24h+")` when the estimate exceeds 24 hours.
fn format_hours(h: f64) -> Option<String> {
    if h <= 0.0 || h.is_nan() || h.is_infinite() {
        return None;
    }
    if h > 24.0 {
        return Some("24h+".to_owned());
    }
    let total_mins = (h * 60.0).round() as u64;
    let hrs = total_mins / 60;
    let mins = total_mins % 60;
    Some(if hrs == 0 {
        format!("{}m", mins)
    } else {
        format!("{}h {}m", hrs, mins)
    })
}

fn battery_sub_text(bat: &BatteryState) -> String {
    // Compute a time estimate string, or "Calculating..." when power_now is 0.
    let time_str: Option<String> = match (bat.power_w, bat.energy_now_wh, bat.energy_full_wh) {
        (Some(power), Some(energy_now), Some(energy_full)) => {
            if power < 0.001 {
                // power_now == 0 — meter hasn't settled yet.
                Some("Calculating...".to_owned())
            } else if bat.charging {
                let to_full = (energy_full - energy_now).max(0.0);
                match format_hours(to_full / power) {
                    Some(t) => Some(format!("{} to full", t)),
                    None => Some("Calculating...".to_owned()),
                }
            } else {
                match format_hours(energy_now / power) {
                    Some(t) => Some(format!("{} remaining", t)),
                    None => Some("Calculating...".to_owned()),
                }
            }
        }
        _ => None,
    };

    if bat.charging {
        match time_str.as_deref() {
            Some("Calculating...") | None => "Charging".to_owned(),
            Some(t) => format!("Charging — {}", t),
        }
    } else if bat.capacity == 100 {
        "Fully charged".to_owned()
    } else {
        match time_str.as_deref() {
            Some("Calculating...") => "On battery — Calculating...".to_owned(),
            None => "On battery".to_owned(),
            Some(t) => format!("On battery — {}", t),
        }
    }
}

// ---------------------------------------------------------------------------
// Governor helpers
// ---------------------------------------------------------------------------

fn read_governor() -> GovernorProfile {
    read_sysfs(CPU_GOVERNOR)
        .map(|s| GovernorProfile::from_sysfs(&s))
        .unwrap_or(GovernorProfile::Balanced)
}

fn read_available_governors() -> Vec<String> {
    read_sysfs(CPU_AVAIL_GOVERNORS)
        .map(|s| s.split_whitespace().map(str::to_owned).collect())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Widget state
// ---------------------------------------------------------------------------

struct PowerState {
    battery: Option<BatteryState>,
    governor: GovernorProfile,
}

impl PowerState {
    fn read(bat_path: Option<&str>) -> Self {
        Self {
            battery: bat_path.and_then(read_battery),
            governor: read_governor(),
        }
    }
}

// ---------------------------------------------------------------------------
// Battery refresh handles — shared between PowerSection and the 30-s timer
// ---------------------------------------------------------------------------

/// Cheap GTK widget handles shared via `Rc` so the periodic timer can push
/// updates without borrowing `PowerSection`.
struct BatteryHandles {
    bat_path: String,
    icon_lbl: gtk4::Label,
    level_lbl: gtk4::Label,
    sub_lbl: gtk4::Label,
    level_bar: gtk4::LevelBar,
}

impl BatteryHandles {
    fn apply(&self, bat: &BatteryState) {
        self.icon_lbl
            .set_label(battery_icon(bat.capacity, bat.charging));
        self.level_lbl.set_label(&format!("{}%", bat.capacity));
        self.sub_lbl.set_label(&battery_sub_text(bat));
        self.level_bar.set_value(bat.capacity as f64 / 100.0);

        if bat.capacity < 20 {
            self.level_bar.add_css_class("low");
        } else {
            self.level_bar.remove_css_class("low");
        }
        if bat.charging {
            self.level_bar.add_css_class("charging");
        } else {
            self.level_bar.remove_css_class("charging");
        }
    }
}

// ---------------------------------------------------------------------------
// PowerSection
// ---------------------------------------------------------------------------

pub struct PowerSection {
    root: gtk4::Box,
    /// Cached battery sysfs path (None on desktops without a battery).
    bat_path: Option<String>,
    state: RefCell<PowerState>,

    // Battery widget handles (only present when a battery was found).
    bat_handles: Option<Rc<BatteryHandles>>,

    // Governor buttons
    btn_performance: gtk4::Button,
    btn_balanced: gtk4::Button,
    btn_powersave: gtk4::Button,
}

impl PowerSection {
    pub fn new() -> Self {
        // ── Root section box ────────────────────────────────────────────────
        let root = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(12)
            .build();
        root.add_css_class("section");

        // ── Section title ─────────────────────────────────────────────────
        let title = gtk4::Label::builder()
            .label("POWER")
            .halign(gtk4::Align::Start)
            .build();
        title.add_css_class("section-title");
        root.append(&title);

        // ── Discover battery path ────────────────────────────────────────
        let bat_path = find_battery_path();
        if bat_path.is_none() {
            log::info!("No battery found; battery section hidden.");
        }

        // ── Read initial state ────────────────────────────────────────────
        let state = PowerState::read(bat_path.as_deref());

        // ── Battery row (conditional) ─────────────────────────────────────
        let bat_handles: Option<Rc<BatteryHandles>> = if let Some(ref bat) = state.battery {
            let bat_row = gtk4::Box::builder()
                .orientation(gtk4::Orientation::Vertical)
                .spacing(4)
                .build();
            bat_row.add_css_class("battery-row");

            // Top line: icon + percentage
            let top_row = gtk4::Box::builder()
                .orientation(gtk4::Orientation::Horizontal)
                .spacing(8)
                .valign(gtk4::Align::Center)
                .build();

            let icon_lbl = gtk4::Label::builder()
                .label(battery_icon(bat.capacity, bat.charging))
                .halign(gtk4::Align::Start)
                .build();
            icon_lbl.add_css_class("battery-icon");

            let level_lbl = gtk4::Label::builder()
                .label(&format!("{}%", bat.capacity))
                .halign(gtk4::Align::Start)
                .build();
            level_lbl.add_css_class("battery-level");

            top_row.append(&icon_lbl);
            top_row.append(&level_lbl);

            // Sub text: charging/time
            let sub_lbl = gtk4::Label::builder()
                .label(&battery_sub_text(bat))
                .halign(gtk4::Align::Start)
                .wrap(true)
                .build();
            sub_lbl.add_css_class("battery-sub");

            // Level bar
            let level_bar = gtk4::LevelBar::builder()
                .min_value(0.0)
                .max_value(1.0)
                .value(bat.capacity as f64 / 100.0)
                .build();
            level_bar.add_css_class("battery-bar");
            if bat.capacity < 20 {
                level_bar.add_css_class("low");
            }
            if bat.charging {
                level_bar.add_css_class("charging");
            }

            bat_row.append(&top_row);
            bat_row.append(&sub_lbl);
            bat_row.append(&level_bar);
            root.append(&bat_row);

            Some(Rc::new(BatteryHandles {
                bat_path: bat_path.as_deref().unwrap_or("").to_owned(),
                icon_lbl,
                level_lbl,
                sub_lbl,
                level_bar,
            }))
        } else {
            None
        };

        // ── CPU governor section ──────────────────────────────────────────
        let cpu_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(6)
            .build();
        cpu_box.add_css_class("cpu-governor-section");

        let cpu_label = gtk4::Label::builder()
            .label("CPU Profile")
            .halign(gtk4::Align::Start)
            .build();
        cpu_label.add_css_class("cpu-governor-label");
        cpu_box.append(&cpu_label);

        let btn_row = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(4)
            .homogeneous(true)
            .build();
        btn_row.add_css_class("governor-btn-row");

        let btn_performance = gtk4::Button::with_label("Performance");
        let btn_balanced = gtk4::Button::with_label("Balanced");
        let btn_powersave = gtk4::Button::with_label("Powersave");

        btn_performance.add_css_class("governor-btn");
        btn_balanced.add_css_class("governor-btn");
        btn_powersave.add_css_class("governor-btn");

        btn_row.append(&btn_performance);
        btn_row.append(&btn_balanced);
        btn_row.append(&btn_powersave);
        cpu_box.append(&btn_row);
        root.append(&cpu_box);

        // ── Power actions separator ───────────────────────────────────────
        let sep = gtk4::Separator::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .build();
        root.append(&sep);

        // ── Power actions row ─────────────────────────────────────────────
        let actions_row = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(4)
            .homogeneous(true)
            .build();
        actions_row.add_css_class("power-actions-row");

        // Helper: build one icon-button + label column.
        // Returns (column_box, button, icon_label, text_label).
        let make_action_btn = |icon: &str, name: &str, destructive: bool| {
            let col = gtk4::Box::builder()
                .orientation(gtk4::Orientation::Vertical)
                .spacing(4)
                .halign(gtk4::Align::Center)
                .build();

            let icon_lbl = gtk4::Label::builder().label(icon).build();
            let btn = gtk4::Button::builder().child(&icon_lbl).build();
            btn.add_css_class("toggle-btn");
            if destructive {
                btn.add_css_class("destructive");
            }

            let text_lbl = gtk4::Label::builder().label(name).build();
            text_lbl.add_css_class("toggle-label");

            col.append(&btn);
            col.append(&text_lbl);
            (col, btn, icon_lbl, text_lbl)
        };

        // ── Lock — hide panel first, then lock ────────────────────────────
        let (col_lock, btn_lock, _, _) = make_action_btn("󰌾", "Lock", false);
        btn_lock.connect_clicked(|btn| {
            hide_panel_for_widget(btn.upcast_ref());
            let _ = std::process::Command::new("loginctl")
                .arg("lock-session")
                .spawn()
                .map_err(|e| log::error!("Failed to spawn loginctl lock-session: {}", e));
        });
        actions_row.append(&col_lock);

        // ── Suspend — hide panel first, then suspend ──────────────────────
        let (col_suspend, btn_suspend, _, _) = make_action_btn("󰤄", "Suspend", false);
        btn_suspend.connect_clicked(|btn| {
            hide_panel_for_widget(btn.upcast_ref());
            let _ = std::process::Command::new("systemctl")
                .arg("suspend")
                .spawn()
                .map_err(|e| log::error!("Failed to spawn systemctl suspend: {}", e));
        });
        actions_row.append(&col_suspend);

        // ── Logout ────────────────────────────────────────────────────────
        let (col_logout, btn_logout, _, _) = make_action_btn("󰍃", "Logout", false);
        btn_logout.connect_clicked(|_| {
            let _ = std::process::Command::new("swaymsg")
                .arg("exit")
                .spawn()
                .map_err(|e| log::error!("Failed to spawn swaymsg exit: {}", e));
        });
        actions_row.append(&col_logout);

        // ── Reboot (destructive — needs confirmation with countdown) ──────
        let (col_reboot, btn_reboot, reboot_icon_lbl, reboot_text_lbl) =
            make_action_btn("󰜉", "Reboot", true);
        {
            let pending = Rc::new(Cell::new(false));
            let countdown = Rc::new(Cell::new(0u32));
            btn_reboot.connect_clicked(move |btn| {
                if pending.get() {
                    // Second click within the window — execute.
                    let _ = std::process::Command::new("systemctl")
                        .arg("reboot")
                        .spawn()
                        .map_err(|e| log::error!("Failed to spawn systemctl reboot: {}", e));
                } else {
                    // First click — start 3-second confirmation countdown.
                    pending.set(true);
                    countdown.set(3);
                    reboot_icon_lbl.set_label("?");
                    reboot_text_lbl.set_label("Reboot? (3)");
                    btn.add_css_class("confirming");

                    let pending_c = pending.clone();
                    let countdown_c = countdown.clone();
                    let icon_c = reboot_icon_lbl.clone();
                    let text_c = reboot_text_lbl.clone();
                    let btn_c = btn.clone();
                    glib::timeout_add_seconds_local(1, move || {
                        if !pending_c.get() {
                            return glib::ControlFlow::Break;
                        }
                        let n = countdown_c.get().saturating_sub(1);
                        countdown_c.set(n);
                        if n == 0 {
                            pending_c.set(false);
                            icon_c.set_label("󰜉");
                            text_c.set_label("Reboot");
                            btn_c.remove_css_class("confirming");
                            glib::ControlFlow::Break
                        } else {
                            text_c.set_label(&format!("Reboot? ({})", n));
                            glib::ControlFlow::Continue
                        }
                    });
                }
            });
        }
        actions_row.append(&col_reboot);

        // ── Shutdown (destructive — needs confirmation with countdown) ────
        let (col_shutdown, btn_shutdown, shutdown_icon_lbl, shutdown_text_lbl) =
            make_action_btn("󰐥", "Shutdown", true);
        {
            let pending = Rc::new(Cell::new(false));
            let countdown = Rc::new(Cell::new(0u32));
            btn_shutdown.connect_clicked(move |btn| {
                if pending.get() {
                    // Second click within the window — execute.
                    let _ = std::process::Command::new("systemctl")
                        .arg("poweroff")
                        .spawn()
                        .map_err(|e| log::error!("Failed to spawn systemctl poweroff: {}", e));
                } else {
                    // First click — start 3-second confirmation countdown.
                    pending.set(true);
                    countdown.set(3);
                    shutdown_icon_lbl.set_label("?");
                    shutdown_text_lbl.set_label("Shutdown? (3)");
                    btn.add_css_class("confirming");

                    let pending_c = pending.clone();
                    let countdown_c = countdown.clone();
                    let icon_c = shutdown_icon_lbl.clone();
                    let text_c = shutdown_text_lbl.clone();
                    let btn_c = btn.clone();
                    glib::timeout_add_seconds_local(1, move || {
                        if !pending_c.get() {
                            return glib::ControlFlow::Break;
                        }
                        let n = countdown_c.get().saturating_sub(1);
                        countdown_c.set(n);
                        if n == 0 {
                            pending_c.set(false);
                            icon_c.set_label("󰐥");
                            text_c.set_label("Shutdown");
                            btn_c.remove_css_class("confirming");
                            glib::ControlFlow::Break
                        } else {
                            text_c.set_label(&format!("Shutdown? ({})", n));
                            glib::ControlFlow::Continue
                        }
                    });
                }
            });
        }
        actions_row.append(&col_shutdown);
        root.append(&actions_row);

        // Apply initial active governor class.
        Self::apply_governor_active_class(
            &state.governor,
            &btn_performance,
            &btn_balanced,
            &btn_powersave,
        );

        // ── Governor button click handlers ────────────────────────────────
        connect_governor_btn(
            &btn_performance,
            GovernorProfile::Performance,
            &btn_performance,
            &btn_balanced,
            &btn_powersave,
        );
        connect_governor_btn(
            &btn_balanced,
            GovernorProfile::Balanced,
            &btn_performance,
            &btn_balanced,
            &btn_powersave,
        );
        connect_governor_btn(
            &btn_powersave,
            GovernorProfile::Powersave,
            &btn_performance,
            &btn_balanced,
            &btn_powersave,
        );

        // ── Periodic battery refresh every 30 s ───────────────────────────
        if let Some(ref handles) = bat_handles {
            let handles_weak = Rc::downgrade(handles);
            glib::timeout_add_seconds_local(30, move || {
                let Some(h) = handles_weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                // Only refresh when the widget is visible (mapped to screen).
                if !h.level_bar.is_mapped() {
                    return glib::ControlFlow::Continue;
                }
                if let Some(bat) = read_battery(&h.bat_path) {
                    h.apply(&bat);
                } else {
                    log::error!("Battery info unavailable during periodic refresh.");
                }
                glib::ControlFlow::Continue
            });
        }

        Self {
            root,
            bat_path,
            state: RefCell::new(state),
            bat_handles,
            btn_performance,
            btn_balanced,
            btn_powersave,
        }
    }

    /// Re-read sysfs and update all widgets.
    pub fn refresh(&self) {
        let new_state = PowerState::read(self.bat_path.as_deref());

        if let (Some(bat), Some(handles)) = (&new_state.battery, &self.bat_handles) {
            handles.apply(bat);
        }

        Self::apply_governor_active_class(
            &new_state.governor,
            &self.btn_performance,
            &self.btn_balanced,
            &self.btn_powersave,
        );

        *self.state.borrow_mut() = new_state;
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    // ── Private helpers ───────────────────────────────────────────────────

    fn apply_governor_active_class(
        active: &GovernorProfile,
        btn_perf: &gtk4::Button,
        btn_bal: &gtk4::Button,
        btn_save: &gtk4::Button,
    ) {
        set_active(btn_perf, *active == GovernorProfile::Performance);
        set_active(btn_bal, *active == GovernorProfile::Balanced);
        set_active(btn_save, *active == GovernorProfile::Powersave);
    }
}

// ---------------------------------------------------------------------------
// Standalone helpers
// ---------------------------------------------------------------------------

fn set_active(btn: &gtk4::Button, active: bool) {
    if active {
        btn.add_css_class("active");
    } else {
        btn.remove_css_class("active");
    }
}

/// Walk up the widget hierarchy to find the containing `gtk4::Window` and
/// hide it. Used by Lock and Suspend to close the panel before acting.
fn hide_panel_for_widget(widget: &gtk4::Widget) {
    if let Some(root) = widget.root() {
        if let Ok(window) = root.downcast::<gtk4::Window>() {
            window.set_visible(false);
        }
    }
}

/// Wire up a governor button: show "pending" state while pkexec runs, then
/// re-read the actual governor and update the active class for all three
/// buttons regardless of success or failure.
fn connect_governor_btn(
    target_btn: &gtk4::Button,
    profile: GovernorProfile,
    b_perf: &gtk4::Button,
    b_bal: &gtk4::Button,
    b_save: &gtk4::Button,
) {
    let b_perf = b_perf.clone();
    let b_bal = b_bal.clone();
    let b_save = b_save.clone();

    target_btn.connect_clicked(move |btn| {
        // Show pending state immediately.
        btn.add_css_class("pending");
        btn.set_sensitive(false);

        let b_perf_c = b_perf.clone();
        let b_bal_c = b_bal.clone();
        let b_save_c = b_save.clone();
        let btn_c = btn.clone();

        spawn_set_governor(profile.clone(), move |_success| {
            btn_c.remove_css_class("pending");
            btn_c.set_sensitive(true);
            // Re-read the actual governor to reflect reality (pkexec may have
            // been cancelled, or the kernel may have rejected the value).
            let actual = read_governor();
            PowerSection::apply_governor_active_class(&actual, &b_perf_c, &b_bal_c, &b_save_c);
        });
    });
}

/// Spawn `pkexec` in a background thread to write the chosen governor to all
/// CPU cores. Calls `on_done(success)` back on the main thread via GLib's
/// idle queue when the process exits.
///
/// The available governors list is checked before spawning so we only attempt
/// governors the kernel actually supports.
fn spawn_set_governor<F>(profile: GovernorProfile, on_done: F)
where
    F: FnOnce(bool) + 'static,
{
    let available = read_available_governors();
    let target = profile.sysfs_name();

    let governor = if available.iter().any(|g| g == target) {
        target.to_owned()
    } else if profile == GovernorProfile::Balanced && available.iter().any(|g| g == "ondemand") {
        "ondemand".to_owned()
    } else {
        log::warn!(
            "Governor '{}' not available (available: {}). Skipping.",
            target,
            available.join(", ")
        );
        on_done(false);
        return;
    };

    let cmd = format!(
        "echo {} | tee /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor",
        governor
    );

    log::info!("Requesting CPU governor change to '{}'", governor);

    // Use a channel: the worker thread sends the result; a GLib idle source on
    // the main thread drains it and calls `on_done`.  This keeps `on_done`
    // (which holds GTK widget handles, i.e. non-Send Rc) on the main thread.
    let (tx, rx) = mpsc::channel::<bool>();

    std::thread::spawn(move || {
        let success = match std::process::Command::new("pkexec")
            .args(["sh", "-c", &cmd])
            .status()
        {
            Ok(status) => {
                if status.success() {
                    log::info!("Governor changed to '{}'", governor);
                    true
                } else {
                    log::warn!(
                        "pkexec exited with status {:?} for governor '{}'",
                        status.code(),
                        governor
                    );
                    false
                }
            }
            Err(e) => {
                log::error!("Failed to spawn pkexec for governor change: {}", e);
                false
            }
        };
        let _ = tx.send(success);
    });

    // `on_done` is non-Send (may capture Rc<…>) so we wrap it in Option and
    // drive it from a main-thread idle callback that polls the channel.
    let mut on_done = Some(on_done);
    glib::idle_add_local(move || match rx.try_recv() {
        Ok(success) => {
            if let Some(f) = on_done.take() {
                f(success);
            }
            glib::ControlFlow::Break
        }
        Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
        Err(mpsc::TryRecvError::Disconnected) => {
            // Worker thread panicked or channel broken.
            if let Some(f) = on_done.take() {
                f(false);
            }
            glib::ControlFlow::Break
        }
    });
}
