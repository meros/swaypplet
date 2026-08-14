use std::cell::{Cell, RefCell};
use std::process::Command;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4_layer_shell::Edge;

use crate::anim;
use crate::layer_shell::{self, LayerShellConfig};
use crate::spawn::spawn_work;

const OSD_TIMEOUT_MS: u32 = 1500;
/// One press of a volume key. The ceiling is the over-amplification limit
/// the panel's slider also stops at.
const VOLUME_STEP: f64 = 0.05;
use crate::audio::VOLUME_CEILING;
const BRIGHTNESS_STEP_UP: &str = "5%+";
const BRIGHTNESS_STEP_DOWN: &str = "5%-";

use crate::icons;

// ── Commands ─────────────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
pub enum OsdCommand {
    OutputVolumeRaise,
    OutputVolumeLower,
    OutputVolumeMuteToggle,
    InputVolumeMuteToggle,
    BrightnessRaise,
    BrightnessLower,
    CapsLock,
    NumLock,
    ScrollLock,
}

impl OsdCommand {
    pub fn parse(args: &[String]) -> Option<Self> {
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--output-volume" => {
                    let action = args.get(i + 1)?;
                    return match action.as_str() {
                        "raise" => Some(Self::OutputVolumeRaise),
                        "lower" => Some(Self::OutputVolumeLower),
                        "mute-toggle" => Some(Self::OutputVolumeMuteToggle),
                        _ => None,
                    };
                }
                "--input-volume" => {
                    let action = args.get(i + 1)?;
                    return match action.as_str() {
                        "mute-toggle" => Some(Self::InputVolumeMuteToggle),
                        _ => None,
                    };
                }
                "--brightness" => {
                    let action = args.get(i + 1)?;
                    return match action.as_str() {
                        "raise" => Some(Self::BrightnessRaise),
                        "lower" => Some(Self::BrightnessLower),
                        _ => None,
                    };
                }
                "--caps-lock" => return Some(Self::CapsLock),
                "--num-lock" => return Some(Self::NumLock),
                "--scroll-lock" => return Some(Self::ScrollLock),
                _ => {}
            }
            i += 1;
        }
        None
    }
}

// ── OSD result after performing action ───────────────────────────────────────

enum OsdDisplay {
    Bar {
        icon: String,
        fraction: f64,
        text: String,
    },
    Indicator {
        icon: String,
        label: String,
        active: bool,
    },
}

// ── Action execution + state reading ─────────────────────────────────────────

/// The commands that still need a process spawned. Volume left with the
/// `wpctl` dependency: it is answered from `crate::audio`'s snapshot on the
/// GTK thread, which is both faster and the only way the OSD and the panel
/// slider can agree on what the volume is.
fn execute_command(cmd: &OsdCommand) -> OsdDisplay {
    match cmd {
        OsdCommand::BrightnessRaise => {
            let _ = Command::new("brightnessctl")
                .args(["set", BRIGHTNESS_STEP_UP])
                .output();
            read_brightness_display()
        }
        OsdCommand::BrightnessLower => {
            let _ = Command::new("brightnessctl")
                .args(["set", BRIGHTNESS_STEP_DOWN])
                .output();
            read_brightness_display()
        }
        OsdCommand::CapsLock => {
            read_lock_display("capslock", icons::CAPS_ON, icons::CAPS_OFF, "CAPS")
        }
        OsdCommand::NumLock => read_lock_display("numlock", icons::NUM_ON, icons::NUM_OFF, "NUM"),
        OsdCommand::ScrollLock => read_lock_display("scrolllock", "S", "s", "SCROLL"),

        // Handled on the main thread from the sound server's snapshot
        // (`Osd::volume_key`); reaching here means the panel had no audio
        // connection, and there is nothing to show.
        OsdCommand::OutputVolumeRaise
        | OsdCommand::OutputVolumeLower
        | OsdCommand::OutputVolumeMuteToggle
        | OsdCommand::InputVolumeMuteToggle => volume_display(0.0, true, false),
    }
}

/// Turn a volume into what the OSD draws.
/// What the card would read for this level, as the comparison that decides
/// whether a redraw is worth restarting the auto-hide timer for.
fn drawn_key(volume: f64, muted: bool) -> (i32, bool) {
    ((volume * 100.0).round() as i32, muted)
}

fn volume_display(volume: f64, muted: bool, is_mic: bool) -> OsdDisplay {
    let icon = icons::volume_icon(volume, muted, is_mic);

    let pct = (volume * 100.0).round() as u32;
    let fraction = if muted { 0.0 } else { volume.min(1.5) / 1.5 };

    OsdDisplay::Bar {
        icon: icon.to_string(),
        fraction,
        text: if muted {
            "Muted".to_string()
        } else {
            format!("{}%", pct)
        },
    }
}

