use std::cell::RefCell;
use std::process::Command;
use std::rc::Rc;
use std::thread;
use std::time::Duration;

use gtk4::prelude::*;
use log::{error, warn};

use crate::icons;

const ICON_ACTIVE_CHECK: &str = "●";

// ── Data types ────────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
struct VolumeState {
    volume: f64, // 0.0 – 1.5
    muted: bool,
}

#[derive(Clone, Debug)]
struct Device {
    id: String,
    name: String,
    is_default: bool,
}

/// A per-application playback stream (PipeWire `Stream/Output/Audio` node).
#[derive(Clone, Debug)]
struct Stream {
    id: String,
    name: String,
    vol: VolumeState,
}

/// Result of a full state read. `None` means wpctl was unavailable.
#[derive(Clone, Debug)]
enum FetchedState {
    Ok(AudioState),
    Unavailable(String),
}

#[derive(Clone, Debug, Default)]
struct AudioState {
    sink: Option<VolumeState>,
    source: Option<VolumeState>,
    sinks: Vec<Device>,
    sources: Vec<Device>,
    streams: Vec<Stream>,
}

// ── Device name cleanup ───────────────────────────────────────────────────────

/// Strip common Intel/AMD audio controller prefixes from device names so that
/// the user sees concise names like "Speaker", "Headphones", or
/// "HDMI / DisplayPort 1".
///
/// Examples:
/// - "Lunar Lake-M HD Audio Controller Speaker"        → "Speaker"
/// - "Alder Lake PCH-P High Definition Audio Speaker"  → "Speaker"
/// - "AMD Rembrandt Radeon High Definition Audio HDMI / DisplayPort 1"
///                                                      → "HDMI / DisplayPort 1"
pub fn clean_device_name(name: &str) -> String {
    // Ordered from most-specific to least-specific so the first match wins.
    const PREFIXES: &[&str] = &[
        // Intel – generation-specific names (keep alphabetical within brand)
        "Alder Lake PCH-P High Definition Audio ",
        "Alder Lake-S PCH High Definition Audio ",
        "Alderlake-S HD Audio ",
        "Broadwell-U Audio Controller ",
        "Cannon Lake PCH cAVS ",
        "Comet Lake PCH cAVS ",
        "Ice Lake-LP Smart Sound Technology Audio Controller ",
        "Jasper Lake HD Audio ",
        "Lunar Lake-M HD Audio Controller ",
        "Meteor Lake-P HD Audio Controller ",
        "Raptor Lake-P/U/H cAVS ",
        "Skylake Audio Controller ",
        "Tiger Lake-LP Smart Sound Technology Audio Controller ",
        "Tiger Lake-H HD Audio Controller ",
        "Wildcat Point-LP High Definition Audio Controller ",
        // Intel – generic patterns
        "Intel High Definition Audio ",
        "Intel HD Audio ",
        // AMD – generation-specific
        "AMD Rembrandt Radeon High Definition Audio ",
        "AMD Renoir Radeon High Definition Audio ",
        "AMD Navi 21/23 HDMI/DP Audio ",
        "AMD Navi 31 HDMI/DP Audio ",
        "Raven/Raven2/Fenghuang HDMI/DP Audio ",
        "Starship/Matisse HD Audio Controller ",
        // AMD – generic
        "AMD High Definition Audio ",
        "AMD HD Audio ",
        // NVIDIA – generic
        "NVIDIA High Definition Audio ",
        "NVIDIA HD Audio ",
    ];

    for prefix in PREFIXES {
        if let Some(rest) = name.strip_prefix(prefix) {
            let cleaned = rest.trim();
            if !cleaned.is_empty() {
                return cleaned.to_string();
            }
        }
    }

    name.trim().to_string()
}

// ── wpctl helpers (blocking — run on background threads) ─────────────────────

/// Run wpctl synchronously. Returns `None` when the binary is missing or the
/// command exits with a non-zero status.
fn wpctl_blocking(args: &[&str]) -> Option<String> {
    let out = Command::new("wpctl")
        .args(args)
        .output()
        .map_err(|e| {
            warn!("wpctl spawn error: {e}");
        })
        .ok()?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        warn!(
            "wpctl {:?} failed ({}): {}",
            args,
            out.status,
            stderr.trim()
        );
        return None;
    }

    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn parse_volume_line(line: &str) -> Option<VolumeState> {
    // "Volume: 0.75" or "Volume: 0.75 [MUTED]"
    let rest = line.trim().strip_prefix("Volume:")?;
    let mut parts = rest.split_whitespace();
    let vol_str = parts.next()?;
    let volume: f64 = vol_str.parse().ok()?;
    let muted = rest.contains("[MUTED]");
    Some(VolumeState { volume, muted })
}

