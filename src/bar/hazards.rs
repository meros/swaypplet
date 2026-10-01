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
//! - **Session inhibitors** (No Sleep, No Lock): one glyph each, driven
//!   off `services::inhibit`'s observed state. That module owns the
//!   readings and the wording; this lane only decides where the glyph
//!   sits, so a third inhibitor arrives here for free. It is a push
//!   path rather than a poll: whoever establishes a state publishes it
//!   (the panel's tiles on toggle, `inhibit::prime` at startup), which
//!   is also what fixed the standalone `swaypplet bar` process being
//!   blind until the first toggle.
//! - **Binding mode**: non-default sway modes off the existing IPC
//!   subscription's `mode` event; the mode name lives in the tooltip.
//! - **Microphone**: something is recording. The sound server pushes this
//!   (`services::audio`), so it costs no timer, and its stand-down (P10) is
//!   exactly the recorder list going empty. The tooltip names what is
//!   listening, which is the question the glyph provokes.
//! - **Banned wired adapter**: NetworkManager has been told to leave an
//!   ethernet adapter alone (`services::network::blocked`), by hand from
//!   the panel or by the host when the link dropped while plugged in. The
//!   machine is on Wi-Fi without the owner having chosen it in the moment,
//!   which is what the glyph says. Stand-down: the adapter is unplugged or
//!   the ban lifted in the panel. NM's device signals drive it.
//!
//! Camera and screencast were meant to ship beside the microphone and
//! cannot yet. Neither has a signal a third party can read: v4l2 has no
//! in-use broadcast, and `org.freedesktop.portal.Camera` reports only
//! `IsCameraPresent` while `ScreenCast` exposes methods to *start* a cast
//! and no way to enumerate live ones. Both are visible in PipeWire's node
//! graph, which this process cannot reach — see `services::audio` on the
//! bindgen collision that keeps libpipewire out of the build. They are
//! blocked on that, not on design.
//!
//! Cadence: in-process events and the existing sway subscription — this
//! module adds no timer and no poll.

use std::rc::Rc;

use gtk4::prelude::*;

use crate::services::audio::AudioService;
use crate::services::inhibit::{self, Inhibitor};
use crate::services::network;
use crate::sway::ipc::SwayService;
use crate::ui;

// ── Widget ──────────────────────────────────────────────────────────────

/// The hazard lane for one bar window.
pub fn build(sway: &Rc<SwayService>, audio: &Rc<AudioService>) -> gtk4::Box {
    let lane = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
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

    let (wired, wired_glyph) = hazard("󰈂");
    lane.append(&wired);
    let apply_wired = move || {
        let banned = network::blocked::BANNED.with(|b| b.with(Vec::clone));
        if let Some(text) = banned_tooltip(&banned) {
            wired_glyph.set_tooltip_text(Some(&text));
        }
        wired.set_reveal_child(!banned.is_empty());
    };
    apply_wired();
    network::blocked::BANNED.with(|b| b.connect_change(apply_wired));
    network::blocked::start();

    let (rec, rec_glyph) = hazard("󰑋");
    rec_glyph.set_tooltip_text(Some("Screen recording in progress"));
    // Recording is the one hazard in red: it is capturing now, and the
    // owner may want to stop it now.
    ui::glyph::adopt(&rec_glyph, ui::Text::Body, ui::Tone::Danger);
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
/// default of `SwayState` — never a hazard. Nor is a mode of swaypplet's own
/// (`swaypplet-jump`, which Super+Tab enters while the switcher is on
/// screen): its surface already says the keys are captured, so the glyph
/// would only flash beside it.
fn armed_mode(mode: &str) -> Option<&str> {
    (!mode.is_empty() && mode != "default" && !mode.starts_with("swaypplet-")).then_some(mode)
}

/// What the banned-adapter glyph says, or `None` when nothing is banned.
fn banned_tooltip(banned: &[String]) -> Option<String> {
    match banned {
        [] => None,
        [one] => Some(format!(
            "Ethernet {one} blocked: on Wi-Fi until it is unplugged"
        )),
        many => Some(format!(
            "Ethernet {} blocked: on Wi-Fi until they are unplugged",
            many.join(", ")
        )),
    }
}

/// One appear-only glyph: a warning-toned label (armed, not act-now: red
/// stays "act now", vision P3) behind a 200 ms structural Revealer,
/// collapsed to zero width when its condition is clear.
fn hazard(glyph: &str) -> (gtk4::Revealer, gtk4::Label) {
    let label = gtk4::Label::builder()
        .label(glyph)
        .css_classes(["bar-hazard"])
        .build();
    ui::glyph::adopt(&label, ui::Text::Body, ui::Tone::Warning);
    let revealer = ui::revealer(
        gtk4::RevealerTransitionType::SlideRight,
        crate::tokens::motion::EXPAND,
    );
    revealer.set_child(Some(&label));
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

    #[test]
    fn the_banned_glyph_names_the_adapters() {
        assert_eq!(banned_tooltip(&[]), None);
        assert_eq!(
            banned_tooltip(&["enp0s13f0u2u1".into()]).as_deref(),
            Some("Ethernet enp0s13f0u2u1 blocked: on Wi-Fi until it is unplugged")
        );
        assert!(
            banned_tooltip(&["enp1".into(), "enp2".into()])
                .is_some_and(|t| t.contains("enp1, enp2") && t.contains("they are"))
        );
    }

    #[test]
    fn swaypplets_own_modes_do_not_arm_the_hazard() {
        // Super+Tab's switcher holds this mode while its card is up.
        assert_eq!(armed_mode("swaypplet-jump"), None);
    }
}
