//! Where the sun is, for the two things that follow it: the automatic mode
//! (docs/design-system.md §2.1) and the night light (`services::gamma`).
//! One calculation and one location, so the screen warms and the theme
//! darkens against the same horizon.
//!
//! Both read [`daylight_now`]: how far into the day it is, as a sun
//! elevation in degrees. That is the sun's own elevation where the Day and
//! night settings (`settings::schema::Daylight`) leave it alone, moved in
//! time by their sunrise and sunset offsets, or drawn from their fixed clock
//! times. So the mode's band and the night light's twilight ramp work the
//! same whichever way the day is defined.
//!
//! The NOAA low-precision solar position: good to a few hundredths of a
//! degree for this century, which is far below the 6° band the mode switch
//! uses. No network, no daemon, no ephemeris file.

use crate::settings::schema::Daylight;
use crate::tokens::Mode;

/// Where the sun is computed for: the place picked on the Day and night
/// group's map, or `None` until one is. Never guessed from the time zone:
/// a zone is politics (one clock across China, Spain on Berlin's), and a
/// sunrise follows from where you are.
pub fn location() -> Option<(f64, f64)> {
    crate::settings::store::with(|s| s.daylight()).place()
}

/// How far into the day it is now, as a sun elevation in degrees, or `None`
/// where the sun decides and no location is known.
pub fn daylight_now() -> Option<f64> {
    let d = crate::settings::store::with(|s| s.daylight());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    daylight_at(&d, location(), now, local_minute_now())
}

/// [`daylight_now`] for a given moment: `unix` seconds, which is
/// `minute_of_day` minutes (with the seconds as a fraction) past local
/// midnight.
pub fn daylight_at(
    d: &Daylight,
    location: Option<(f64, f64)>,
    unix: f64,
    minute_of_day: f64,
) -> Option<f64> {
    if d.fixed_times {
        return Some(fixed_elevation(
            f64::from(d.day_from()),
            f64::from(d.night_from()),
            minute_of_day,
        ));
    }
    let (lat, lon) = location?;
    Some(shifted_elevation(
        lat,
        lon,
        unix,
        d.sunrise_offset_m,
        d.sunset_offset_m,
    ))
}

/// Minutes since local midnight, with the seconds as a fraction. Noon on a
/// clock the platform cannot read.
fn local_minute_now() -> f64 {
    glib::DateTime::now_local()
        .map(|t| f64::from(t.hour()) * 60.0 + f64::from(t.minute()) + t.seconds() / 60.0)
        .unwrap_or(12.0 * 60.0)
}

/// The sun's elevation with sunrise moved by `rise_m` minutes and sunset by
/// `set_m` (positive is later): the elevation it had that many minutes ago,
/// taking the sunrise offset while the sun climbs and the sunset offset
/// while it sinks. The whole curve moves, so the mode's band and the night
/// light's twilight keep their length and shift in time.
pub fn shifted_elevation(lat: f64, lon: f64, unix: f64, rise_m: i16, set_m: i16) -> f64 {
    let rising = elevation(lat, lon, unix + 300.0) > elevation(lat, lon, unix - 300.0);
    let offset = if rising { rise_m } else { set_m };
    elevation(lat, lon, unix - f64::from(offset) * 60.0)
}

/// Degrees a fixed-time day and night move per minute around their clock
/// times: 9° in 30 minutes, the span of the night light's twilight ramp, so
/// the screen cools over the half hour before the day starts and warms over
/// the half hour around the night's start.
const FIXED_SLOPE: f64 = 9.0 / 30.0;

/// A sun elevation drawn from fixed clock times, in minutes since midnight:
/// exactly [`BAND`] above the horizon at `day_from`, so the mode turns light
/// on the minute, and exactly `BAND` below at `night_from`, so it turns dark
/// on the minute. Between the two it climbs or sinks at [`FIXED_SLOPE`] and
/// levels off far from either time. Equal times are all day.
pub fn fixed_elevation(day_from: f64, night_from: f64, minute: f64) -> f64 {
    const HIGH: f64 = 45.0;
    if (day_from - night_from).abs() < f64::EPSILON {
        return HIGH;
    }
    let since = |from: f64| (minute - from).rem_euclid(1440.0);
    let until = |to: f64| (to - minute).rem_euclid(1440.0);
    let in_day = since(day_from) < (night_from - day_from).rem_euclid(1440.0);
    let e = if in_day {
        (BAND + FIXED_SLOPE * since(day_from)).min(-BAND + FIXED_SLOPE * until(night_from))
    } else {
        (-BAND - FIXED_SLOPE * since(night_from)).max(BAND - FIXED_SLOPE * until(day_from))
    };
    e.clamp(-HIGH, HIGH)
}