fn get_volume_blocking(target: &str) -> Option<VolumeState> {
    let out = wpctl_blocking(&["get-volume", target])?;
    parse_volume_line(&out)
}

/// (sinks, sources, playback streams as `(id, name)`).
type StatusSnapshot = (Vec<Device>, Vec<Device>, Vec<(String, String)>);

/// Parse `wpctl status` to extract audio sinks, sources, and playback
/// streams (as `(id, name)` — volumes are fetched separately).
///
/// The relevant section looks like:
/// ```
///  Audio
///   ├─ Sinks:
///   │   41. Headphones           [vol: 1.00]
///   │ * 42. Speakers             [vol: 0.80]
///   ├─ Sources:
///   │   43. Microphone           [vol: 1.00]
///   └─ Streams:
///        97. Firefox
///             98. output_FL    > Speakers:playback_FL    [active]
/// ```
///
/// Stream port lines link with `>` for playback and `<` for capture; only
/// streams with a `>` port are kept (a capture stream in the mixer would just
/// duplicate the mic row).
fn parse_status_blocking() -> Option<StatusSnapshot> {
    let out = wpctl_blocking(&["status"])?;
    Some(parse_status(&out))
}

fn parse_status(out: &str) -> StatusSnapshot {
    let mut sinks: Vec<Device> = Vec::new();
    let mut sources: Vec<Device> = Vec::new();
    // (id, name, has_playback_port)
    let mut streams: Vec<(String, String, bool)> = Vec::new();

    #[derive(PartialEq)]
    enum Section {
        None,
        Sinks,
        Sources,
        Streams,
    }

    let mut section = Section::None;
    let mut in_audio_block = false;

    for line in out.lines() {
        let stripped = line.trim();

        if stripped == "Audio" {
            in_audio_block = true;
            continue;
        }
        // Once we leave the Audio block (next top-level heading), stop.
        if in_audio_block
            && !line.starts_with(' ')
            && !line.starts_with('\t')
            && !stripped.is_empty()
            && stripped != "Audio"
        {
            in_audio_block = false;
        }
        if !in_audio_block {
            continue;
        }

        if stripped.contains("Sinks:") {
            section = Section::Sinks;
            continue;
        }
        if stripped.contains("Sources:") {
            section = Section::Sources;
            continue;
        }
        if stripped.contains("Streams:") {
            section = Section::Streams;
            continue;
        }
        // Lines that are section headers for other things (Devices, Filters…) end our interest.
        if stripped.ends_with(':') {
            section = Section::None;
            continue;
        }

        if section == Section::None {
            continue;
        }

        // Each device line contains a numeric ID followed by a dot.
        // Strip tree-drawing characters (│, ├, └, ─, *, spaces).
        let clean = stripped
            .trim_start_matches(['│', '├', '└', '─', ' ', '\t', '*'])
            .trim();

        let is_default = stripped.contains('*');

        if section == Section::Streams {
            // Port lines link the stream to a device: `>` playback, `<` capture.
            if clean.contains('>') || clean.contains('<') {
                if clean.contains('>')
                    && let Some(last) = streams.last_mut()
                {
                    last.2 = true;
                }
                continue;
            }
        }

        // "42. Speakers  [vol: 0.80]"
        if let Some(dot_pos) = clean.find(". ") {
            let id_str = &clean[..dot_pos];
            if id_str.chars().all(|c| c.is_ascii_digit()) {
                let rest = &clean[dot_pos + 2..];
                // Raw name is everything before the first '[' (metadata)
                let raw_name = rest.split('[').next().unwrap_or(rest).trim();
                let name = clean_device_name(raw_name);

                match section {
                    Section::Sinks | Section::Sources => {
                        let dev = Device {
                            id: id_str.to_string(),
                            name,
                            is_default,
                        };
                        if section == Section::Sinks {
                            sinks.push(dev);
                        } else {
                            sources.push(dev);
                        }
                    }
                    Section::Streams => {
                        streams.push((id_str.to_string(), name, false));
                    }
                    Section::None => {}
                }
            }
        }
    }

    let playback_streams = streams
        .into_iter()
        .filter(|(_, _, playback)| *playback)
        .map(|(id, name, _)| (id, name))
        .collect();

    (sinks, sources, playback_streams)
}

/// Get the PipeWire node ID for a default target (e.g. "@DEFAULT_AUDIO_SINK@").
/// Returns just the numeric ID string — cheap to call for change detection.
fn get_default_id(target: &str) -> Option<String> {
    let out = wpctl_blocking(&["inspect", target])?;
    // First line: "id 56, type PipeWire:Interface:Node"
    let first_line = out.lines().next()?;
    let rest = first_line.strip_prefix("id ")?;
    let id = rest.split(',').next()?.trim();
    Some(id.to_string())
}

