//! What may show on an extra screen: a virtual output made for a TV or a
//! VNC viewer (`services::cast`, `displays::naming::is_virtual`).
//!
//! An extra screen is somebody else's view of this desktop, often a TV in a
//! room full of people, so by default nothing of the shell's goes there: no
//! bar, no workspace names, no volume card, no key sheet, no pins, no
//! notifications. Each has a switch in the `extra_screens` settings section
//! (the Displays pane's "On extra screens" group), off by default.
//!
//! Two kinds of surface, two rules:
//!
//! - **One per output** (the bar, the workspace names, the OSD card, the key
//!   sheet): [`shows`] says whether an output gets one, and `PerMonitor`
//!   neither builds nor keeps one where it says no. A switch flipped, or an
//!   extra screen made or removed, reconciles the set.
//! - **Placed on the focused output** (pins, notification popups):
//!   [`place`] keeps the focused output, unless it is an extra screen that
//!   may not have them; then a screen of the person's own.
//!
//! A session where every output is virtual (the nested harness,
//! `dev/render.sh`) has no screen to prefer, and treats its outputs as
//! screens; the same rule `displays::matcher::screens` uses for profiles.

use crate::services::displays::naming::{is_builtin, is_virtual};
use crate::settings::store::{self, ExtraScreens};

/// A surface with an extra-screen switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Bar,
    WorkspaceNames,
    Osd,
    KeySheet,
    Pins,
    Notifications,
}

impl Kind {
    /// The switch for this surface in `s`.
    pub fn allowed_in(self, s: &ExtraScreens) -> bool {
        match self {
            Kind::Bar => s.bar,
            Kind::WorkspaceNames => s.workspace_names,
            Kind::Osd => s.osd,
            Kind::KeySheet => s.key_sheet,
            Kind::Pins => s.pins,
            Kind::Notifications => s.notifications,
        }
    }

    /// The switch as the settings are now.
    pub fn allowed(self) -> bool {
        self.allowed_in(&store::with(|s| s.extra_screens()))
    }
}

/// Whether `connector` is an extra screen among `connected`: virtual, with
/// at least one output that is not.
pub fn is_extra(connector: &str, connected: &[String]) -> bool {
    is_virtual(connector) && connected.iter().any(|c| !is_virtual(c))
}

/// Whether an output gets its own one of a per-output surface.
pub fn shows(allowed: bool, connector: &str, connected: &[String]) -> bool {
    allowed || !is_extra(connector, connected)
}

/// Where a surface placed on the focused output goes.
///
/// The focused output, unless that is an extra screen and `allowed` is off.
/// Then one of the person's own screens: the one this surface last went on
/// (`last`), else the built-in panel, else the first. Nothing is held back,
/// only placed. Without an answer from sway (`focused` is `None`) the
/// compositor places it, as before, unless an extra screen exists and must
/// be avoided: then a screen is named.
pub fn place(
    focused: Option<&str>,
    connected: &[String],
    allowed: bool,
    last: Option<&str>,
) -> Option<String> {
    let screens: Vec<&str> = connected
        .iter()
        .map(String::as_str)
        .filter(|c| !is_virtual(c))
        .collect();
    let avoid = !allowed && !screens.is_empty() && connected.iter().any(|c| is_virtual(c));
    match focused {
        Some(f) if !(avoid && is_virtual(f)) => return Some(f.to_string()),
        None if !avoid => return None,
        _ => {}
    }
    last.filter(|l| screens.contains(l))
        .or_else(|| screens.iter().copied().find(|s| is_builtin(s)))
        .or_else(|| screens.first().copied())
        .map(str::to_string)
}