fn read_brightness_display() -> OsdDisplay {
    let output = Command::new("brightnessctl").arg("-m").output().ok();

    let pct = output
        .and_then(|o| {
            let text = String::from_utf8_lossy(&o.stdout).to_string();
            let line = text.lines().next()?.to_string();
            let field = line
                .split(',')
                .nth(3)?
                .trim()
                .trim_end_matches('%')
                .to_string();
            field.parse::<u32>().ok()
        })
        .unwrap_or(0);

    OsdDisplay::Bar {
        icon: icons::BRIGHTNESS.to_string(),
        fraction: pct as f64 / 100.0,
        text: format!("{}%", pct),
    }
}

fn read_lock_display(lock_name: &str, icon_on: &str, icon_off: &str, label: &str) -> OsdDisplay {
    // Read from /sys/class/leds/input*::{lock_name}/brightness
    let active = std::fs::read_dir("/sys/class/leds/")
        .ok()
        .and_then(|entries| {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.ends_with(&format!("::{}", lock_name)) {
                    let path = entry.path().join("brightness");
                    if let Ok(val) = std::fs::read_to_string(&path) {
                        return Some(val.trim() == "1");
                    }
                }
            }
            None
        })
        .unwrap_or(false);

    OsdDisplay::Indicator {
        icon: if active { icon_on } else { icon_off }.to_string(),
        label: format!("{} {}", label, if active { "ON" } else { "OFF" }),
        active,
    }
}

// ── OSD Widget ───────────────────────────────────────────────────────────────

/// Volume/brightness route into the bar's decision slot (icon, fraction,
/// text) → handled? Installed by app.rs once the bar exists.
type BarRoute = Box<dyn Fn(&str, f64, &str) -> bool>;

/// How long after the last press the server's own number takes over.
///
/// Long enough to cover a key repeat gap (~33 ms) and the round trip, short
/// enough to land well inside the OSD's 1500 ms on screen.
const SETTLE_MS: u64 = 150;

/// What the card is currently showing, so a snapshot from the sound server
/// knows whether it has anything to say about it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Showing {
    /// Brightness, a lock indicator, or nothing at all.
    Other,
    OutputVolume,
    InputVolume,
}

/// The level the volume keys have asked for and the server has not yet
/// confirmed.
///
/// Presses accumulate here rather than each one reading the last snapshot:
/// a burst arrives faster than the server echoes, so every press in it
/// would otherwise compute the same "current + one step" and the card would
/// sit still while the volume moved.
#[derive(Default)]
struct Pending {
    level: Option<f64>,
    settle: Option<glib::SourceId>,
}

#[derive(Clone)]
pub struct Osd {
    icon_label: gtk4::Label,
    bar: gtk4::ProgressBar,
    text_label: gtk4::Label,
    // For indicator mode (caps lock etc.)
    indicator_label: gtk4::Label,
    bar_box: gtk4::Box,
    reveal: anim::Reveal,
    timeout_id: Rc<RefCell<Option<glib::SourceId>>>,
    bar_route: Rc<RefCell<Option<BarRoute>>>,
    /// Set once the panel exists. Absent only in the standalone paths that
    /// never send a volume command.
    audio: Rc<RefCell<Option<Rc<crate::audio::AudioService>>>>,
    showing: Rc<Cell<Showing>>,
    pending: Rc<RefCell<Pending>>,
    /// The rounded percentage and mute currently on the card. A snapshot
    /// that would redraw the same thing is dropped, because showing a card
    /// restarts its 1500 ms life and unrelated audio events are frequent.
    drawn: Rc<Cell<Option<(i32, bool)>>>,
}