/// Snapshot of default device IDs for change detection.
#[derive(Clone, Default, PartialEq)]
struct DefaultIds {
    sink: Option<String>,
    source: Option<String>,
}

fn read_default_ids_blocking() -> DefaultIds {
    DefaultIds {
        sink: get_default_id("@DEFAULT_AUDIO_SINK@"),
        source: get_default_id("@DEFAULT_AUDIO_SOURCE@"),
    }
}

/// Collect the full audio state. Called on a background thread.
fn read_state_blocking() -> FetchedState {
    let sink = get_volume_blocking("@DEFAULT_AUDIO_SINK@");
    let source = get_volume_blocking("@DEFAULT_AUDIO_SOURCE@");

    let Some((sinks, sources, stream_nodes)) = parse_status_blocking() else {
        error!("wpctl status unavailable — WirePlumber may not be running");
        return FetchedState::Unavailable("WirePlumber not available".to_string());
    };

    // A stream can vanish between the status read and the volume read — skip.
    let streams = stream_nodes
        .into_iter()
        .filter_map(|(id, name)| get_volume_blocking(&id).map(|vol| Stream { id, name, vol }))
        .collect();

    FetchedState::Ok(AudioState {
        sink,
        source,
        sinks,
        sources,
        streams,
    })
}

// ── Widget helpers ────────────────────────────────────────────────────────────

fn volume_icon(state: &VolumeState, is_mic: bool) -> &'static str {
    icons::volume_icon(state.volume, state.muted, is_mic)
}

fn pct_text(vol: f64) -> String {
    format!("{}%", (vol * 100.0).round() as u32)
}

// ── Volume row ────────────────────────────────────────────────────────────────

struct VolumeRow {
    container: gtk4::Box,
    icon_btn: gtk4::Button,
    scale: gtk4::Scale,
    pct_label: gtk4::Label,
}

impl VolumeRow {
    fn new(is_mic: bool) -> Self {
        let container = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(6)
            .build();
        container.add_css_class("volume-row");

        let icon_btn = gtk4::Button::with_label(if is_mic {
            icons::MIC
        } else {
            icons::SPEAKER_HIGH
        });
        icon_btn.add_css_class("volume-icon-btn");
        icon_btn.set_focusable(true);

        // Scale range: 0–150 (maps to 0–1.5 in wpctl units).
        // Marks at 0, 50, 100 and 150. Values >100 are over-amplification.
        let scale = gtk4::Scale::with_range(gtk4::Orientation::Horizontal, 0.0, 150.0, 1.0);
        scale.set_hexpand(true);
        scale.set_draw_value(false);
        scale.add_mark(0.0, gtk4::PositionType::Bottom, None);
        scale.add_mark(50.0, gtk4::PositionType::Bottom, None);
        scale.add_mark(100.0, gtk4::PositionType::Bottom, Some("100%"));
        scale.add_mark(150.0, gtk4::PositionType::Bottom, None);

        let pct_label = gtk4::Label::new(Some("0%"));
        pct_label.add_css_class("volume-pct");
        pct_label.set_width_chars(5);
        pct_label.set_xalign(1.0);

        container.append(&icon_btn);
        container.append(&scale);
        container.append(&pct_label);

        VolumeRow {
            container,
            icon_btn,
            scale,
            pct_label,
        }
    }

    fn update(&self, state: &VolumeState, is_mic: bool) {
        self.icon_btn.set_label(volume_icon(state, is_mic));
        let pct_val = (state.volume * 100.0).round();
        self.scale.set_value(pct_val);
        self.pct_label.set_text(&pct_text(state.volume));

        // Visual cue for over-amplification (> 100 %).
        if state.volume > 1.0 {
            self.scale.add_css_class("overamplified");
        } else {
            self.scale.remove_css_class("overamplified");
        }
    }
}

// ── Device list ───────────────────────────────────────────────────────────────

struct DeviceList {
    container: gtk4::Box,
}

impl DeviceList {
    fn new() -> Self {
        let container = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(2)
            .build();
        container.add_css_class("device-list");
        DeviceList { container }
    }

