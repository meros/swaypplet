//! The battery and the power profile: the state behind the panel's power
//! section and the bar's battery pill and decision slot, and the words both
//! of them say about it. No widgets here.
//!
//! The battery is read from sysfs, and `services::battery` decides *when*:
//! on UPower's change signals, never on a timer. UPower's own time
//! estimates, which it smooths over minutes, replace the instantaneous
//! `energy / power_now` division when it has them.
//!
//! The profile is power-profiles-daemon's where that runs (settable), and
//! otherwise what the firmware and the CPU say (read-only): on this host
//! auto-cpufreq owns both, and power-profiles-daemon is off because the two
//! fight.

use std::fs;

// ---------------------------------------------------------------------------
// Sysfs helpers
// ---------------------------------------------------------------------------

const CPU_GOVERNOR: &str = "/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor";
const CPU_EPP: &str = "/sys/devices/system/cpu/cpu0/cpufreq/energy_performance_preference";
const PLATFORM_PROFILE: &str = "/sys/firmware/acpi/platform_profile";

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
    /// Charge cycles, when the firmware counts them.
    pub(crate) cycles: Option<u32>,
    /// The firmware's charge window: starts charging below `start`, stops
    /// at `end`. Root-only to change (sysfs), so shown, never set, here.
    pub(crate) charge_start: Option<u8>,
    pub(crate) charge_end: Option<u8>,
    /// UPower's smoothed estimates, seconds; `None` when it has none yet.
    pub(crate) upower_to_empty_s: Option<u64>,
    pub(crate) upower_to_full_s: Option<u64>,
    /// Whether a charger is connected: any supply that is not a battery
    /// reports online. `None` when the machine lists no such supply.
    pub(crate) on_mains: Option<bool>,
}

/// The power profile and who owns it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Profile {
    /// power-profiles-daemon: `power-saver`, `balanced` or `performance`,
    /// settable. `degraded` is its reason when performance is held back
    /// (`lap-detected`, `high-operating-temperature`); `holds` are the
    /// applications holding a profile.
    Daemon {
        active: String,
        choices: Vec<String>,
        degraded: Option<String>,
        holds: Vec<String>,
    },
    /// No daemon: what the firmware and the CPU are set to, by whoever owns
    /// them. Read-only.
    Firmware {
        platform: Option<String>,
        epp: Option<String>,
        governor: Option<String>,
        owner: Option<&'static str>,
    },
}

impl Profile {
    /// The one word for it: the daemon's profile, else the firmware's.
    pub(crate) fn name(&self) -> String {
        let pretty = |p: &str| match p {
            "power-saver" | "low-power" | "quiet" => "Power saver".to_string(),
            "balanced" => "Balanced".to_string(),
            "performance" => "Performance".to_string(),
            other => other.to_string(),
        };
        match self {
            Profile::Daemon { active, .. } => pretty(active),
            Profile::Firmware { platform, epp, governor, .. } => platform
                .as_deref()
                .or(epp.as_deref())
                .or(governor.as_deref())
                .map_or_else(|| "Unknown".to_string(), pretty),
        }
    }

    /// The line under it: why it is what it is.
    pub(crate) fn detail(&self) -> String {
        match self {
            Profile::Daemon { degraded: Some(why), .. } => {
                format!("Performance held back: {}", why.replace('-', " "))
            }
            Profile::Daemon { holds, .. } if !holds.is_empty() => {
                format!("Held by {}", holds.join(", "))
            }
            Profile::Daemon { .. } => "power-profiles-daemon".to_string(),
            Profile::Firmware { epp, governor, owner, .. } => {
                // Who owns it first: that is why there is no switch here.
                let mut parts = vec![match owner {
                    Some(o) => format!("Managed by {o}"),
                    None => "Read only".to_string(),
                }];
                match (epp, governor) {
                    (Some(e), _) => parts.push(format!("CPU {}", e.replace('_', " "))),
                    (None, Some(g)) => parts.push(format!("governor {g}")),
                    (None, None) => {}
                }
                parts.join(" · ")
            }
        }
    }
}

impl BatteryState {
    /// Running on the battery: no charger connected. The charge state alone
    /// cannot say this: a ThinkPad at its charge threshold reports "Not
    /// charging" and, around it, "Discharging" for a moment while plugged in
    /// (seen 2026-10-02 at 99 %, UPower state flipping with OnBattery false).
    /// The charge state is the fallback only where no charger is listed.
    pub(crate) fn on_battery(&self) -> bool {
        match self.on_mains {
            Some(mains) => !mains,
            None => self.state == ChargeState::Discharging,
        }
    }

