use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4::gdk;
use gtk4::prelude::*;
use gtk4_layer_shell::Edge;

use swaypplet_base::spawn::{Timeout, spawn_work};
use swaypplet_system::brightness;
use swaypplet_ui::shell::{Namespace, PerMonitor, Surface};

const OSD_TIMEOUT_MS: u32 = 1500;

/// One press of a volume or brightness key, from the Bar tab's Keys group.
/// The ceiling is the over-amplification limit the panel's slider also
/// stops at (`swaypplet_widgets::audio`), and where the OSD's bar is full.
fn keys() -> swaypplet_model::store::Keys {
    swaypplet_model::store::with(|s| s.keys())
}

use swaypplet_ui::ui::icons;

// ── Commands ─────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

#[derive(Clone)]
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
/// `wpctl` dependency: it is answered from `swaypplet_audio`'s snapshot on the
/// GTK thread, which is both faster and the only way the OSD and the panel
/// slider can agree on what the volume is.
fn execute_command(cmd: &OsdCommand) -> OsdDisplay {
    match cmd {
        OsdCommand::BrightnessRaise => {
            brightness::adjust(keys().brightness_step.into(), true);
            read_brightness_display()
        }
        OsdCommand::BrightnessLower => {
            brightness::adjust(keys().brightness_step.into(), false);
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
        | OsdCommand::InputVolumeMuteToggle => volume_display(0.0, true, false, 1.0),
    }
}

/// Turn a volume into what the OSD draws.
/// What the card would read for this level, as the comparison that decides
/// whether a redraw is worth restarting the auto-hide timer for.
fn drawn_key(volume: f64, muted: bool) -> (i32, bool) {
    ((volume * 100.0).round() as i32, muted)
}

/// `ceiling` is where the bar is full: the end the panel's slider has too,
/// so a level fills the same share of both.
fn volume_display(volume: f64, muted: bool, is_mic: bool, ceiling: f64) -> OsdDisplay {
    let icon = icons::volume_icon(volume, muted, is_mic);

    let pct = (volume * 100.0).round() as u32;
    let fraction = if muted {
        0.0
    } else {
        (volume / ceiling).clamp(0.0, 1.0)
    };

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
    // A failed read draws 0 %, as it always has.
    let pct = brightness::read().unwrap_or(0);

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
/// text) → handled? Installed by `swaypplet_app` once the bar exists.
type BarRoute = Box<dyn Fn(&str, f64, &str) -> bool>;
type Hold = Box<dyn Fn() -> bool>;

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
    settle: Rc<Timeout>,
}

/// The card on one output. A layer surface is bound to its `wl_output`, so
/// a card is built for a monitor and destroyed with it, never moved.
struct Card {
    surface: Surface,
    icon_label: gtk4::Label,
    bar: gtk4::ProgressBar,
    text_label: gtk4::Label,
    // For indicator mode (caps lock etc.)
    indicator_label: gtk4::Label,
    bar_box: gtk4::Box,
}

impl Card {
    fn new(app: &gtk4::Application, monitor: &gdk::Monitor) -> Card {
        let surface = Surface::builder(app, Namespace::Osd)
            .monitor(Some(monitor))
            .anchor(&[Edge::Bottom])
            .margin(Edge::Bottom, 72)
            .card(swaypplet_ui::ui::Card::Thin)
            // The window's geometry was tuned with this apron's padding.
            .apron(gtk4::Align::Center, "osd-wrapper")
            .build();

        // Vertical layout: icon → bar → percentage
        let outer = surface.card();
        outer.set_halign(gtk4::Align::Center);
        outer.add_css_class("osd-container");

        let icon_label = gtk4::Label::builder()
            .label("")
            .halign(gtk4::Align::Fill)
            .hexpand(true)
            .xalign(0.5)
            .build();
        icon_label.add_css_class("osd-icon");
        swaypplet_ui::ui::glyph::adopt(
            &icon_label,
            swaypplet_ui::ui::Text::Display,
            swaypplet_ui::ui::Tone::Fg,
        );

        // Bar mode: bar + percentage below
        let bar_box = swaypplet_ui::ui::vbox(3);

        let bar = gtk4::ProgressBar::builder().hexpand(true).build();
        swaypplet_ui::ui::progress::adopt(&bar);

        let text_label = gtk4::Label::builder()
            .label("")
            .halign(gtk4::Align::Center)
            .build();
        swaypplet_ui::ui::set_text_style(
            &text_label,
            swaypplet_ui::ui::Text::Title,
            swaypplet_ui::ui::Tone::Fg,
        );
        swaypplet_ui::ui::set_numeric(&text_label, true);

        bar_box.append(&bar);
        bar_box.append(&text_label);

        // Indicator mode (caps lock etc.)
        let indicator_label = gtk4::Label::builder()
            .label("")
            .halign(gtk4::Align::Center)
            .build();
        swaypplet_ui::ui::set_text_style(
            &indicator_label,
            swaypplet_ui::ui::Text::TitleSm,
            swaypplet_ui::ui::Tone::Muted,
        );
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
        surface.set_content(&content);

        Card {
            surface,
            icon_label,
            bar,
            text_label,
            indicator_label,
            bar_box,
        }
    }

    fn draw(&self, display: &OsdDisplay) {
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
                // The accent says the lock is on; off is muted.
                let tone = if *active {
                    swaypplet_ui::ui::Tone::Accent
                } else {
                    swaypplet_ui::ui::Tone::Muted
                };
                swaypplet_ui::ui::set_text_style(
                    &self.indicator_label,
                    swaypplet_ui::ui::Text::TitleSm,
                    tone,
                );
            }
        }
    }
}