    /// Rebuild the device rows for the given list.
    fn update(&self, devices: &[Device], on_select: impl Fn(String) + Clone + 'static) {
        // Remove all existing children.
        while let Some(child) = self.container.first_child() {
            self.container.remove(&child);
        }

        for device in devices {
            let row = gtk4::Box::builder()
                .orientation(gtk4::Orientation::Horizontal)
                .spacing(8)
                .build();
            row.add_css_class("device-row");
            if device.is_default {
                row.add_css_class("device-row-active");
            }

            if device.is_default {
                let check = gtk4::Label::new(Some(ICON_ACTIVE_CHECK));
                check.add_css_class("device-active-dot");
                row.append(&check);
            } else {
                // Reserve the same width as the indicator to keep names aligned.
                let spacer = gtk4::Label::new(Some(" "));
                spacer.add_css_class("device-active-spacer");
                row.append(&spacer);
            }

            let name_label = gtk4::Label::new(Some(&device.name));
            name_label.set_hexpand(true);
            name_label.set_xalign(0.0);
            name_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
            row.append(&name_label);

            // Wrap in a GestureClick to make the whole row clickable.
            let gesture = gtk4::GestureClick::new();
            let id = device.id.clone();
            let cb = on_select.clone();
            gesture.connect_released(move |_, _, _, _| {
                cb(id.clone());
            });
            row.add_controller(gesture);

            row.set_focusable(true);
            row.set_can_focus(true);

            self.container.append(&row);
        }
    }
}

// ── Placeholder for wpctl-unavailable state ───────────────────────────────────

struct UnavailableBanner {
    label: gtk4::Label,
}

impl UnavailableBanner {
    fn new() -> Self {
        let label = gtk4::Label::new(Some("WirePlumber not available"));
        label.add_css_class("audio-unavailable");
        label.set_xalign(0.0);
        UnavailableBanner { label }
    }
}

// ── AudioSection ──────────────────────────────────────────────────────────────

#[allow(dead_code)]
struct Widgets {
    // Summary row (always visible)
    summary_btn: gtk4::Button,
    summary_icon: gtk4::Label,
    summary_text: gtk4::Label,
    summary_arrow: gtk4::Label,
    detail_revealer: gtk4::Revealer,
    // Output (sink)
    sink_row: VolumeRow,
    sink_devices: DeviceList,
    // Per-application playback streams
    streams_container: gtk4::Box, // wraps toggle + revealer, hidden when no streams
    streams_revealer: gtk4::Revealer,
    streams_list: gtk4::Box,
    // Input (source)
    source_row: VolumeRow,
    source_row_container: gtk4::Box, // wraps source_row + source_devices, shown/hidden
    source_devices: DeviceList,
    // Content containers
    content: gtk4::Box,             // shown when wpctl is available
    unavailable: UnavailableBanner, // shown when wpctl is unavailable
}

#[allow(dead_code)]
pub struct AudioSection {
    root: gtk4::Box,
    widgets: Rc<Widgets>,
    /// Guard flag: true while we are programmatically updating the scale value
    /// so we don't feed our own update back as a user gesture.
    updating: Rc<RefCell<bool>>,
}