    /// A made-up battery for the render harness's fixtures.
    pub(crate) fn fixture(
        capacity: u8,
        state: ChargeState,
        watts: f64,
        to_empty_s: Option<u64>,
        to_full_s: Option<u64>,
    ) -> BatteryState {
        BatteryState {
            capacity,
            charging: state == ChargeState::Charging,
            state,
            power_w: Some(watts),
            energy_now_wh: Some(f64::from(capacity) * 0.7),
            energy_full_wh: Some(70.0),
            health_pct: Some(94),
            cycles: Some(168),
            charge_start: Some(75),
            charge_end: Some(80),
            upower_to_empty_s: to_empty_s,
            upower_to_full_s: to_full_s,
            on_mains: None,
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

    let pct = |f: &str| read_sysfs(&format!("{bat_path}/{f}")).and_then(|s| s.parse::<u8>().ok());
    Some(BatteryState {
        capacity,
        charging,
        state,
        power_w,
        energy_now_wh,
        energy_full_wh,
        health_pct,
        cycles: read_sysfs(&format!("{bat_path}/cycle_count"))
            .and_then(|s| s.parse().ok())
            .filter(|c| *c > 0),
        charge_start: pct("charge_control_start_threshold"),
        charge_end: pct("charge_control_end_threshold"),
        upower_to_empty_s: None,
        upower_to_full_s: None,
        on_mains: mains_online(),
    })
}

/// Whether any supply that is not a battery reports online=1 (the AC
/// adapter, a USB-C source); `None` when the machine lists none.
fn mains_online() -> Option<bool> {
    let entries = std::fs::read_dir("/sys/class/power_supply").ok()?;
    let mut seen = false;
    for e in entries.flatten() {
        let read = |f: &str| std::fs::read_to_string(e.path().join(f)).unwrap_or_default();
        if read("type").trim() == "Battery" {
            continue;
        }
        seen = true;
        if read("online").trim() == "1" {
            return Some(true);
        }
    }
    seen.then_some(false)
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
        let upower = match bat.state {
            ChargeState::Charging => bat.upower_to_full_s,
            _ => bat.upower_to_empty_s,
        };
        if let Some(s) = upower {
            return format_hours(s as f64 / 3600.0);
        }
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
    match (bat.state, bat.upower_to_full_s, bat.upower_to_empty_s) {
        (ChargeState::Charging, Some(s), _) => return format_hours(s as f64 / 3600.0),
        (ChargeState::Discharging | ChargeState::Unknown, _, Some(s)) => {
            return format_hours(s as f64 / 3600.0);
        }
        _ => {}
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
    if let Some(s) = bat.upower_to_empty_s {
        return format_hours(s as f64 / 3600.0);
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

/// The firmware's charge window as words ("Charges to 80 %, from 75 %"),
/// `None` when it charges to full or does not say.
pub(crate) fn charge_limit_text(bat: &BatteryState) -> Option<String> {
    let end = bat.charge_end.filter(|e| *e < 100)?;
    Some(match bat.charge_start.filter(|s| *s > 0 && *s < end) {
        Some(start) => format!("Charges to {end} %, from {start} %"),
        None => format!("Charges to {end} %"),
    })
}

/// Health and cycles as one line ("Health 94 % · 168 cycles").
pub(crate) fn health_text(bat: &BatteryState) -> Option<String> {
    match (bat.health_pct, bat.cycles) {
        (Some(h), Some(c)) => Some(format!("Health {h} % · {c} cycles")),
        (Some(h), None) => Some(format!("Health {h} %")),
        (None, Some(c)) => Some(format!("{c} cycles")),
        (None, None) => None,
    }
}

/// Present draw ("7.2 W") for the bar battery popover; `None` when sysfs
/// gives no settled power_now reading.
pub(crate) fn watts_text(bat: &BatteryState) -> Option<String> {
    bat.power_w
        .filter(|w| *w >= 0.001)
        .map(|w| format!("{w:.1} W"))
}

// ---------------------------------------------------------------------------
// The power profile
// ---------------------------------------------------------------------------

/// The profile, from power-profiles-daemon when it is on the bus, else from
/// sysfs. Blocking (one D-Bus round trip at most); call from a worker. Read
/// when the section opens and after a change, never on a timer.
pub(crate) fn read_profile() -> Profile {
    if let Some(p) = read_daemon_profile() {
        return p;
    }
    let quiet = |p: &str| std::fs::read_to_string(p).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    Profile::Firmware {
        platform: quiet(PLATFORM_PROFILE),
        epp: quiet(CPU_EPP),
        governor: quiet(CPU_GOVERNOR),
        owner: std::path::Path::new("/run/current-system/sw/bin/auto-cpufreq")
            .exists()
            .then_some("auto-cpufreq"),
    }
}

/// power-profiles-daemon's names, newest first: it moved under UPower in
/// 0.20 and keeps the old name as an alias.
const PPD: [(&str, &str, &str); 2] = [
    (
        "org.freedesktop.UPower.PowerProfiles",
        "/org/freedesktop/UPower/PowerProfiles",
        "org.freedesktop.UPower.PowerProfiles",
    ),
    ("net.hadess.PowerProfiles", "/net/hadess/PowerProfiles", "net.hadess.PowerProfiles"),
];

fn read_daemon_profile() -> Option<Profile> {
    use std::collections::HashMap;
    use zbus::zvariant::OwnedValue;
    let conn = zbus::blocking::Connection::system().ok()?;
    let dbus = zbus::blocking::fdo::DBusProxy::new(&conn).ok()?;
    for (name, path, iface) in PPD {
        let bus_name = zbus::names::BusName::try_from(name).ok()?;
        if !dbus.name_has_owner(bus_name).unwrap_or(false) {
            continue;
        }
        let proxy = zbus::blocking::Proxy::new(&conn, name, path, iface).ok()?;
        let active: String = proxy.get_property("ActiveProfile").ok()?;
        let profiles: Vec<HashMap<String, OwnedValue>> =
            proxy.get_property("Profiles").unwrap_or_default();
        let choices = profiles
            .iter()
            .filter_map(|p| p.get("Profile").and_then(|v| v.downcast_ref::<&str>().ok().map(str::to_string)))
            .collect();
        let degraded: String = proxy.get_property("PerformanceDegraded").unwrap_or_default();
        let holds: Vec<HashMap<String, OwnedValue>> =
            proxy.get_property("ActiveProfileHolds").unwrap_or_default();
        let holds = holds
            .iter()
            .filter_map(|h| h.get("ApplicationId").and_then(|v| v.downcast_ref::<&str>().ok().map(str::to_string)))
            .collect();
        return Some(Profile::Daemon {
            active,
            choices,
            degraded: (!degraded.is_empty()).then_some(degraded),
            holds,
        });
    }
    None
}

/// Ask power-profiles-daemon for `profile`. Blocking; a worker only. Only
/// reachable from a control that exists when [`Profile::settable`].
pub(crate) fn set_profile(profile: &str) -> bool {
    let Ok(conn) = zbus::blocking::Connection::system() else {
        return false;
    };
    for (name, path, iface) in PPD {
        let Ok(proxy) = zbus::blocking::Proxy::new(&conn, name, path, iface) else {
            continue;
        };
        if proxy.set_property("ActiveProfile", profile).is_ok() {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    /// A charger that is connected wins over a battery that says
    /// "Discharging" for a moment at its charge threshold; with no charger
    /// listed, the charge state decides.
    #[test]
    fn on_battery_follows_the_charger_not_the_charge_state() {
        let mut b = charged(ChargeState::Discharging, 99, 69.0, 70.0);
        b.on_mains = Some(true);
        assert!(!b.on_battery());
        b.on_mains = Some(false);
        assert!(b.on_battery());
        b.on_mains = None;
        assert!(b.on_battery());
        b.state = ChargeState::Idle;
        assert!(!b.on_battery());
    }

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
            cycles: None,
            charge_start: None,
            charge_end: None,
            upower_to_empty_s: None,
            upower_to_full_s: None,
            on_mains: None,
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
            cycles: None,
            charge_start: None,
            charge_end: None,
            upower_to_empty_s: None,
            upower_to_full_s: None,
            on_mains: None,
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

    #[test]
    fn upower_estimates_win_over_the_instant_division() {
        let mut d = charged(ChargeState::Discharging, 50, 50.0, 100.0);
        d.upower_to_empty_s = Some(2 * 3600 + 30 * 60);
        assert_eq!(eta_text(&d).as_deref(), Some("2h 30m"));
        assert_eq!(time_to_empty_text(&d).as_deref(), Some("2h 30m"));
    }

    #[test]
    fn the_charge_window_is_said_only_when_it_limits() {
        let mut b = bat(None);
        b.charge_end = Some(100);
        assert_eq!(charge_limit_text(&b), None);
        b.charge_end = Some(80);
        b.charge_start = Some(75);
        assert_eq!(charge_limit_text(&b).as_deref(), Some("Charges to 80 %, from 75 %"));
        b.charge_start = Some(0);
        assert_eq!(charge_limit_text(&b).as_deref(), Some("Charges to 80 %"));
    }

    #[test]
    fn a_profile_says_what_it_is_and_why() {
        let fw = Profile::Firmware {
            platform: Some("low-power".into()),
            epp: Some("balance_power".into()),
            governor: Some("powersave".into()),
            owner: Some("auto-cpufreq"),
        };
        assert_eq!(fw.name(), "Power saver");
        assert_eq!(fw.detail(), "Managed by auto-cpufreq · CPU balance power");
        let held = Profile::Daemon {
            active: "performance".into(),
            choices: vec![],
            degraded: Some("lap-detected".into()),
            holds: vec![],
        };
        assert_eq!(held.name(), "Performance");
        assert_eq!(held.detail(), "Performance held back: lap detected");
    }
}
