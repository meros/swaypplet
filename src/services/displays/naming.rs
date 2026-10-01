//! Names a person reads for a display, and the key a name they gave it is
//! stored under (`services::devices::DeviceKey::Display`).
//!
//! The connector ("eDP-1", "DP-3") names a port, not a screen: the same
//! monitor is DP-3 on one dock port and DP-5 on the next. The automatic
//! name is what the screen says about itself, and the connector moves to
//! the subtitle.

use crate::services::devices::short_vendor;

/// A panel wired inside the machine: eDP, LVDS or DSI.
pub fn is_builtin(connector: &str) -> bool {
    ["eDP", "LVDS", "DSI"]
        .iter()
        .any(|p| connector.starts_with(p))
}

/// "Built-in display" for a laptop panel; otherwise the make, without its
/// legal form, and the model ("Samsung Display ATNA40HQ02-0", "NON
/// 28H2U"). The connector when the screen says nothing.
pub fn auto_name(connector: &str, make: &str, model: &str) -> String {
    if is_builtin(connector) {
        return "Built-in display".into();
    }
    let make = known(make).map(short_vendor);
    let model = known(model).map(str::to_string);
    let parts: Vec<String> = make.into_iter().chain(model).collect();
    if parts.is_empty() {
        connector.to_string()
    } else {
        parts.join(" ")
    }
}

/// What a stored name follows: make, model and serial, so a monitor keeps
/// its name on any port. Monitors without a serial (sway's "Unknown", or a
/// run of zeros) share it by make and model; a built-in panel is the one on
/// its connector.
pub fn key(connector: &str, make: &str, model: &str, serial: &str) -> String {
    if is_builtin(connector) {
        return format!("builtin|{connector}");
    }
    let field = |s: &str| known(s).unwrap_or_default().to_string();
    match known(serial).filter(|s| !s.chars().all(|c| c == '0')) {
        Some(serial) => format!("{}|{}|{serial}", field(make), field(model)),
        None => format!("{}|{}", field(make), field(model)),
    }
}

/// A field with its padding off, absent when it says nothing.
fn known(s: &str) -> Option<&str> {
    let s = s.trim();
    (!s.is_empty() && s != "Unknown").then_some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_laptop_panel_is_the_built_in_display() {
        assert!(is_builtin("eDP-1"));
        assert!(is_builtin("LVDS-1"));
        assert!(is_builtin("DSI-1"));
        assert!(!is_builtin("DP-3"));
        assert!(!is_builtin("HDMI-A-1"));
        assert_eq!(
            auto_name("eDP-1", "Samsung Display Corp.", "ATNA40HQ02-0 "),
            "Built-in display"
        );
    }

    #[test]
    fn an_external_screen_is_its_make_and_model() {
        assert_eq!(auto_name("DP-3", "NON", "28H2U"), "NON 28H2U");
        assert_eq!(
            auto_name("DP-1", "Dell Inc.", "DELL U2723QE"),
            "Dell DELL U2723QE"
        );
        assert_eq!(auto_name("HDMI-A-1", "Unknown", "Unknown"), "HDMI-A-1");
        assert_eq!(auto_name("DP-2", "  ", "LG HDR 4K "), "LG HDR 4K");
    }

    #[test]
    fn the_key_follows_the_screen_not_the_port() {
        assert_eq!(
            key("DP-3", "Dell Inc.", "U2723QE", "ABC123"),
            key("DP-5", "Dell Inc.", "U2723QE", "ABC123")
        );
        assert_eq!(key("DP-3", "NON", "28H2U", "0000000000000"), "NON|28H2U");
        assert_eq!(key("DP-3", "NON", "28H2U", "Unknown"), "NON|28H2U");
        assert_eq!(
            key("eDP-1", "Samsung Display Corp.", "ATNA40HQ02-0 ", "Unknown"),
            "builtin|eDP-1"
        );
    }
}