/// The OSD: one card per output, all showing the same thing at once. A key
/// press has no output, so no one screen is the right one; the one you are
/// looking at is among them either way.
#[derive(Clone)]
pub struct Osd {
    cards: Rc<PerMonitor<Card>>,
    /// What is on the cards, for a monitor plugged in while they are up.
    current: Rc<RefCell<Option<OsdDisplay>>>,
    hide: Rc<Timeout>,
    bar_route: Rc<RefCell<Option<BarRoute>>>,
    /// [`Osd::set_hold`]'s question.
    hold: Rc<RefCell<Option<Hold>>>,
    /// Set once the panel exists. Absent only in the standalone paths that
    /// never send a volume command.
    audio: Rc<RefCell<Option<Rc<swaypplet_audio::AudioService>>>>,
    showing: Rc<Cell<Showing>>,
    pending: Rc<RefCell<Pending>>,
    /// The rounded percentage and mute currently on the card. A snapshot
    /// that would redraw the same thing is dropped, because showing a card
    /// restarts its 1500 ms life and unrelated audio events are frequent.
    drawn: Rc<Cell<Option<(i32, bool)>>>,
}

impl Osd {
    pub fn new(app: &gtk4::Application) -> Self {
        let osd = Osd {
            cards: PerMonitor::new(),
            current: Rc::new(RefCell::new(None)),
            hide: Rc::default(),
            bar_route: Rc::new(RefCell::new(None)),
            hold: Rc::new(RefCell::new(None)),
            audio: Rc::new(RefCell::new(None)),
            showing: Rc::new(Cell::new(Showing::Other)),
            pending: Rc::new(RefCell::new(Pending::default())),
            drawn: Rc::new(Cell::new(None)),
        };
        // One card per output; a monitor plugged in while the cards are up
        // gets what the others show.
        let app = app.clone();
        let current = osd.current.clone();
        // Not on an extra screen unless switched on (`shell::extra`).
        swaypplet_ui::shell::extra::follow(&osd.cards, swaypplet_ui::shell::extra::Kind::Osd);
        osd.cards.watch(move |monitor| {
            let card = Card::new(&app, monitor);
            if let Some(display) = &*current.borrow() {
                card.draw(display);
                card.surface.show();
            }
            card
        });
        osd
    }

