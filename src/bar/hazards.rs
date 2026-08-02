//! Hazard lane — standing amber conditions, right cluster between the
//! tray and the instrument track (docs/BAR_VISION.md, increment 7).
//!
//! Zero width when healthy: the lane owns no margins or padding; each
//! glyph carries its own inside a 200 ms Revealer, so an empty lane
//! measures 0 px with no visibility juggling. Glyphs are amber and
//! static — armed, not act-now — and appear-only: no motion beyond the
//! structural reveal (P2).
//!
//! Two hazards ship now; failed-units is deferred (severable):
//! - **Caffeine**: the idle manager is a systemd unit only knowable by
//!   `systemctl` reads, so the lane subscribes to the Caffeine tile's
//!   in-process path (widgets/tiles.rs `on_state`) instead of polling.
//!   The hosted process builds the panel — and reads tile state — at
//!   startup, so the initial state lands then; the standalone
//!   `swaypplet bar` process has no panel and shows caffeine only from
//!   the first toggle onward (accepted in the vision).
//! - **Binding mode**: non-default sway modes off the existing IPC
//!   subscription's `mode` event; the mode name lives in the tooltip.
//!
//! Cadence: in-process events and the existing sway subscription — this
//! module adds no timer and no poll.

use std::rc::Rc;

use gtk4::prelude::*;

use crate::service::Observed;
use crate::sway_ipc::SwayService;

// ── Caffeine state (in-process, main thread) ────────────────────────────

thread_local! {
    /// Fed by the Caffeine tile's read/toggle path; main-thread only,
    /// like every widget consumer.
    static CAFFEINE: Observed<bool> = Observed::new(false);
}

/// Publish the Caffeine tile's established state (widgets/tiles.rs).
pub(crate) fn set_caffeine(armed: bool) {
    CAFFEINE.with(|c| c.set_if_changed(armed));
}

// ── Widget ──────────────────────────────────────────────────────────────

/// The hazard lane for one bar window.
pub fn build(sway: &Rc<SwayService>) -> gtk4::Box {
    let lane = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .css_classes(["bar-hazards"])
        .build();

    let (caffeine, caffeine_glyph) = hazard("󰅶");
    caffeine_glyph.set_tooltip_text(Some("Caffeine: idle lock and screen blank suspended"));
    lane.append(&caffeine);
    let (mode, mode_glyph) = hazard("󰌌");
    lane.append(&mode);

    // Leftover observers after an output unplug paint unmapped widgets —
    // same leak-tolerant story as board.rs.
    CAFFEINE.with(|c| {
        c.connect_change({
            let caffeine = caffeine.clone();
            move || CAFFEINE.with(|c| caffeine.set_reveal_child(c.with(|v| *v)))
        });
        caffeine.set_reveal_child(c.with(|v| *v));
    });

    let apply_mode = {
        let (mode, glyph, sway) = (mode.clone(), mode_glyph, sway.clone());
        move || {
            let name = sway.snapshot().binding_mode;
            match armed_mode(&name) {
                Some(name) => {
                    glyph.set_tooltip_text(Some(&format!("Mode: {name}")));
                    mode.set_reveal_child(true);
                }
                None => mode.set_reveal_child(false),
            }
        }
    };
    apply_mode();
    sway.connect_change(apply_mode);

    lane
}

/// A non-default binding mode, if one is active. "" is the pre-snapshot
/// default of `SwayState` — never a hazard.
fn armed_mode(mode: &str) -> Option<&str> {
    (!mode.is_empty() && mode != "default").then_some(mode)
}

/// One appear-only glyph: amber label behind a 200 ms structural
/// Revealer, collapsed to zero width when its condition is clear.
fn hazard(glyph: &str) -> (gtk4::Revealer, gtk4::Label) {
    let label = gtk4::Label::builder()
        .label(glyph)
        .css_classes(["bar-hazard"])
        .build();
    let revealer = gtk4::Revealer::builder()
        .transition_type(gtk4::RevealerTransitionType::SlideRight)
        .transition_duration(200)
        .reveal_child(false)
        .child(&label)
        .build();
    (revealer, label)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_non_default_named_modes_arm_the_hazard() {
        assert_eq!(armed_mode("default"), None);
        // Pre-snapshot SwayState default: unknown is not a hazard.
        assert_eq!(armed_mode(""), None);
        assert_eq!(armed_mode("resize"), Some("resize"));
    }
}