impl Osd {
    pub fn new(app: &gtk4::Application) -> Self {
        static OSD_CONFIG: LayerShellConfig = LayerShellConfig {
            namespace: "swaypplet-osd",
            layer: gtk4_layer_shell::Layer::Overlay,
            exclusive: false,
            default_width: None,
            default_height: None,
            anchors: &[(Edge::Bottom, true)],
            margins: &[(Edge::Bottom, 72)],
            keyboard_mode: gtk4_layer_shell::KeyboardMode::None,
        };
        let window = layer_shell::create_layer_window(app, &OSD_CONFIG);
        window.set_resizable(false);
        window.set_decorated(false);

        // Transparent apron around the card (48px CSS padding); must stay
        // alpha-0 so compositor blur clips to the card
        let wrapper = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .halign(gtk4::Align::Center)
            .valign(gtk4::Align::Center)
            .build();
        wrapper.add_css_class("osd-wrapper");

        // Vertical layout: icon → bar → percentage
        let outer = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(0)
            .halign(gtk4::Align::Center)
            .build();
        outer.add_css_class("glass-card");
        outer.add_css_class("osd-container");

        let icon_label = gtk4::Label::builder()
            .label("")
            .halign(gtk4::Align::Fill)
            .hexpand(true)
            .xalign(0.5)
            .build();
        icon_label.add_css_class("osd-icon");

        // Bar mode: bar + percentage below
        let bar_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(8)
            .build();

        let bar = gtk4::ProgressBar::builder().hexpand(true).build();
        bar.add_css_class("osd-bar");

        let text_label = gtk4::Label::builder()
            .label("")
            .halign(gtk4::Align::Center)
            .build();
        text_label.add_css_class("osd-text");

        bar_box.append(&bar);
        bar_box.append(&text_label);

        // Indicator mode (caps lock etc.)
        let indicator_label = gtk4::Label::builder()
            .label("")
            .halign(gtk4::Align::Center)
            .build();
        indicator_label.add_css_class("osd-indicator");
        indicator_label.set_visible(false);

        // Content sits on the glass and fades over the full duration; the
        // card (`outer`) is the pane whose tint arrives fast (motion on
        // glass, anim.rs). Pure crossfade — no settle on the OSD.
        let content = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(0)
            .build();
        content.append(&icon_label);
        content.append(&bar_box);
        content.append(&indicator_label);
        outer.append(&content);

        wrapper.append(&outer);
        window.set_child(Some(&wrapper));

        let reveal = anim::Reveal::new(&window, &outer).content(&content);

        Osd {
            icon_label,
            bar,
            text_label,
            indicator_label,
            bar_box,
            reveal,
            timeout_id: Rc::new(RefCell::new(None)),
            bar_route: Rc::new(RefCell::new(None)),
            audio: Rc::new(RefCell::new(None)),
            showing: Rc::new(Cell::new(Showing::Other)),
            pending: Rc::new(RefCell::new(Pending::default())),
            drawn: Rc::new(Cell::new(None)),
        }
    }

    /// Install the bar route (docs/BAR_VISION.md, increment 5): while the
    /// focused window is not fullscreen, volume/brightness displays render
    /// as the decision-slot interjection instead of the center card. Lock
    /// indicators (caps/num/scroll) always keep the card.
    pub fn set_bar_route(&self, route: impl Fn(&str, f64, &str) -> bool + 'static) {
        *self.bar_route.borrow_mut() = Some(Box::new(route));
    }

    /// Hand the OSD the sound server connection, so volume keys are answered
    /// from the same snapshot the panel's slider draws.
    pub fn set_audio(&self, audio: Rc<crate::audio::AudioService>) {
        *self.audio.borrow_mut() = Some(audio.clone());
        let osd = self.clone();
        audio.connect_change(move || osd.reconcile());
    }

    /// Redraw a volume card from the server's own number.
    ///
    /// The level drawn on the key press is a prediction, and on a sink that
    /// carries its volume in hardware it is a wrong one: the server
    /// quantises to the hardware's dB steps, so asking for 70% comes back
    /// as 72%. The prediction is what makes the card feel instant; this is
    /// what makes it true. It also covers the volume moving with no key
    /// press at all, which is what the speakers' own knob does.
    ///
    /// Skipped while presses are still arriving, so a burst shows where it
    /// is heading rather than where it has got to.
    fn reconcile(&self) {
        if self.pending.borrow().level.is_some() {
            return;
        }
        let Some(audio) = self.audio.borrow().clone() else {
            return;
        };
        let state = audio.snapshot();
        let (volume, is_mic) = match self.showing.get() {
            Showing::OutputVolume => (state.sink, false),
            Showing::InputVolume => (state.source, true),
            Showing::Other => return,
        };
        let Some(volume) = volume else { return };
        let key = drawn_key(volume.volume, volume.muted);
        if self.drawn.get() == Some(key) {
            return;
        }
        self.drawn.set(Some(key));
        self.show_display(&volume_display(volume.volume, volume.muted, is_mic));
    }

    /// Note a press, and return the level to draw for it.
    ///
    /// Each press advances the pending level rather than the snapshot's, so
    /// ten of them in one burst move ten steps on the card and ten steps at
    /// the server. The settle timer hands the card back to [`reconcile`]
    /// once the presses stop.
    fn advance(&self, from: f64, delta: f64) -> f64 {
        let mut pending = self.pending.borrow_mut();
        let target = (pending.level.unwrap_or(from) + delta).clamp(0.0, VOLUME_CEILING);
        pending.level = Some(target);

        if let Some(id) = pending.settle.take() {
            id.remove();
        }
        let slot = self.pending.clone();
        let osd = self.clone();
        pending.settle = Some(glib::timeout_add_local_once(
            std::time::Duration::from_millis(SETTLE_MS),
            move || {
                {
                    let mut pending = slot.borrow_mut();
                    pending.settle = None;
                    pending.level = None;
                }
                osd.reconcile();
            },
        ));

        target
    }

