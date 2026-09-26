//! The battery and the CPU governor, read from sysfs: the state behind the
//! panel's power section and the bar's battery pill and decision slot, and
//! the words both of them say about it. No widgets here.

use std::fs;

// ---------------------------------------------------------------------------
// Sysfs helpers
// ---------------------------------------------------------------------------

const CPU_GOVERNOR: &str = "/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor";

fn read_sysfs(path: &str) -> Option<String> {
    match fs::read_to_string(path) {
        Ok(s) => {
            let trimmed = s.trim().to_owned();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
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
pub(crate) fn find_battery_path() -> Option<String> {
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

/// What the charger is doing, straight from `/sys/class/power_supply/BAT*/status`.
///
/// The distinction that matters is `Idle`: a ThinkPad holding at its
/// `charge_control_end_threshold` reports "Not charging" while plugged in.
/// Collapsing that into a charging bool made it indistinguishable from
/// running on battery, so the bar showed a discharge icon and a time-to-empty
/// for a machine sitting on mains.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChargeState {
    Charging,
    Discharging,
    /// Plugged in, battery at 100 %.
    Full,
    /// Plugged in, not drawing — typically holding at a charge threshold.
    Idle,
    Unknown,
}

impl ChargeState {
    fn parse(status: &str) -> Self {
        if status.eq_ignore_ascii_case("Charging") {
            Self::Charging
        } else if status.eq_ignore_ascii_case("Discharging") {
            Self::Discharging
        } else if status.eq_ignore_ascii_case("Full") {
            Self::Full
        } else if status.eq_ignore_ascii_case("Not charging") {
            Self::Idle
        } else {
            Self::Unknown
        }
    }

    /// On mains, whether or not current is flowing.
    pub(crate) fn plugged(self) -> bool {
        matches!(self, Self::Charging | Self::Full | Self::Idle)
    }
}

/// At or above this, the charge is close enough to done that a countdown is
/// noise rather than information.
pub(crate) const ALMOST_FULL_PCT: u8 = 95;

#[derive(Debug, Clone)]
pub(crate) struct BatteryState {
    /// 0–100
    pub(crate) capacity: u8,
    pub(crate) charging: bool,
    pub(crate) state: ChargeState,
    /// Watts (power_now / 1_000_000)
    power_w: Option<f64>,
    /// Wh remaining
    energy_now_wh: Option<f64>,
    /// Wh at full
    energy_full_wh: Option<f64>,
    /// Battery health as percentage of design capacity (energy_full / energy_full_design * 100)
    pub(crate) health_pct: Option<u8>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum GovernorProfile {
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

pub(crate) fn read_battery(bat_path: &str) -> Option<BatteryState> {
    let capacity: u8 = read_sysfs(&format!("{}/capacity", bat_path))
        .and_then(|s| s.parse().ok())
        .or_else(|| {
            log::error!(
                "Battery info unavailable: cannot read {}/capacity",
                bat_path
            );
            None
        })?;

    let status = read_sysfs(&format!("{}/status", bat_path)).unwrap_or_else(|| {
        log::warn!("Cannot read {}/status, assuming Discharging", bat_path);
        "Discharging".to_owned()
    });

    let state = ChargeState::parse(&status);
    // Kept as "the bolt should show": strictly current flowing in, so a
    // full-but-plugged battery no longer claims to be charging.
    let charging = state == ChargeState::Charging;

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
        state,
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
pub(crate) fn battery_icon(capacity: u8, charging: bool) -> &'static str {
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
    let estimate = |wh: Option<f64>| -> Option<String> {
        let power = bat.power_w.filter(|w| *w >= 0.001)?;
        format_hours(wh? / power)
    };

    match bat.state {
        ChargeState::Charging => {
            let to_full = match (bat.energy_full_wh, bat.energy_now_wh) {
                (Some(full), Some(now)) => Some((full - now).max(0.0)),
                _ => None,
            };
            match estimate(to_full) {
                Some(t) => format!("Charging — {} to full", t),
                None => "Charging".to_owned(),
            }
        }
        ChargeState::Full => "Fully charged".to_owned(),
        // Plugged but idle: a ThinkPad holding at its charge threshold. Saying
        // "On battery" here was the old bug; there is no direction to report.
        ChargeState::Idle => "Plugged in".to_owned(),
        ChargeState::Discharging | ChargeState::Unknown => match estimate(bat.energy_now_wh) {
            Some(t) => format!("On battery — {} remaining", t),
            None => "On battery".to_owned(),
        },
    }
}

/// Time estimate for the bar's battery segment: time-to-full while charging,
/// time-to-empty while discharging.
///
/// `None` once the charge is effectively done — plugged and not actively
/// charging, or at/above [`ALMOST_FULL_PCT`] — because a countdown there says
/// nothing the icon has not already said. Also `None` when the meter has not
/// settled (`power_now == 0`), rather than showing a fabricated number.
pub(crate) fn eta_text(bat: &BatteryState) -> Option<String> {
    if bat.capacity >= ALMOST_FULL_PCT {
        return None;
    }
    let power = bat.power_w.filter(|w| *w >= 0.001)?;
    match bat.state {
        ChargeState::Charging => {
            let to_full = (bat.energy_full_wh? - bat.energy_now_wh?).max(0.0);
            format_hours(to_full / power)
        }
        ChargeState::Discharging | ChargeState::Unknown => format_hours(bat.energy_now_wh? / power),
        ChargeState::Full | ChargeState::Idle => None,
    }
}

/// Time-to-empty ("1h 5m") while discharging; `None` when charging or
/// when the estimate is unreliable (missing sysfs fields, unsettled
/// meter). Feeds the bar decision slot's battery-critical occupant.
pub(crate) fn time_to_empty_text(bat: &BatteryState) -> Option<String> {
    if bat.charging {
        return None;
    }
    match (bat.power_w, bat.energy_now_wh) {
        (Some(power), Some(energy)) if power >= 0.001 => format_hours(energy / power),
        _ => None,
    }
}

/// Build the summary text for the summary row from battery state.
/// Format: "85% · Charging — 30m to full" or "85% · 3h 20m remaining"
/// Also the bar battery pill's tooltip (src/bar/battery.rs).
pub(crate) fn battery_summary_text(bat: &BatteryState) -> String {
    format!("{}% · {}", bat.capacity, battery_sub_text(bat))
}

/// Present draw ("7.2 W") for the bar battery popover; `None` when sysfs
/// gives no settled power_now reading.
pub(crate) fn watts_text(bat: &BatteryState) -> Option<String> {
    bat.power_w
        .filter(|w| *w >= 0.001)
        .map(|w| format!("{w:.1} W"))
}

// ---------------------------------------------------------------------------
// Governor helpers
// ---------------------------------------------------------------------------

pub(crate) fn read_governor() -> GovernorProfile {
    read_sysfs(CPU_GOVERNOR)
        .map(|s| GovernorProfile::from_sysfs(&s))
        .unwrap_or(GovernorProfile::Balanced)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bat(power_w: Option<f64>) -> BatteryState {
        BatteryState {
            capacity: 50,
            charging: false,
            state: ChargeState::Discharging,
            power_w,
            energy_now_wh: None,
            energy_full_wh: None,
            health_pct: None,
        }
    }

    fn charged(state: ChargeState, capacity: u8, now: f64, full: f64) -> BatteryState {
        BatteryState {
            capacity,
            charging: state == ChargeState::Charging,
            state,
            power_w: Some(10.0),
            energy_now_wh: Some(now),
            energy_full_wh: Some(full),
            health_pct: None,
        }
    }

    #[test]
    fn eta_counts_down_while_discharging_and_up_while_charging() {
        let d = charged(ChargeState::Discharging, 50, 50.0, 100.0);
        assert_eq!(eta_text(&d).as_deref(), Some("5h 0m"));
        let c = charged(ChargeState::Charging, 50, 50.0, 100.0);
        assert_eq!(eta_text(&c).as_deref(), Some("5h 0m"));
    }

    #[test]
    fn eta_goes_quiet_near_full_and_on_idle_mains() {
        // At/above the almost-full mark, no countdown in either direction.
        let c = charged(ChargeState::Charging, ALMOST_FULL_PCT, 95.0, 100.0);
        assert_eq!(eta_text(&c), None);
        // Plugged and holding at a threshold: there is no direction to report.
        let idle = charged(ChargeState::Idle, 60, 60.0, 100.0);
        assert_eq!(eta_text(&idle), None);
        let full = charged(ChargeState::Full, 60, 60.0, 100.0);
        assert_eq!(eta_text(&full), None);
    }

    #[test]
    fn idle_mains_is_not_reported_as_running_on_battery() {
        let idle = charged(ChargeState::Idle, 95, 95.0, 100.0);
        assert_eq!(battery_sub_text(&idle), "Plugged in");
        assert!(ChargeState::Idle.plugged());
        assert!(!ChargeState::Discharging.plugged());
    }

    #[test]
    fn watts_text_skips_unsettled_meters() {
        assert_eq!(watts_text(&bat(Some(7.24))), Some("7.2 W".into()));
        // power_now == 0 — meter hasn't settled yet.
        assert_eq!(watts_text(&bat(Some(0.0))), None);
        assert_eq!(watts_text(&bat(None)), None);
    }
}
