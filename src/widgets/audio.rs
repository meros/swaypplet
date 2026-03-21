use std::cell::RefCell;
use std::process::Command;
use std::rc::Rc;
use std::thread;

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
        warn!("wpctl {:?} failed ({}): {}", args, out.status, stderr.trim());
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

/// Parse `wpctl status` to extract audio sinks and sources.
///
/// The relevant section looks like:
/// ```
///  Audio
///   ├─ Sinks:
///   │   41. Headphones           [vol: 1.00]
///   │ * 42. Speakers             [vol: 0.80]
///   └─ Sources:
///       43. Microphone           [vol: 1.00]
/// ```
fn parse_status_blocking() -> Option<(Vec<Device>, Vec<Device>)> {
    let out = wpctl_blocking(&["status"])?;

    let mut sinks: Vec<Device> = Vec::new();
    let mut sources: Vec<Device> = Vec::new();

    #[derive(PartialEq)]
    enum Section {
        None,
        Sinks,
        Sources,
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
        // Lines that are section headers for other things (Filters, Streams…) end our interest.
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

        // "42. Speakers  [vol: 0.80]"
        if let Some(dot_pos) = clean.find(". ") {
            let id_str = &clean[..dot_pos];
            if id_str.chars().all(|c| c.is_ascii_digit()) {
                let rest = &clean[dot_pos + 2..];
                // Raw name is everything before the first '[' (metadata)
                let raw_name = rest.split('[').next().unwrap_or(rest).trim();
                let name = clean_device_name(raw_name);

                let dev = Device {
                    id: id_str.to_string(),
                    name,
                    is_default,
                };
                match section {
                    Section::Sinks => sinks.push(dev),
                    Section::Sources => sources.push(dev),
                    Section::None => {}
                }
            }
        }
    }

    Some((sinks, sources))
}

/// Collect the full audio state. Called on a background thread.
fn read_state_blocking() -> FetchedState {
    let sink = get_volume_blocking("@DEFAULT_AUDIO_SINK@");
    let source = get_volume_blocking("@DEFAULT_AUDIO_SOURCE@");

    let Some((sinks, sources)) = parse_status_blocking() else {
        error!("wpctl status unavailable — WirePlumber may not be running");
        return FetchedState::Unavailable("WirePlumber not available".to_string());
    };

    FetchedState::Ok(AudioState {
        sink,
        source,
        sinks,
        sources,
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

struct Widgets {
    // Summary row (always visible)
    summary_icon: gtk4::Label,
    summary_text: gtk4::Label,
    summary_arrow: gtk4::Label,
    detail_revealer: gtk4::Revealer,
    // Output (sink)
    sink_row: VolumeRow,
    sink_devices: DeviceList,
    // Input (source)
    source_row: VolumeRow,
    source_row_container: gtk4::Box, // wraps source_row + source_devices, shown/hidden
    source_devices: DeviceList,
    // Content containers
    content: gtk4::Box,             // shown when wpctl is available
    unavailable: UnavailableBanner, // shown when wpctl is unavailable
}

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
        let summary_row = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(6)
            .build();
        summary_row.add_css_class("section-summary");

        let summary_icon = gtk4::Label::new(Some(icons::SPEAKER_HIGH));
        summary_icon.add_css_class("section-summary-icon");

        let summary_text = gtk4::Label::new(Some("—"));
        summary_text.add_css_class("section-summary-label");
        summary_text.set_hexpand(true);
        summary_text.set_xalign(0.0);
        summary_text.set_ellipsize(gtk4::pango::EllipsizeMode::End);

        let summary_arrow = gtk4::Label::new(Some("▸"));
        summary_arrow.add_css_class("section-expand-arrow");

        summary_row.append(&summary_icon);
        summary_row.append(&summary_text);
        summary_row.append(&summary_arrow);

        // ── Detail revealer (collapsed by default) ───────────────────────────
        let detail_revealer = gtk4::Revealer::builder()
            .transition_type(gtk4::RevealerTransitionType::SlideDown)
            .transition_duration(200)
            .reveal_child(false)
            .build();

        // Wire the summary row click to toggle the detail revealer.
        {
            let rev = detail_revealer.clone();
            let arrow = summary_arrow.clone();
            let gesture = gtk4::GestureClick::new();
            gesture.connect_released(move |_, _, _, _| {
                let revealed = rev.reveals_child();
                rev.set_reveal_child(!revealed);
                arrow.set_label(if revealed { "▸" } else { "▾" });
            });
            summary_row.add_controller(gesture);
        }

        root.append(&summary_row);
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
            summary_icon,
            summary_text,
            summary_arrow,
            detail_revealer,
            sink_row,
            sink_devices,
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

        section
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

    // ── Public API ────────────────────────────────────────────────────────────

    /// Kick off a non-blocking state refresh. The UI is updated asynchronously
    /// via the GLib main loop once the background thread completes.
    pub fn refresh(&self) {
        Self::schedule_refresh(self.widgets.clone(), self.updating.clone());
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }
}