    pub fn trigger(&self, cmd: &OsdCommand) {
        // Volume never leaves the main thread: the answer is already in the
        // snapshot, and the command back to the server is a channel send.
        if let Some(display) = self.volume_key(cmd) {
            self.show_display(&display);
            return;
        }

        // Nothing the sound server says bears on a brightness or lock card.
        self.showing.set(Showing::Other);

        // brightnessctl can hang; run it off the GTK main thread so a stuck
        // command doesn't freeze the panel.
        let cmd = *cmd;
        let osd = self.clone();
        spawn_work(
            move || execute_command(&cmd),
            move |display| osd.show_display(&display),
        );
    }

    /// Answer a volume key, or `None` if this is not one.
    ///
    /// The new level is computed and drawn here rather than read back after
    /// the fact: a read-back races the server's own event, and showing a
    /// number the key press did not produce is worse than showing it a
    /// millisecond early.
    fn volume_key(&self, cmd: &OsdCommand) -> Option<OsdDisplay> {
        use crate::audio::Command as AudioCommand;

        let audio = self.audio.borrow().clone()?;
        let state = audio.snapshot();

        let current = match cmd {
            OsdCommand::OutputVolumeRaise
            | OsdCommand::OutputVolumeLower
            | OsdCommand::OutputVolumeMuteToggle => state.sink.clone()?,
            OsdCommand::InputVolumeMuteToggle => state.source.clone()?,
            _ => return None,
        };

        let (level, muted, is_mic) = match cmd {
            OsdCommand::OutputVolumeRaise => {
                self.showing.set(Showing::OutputVolume);
                audio.send(AudioCommand::AdjustSinkVolume(VOLUME_STEP));
                (
                    self.advance(current.volume, VOLUME_STEP),
                    current.muted,
                    false,
                )
            }
            OsdCommand::OutputVolumeLower => {
                self.showing.set(Showing::OutputVolume);
                audio.send(AudioCommand::AdjustSinkVolume(-VOLUME_STEP));
                (
                    self.advance(current.volume, -VOLUME_STEP),
                    current.muted,
                    false,
                )
            }
            OsdCommand::OutputVolumeMuteToggle => {
                self.showing.set(Showing::OutputVolume);
                audio.send(AudioCommand::ToggleSinkMute);
                (current.volume, !current.muted, false)
            }
            OsdCommand::InputVolumeMuteToggle => {
                self.showing.set(Showing::InputVolume);
                audio.send(AudioCommand::ToggleSourceMute);
                (current.volume, !current.muted, true)
            }
            _ => return None,
        };

        self.drawn.set(Some(drawn_key(level, muted)));
        Some(volume_display(level, muted, is_mic))
    }

    fn show_display(&self, display: &OsdDisplay) {
        if let OsdDisplay::Bar {
            icon,
            fraction,
            text,
        } = display
            && let Some(route) = &*self.bar_route.borrow()
            && route(icon, *fraction, text)
        {
            return;
        }
        match display {
            OsdDisplay::Bar {
                icon,
                fraction,
                text,
            } => {
                self.icon_label.set_label(icon);
                self.bar.set_fraction(*fraction);
                self.text_label.set_label(text);
                self.bar_box.set_visible(true);
                self.indicator_label.set_visible(false);
            }
            OsdDisplay::Indicator {
                icon,
                label,
                active,
            } => {
                self.icon_label.set_label(icon);
                self.indicator_label.set_label(label);
                self.bar_box.set_visible(false);
                self.indicator_label.set_visible(true);
                if *active {
                    self.indicator_label.add_css_class("osd-indicator-active");
                } else {
                    self.indicator_label
                        .remove_css_class("osd-indicator-active");
                }
            }
        }

        // Fade in (motion on glass, anim.rs). Retriggering mid-exit
        // reverses the fade from its current opacities; opacity never
        // unmaps the content, so the auto-sized surface stays put during
        // the transition (the old spacer overlay is gone with the
        // revealer).
        self.reveal.show();

        // Cancel previous timeout
        if let Some(id) = self.timeout_id.borrow_mut().take() {
            id.remove();
        }

        // Auto-hide after timeout: fade out, then unmap (Reveal hides the
        // window when the exit finishes)
        let reveal_c = self.reveal.clone();
        let timeout_ref = self.timeout_id.clone();
        let showing = self.showing.clone();
        let drawn = self.drawn.clone();
        let id = glib::timeout_add_local_once(
            std::time::Duration::from_millis(OSD_TIMEOUT_MS as u64),
            move || {
                *timeout_ref.borrow_mut() = None;
                showing.set(Showing::Other);
                drawn.set(None);
                reveal_c.hide();
            },
        );
        *self.timeout_id.borrow_mut() = Some(id);
    }
}