    /// Install the bar route (docs/BAR_VISION.md, increment 5): while the
    /// focused window is not fullscreen, volume/brightness displays render
    /// as the decision-slot interjection instead of the center card. Lock
    /// indicators (caps/num/scroll) always keep the card.
    pub fn set_bar_route(&self, route: impl Fn(&str, f64, &str) -> bool + 'static) {
        *self.bar_route.borrow_mut() = Some(Box::new(route));
    }

    /// Install the question "hold the card now?", asked per display: true
    /// while the screen is shared (`swaypplet_notifyd::context`).
    pub fn set_hold(&self, hold: impl Fn() -> bool + 'static) {
        *self.hold.borrow_mut() = Some(Box::new(hold));
    }

    /// Hand the OSD the sound server connection, so volume keys are answered
    /// from the same snapshot the panel's slider draws.
    pub fn set_audio(&self, audio: Rc<swaypplet_audio::AudioService>) {
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
        self.show_display(&volume_display(
            volume.volume,
            volume.muted,
            is_mic,
            keys().volume_ceiling(),
        ));
    }

    /// Note a press, and return the level to draw for it.
    ///
    /// Each press advances the pending level rather than the snapshot's, so
    /// ten of them in one burst move ten steps on the card and ten steps at
    /// the server. The settle timer hands the card back to [`reconcile`]
    /// once the presses stop.
    fn advance(&self, from: f64, delta: f64) -> f64 {
        let mut pending = self.pending.borrow_mut();
        let target = (pending.level.unwrap_or(from) + delta).clamp(0.0, keys().volume_ceiling());
        pending.level = Some(target);

        let slot = self.pending.clone();
        let osd = self.clone();
        pending
            .settle
            .arm(std::time::Duration::from_millis(SETTLE_MS), move || {
                slot.borrow_mut().level = None;
                osd.reconcile();
            });

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
        use swaypplet_audio::Command as AudioCommand;

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
                let step = f64::from(keys().volume_step) / 100.0;
                // The server clamps at its own 150 %; the setting's ceiling
                // is applied to what is asked for, so a press at the ceiling
                // asks for nothing rather than for a step the card must
                // then take back.
                let room = (keys().volume_ceiling() - current.volume).max(0.0);
                audio.send(AudioCommand::AdjustSinkVolume(step.min(room)));
                (self.advance(current.volume, step), current.muted, false)
            }
            OsdCommand::OutputVolumeLower => {
                self.showing.set(Showing::OutputVolume);
                let step = f64::from(keys().volume_step) / 100.0;
                audio.send(AudioCommand::AdjustSinkVolume(-step));
                (self.advance(current.volume, -step), current.muted, false)
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
        Some(volume_display(
            level,
            muted,
            is_mic,
            keys().volume_ceiling(),
        ))
    }

    /// A one-line notice on the indicator card: an icon over a word or
    /// two, on every output, gone after the usual 1.5 s. For a state change
    /// that has nothing else to show, such as pinning the workspace you are
    /// on (`jump/pin.rs`).
    pub fn notice(&self, icon: &str, label: &str) {
        self.showing.set(Showing::Other);
        self.show_display(&OsdDisplay::Indicator {
            icon: icon.to_string(),
            label: label.to_string(),
            active: true,
        });
    }