/// The moments today the mode turns light or dark, as minutes since local
/// midnight, in order: for the settings to say what the choices add up to.
/// Empty where the sun decides and no location is known, or where it never
/// crosses (a polar day or night).
pub fn switches_today(d: &Daylight) -> Vec<(u16, Mode)> {
    let minute_now = local_minute_now();
    let unix_now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    let midnight = unix_now - minute_now * 60.0;
    let location = location();
    let at = |m: u16| daylight_at(d, location, midnight + f64::from(m) * 60.0, f64::from(m));
    let Some(first) = at(0) else {
        return Vec::new();
    };
    let mut shown = if first > 0.0 { Mode::Light } else { Mode::Dark };
    let mut out = Vec::new();
    for m in 1..1440 {
        let Some(e) = at(m) else { break };
        let next = mode(e, shown);
        if next != shown {
            out.push((m, next));
            shown = next;
        }
    }
    out
}

/// The sun's elevation above the horizon in degrees, at `lat`/`lon`
/// (degrees, east positive) and `unix` seconds.
pub fn elevation(lat: f64, lon: f64, unix: f64) -> f64 {
    let n = unix / 86_400.0 + 2_440_587.5 - 2_451_545.0; // days since J2000.0
    let l = (280.460 + 0.985_647_4 * n).rem_euclid(360.0);
    let g = (357.528 + 0.985_600_3 * n).rem_euclid(360.0).to_radians();
    let lambda = (l + 1.915 * g.sin() + 0.020 * (2.0 * g).sin()).to_radians();
    let eps = (23.439 - 0.000_000_4 * n).to_radians();
    let ra = (eps.cos() * lambda.sin()).atan2(lambda.cos());
    let dec = (eps.sin() * lambda.sin()).asin();
    let gmst_h = (18.697_374_558 + 24.065_709_824_419_08 * n).rem_euclid(24.0);
    let ha = (gmst_h * 15.0 + lon).to_radians() - ra;
    let lat = lat.to_radians();
    (lat.sin() * dec.sin() + lat.cos() * dec.cos() * ha.cos())
        .asin()
        .to_degrees()
}

/// Light once the sun is this far above the horizon, dark once it is this
/// far below; in between, whatever the mode already was. The band keeps a
/// switch from flickering at dawn and keeps light mode out of the dimmest
/// civil twilight.
pub const BAND: f64 = 3.0;