impl AudioSection {
    pub fn new() -> Self {
        let root = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(6)
            .build();
        root.add_css_class("section");

        // ── Summary row (always visible, toggles detail revealer) ─────────────
        let summary_content = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(6)
            .build();

        let summary_icon = gtk4::Label::new(Some(icons::SPEAKER_HIGH));
        summary_icon.add_css_class("section-summary-icon");

        let summary_text = gtk4::Label::new(Some("—"));
        summary_text.add_css_class("section-summary-label");
        summary_text.set_hexpand(true);
        summary_text.set_xalign(0.0);
        summary_text.set_ellipsize(gtk4::pango::EllipsizeMode::End);

        let summary_arrow = gtk4::Label::new(Some("▸"));
        summary_arrow.add_css_class("section-expand-arrow");

        summary_content.append(&summary_icon);
        summary_content.append(&summary_text);
        summary_content.append(&summary_arrow);

        let summary_btn = gtk4::Button::builder().child(&summary_content).build();
        summary_btn.add_css_class("section-summary");

        // ── Detail revealer (collapsed by default) ───────────────────────────
        let detail_revealer = gtk4::Revealer::builder()
            .transition_type(gtk4::RevealerTransitionType::SlideDown)
            .transition_duration(200)
            .reveal_child(false)
            .build();

        // Wire the summary button click to toggle the detail revealer.
        {
            let rev = detail_revealer.clone();
            let arrow = summary_arrow.clone();
            summary_btn.connect_clicked(move |_| {
                let revealed = rev.reveals_child();
                rev.set_reveal_child(!revealed);
                arrow.set_label(if revealed { "▸" } else { "▾" });
            });
        }

        root.append(&summary_btn);
        root.append(&detail_revealer);

        // ── Detail content box ────────────────────────────────────────────────
        let detail_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(6)
            .build();
        detail_revealer.set_child(Some(&detail_box));

        // ── Unavailable banner (hidden by default) ───────────────────────────
        let unavailable = UnavailableBanner::new();
        unavailable.label.set_visible(false);
        detail_box.append(&unavailable.label);

        // ── Content box (all normal UI lives here) ───────────────────────────
        let content = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(6)
            .build();

        // ── Output volume row ────────────────────────────────────────────────
        let sink_row = VolumeRow::new(false);
        content.append(&sink_row.container);

        // ── Output device list (collapsible) ─────────────────────────────────
        let sink_devices = DeviceList::new();
        let sink_revealer = gtk4::Revealer::builder()
            .transition_type(gtk4::RevealerTransitionType::SlideDown)
            .transition_duration(200)
            .reveal_child(false)
            .child(&sink_devices.container)
            .build();
        let sink_toggle = gtk4::Button::builder()
            .label("▸ Output Devices")
            .hexpand(true)
            .build();
        sink_toggle.add_css_class("section-expander");
        {
            let rev = sink_revealer.clone();
            sink_toggle.connect_clicked(move |btn| {
                let revealed = rev.reveals_child();
                rev.set_reveal_child(!revealed);
                btn.set_label(if revealed {
                    "▸ Output Devices"
                } else {
                    "▾ Output Devices"
                });
            });
        }
        content.append(&sink_toggle);
        content.append(&sink_revealer);

        // ── Per-application streams (collapsible, hidden when empty) ─────────
        let streams_list = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(2)
            .build();
        streams_list.add_css_class("stream-list");
        let streams_revealer = gtk4::Revealer::builder()
            .transition_type(gtk4::RevealerTransitionType::SlideDown)
            .transition_duration(200)
            .reveal_child(false)
            .child(&streams_list)
            .build();
        let streams_toggle = gtk4::Button::builder()
            .label("▸ Applications")
            .hexpand(true)
            .build();
        streams_toggle.add_css_class("section-expander");
        {
            let rev = streams_revealer.clone();
            streams_toggle.connect_clicked(move |btn| {
                let revealed = rev.reveals_child();
                rev.set_reveal_child(!revealed);
                btn.set_label(if revealed {
                    "▸ Applications"
                } else {
                    "▾ Applications"
                });
            });
        }
        let streams_container = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(6)
            .visible(false)
            .build();
        streams_container.append(&streams_toggle);
        streams_container.append(&streams_revealer);
        content.append(&streams_container);

        // ── Input section (conditionally visible) ────────────────────────────
        let source_row_container = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(6)
            .visible(false)
            .build();
        source_row_container.add_css_class("source-section");

        let source_row = VolumeRow::new(true);
        source_row_container.append(&source_row.container);

        let source_devices = DeviceList::new();
        let source_revealer = gtk4::Revealer::builder()
            .transition_type(gtk4::RevealerTransitionType::SlideDown)
            .transition_duration(200)
            .reveal_child(false)
            .child(&source_devices.container)
            .build();
        let source_toggle = gtk4::Button::builder()
            .label("▸ Input Devices")
            .hexpand(true)
            .build();
        source_toggle.add_css_class("section-expander");
        {
            let rev = source_revealer.clone();
            source_toggle.connect_clicked(move |btn| {
                let revealed = rev.reveals_child();
                rev.set_reveal_child(!revealed);
                btn.set_label(if revealed {
                    "▸ Input Devices"
                } else {
                    "▾ Input Devices"
                });
            });
        }
        source_row_container.append(&source_toggle);
        source_row_container.append(&source_revealer);

        content.append(&source_row_container);
        detail_box.append(&content);

        let widgets = Rc::new(Widgets {
            summary_btn,
            summary_icon,
            summary_text,
            summary_arrow,
            detail_revealer,
            sink_row,
            sink_devices,
            streams_container,
            streams_revealer,
            streams_list,
            source_row,
            source_row_container,
            source_devices,
            content,
            unavailable,
        });

        let updating = Rc::new(RefCell::new(false));

        let section = AudioSection {
            root,
            widgets,
            updating,
        };

        section.connect_signals();
        section.refresh();
        section.start_default_monitor();

        section
    }

    /// Poll for default sink/source changes every 2 seconds. When the default
    /// device changes (e.g. headphones plugged in), trigger a full refresh so
    /// the volume slider and device list reflect the new default.
    fn start_default_monitor(&self) {
        let w = self.widgets.clone();
        let updating = self.updating.clone();
        let last_ids: Rc<RefCell<DefaultIds>> = Rc::new(RefCell::new(DefaultIds::default()));

        glib::timeout_add_local(Duration::from_secs(2), move || {
            let w2 = w.clone();
            let upd2 = updating.clone();
            let last = last_ids.clone();

            let (tx, rx) = std::sync::mpsc::channel::<DefaultIds>();
            thread::spawn(move || {
                let _ = tx.send(read_default_ids_blocking());
            });

            // Poll the one-shot channel from the GLib main loop (non-blocking).
            glib::idle_add_local_once(move || match rx.try_recv() {
                Ok(new_ids) => Self::apply_default_ids(&w2, &upd2, &last, new_ids),
                Err(_) => Self::poll_default_ids(w2, upd2, last, rx),
            });

            glib::ControlFlow::Continue
        });
    }

