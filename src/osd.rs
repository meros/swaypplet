use std::cell::RefCell;
use std::process::Command;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4_layer_shell::Edge;

use crate::layer_shell::{self, LayerShellConfig};

const OSD_TIMEOUT_MS: u32 = 1500;
const VOLUME_STEP_PLUS: &str = "5%+";
const VOLUME_STEP_MINUS: &str = "5%-";
const BRIGHTNESS_STEP_UP: &str = "5%+";
const BRIGHTNESS_STEP_DOWN: &str = "5%-";

use crate::icons;

// ── Commands ─────────────────────────────────────────────────────────────────

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
    Bar { icon: String, fraction: f64, text: String },
    Indicator { icon: String, label: String, active: bool },
}

// ── Action execution + state reading ─────────────────────────────────────────

fn execute_command(cmd: &OsdCommand) -> OsdDisplay {
    match cmd {
        OsdCommand::OutputVolumeRaise => {
            let _ = Command::new("wpctl")
                .args(["set-volume", "-l", "1.5", "@DEFAULT_AUDIO_SINK@", VOLUME_STEP_PLUS])
                .output();
            read_volume_display("@DEFAULT_AUDIO_SINK@", false)
        }
        OsdCommand::OutputVolumeLower => {
            let _ = Command::new("wpctl")
                .args(["set-volume", "@DEFAULT_AUDIO_SINK@", VOLUME_STEP_MINUS])
                .output();
            read_volume_display("@DEFAULT_AUDIO_SINK@", false)
        }
        OsdCommand::OutputVolumeMuteToggle => {
            let _ = Command::new("wpctl")
                .args(["set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"])
                .output();
            read_volume_display("@DEFAULT_AUDIO_SINK@", false)
        }
        OsdCommand::InputVolumeMuteToggle => {
            let _ = Command::new("wpctl")
                .args(["set-mute", "@DEFAULT_AUDIO_SOURCE@", "toggle"])
                .output();
            read_volume_display("@DEFAULT_AUDIO_SOURCE@", true)
        }
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
        OsdCommand::CapsLock => read_lock_display("capslock", icons::CAPS_ON, icons::CAPS_OFF, "CAPS"),
        OsdCommand::NumLock => read_lock_display("numlock", icons::NUM_ON, icons::NUM_OFF, "NUM"),
        OsdCommand::ScrollLock => read_lock_display("scrolllock", "S", "s", "SCROLL"),
    }
}

fn read_volume_display(target: &str, is_mic: bool) -> OsdDisplay {
    let output = Command::new("wpctl")
        .args(["get-volume", target])
        .output()
        .ok();

    let (volume, muted) = output
        .and_then(|o| {
            let text = String::from_utf8_lossy(&o.stdout).to_string();
            let rest = text.trim().strip_prefix("Volume:")?.to_string();
            let mut parts = rest.split_whitespace();
            let vol: f64 = parts.next()?.parse().ok()?;
            let muted = rest.contains("[MUTED]");
            Some((vol, muted))
        })
        .unwrap_or((0.0, false));

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
            let field = line.split(',').nth(3)?.trim().trim_end_matches('%').to_string();
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

pub struct Osd {
    window: gtk4::Window,
    icon_label: gtk4::Label,
    bar: gtk4::ProgressBar,
    text_label: gtk4::Label,
    // For indicator mode (caps lock etc.)
    indicator_label: gtk4::Label,
    bar_box: gtk4::Box,
    timeout_id: Rc<RefCell<Option<glib::SourceId>>>,
}

impl Osd {
    pub fn new(app: &gtk4::Application) -> Self {
        static OSD_CONFIG: LayerShellConfig = LayerShellConfig {
            namespace: "swaypplet-osd",
            default_width: None,
            default_height: None,
            anchors: &[(Edge::Bottom, true)],
            margins: &[(Edge::Bottom, 72)],
            keyboard_mode: gtk4_layer_shell::KeyboardMode::None,
        };
        let window = layer_shell::create_layer_window(app, &OSD_CONFIG);
        window.set_resizable(false);
        window.set_decorated(false);

        // Shadow wrapper — transparent padding gives room for drop shadow
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
        outer.add_css_class("osd-container");

        let icon_label = gtk4::Label::builder()
            .label("")
            .halign(gtk4::Align::Center)
            .valign(gtk4::Align::Center)
            .xalign(0.5)
            .yalign(0.5)
            .justify(gtk4::Justification::Center)
            .build();
        icon_label.add_css_class("osd-icon");

        // Bar mode: bar + percentage below
        let bar_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(8)
            .build();

        let bar = gtk4::ProgressBar::builder()
            .hexpand(true)
            .build();
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

        outer.append(&icon_label);
        outer.append(&bar_box);
        outer.append(&indicator_label);

        wrapper.append(&outer);
        window.set_child(Some(&wrapper));

        Osd {
            window,
            icon_label,
            bar,
            text_label,
            indicator_label,
            bar_box,
            timeout_id: Rc::new(RefCell::new(None)),
        }
    }

    pub fn trigger(&self, cmd: &OsdCommand) {
        let display = execute_command(cmd);
        self.show_display(&display);
    }

    fn show_display(&self, display: &OsdDisplay) {
        match display {
            OsdDisplay::Bar { icon, fraction, text } => {
                self.icon_label.set_label(icon);
                self.bar.set_fraction(*fraction);
                self.text_label.set_label(text);
                self.bar_box.set_visible(true);
                self.indicator_label.set_visible(false);
            }
            OsdDisplay::Indicator { icon, label, active } => {
                self.icon_label.set_label(icon);
                self.indicator_label.set_label(label);
                self.bar_box.set_visible(false);
                self.indicator_label.set_visible(true);
                if *active {
                    self.indicator_label.add_css_class("osd-indicator-active");
                } else {
                    self.indicator_label.remove_css_class("osd-indicator-active");
                }
            }
        }

        self.window.set_visible(true);

        // Cancel previous timeout
        if let Some(id) = self.timeout_id.borrow_mut().take() {
            id.remove();
        }

        // Auto-hide after timeout
        let window_c = self.window.clone();
        let timeout_ref = self.timeout_id.clone();
        let id = glib::timeout_add_local_once(
            std::time::Duration::from_millis(OSD_TIMEOUT_MS as u64),
            move || {
                window_c.set_visible(false);
                *timeout_ref.borrow_mut() = None;
            },
        );
        *self.timeout_id.borrow_mut() = Some(id);
    }
}
