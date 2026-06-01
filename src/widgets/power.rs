use std::cell::{Cell, RefCell};
use std::fs;
use std::rc::Rc;

use gtk4::prelude::*;

// ---------------------------------------------------------------------------
// Sysfs helpers
// ---------------------------------------------------------------------------

const CPU_GOVERNOR: &str = "/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor";

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
    /// Battery health as percentage of design capacity (energy_full / energy_full_design * 100)
    health_pct: Option<u8>,
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

    let energy_full_design_wh = read_sysfs(&format!("{}/energy_full_design", bat_path))
        .and_then(|s| s.parse::<u64>().ok())
        .map(|uwh| uwh as f64 / 1_000_000.0);

    let health_pct = match (energy_full_wh, energy_full_design_wh) {
        (Some(full), Some(design)) if design > 0.0 => {
            Some(((full / design) * 100.0).round().min(100.0) as u8)
        }
        _ => None,
    };

    Some(BatteryState {
        capacity,
        charging,
        power_w,
        energy_now_wh,
        energy_full_wh,
        health_pct,
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

/// Build the summary text for the summary row from battery state.
/// Format: "85% · Charging — 30m to full" or "85% · 3h 20m remaining"
fn battery_summary_text(bat: &BatteryState) -> String {
    format!("{}% · {}", bat.capacity, battery_sub_text(bat))
}

// ---------------------------------------------------------------------------
// Governor helpers
// ---------------------------------------------------------------------------

fn read_governor() -> GovernorProfile {
    read_sysfs(CPU_GOVERNOR)
        .map(|s| GovernorProfile::from_sysfs(&s))
        .unwrap_or(GovernorProfile::Balanced)
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
    /// Summary row icon label (always visible).
    summary_icon: gtk4::Label,
    /// Summary row text label (always visible).
    summary_text: gtk4::Label,
    /// Detail: battery level bar (inside revealer).
    health_lbl: gtk4::Label,
    level_bar: gtk4::LevelBar,
}

impl BatteryHandles {
    fn apply(&self, bat: &BatteryState) {
        // Update summary row.
        self.summary_icon
            .set_label(battery_icon(bat.capacity, bat.charging));
        self.summary_text.set_label(&battery_summary_text(bat));

        // Update detail widgets.
        self.level_bar.set_value(bat.capacity as f64 / 100.0);

        if let Some(health) = bat.health_pct {
            self.health_lbl.set_label(&format!("Health: {}%", health));
            self.health_lbl.set_visible(true);
        } else {
            self.health_lbl.set_visible(false);
        }

        if bat.charging {
            self.level_bar.add_css_class("charging");
        } else {
            self.level_bar.remove_css_class("charging");
        }

        if bat.capacity > 60 {
            self.level_bar.add_css_class("battery-high");
            self.level_bar.remove_css_class("battery-medium");
            self.level_bar.remove_css_class("battery-low");
        } else if bat.capacity >= 20 {
            self.level_bar.add_css_class("battery-medium");
            self.level_bar.remove_css_class("battery-high");
            self.level_bar.remove_css_class("battery-low");
        } else {
            self.level_bar.add_css_class("battery-low");
            self.level_bar.remove_css_class("battery-high");
            self.level_bar.remove_css_class("battery-medium");
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

    // Summary row button + labels (always visible).
    #[allow(dead_code)]
    summary_btn: gtk4::Button,
    summary_icon: gtk4::Label,
    summary_text: gtk4::Label,
    // Stored to allow toggling arrow glyph; mutated only via cloned handle in closure.
    #[allow(dead_code)]
    summary_arrow: gtk4::Label,

    // Governor info label (inside revealer).
    governor_label: gtk4::Label,

    // Revealer for detail content.
    // Stored for ownership; toggled via cloned handle in gesture closure.
    #[allow(dead_code)]
    detail_revealer: gtk4::Revealer,
}

impl PowerSection {
    pub fn new() -> Self {
        // ── Root section box ────────────────────────────────────────────────
        let root = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(6)
            .build();
        root.add_css_class("section");

        // ── Discover battery path ────────────────────────────────────────
        let bat_path = find_battery_path();
        if bat_path.is_none() {
            log::info!("No battery found; battery section hidden.");
        }

        // ── Read initial state ────────────────────────────────────────────
        let state = PowerState::read(bat_path.as_deref());

        // ── Summary row (always visible) ──────────────────────────────────
        let summary_content = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(8)
            .valign(gtk4::Align::Center)
            .build();

        // Determine initial icon and text for the summary.
        let (initial_icon, initial_text) = if let Some(ref bat) = state.battery {
            (
                battery_icon(bat.capacity, bat.charging).to_owned(),
                battery_summary_text(bat),
            )
        } else {
            (
                "󰻠".to_owned(),
                format_governor_info(&state.governor),
            )
        };

        let summary_icon = gtk4::Label::builder()
            .label(&initial_icon)
            .halign(gtk4::Align::Start)
            .build();
        summary_icon.add_css_class("section-summary-icon");

        let summary_text = gtk4::Label::builder()
            .label(&initial_text)
            .halign(gtk4::Align::Start)
            .hexpand(true)
            .xalign(0.0)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .build();
        summary_text.add_css_class("section-summary-label");

        let summary_arrow = gtk4::Label::builder()
            .label("▸")
            .halign(gtk4::Align::End)
            .build();
        summary_arrow.add_css_class("section-expand-arrow");

        summary_content.append(&summary_icon);
        summary_content.append(&summary_text);
        summary_content.append(&summary_arrow);

        let summary_btn = gtk4::Button::builder()
            .child(&summary_content)
            .build();
        summary_btn.add_css_class("section-summary");
        root.append(&summary_btn);

        // ── Detail revealer ───────────────────────────────────────────────
        let detail_revealer = gtk4::Revealer::builder()
            .transition_type(gtk4::RevealerTransitionType::SlideDown)
            .transition_duration(200)
            .reveal_child(false)
            .build();

        let detail_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(8)
            .build();

        // ── Battery detail widgets (conditional) ──────────────────────────
        let bat_handles: Option<Rc<BatteryHandles>> = if let Some(ref bat) = state.battery {
            let bat_detail = gtk4::Box::builder()
                .orientation(gtk4::Orientation::Vertical)
                .spacing(4)
                .build();
            bat_detail.add_css_class("battery-row");

            // Health label
            let health_lbl = gtk4::Label::builder()
                .halign(gtk4::Align::Start)
                .visible(false)
                .build();
            health_lbl.add_css_class("battery-health");
            if let Some(health) = bat.health_pct {
                health_lbl.set_label(&format!("Health: {}%", health));
                health_lbl.set_visible(true);
            }

            // Level bar
            let level_bar = gtk4::LevelBar::builder()
                .min_value(0.0)
                .max_value(1.0)
                .value(bat.capacity as f64 / 100.0)
                .build();
            level_bar.add_css_class("battery-bar");
            if bat.charging {
                level_bar.add_css_class("charging");
            }
            if bat.capacity > 60 {
                level_bar.add_css_class("battery-high");
            } else if bat.capacity >= 20 {
                level_bar.add_css_class("battery-medium");
            } else {
                level_bar.add_css_class("battery-low");
            }

            bat_detail.append(&level_bar);
            bat_detail.append(&health_lbl);
            detail_box.append(&bat_detail);

            Some(Rc::new(BatteryHandles {
                bat_path: bat_path.as_deref().unwrap_or("").to_owned(),
                summary_icon: summary_icon.clone(),
                summary_text: summary_text.clone(),
                health_lbl,
                level_bar,
            }))
        } else {
            None
        };

        // ── CPU governor info (managed by auto-cpufreq) ─────────────────
        let cpu_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(8)
            .build();
        cpu_box.add_css_class("cpu-info-row");

        let cpu_icon = gtk4::Label::builder()
            .label("󰻠")
            .halign(gtk4::Align::Start)
            .build();
        cpu_icon.add_css_class("cpu-info-icon");

        let cpu_text_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(2)
            .build();

        let governor_label = gtk4::Label::builder()
            .label(&format_governor_info(&state.governor))
            .halign(gtk4::Align::Start)
            .build();
        governor_label.add_css_class("cpu-info-governor");

        let cpu_managed = gtk4::Label::builder()
            .label("Managed by auto-cpufreq")
            .halign(gtk4::Align::Start)
            .build();
        cpu_managed.add_css_class("cpu-info-managed");

        cpu_text_box.append(&governor_label);
        cpu_text_box.append(&cpu_managed);
        cpu_box.append(&cpu_icon);
        cpu_box.append(&cpu_text_box);
        detail_box.append(&cpu_box);

        detail_revealer.set_child(Some(&detail_box));
        root.append(&detail_revealer);

        // ── Toggle detail on summary button click ─────────────────────────
        {
            let revealer_c = detail_revealer.clone();
            let arrow_c = summary_arrow.clone();
            summary_btn.connect_clicked(move |_| {
                let expanded = !revealer_c.reveals_child();
                revealer_c.set_reveal_child(expanded);
                arrow_c.set_label(if expanded { "▾" } else { "▸" });
            });
        }

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
            summary_btn,
            state: RefCell::new(state),
            bat_handles,
            summary_icon,
            summary_text,
            summary_arrow,
            governor_label,
            detail_revealer,
        }
    }

    /// Re-read sysfs and update all widgets.
    pub fn refresh(&self) {
        let new_state = PowerState::read(self.bat_path.as_deref());

        if let (Some(bat), Some(handles)) = (&new_state.battery, &self.bat_handles) {
            // BatteryHandles::apply updates summary_icon and summary_text as well.
            handles.apply(bat);
        } else if new_state.battery.is_none() {
            // Desktop without battery: show governor in summary.
            self.summary_icon.set_label("󰻠");
            self.summary_text
                .set_label(&format_governor_info(&new_state.governor));
        }

        self.governor_label
            .set_label(&format_governor_info(&new_state.governor));

        *self.state.borrow_mut() = new_state;
    }

    /// Switch into page mode: reveal detail immediately, hide the summary
    /// toggle row.
    pub fn expand_for_page(&self) {
        self.summary_btn.set_visible(false);
        self.detail_revealer.set_transition_duration(0);
        self.detail_revealer.set_reveal_child(true);
        self.detail_revealer.set_transition_duration(200);
        self.summary_arrow.set_label("▾");
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }

}

// ---------------------------------------------------------------------------
// Standalone helpers
// ---------------------------------------------------------------------------

/// Walk up the widget hierarchy to find the containing `gtk4::Window` and
/// hide it. Used by Lock and Suspend to close the panel before acting.
fn hide_panel_for_widget(widget: &gtk4::Widget) {
    if let Some(root) = widget.root() {
        if let Ok(window) = root.downcast::<gtk4::Window>() {
            window.set_visible(false);
        }
    }
}

fn format_governor_info(gov: &GovernorProfile) -> String {
    match gov {
        GovernorProfile::Performance => "Performance".to_owned(),
        GovernorProfile::Balanced => "Balanced".to_owned(),
        GovernorProfile::Powersave => "Powersave".to_owned(),
        GovernorProfile::Other(s) => s.clone(),
    }
}

// ---------------------------------------------------------------------------
// Session rail — icon-only Lock / Suspend / Logout / Reboot / Shutdown
// ---------------------------------------------------------------------------

/// Build the vertical session-action rail used in the start-menu's left rail.
/// Buttons are icon-only (label via tooltip). Reboot and Shutdown are
/// destructive, so they require a confirming second click within 3 s — shown
/// as a red `.confirming` pulse plus a "Click again…" tooltip, since there is
/// no text label to render a countdown into.
pub fn build_session_rail() -> gtk4::Box {
    let col = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(6)
        .build();
    col.add_css_class("rail-session");

    // Lock — hide the panel first, then lock the session.
    let lock = rail_btn("󰌾", "Lock", false);
    lock.connect_clicked(|b| {
        hide_panel_for_widget(b.upcast_ref());
        spawn_session_cmd("loginctl", &["lock-session"]);
    });
    col.append(&lock);

    // Suspend — hide the panel first, then suspend.
    let suspend = rail_btn("󰤄", "Suspend", false);
    suspend.connect_clicked(|b| {
        hide_panel_for_widget(b.upcast_ref());
        spawn_session_cmd("systemctl", &["suspend"]);
    });
    col.append(&suspend);

    // Logout.
    let logout = rail_btn("󰍃", "Logout", false);
    logout.connect_clicked(|_| spawn_session_cmd("swaymsg", &["exit"]));
    col.append(&logout);

    // Reboot (destructive — confirm).
    let reboot = rail_btn("󰜉", "Reboot", true);
    wire_confirm(&reboot, "Reboot", || {
        spawn_session_cmd("systemctl", &["reboot"])
    });
    col.append(&reboot);

    // Shutdown (destructive — confirm).
    let shutdown = rail_btn("󰐥", "Shutdown", true);
    wire_confirm(&shutdown, "Shutdown", || {
        spawn_session_cmd("systemctl", &["poweroff"])
    });
    col.append(&shutdown);

    col
}

/// One icon-only rail button: a glyph child + tooltip, `.rail-btn` styling
/// (plus `.rail-btn-danger` for destructive actions).
fn rail_btn(icon: &str, tooltip: &str, danger: bool) -> gtk4::Button {
    let lbl = gtk4::Label::new(Some(icon));
    let btn = gtk4::Button::builder().child(&lbl).build();
    btn.add_css_class("rail-btn");
    if danger {
        btn.add_css_class("rail-btn-danger");
    }
    btn.set_tooltip_text(Some(tooltip));
    btn
}

/// Arm a two-click confirmation on `btn`: the first click starts a 3 s window
/// (`.confirming` pulse + "Click again…" tooltip); a second click inside the
/// window runs `exec`. The window auto-clears after 3 s.
fn wire_confirm<F: Fn() + 'static>(btn: &gtk4::Button, verb: &'static str, exec: F) {
    let pending = Rc::new(Cell::new(false));
    btn.connect_clicked(move |b| {
        if pending.get() {
            exec();
            return;
        }
        pending.set(true);
        b.add_css_class("confirming");
        b.set_tooltip_text(Some(&format!("Click again to {}", verb.to_lowercase())));

        let pending_c = pending.clone();
        let b_c = b.clone();
        glib::timeout_add_seconds_local(3, move || {
            pending_c.set(false);
            b_c.remove_css_class("confirming");
            b_c.set_tooltip_text(Some(verb));
            glib::ControlFlow::Break
        });
    });
}

fn spawn_session_cmd(cmd: &str, args: &[&str]) {
    if let Err(e) = std::process::Command::new(cmd).args(args).spawn() {
        log::error!("Failed to spawn {} {:?}: {}", cmd, args, e);
    }
}
