//! Where the sun is, for the two things that follow it: the automatic mode
//! (docs/design-system.md §2.1) and the night light (`services::gamma`).
//! One calculation and one location, so the screen warms and the theme
//! darkens against the same horizon.
//!
//! The NOAA low-precision solar position: good to a few hundredths of a
//! degree for this century, which is far below the 6° band the mode switch
//! uses. No network, no daemon, no ephemeris file.

use crate::tokens::Mode;

/// Where the sun is computed for: `/etc/swaypplet/theme.json`
/// (`{"latitude": …, "longitude": …}`, written by Nix from
/// `theme/location.nix`), or `SWAYPPLET_THEME_CONFIG`.
pub fn location() -> Option<(f64, f64)> {
    let path = std::env::var("SWAYPPLET_THEME_CONFIG")
        .unwrap_or_else(|_| "/etc/swaypplet/theme.json".to_string());
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    Some((v["latitude"].as_f64()?, v["longitude"].as_f64()?))
}

/// The sun's elevation here and now, or `None` where no location is known.
pub fn elevation_now() -> Option<f64> {
    let (lat, lon) = location()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    Some(elevation(lat, lon, now))
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
}
