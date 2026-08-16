//! Hazard lane — standing amber conditions, right cluster between the
//! tray and the instrument track (docs/BAR_VISION.md, increment 7).
//!
//! Zero width when healthy: the lane owns no margins or padding; each
//! glyph carries its own inside a 200 ms Revealer, so an empty lane
//! measures 0 px with no visibility juggling. Glyphs are amber and
//! static — armed, not act-now — and appear-only: no motion beyond the
//! structural reveal (P2).
//!
//! Hazards shipping now; failed-units is deferred (severable):
//! - **Session inhibitors** (Awake, Stay Lit, Clamshell): one glyph each,
//!   driven off `crate::inhibit`'s observed state. That module owns the
//!   readings and the wording; this lane only decides where the glyph
//!   sits, so a fourth inhibitor arrives here for free. It is a push
//!   path rather than a poll: whoever establishes a state publishes it
//!   (the panel's tiles on toggle, `inhibit::prime` at startup), which
//!   is also what fixed the standalone `swaypplet bar` process being
//!   blind until the first toggle.
//! - **Binding mode**: non-default sway modes off the existing IPC
//!   subscription's `mode` event; the mode name lives in the tooltip.
//! - **Microphone**: something is recording. The sound server pushes this
//!   (`crate::audio`), so it costs no timer, and its stand-down (P10) is
//!   exactly the recorder list going empty. The tooltip names what is
//!   listening, which is the question the glyph provokes.
//!
//! Camera and screencast were meant to ship beside the microphone and
//! cannot yet. Neither has a signal a third party can read: v4l2 has no
//! in-use broadcast, and `org.freedesktop.portal.Camera` reports only
//! `IsCameraPresent` while `ScreenCast` exposes methods to *start* a cast
//! and no way to enumerate live ones. Both are visible in PipeWire's node
//! graph, which this process cannot reach — see `crate::audio` on the
//! bindgen collision that keeps libpipewire out of the build. They are
//! blocked on that, not on design.
//!
//! Cadence: in-process events and the existing sway subscription — this
//! module adds no timer and no poll.

use std::rc::Rc;

use gtk4::prelude::*;

use crate::audio::AudioService;
use crate::inhibit::{self, Inhibitor};
use crate::sway_ipc::SwayService;

// ── Widget ──────────────────────────────────────────────────────────────

/// The hazard lane for one bar window.
pub fn build(sway: &Rc<SwayService>, audio: &Rc<AudioService>) -> gtk4::Box {
    let lane = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .css_classes(["bar-hazards"])
        .build();

    for which in Inhibitor::ALL {
        let (revealer, glyph) = hazard(which.icon());
        glyph.set_tooltip_text(Some(which.tooltip_on()));
        lane.append(&revealer);
        inhibit::observed(which, |cell| {
            revealer.set_reveal_child(cell.with(|armed| *armed));
            cell.connect_change(move || {
                inhibit::observed(which, |cell| {
                    revealer.set_reveal_child(cell.with(|armed| *armed))
                })
            });
        });
    }
    // A bar without a panel beside it never sees a toggle, so ask once.
    inhibit::prime();

    let (mode, mode_glyph) = hazard("󰌌");
    lane.append(&mode);
    let (mic, mic_glyph) = hazard("󰍬");
    lane.append(&mic);

    let (rec, rec_glyph) = hazard("󰑋");
    rec_glyph.set_tooltip_text(Some("Screen recording in progress"));
    rec_glyph.add_css_class("bar-hazard-rec");
    lane.append(&rec);

    crate::screenshot::record::RECORDING_OBSERVED.with(|r| {
        r.connect_change({
            let rec = rec.clone();
            move || {
                crate::screenshot::record::RECORDING_OBSERVED
                    .with(|r| rec.set_reveal_child(r.with(|v| *v)))
            }
        });
        rec.set_reveal_child(r.with(|v| *v));
    });

    let apply_mic = {
        let (mic, glyph, audio) = (mic.clone(), mic_glyph, audio.clone());
        move || {
            let state = audio.snapshot();
            let names = state.recorder_names();
            glyph.set_tooltip_text(Some(&match names.len() {
                0 => "Microphone in use".to_string(),
                _ => format!("Microphone: {}", names.join(", ")),
            }));
            mic.set_reveal_child(state.microphone_in_use());
        }
    };
    apply_mic();
    audio.connect_change(apply_mic);

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