/// Keep `set` off extra screens unless `kind`'s switch is on, and follow
/// the switch: flipped while an extra screen runs, the surface there is
/// built or dropped at once. Call before `PerMonitor::watch`.
pub fn follow<T: 'static>(set: &std::rc::Rc<super::PerMonitor<T>>, kind: Kind) {
    set.only(move |connector, connected| shows(kind.allowed(), connector, connected));
    let weak = std::rc::Rc::downgrade(set);
    // Any settings change lands here; a reconciliation that changes nothing
    // builds and drops nothing.
    store::observe(move || {
        if let Some(set) = weak.upgrade() {
            set.refresh();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outs(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn every_switch_is_off_by_default() {
        let d = ExtraScreens::default();
        for k in [
            Kind::Bar,
            Kind::WorkspaceNames,
            Kind::Osd,
            Kind::KeySheet,
            Kind::Pins,
            Kind::Notifications,
        ] {
            assert!(!k.allowed_in(&d), "{k:?} is on by default");
        }
    }

    #[test]
    fn each_switch_turns_on_its_own_surface_only() {
        let on = ExtraScreens {
            osd: true,
            ..ExtraScreens::default()
        };
        assert!(Kind::Osd.allowed_in(&on));
        assert!(!Kind::Bar.allowed_in(&on) && !Kind::KeySheet.allowed_in(&on));
    }

    #[test]
    fn a_per_output_surface_skips_an_extra_screen_unless_switched_on() {
        let desk = outs(&["eDP-1", "DP-3", "HEADLESS-2"]);
        assert!(!shows(false, "HEADLESS-2", &desk));
        assert!(shows(true, "HEADLESS-2", &desk));
        for on in [false, true] {
            assert!(shows(on, "eDP-1", &desk) && shows(on, "DP-3", &desk));
        }
        // The nested harness: only virtual outputs, all of them screens.
        let nested = outs(&["HEADLESS-1", "HEADLESS-2"]);
        assert!(shows(false, "HEADLESS-2", &nested));
        assert!(!is_extra("HEADLESS-2", &nested));
    }

    #[test]
    fn by_default_a_placed_surface_never_lands_on_an_extra_screen() {
        let desk = outs(&["eDP-1", "DP-3", "HEADLESS-2"]);
        // Focus on the TV's screen: the last screen it used.
        assert_eq!(
            place(Some("HEADLESS-2"), &desk, false, Some("DP-3")).as_deref(),
            Some("DP-3")
        );
        // None yet, or that screen is gone: the built-in panel.
        assert_eq!(
            place(Some("HEADLESS-2"), &desk, false, None).as_deref(),
            Some("eDP-1")
        );
        assert_eq!(
            place(Some("HEADLESS-2"), &desk, false, Some("HDMI-A-1")).as_deref(),
            Some("eDP-1")
        );
        // No built-in panel: the first screen.
        let no_laptop = outs(&["HEADLESS-2", "DP-3"]);
        assert_eq!(
            place(Some("HEADLESS-2"), &no_laptop, false, None).as_deref(),
            Some("DP-3")
        );
        // Sway did not answer: a screen is named rather than left to the
        // compositor, which would use the focused output.
        assert_eq!(place(None, &desk, false, None).as_deref(), Some("eDP-1"));
    }

    #[test]
    fn switched_on_it_may_go_to_the_extra_screen() {
        let desk = outs(&["eDP-1", "DP-3", "HEADLESS-2"]);
        assert_eq!(
            place(Some("HEADLESS-2"), &desk, true, Some("DP-3")).as_deref(),
            Some("HEADLESS-2")
        );
        assert_eq!(
            place(None, &desk, true, None),
            None,
            "the compositor's, as before"
        );
    }

    #[test]
    fn real_screens_are_unaffected_either_way() {
        let desk = outs(&["eDP-1", "DP-3", "HEADLESS-2"]);
        for on in [false, true] {
            assert_eq!(
                place(Some("DP-3"), &desk, on, Some("eDP-1")).as_deref(),
                Some("DP-3")
            );
            assert_eq!(
                place(Some("eDP-1"), &desk, on, None).as_deref(),
                Some("eDP-1")
            );
        }
        let plain = outs(&["eDP-1", "DP-3"]);
        assert_eq!(place(None, &plain, false, None), None);
        let nested = outs(&["HEADLESS-1", "HEADLESS-2"]);
        assert_eq!(
            place(Some("HEADLESS-2"), &nested, false, None).as_deref(),
            Some("HEADLESS-2")
        );
    }
}