    /// Re-queue itself (non-blocking) until the background thread delivers
    /// the new default-device IDs.
    fn poll_default_ids(
        w: Rc<Widgets>,
        updating: Rc<RefCell<bool>>,
        last: Rc<RefCell<DefaultIds>>,
        rx: std::sync::mpsc::Receiver<DefaultIds>,
    ) {
        glib::idle_add_local_once(move || match rx.try_recv() {
            Ok(new_ids) => Self::apply_default_ids(&w, &updating, &last, new_ids),
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                Self::poll_default_ids(w, updating, last, rx);
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                error!("default-device monitor background thread disconnected unexpectedly");
            }
        });
    }

    fn apply_default_ids(
        w: &Rc<Widgets>,
        updating: &Rc<RefCell<bool>>,
        last: &Rc<RefCell<DefaultIds>>,
        new_ids: DefaultIds,
    ) {
        let mut prev = last.borrow_mut();
        if *prev != new_ids {
            *prev = new_ids;
            drop(prev);
            Self::schedule_refresh(w.clone(), updating.clone());
        }
    }

    fn connect_signals(&self) {
        let w = self.widgets.clone();
        let updating = self.updating.clone();

        // ── Sink mute toggle ──────────────────────────────────────────────────
        {
            let w2 = w.clone();
            let upd = updating.clone();
            w.sink_row.icon_btn.connect_clicked(move |_| {
                // Fire-and-forget: run wpctl on a background thread.
                thread::spawn(|| {
                    wpctl_blocking(&["set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"]);
                });
                Self::schedule_refresh(w2.clone(), upd.clone());
            });
        }

        // ── Sink scale (fire-and-forget volume set) ───────────────────────────
        {
            let w2 = w.clone();
            let upd = updating.clone();
            w.sink_row.scale.connect_value_changed(move |scale| {
                if *upd.borrow() {
                    return;
                }
                let raw = scale.value();
                let vol_fraction = raw / 100.0;
                let val_str = format!("{:.2}", vol_fraction);

                // Non-blocking: spawn and forget.
                thread::spawn(move || {
                    wpctl_blocking(&["set-volume", "@DEFAULT_AUDIO_SINK@", &val_str]);
                });

                // Update percentage label and overamp style immediately.
                w2.sink_row.pct_label.set_text(&pct_text(vol_fraction));
                if vol_fraction > 1.0 {
                    w2.sink_row.scale.add_css_class("overamplified");
                } else {
                    w2.sink_row.scale.remove_css_class("overamplified");
                }
            });
        }

        // ── Source mute toggle ────────────────────────────────────────────────
        {
            let w2 = w.clone();
            let upd = updating.clone();
            w.source_row.icon_btn.connect_clicked(move |_| {
                thread::spawn(|| {
                    wpctl_blocking(&["set-mute", "@DEFAULT_AUDIO_SOURCE@", "toggle"]);
                });
                Self::schedule_refresh(w2.clone(), upd.clone());
            });
        }

        // ── Source scale ──────────────────────────────────────────────────────
        {
            let w2 = w.clone();
            let upd = updating.clone();
            w.source_row.scale.connect_value_changed(move |scale| {
                if *upd.borrow() {
                    return;
                }
                let raw = scale.value();
                let vol_fraction = raw / 100.0;
                let val_str = format!("{:.2}", vol_fraction);

                thread::spawn(move || {
                    wpctl_blocking(&["set-volume", "@DEFAULT_AUDIO_SOURCE@", &val_str]);
                });

                w2.source_row.pct_label.set_text(&pct_text(vol_fraction));
                if vol_fraction > 1.0 {
                    w2.source_row.scale.add_css_class("overamplified");
                } else {
                    w2.source_row.scale.remove_css_class("overamplified");
                }
            });
        }
    }

    /// Schedule a full state refresh on a background thread. Results are
    /// delivered back to the main thread via `glib::idle_add_local_once`.
    fn schedule_refresh(w: Rc<Widgets>, updating: Rc<RefCell<bool>>) {
        let (tx, rx) = std::sync::mpsc::channel::<FetchedState>();

        thread::spawn(move || {
            let state = read_state_blocking();
            // Ignore send errors — the UI may have been destroyed.
            let _ = tx.send(state);
        });

        // Poll the one-shot channel from the GLib main loop.
        glib::idle_add_local_once(move || {
            match rx.try_recv() {
                Ok(fetched) => Self::apply_fetched(&w, &updating, fetched),
                Err(_) => {
                    // Background thread not done yet; re-queue.
                    Self::poll_until_ready(w, updating, rx);
                }
            }
        });
    }

    /// Re-queue itself until the background thread has delivered its result.
    fn poll_until_ready(
        w: Rc<Widgets>,
        updating: Rc<RefCell<bool>>,
        rx: std::sync::mpsc::Receiver<FetchedState>,
    ) {
        glib::idle_add_local_once(move || match rx.try_recv() {
            Ok(fetched) => Self::apply_fetched(&w, &updating, fetched),
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                Self::poll_until_ready(w, updating, rx);
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                error!("audio state background thread disconnected unexpectedly");
            }
        });
    }

    fn apply_fetched(w: &Rc<Widgets>, updating: &Rc<RefCell<bool>>, fetched: FetchedState) {
        match fetched {
            FetchedState::Unavailable(msg) => {
                error!("Audio section: {msg}");
                w.content.set_visible(false);
                w.unavailable.label.set_text(&msg);
                w.unavailable.label.set_visible(true);
                w.summary_icon.set_label(icons::SPEAKER_MUTED);
                w.summary_text.set_label("Unavailable");
            }
            FetchedState::Ok(state) => {
                w.unavailable.label.set_visible(false);
                w.content.set_visible(true);
                Self::apply_state(w, updating, &state);
            }
        }
    }

    fn apply_state(w: &Rc<Widgets>, updating: &Rc<RefCell<bool>>, s: &AudioState) {
        *updating.borrow_mut() = true;

        if let Some(ref sink_state) = s.sink {
            w.sink_row.update(sink_state, false);

            // Update the summary row.
            let pct = (sink_state.volume * 100.0).round() as u32;
            let default_sink_name = s
                .sinks
                .iter()
                .find(|d| d.is_default)
                .map(|d| d.name.as_str())
                .unwrap_or("Output");
            w.summary_icon.set_label(volume_icon(sink_state, false));
            w.summary_text
                .set_label(&format!("{pct}% · {default_sink_name}"));
        }

        // Device selectors for sinks
        {
            let w2 = w.clone();
            let upd2 = updating.clone();
            w.sink_devices.update(&s.sinks, move |id| {
                thread::spawn(move || {
                    wpctl_blocking(&["set-default", &id]);
                });
                Self::schedule_refresh(w2.clone(), upd2.clone());
            });
        }

        // Per-application playback streams
        w.streams_container.set_visible(!s.streams.is_empty());
        Self::rebuild_streams(w, updating, &s.streams);

        // Source section visibility
        let has_source = s.source.is_some();
        w.source_row_container.set_visible(has_source);

        if let Some(ref source_state) = s.source {
            w.source_row.update(source_state, true);
        }

        {
            let w2 = w.clone();
            let upd2 = updating.clone();
            w.source_devices.update(&s.sources, move |id| {
                thread::spawn(move || {
                    wpctl_blocking(&["set-default", &id]);
                });
                Self::schedule_refresh(w2.clone(), upd2.clone());
            });
        }

        *updating.borrow_mut() = false;
    }

    /// Rebuild the per-application mixer rows. Rows are recreated from scratch
    /// on every refresh, so each slider is wired to its stream id directly and
    /// needs no `updating` guard: the initial value is set before the handler
    /// is connected.
    fn rebuild_streams(w: &Rc<Widgets>, updating: &Rc<RefCell<bool>>, streams: &[Stream]) {
        while let Some(child) = w.streams_list.first_child() {
            w.streams_list.remove(&child);
        }

        for stream in streams {
            let row = gtk4::Box::builder()
                .orientation(gtk4::Orientation::Horizontal)
                .spacing(6)
                .build();
            row.add_css_class("volume-row");
            row.add_css_class("stream-row");

            let mute_btn = gtk4::Button::with_label(volume_icon(&stream.vol, false));
            mute_btn.add_css_class("volume-icon-btn");
            if stream.vol.muted {
                mute_btn.add_css_class("muted");
            }
            {
                let id = stream.id.clone();
                let w2 = w.clone();
                let upd2 = updating.clone();
                mute_btn.connect_clicked(move |_| {
                    let id = id.clone();
                    thread::spawn(move || {
                        wpctl_blocking(&["set-mute", &id, "toggle"]);
                    });
                    Self::schedule_refresh(w2.clone(), upd2.clone());
                });
            }

            let name = gtk4::Label::new(Some(&stream.name));
            name.add_css_class("stream-name");
            name.set_width_chars(10);
            name.set_max_width_chars(14);
            name.set_ellipsize(gtk4::pango::EllipsizeMode::End);
            name.set_xalign(0.0);

            let scale = gtk4::Scale::with_range(gtk4::Orientation::Horizontal, 0.0, 150.0, 1.0);
            scale.set_hexpand(true);
            scale.set_draw_value(false);
            if stream.vol.volume > 1.0 {
                scale.add_css_class("overamplified");
            }

            let pct_label = gtk4::Label::new(Some(&pct_text(stream.vol.volume)));
            pct_label.add_css_class("volume-pct");
            pct_label.set_width_chars(5);
            pct_label.set_xalign(1.0);

            scale.set_value((stream.vol.volume * 100.0).round());
            {
                let id = stream.id.clone();
                let pct2 = pct_label.clone();
                scale.connect_value_changed(move |scale| {
                    let frac = scale.value() / 100.0;
                    let val_str = format!("{frac:.2}");
                    let id = id.clone();
                    thread::spawn(move || {
                        wpctl_blocking(&["set-volume", &id, &val_str]);
                    });
                    pct2.set_text(&pct_text(frac));
                    if frac > 1.0 {
                        scale.add_css_class("overamplified");
                    } else {
                        scale.remove_css_class("overamplified");
                    }
                });
            }

            row.append(&mute_btn);
            row.append(&name);
            row.append(&scale);
            row.append(&pct_label);
            w.streams_list.append(&row);
        }
    }

    // ── Public API ────────────────────────────────────────────────────────────

    /// Kick off a non-blocking state refresh. The UI is updated asynchronously
    /// via the GLib main loop once the background thread completes.
    pub fn refresh(&self) {
        Self::schedule_refresh(self.widgets.clone(), self.updating.clone());
    }

    #[allow(dead_code)]
    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    /// Dev-preview helper: reveal the detail pane and the mixer rows so a
    /// single headless screenshot shows them (see src/preview.rs).
    pub fn expand_for_preview(&self) {
        self.widgets.detail_revealer.set_reveal_child(true);
        self.widgets.summary_arrow.set_label("▾");
        self.widgets.streams_revealer.set_reveal_child(true);
    }

    /// Clone of the output (sink) volume `gtk4::Scale` (range 0–150) so it can
    /// be hoisted to the start-menu top level. The clone shares the same
    /// underlying `GtkAdjustment`, so it stays in sync with this section.
    pub fn output_volume_scale(&self) -> gtk4::Scale {
        self.widgets.sink_row.scale.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Captured from a real `wpctl status` (Lunar Lake laptop, ffplay running,
    // plus a synthetic capture stream to exercise the `<` exclusion).
    const STATUS: &str = "\
Audio
 ├─ Devices:
 │      50. Lunar Lake-M HD Audio Controller    [alsa]
 │
 ├─ Sinks:
 │     125. Lunar Lake-M HD Audio Controller HDMI / DisplayPort 1 Output [vol: 1.00]
 │  *  128. Internal Speakers                   [vol: 1.00]
 │     194. Lunar Lake-M HD Audio Controller Headphones [vol: 0.00 MUTED]
 │
 ├─ Sources:
 │  *  130. Lunar Lake-M HD Audio Controller Digital Microphone [vol: 1.00]
 │
 ├─ Filters:
 │
 └─ Streams:
       186. SDL Application
             57. output_FR       > Pro 2:playback_AUX1\t[active]
            124. output_FL       > Pro 2:playback_AUX0\t[active]
       201. Chromium
            202. input_MONO      < Digital Microphone:capture_MONO\t[active]

Video
 └─ Streams:

Settings
 └─ Default Configured Devices:
         0. Audio/Sink    bluez_output.80_99_E7_E0_16_FC.1
         1. Audio/Source  alsa_input.pci-0000_00_1f.3-platform-sof_sdw.HiFi__Mic__source
";

    #[test]
    fn parses_sinks_sources_and_playback_streams() {
        let (sinks, sources, streams) = parse_status(STATUS);

        assert_eq!(sinks.len(), 3);
        assert_eq!(sinks[1].id, "128");
        assert_eq!(sinks[1].name, "Internal Speakers");
        assert!(sinks[1].is_default);
        assert!(!sinks[0].is_default);
        // clean_device_name strips the controller prefix
        assert_eq!(sinks[0].name, "HDMI / DisplayPort 1 Output");

        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].id, "130");
        assert!(sources[0].is_default);

        // Only the playback stream survives; the capture stream (`<` ports)
        // and the Video/Settings blocks are ignored.
        assert_eq!(
            streams,
            vec![("186".to_string(), "SDL Application".to_string())]
        );
    }

    #[test]
    fn empty_streams_section() {
        let (_, _, streams) =
            parse_status("Audio\n ├─ Sinks:\n │  *  1. X [vol: 1.0]\n └─ Streams:\n\nVideo\n");
        assert!(streams.is_empty());
    }
}