    fn show_display(&self, display: &OsdDisplay) {
        // Held while the screen is shared: the card would be in the capture.
        // The key itself has done its work by now; only the picture waits.
        if self.hold.borrow().as_ref().is_some_and(|hold| hold()) {
            log::debug!("osd: held while the screen is shared");
            return;
        }
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
        self.cards.for_each(|card| card.draw(display));
        *self.current.borrow_mut() = Some(display.clone());

        // Fade in (motion on glass, anim.rs). Retriggering mid-exit
        // reverses the fade from its current opacities; opacity never
        // unmaps the content, so the auto-sized surface stays put during
        // the transition (the old spacer overlay is gone with the
        // revealer).
        self.cards.for_each(|card| card.surface.show());

        // Auto-hide after timeout, in place of the previous one: fade out,
        // then unmap (Reveal hides the window when the exit finishes)
        let cards = self.cards.clone();
        let current = self.current.clone();
        let showing = self.showing.clone();
        let drawn = self.drawn.clone();
        self.hide.arm(
            std::time::Duration::from_millis(OSD_TIMEOUT_MS as u64),
            move || {
                showing.set(Showing::Other);
                drawn.set(None);
                current.replace(None);
                cards.for_each(|card| card.surface.hide());
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(display: OsdDisplay) -> (f64, String) {
        match display {
            OsdDisplay::Bar { fraction, text, .. } => (fraction, text),
            OsdDisplay::Indicator { .. } => panic!("a volume draws a bar"),
        }
    }

    #[test]
    fn volume_fills_the_bar_up_to_the_ceiling() {
        assert_eq!(
            bar(volume_display(0.5, false, false, 1.0)),
            (0.5, "50%".into())
        );
        assert_eq!(bar(volume_display(1.0, false, false, 1.0)).0, 1.0);
        // With boost on, the panel's slider ends at 150 %, and so does the bar.
        assert_eq!(bar(volume_display(0.75, false, false, 1.5)).0, 0.5);
        assert_eq!(bar(volume_display(1.5, false, false, 1.5)).0, 1.0);
    }

    #[test]
    fn volume_over_the_ceiling_fills_the_bar_and_keeps_the_true_number() {
        assert_eq!(
            bar(volume_display(1.2, false, false, 1.0)),
            (1.0, "120%".into())
        );
    }

    fn parse(line: &str) -> Option<OsdCommand> {
        let words: Vec<String> = line.split_whitespace().map(str::to_string).collect();
        OsdCommand::parse(&words)
    }

    #[test]
    fn each_flag_and_action_is_one_command() {
        use OsdCommand::*;
        for (line, cmd) in [
            ("--output-volume raise", OutputVolumeRaise),
            ("--output-volume lower", OutputVolumeLower),
            ("--output-volume mute-toggle", OutputVolumeMuteToggle),
            ("--input-volume mute-toggle", InputVolumeMuteToggle),
            ("--brightness raise", BrightnessRaise),
            ("--brightness lower", BrightnessLower),
            ("--caps-lock", CapsLock),
            ("--num-lock", NumLock),
            ("--scroll-lock", ScrollLock),
            // Words before the first flag are skipped.
            ("now --caps-lock", CapsLock),
        ] {
            assert_eq!(parse(line), Some(cmd), "{line}");
        }
    }

    #[test]
    fn a_flag_without_its_action_or_with_a_wrong_one_is_nothing() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("--output-volume"), None);
        assert_eq!(parse("--output-volume up"), None);
        assert_eq!(parse("--input-volume raise"), None);
        // The first flag decides, even when a later one would parse.
        assert_eq!(parse("--brightness up --caps-lock"), None);
    }

    #[test]
    fn the_drawn_key_is_the_rounded_percentage_and_the_mute() {
        assert_eq!(drawn_key(0.704, false), (70, false));
        assert_eq!(drawn_key(0.706, true), (71, true));
        assert_eq!(drawn_key(1.5, false), (150, false));
        // Two levels the card would draw as one number are one key.
        assert_eq!(drawn_key(0.701, false), drawn_key(0.699, false));
    }

    #[test]
    fn muted_volume_draws_an_empty_bar() {
        assert_eq!(
            bar(volume_display(0.8, true, false, 1.0)),
            (0.0, "Muted".into())
        );
    }
}
