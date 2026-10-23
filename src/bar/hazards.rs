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
//! - **Extra screen**: a virtual output is on and something is sent to it
//!   (`services::cast`): a TV cast or a VNC viewer. Your desktop is
//!   reaching a screen you may not be looking at, so the lane says so,
//!   and the tooltip says where. `cast::observe` drives it; it stands down
//!   when the session ends, by Stop or by itself (the TV went away, the
//!   encoder died), and a restarted panel starts with none. A click opens
//!   the Helm's Displays page, where Stop is: the one hazard with an
//!   action, because ending it is the one thing a person does about it.
//!   The cast lives in the panel process, so the standalone bar never
//!   shows it.
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
use crate::services::cast::{self, backend::Target};
use crate::services::inhibit::{self, Inhibitor};
use crate::services::network;
use crate::sway::ipc::SwayService;
use crate::ui;

// ── Widget ──────────────────────────────────────────────────────────────

/// The hazard lane for one bar window. `open_displays` is the extra-screen
/// glyph's click.
pub fn build(
    sway: &Rc<SwayService>,
    audio: &Rc<AudioService>,
    open_displays: Option<Rc<dyn Fn()>>,
) -> gtk4::Box {
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
    let apply_wired = Rc::new(move || {
        let banned: Vec<String> = network::blocked::BANNED.with(|b| {
            b.with(|banned| {
                banned
                    .iter()
                    .map(|w| network::display_name(&w.label, w.mac.as_deref()))
                    .collect()
            })
        });
        if let Some(text) = banned_tooltip(&banned) {
            wired_glyph.set_tooltip_text(Some(&text));
        }
        wired.set_reveal_child(!banned.is_empty());
    });
    apply_wired();
    network::blocked::BANNED.with(|b| {
        b.connect_change({
            let apply = apply_wired.clone();
            move || apply()
        })
    });
    // A rename shows in the tooltip without waiting for the next ban.
    crate::services::devices::observe(move || apply_wired());
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

    let (extra, extra_glyph) = hazard(ui::icons::TV);
    lane.append(&extra);
    let clickable = open_displays.is_some();
    if let Some(open) = open_displays {
        let click = gtk4::GestureClick::new();
        click.connect_released(move |_, _, _, _| open());
        extra_glyph.add_controller(click);
        extra_glyph.set_cursor_from_name(Some("pointer"));
    }
    let apply_extra = move || match extra_screen_tooltip(&cast::view(), clickable) {
        Some(text) => {
            extra_glyph.set_tooltip_text(Some(&text));
            extra.set_reveal_child(true);
        }
        None => extra.set_reveal_child(false),
    };
    apply_extra();
    cast::observe(apply_extra);

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

/// What the extra-screen glyph says, or `None` when no extra screen is on
/// (none, or one still starting).
fn extra_screen_tooltip(view: &cast::View, clickable: bool) -> Option<String> {
    let s = view.session.as_ref()?;
    let what = match &s.target {
        Target::Device(d) => format!("Extra screen on {}", d.name),
        Target::Viewer => "Extra screen for a VNC viewer".to_string(),
    };
    let how = if clickable {
        "\nClick to see it in Displays, where Stop is"
    } else {
        ""
    };
    Some(format!("{what}\n{} · {}{how}", s.output, s.location))
}

/// What the banned-adapter glyph says, or `None` when nothing is banned.
fn banned_tooltip(banned: &[String]) -> Option<String> {
    match banned {
        [] => None,
        [one] => Some(format!("{one} blocked: on Wi-Fi until it is unplugged")),
        many => Some(format!(
            "{} blocked: on Wi-Fi until they are unplugged",
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
            banned_tooltip(&["Ethernet on Lenovo dock".into()]).as_deref(),
            Some("Ethernet on Lenovo dock blocked: on Wi-Fi until it is unplugged")
        );
        assert!(
            banned_tooltip(&["Desk".into(), "Ethernet on USB hub".into()])
                .is_some_and(|t| t.contains("Desk, Ethernet on USB hub") && t.contains("they are"))
        );
    }

    fn session(target: Target) -> cast::View {
        cast::View {
            session: Some(cast::SessionView {
                output: "HEADLESS-3".into(),
                target,
                backend: "Cast",
                location: "Vardagsrummet TV, under a second behind".into(),
            }),
            ..cast::View::default()
        }
    }

    #[test]
    fn the_extra_screen_glyph_says_what_runs_and_only_while_it_does() {
        let tv = cast::discovery::Device {
            id: "daf6".into(),
            name: "Vardagsrummet TV".into(),
            model: Some("Chromecast".into()),
            address: "192.168.1.107".into(),
            port: 8009,
            video: true,
            group: false,
        };
        let text = extra_screen_tooltip(&session(Target::Device(tv)), true).unwrap();
        assert!(
            text.starts_with("Extra screen on Vardagsrummet TV\n"),
            "{text}"
        );
        assert!(
            text.contains("HEADLESS-3") && text.contains("Stop"),
            "{text}"
        );
        let vnc = extra_screen_tooltip(&session(Target::Viewer), false).unwrap();
        assert!(vnc.starts_with("Extra screen for a VNC viewer"), "{vnc}");
        assert!(
            !vnc.contains("Click"),
            "the standalone bar promises no click"
        );
        // Nothing on; one starting (busy, no session yet); one that ended,
        // by Stop or by itself (the view drops its session either way).
        assert_eq!(extra_screen_tooltip(&cast::View::default(), true), None);
        let starting = cast::View {
            busy: true,
            ..cast::View::default()
        };
        assert_eq!(extra_screen_tooltip(&starting, true), None);
        let ended = cast::View {
            problem: Some("The TV stopped answering.".into()),
            ..cast::View::default()
        };
        assert_eq!(extra_screen_tooltip(&ended, true), None);
    }

    #[test]
    fn swaypplets_own_modes_do_not_arm_the_hazard() {
        // Super+Tab's switcher holds this mode while its card is up.
        assert_eq!(armed_mode("swaypplet-jump"), None);
    }
}