/// The mode the sun asks for, given the one showing now.
pub fn mode(elevation: f64, now: Mode) -> Mode {
    if elevation >= BAND {
        Mode::Light
    } else if elevation <= -BAND {
        Mode::Dark
    } else {
        now
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Malmö, the night light's location.
    const LAT: f64 = 55.6;
    const LON: f64 = 13.0;

    fn unix(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> f64 {
        // Days from the civil date, no chrono needed (Howard Hinnant's).
        let y = if mo <= 2 { y - 1 } else { y } as i64;
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let mp = (mo as i64 + 9) % 12;
        let doy = (153 * mp + 2) / 5 + d as i64 - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        let days = era * 146_097 + doe - 719_468;
        (days * 86_400 + h as i64 * 3_600 + mi as i64 * 60) as f64
    }

    #[test]
    fn midsummer_noon_is_high_and_midnight_below() {
        // Solar noon in Malmö is about 11:07 UTC; the sun is then
        // 90 - 55.6 + 23.44 = 57.8° up.
        let noon = elevation(LAT, LON, unix(2026, 6, 21, 11, 7));
        assert!((noon - 57.8).abs() < 0.5, "{noon}");
        let midnight = elevation(LAT, LON, unix(2026, 6, 21, 23, 7));
        assert!((midnight - -11.0).abs() < 0.8, "{midnight}");
    }

    #[test]
    fn midwinter_noon_is_low() {
        // 90 - 55.6 - 23.44 = 11.0°.
        let noon = elevation(LAT, LON, unix(2026, 12, 21, 11, 10));
        assert!((noon - 11.0).abs() < 0.5, "{noon}");
    }

    #[test]
    fn the_equinox_sun_crosses_the_horizon_at_about_six_solar() {
        // At the equinox the sun rises at about 06:00 local solar time,
        // 05:08 UTC in Malmö; refraction is not modelled, so geometric
        // rise is within a few minutes of that.
        let before = elevation(LAT, LON, unix(2026, 3, 20, 4, 50));
        let after = elevation(LAT, LON, unix(2026, 3, 20, 5, 30));
        assert!(before < 0.0 && after > 0.0, "{before} {after}");
    }

    #[test]
    fn the_band_holds_the_mode_in_twilight() {
        assert_eq!(mode(10.0, Mode::Dark), Mode::Light);
        assert_eq!(mode(-10.0, Mode::Light), Mode::Dark);
        assert_eq!(mode(1.0, Mode::Dark), Mode::Dark);
        assert_eq!(mode(1.0, Mode::Light), Mode::Light);
        assert_eq!(mode(-2.9, Mode::Light), Mode::Light);
    }

    /// Fixed times switch on the minute: light exactly at the day's start,
    /// dark exactly at the night's, and the night light's ramp (−6° to +3°)
    /// spans the half hour before the day and the half hour around the night.
    #[test]
    fn fixed_times_switch_on_the_minute() {
        let (day, night) = (7.0 * 60.0, 21.0 * 60.0);
        let at = |h: f64, m: f64| fixed_elevation(day, night, h * 60.0 + m);
        assert!((at(7.0, 0.0) - BAND).abs() < 1e-9);
        assert!((at(21.0, 0.0) + BAND).abs() < 1e-9);
        assert_eq!(mode(at(6.0, 59.0), Mode::Dark), Mode::Dark);
        assert_eq!(mode(at(7.0, 0.0), Mode::Dark), Mode::Light);
        assert_eq!(mode(at(20.0, 59.0), Mode::Light), Mode::Light);
        assert_eq!(mode(at(21.0, 0.0), Mode::Light), Mode::Dark);
        assert!((at(6.0, 30.0) + 6.0).abs() < 1e-9, "{}", at(6.0, 30.0));
        assert!(at(12.0, 0.0) > 30.0 && at(2.0, 0.0) < -30.0);
        // A night that wraps midnight the other way round.
        let late = fixed_elevation(22.0 * 60.0, 5.0 * 60.0, 23.0 * 60.0);
        assert!(late > BAND, "{late}");
        // Continuous: no jump anywhere in the day.
        for m in 0..1440 {
            let (a, b) = (
                fixed_elevation(day, night, f64::from(m)),
                fixed_elevation(day, night, f64::from(m) + 1.0),
            );
            assert!((a - b).abs() <= FIXED_SLOPE + 1e-9, "{m}: {a} -> {b}");
        }
    }

    /// An offset moves the sun's crossing by that many minutes, and only the
    /// one it names: a sunset 45 minutes later leaves the morning alone.
    #[test]
    fn offsets_move_the_crossing_by_their_minutes() {
        let day = unix(2026, 9, 28, 0, 0);
        let crossing = |rise: i16, set: i16, evening: bool| {
            let range = if evening {
                12 * 60..24 * 60
            } else {
                0..12 * 60
            };
            range
                .map(|m| day + f64::from(m) * 60.0)
                .find(|&t| {
                    let e = shifted_elevation(LAT, LON, t, rise, set);
                    if evening { e <= 0.0 } else { e >= 0.0 }
                })
                .unwrap()
        };
        let (dusk, dusk_late) = (crossing(0, 0, true), crossing(0, 45, true));
        assert!(
            ((dusk_late - dusk) / 60.0 - 45.0).abs() <= 1.0,
            "{}",
            (dusk_late - dusk) / 60.0
        );
        let (dawn, dawn_same) = (crossing(0, 0, false), crossing(0, 45, false));
        assert_eq!(dawn, dawn_same);
        let dawn_early = crossing(-30, 0, false);
        assert!(((dawn - dawn_early) / 60.0 - 30.0).abs() <= 1.0);
    }
}
