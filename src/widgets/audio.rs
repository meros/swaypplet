//! Audio section: output and input volume, device pickers, per-app mixer.
//!
//! All of the state comes from [`crate::audio`], which holds one connection to
//! the sound server and pushes a snapshot whenever anything changes. This file
//! used to own that too — a `wpctl status` parser, a second `wpctl` call per
//! device for its volume, and a 2-second timer polling for a default-device
//! change — and now owns none of it. The timer is the part worth naming: the
//! section polled twice a second forever so that plugging in headphones would
//! be noticed, and the server had an event for that the whole time.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;

use crate::audio::{AudioService, AudioState, Command, Device, Stream, VolumeState};
use crate::icons;

/// Marks the device currently in use in the sink and source pickers.
const ICON_ACTIVE_CHECK: &str = "●";

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

        // Scale range: 0–150, drawn as a percentage of the server's
        // normal volume (1.0), so 150 is the over-amplification ceiling.
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
            name_label.add_css_class("device-name");
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

// ── Placeholder for a sound server that is not answering ─────────────────────

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
    // Per-application playback streams
    streams_container: gtk4::Box, // wraps toggle + revealer, hidden when no streams
    streams_revealer: gtk4::Revealer,
    streams_list: gtk4::Box,
    // Input (source)
    source_row: VolumeRow,
    source_row_container: gtk4::Box, // wraps source_row + source_devices, shown/hidden
    source_devices: DeviceList,
    // Content containers
    content: gtk4::Box,             // shown when the server is connected
    unavailable: UnavailableBanner, // shown when it is not
}

pub struct AudioSection {
    root: gtk4::Box,
    widgets: Rc<Widgets>,
    audio: Rc<AudioService>,
    /// Guard flag: true while we are programmatically updating the scale value
    /// so we don't feed our own update back as a user gesture.
    updating: Rc<RefCell<bool>>,
}

impl AudioSection {
    pub fn new(audio: Rc<AudioService>) -> Self {
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

        // ── Advanced Audio Settings (pavucontrol / helvum) ───────────────────
        let adv_btn = gtk4::Button::builder()
            .label("󰕾  Advanced Audio Control (pavucontrol / helvum)")
            .halign(gtk4::Align::Fill)
            .build();
        adv_btn.add_css_class("network-adv-btn");
        adv_btn.connect_clicked(|_| {
            let _ = std::process::Command::new("pavucontrol")
                .spawn()
                .or_else(|_| std::process::Command::new("helvum").spawn())
                .or_else(|_| {
                    std::process::Command::new("ghostty")
                        .args(["-e", "pulsemixer"])
                        .spawn()
                });
        });
        detail_box.append(&adv_btn);

        let widgets = Rc::new(Widgets {
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
            widgets: widgets.clone(),
            audio: audio.clone(),
            updating: updating.clone(),
        };

        section.connect_signals();
        section.refresh();

        // Every later redraw is the server telling us something changed —
        // a device appearing, another application taking the volume down,
        // headphones going in. No timer, and nothing to miss between ticks.
        let audio_for_cb = audio.clone();
        audio.connect_change(move || {
            Self::apply_snapshot(&widgets, &updating, &audio_for_cb, &audio_for_cb.snapshot());
        });

        section
    }

    fn connect_signals(&self) {
        let w = self.widgets.clone();
        let updating = self.updating.clone();
        let audio = self.audio.clone();

        // ── Sink mute toggle ──────────────────────────────────────────────────
        {
            let audio = audio.clone();
            w.sink_row.icon_btn.connect_clicked(move |_| {
                // No refresh call: the server's subscription event brings the
                // new state back on its own, for this change and for one made
                // by anything else on the system.
                audio.send(Command::ToggleSinkMute);
            });
        }

        // ── Sink scale (fire-and-forget volume set) ───────────────────────────
        {
            let w2 = w.clone();
            let upd = updating.clone();
            let audio = audio.clone();
            w.sink_row.scale.connect_value_changed(move |scale| {
                if *upd.borrow() {
                    return;
                }
                let vol_fraction = scale.value() / 100.0;
                audio.send(Command::SetSinkVolume(vol_fraction));

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
            let audio = audio.clone();
            w.source_row.icon_btn.connect_clicked(move |_| {
                audio.send(Command::ToggleSourceMute);
            });
        }

        // ── Source scale ──────────────────────────────────────────────────────
        {
            let w2 = w.clone();
            let upd = updating.clone();
            let audio = audio.clone();
            w.source_row.scale.connect_value_changed(move |scale| {
                if *upd.borrow() {
                    return;
                }
                let vol_fraction = scale.value() / 100.0;
                audio.send(Command::SetSourceVolume(vol_fraction));

                w2.source_row.pct_label.set_text(&pct_text(vol_fraction));
                if vol_fraction > 1.0 {
                    w2.source_row.scale.add_css_class("overamplified");
                } else {
                    w2.source_row.scale.remove_css_class("overamplified");
                }
            });
        }
    }

    /// Draw a snapshot. The only entry point now: nothing here reads the
    /// server, it is handed the answer.
    fn apply_snapshot(
        w: &Rc<Widgets>,
        updating: &Rc<RefCell<bool>>,
        audio: &Rc<AudioService>,
        s: &AudioState,
    ) {
        if !s.connected {
            w.content.set_visible(false);
            w.unavailable.label.set_text("Sound server unavailable");
            w.unavailable.label.set_visible(true);
            w.summary_icon.set_label(icons::SPEAKER_MUTED);
            w.summary_text.set_label("Unavailable");
            return;
        }
        w.unavailable.label.set_visible(false);
        w.content.set_visible(true);
        Self::apply_state(w, updating, audio, s);
    }

    fn apply_state(
        w: &Rc<Widgets>,
        updating: &Rc<RefCell<bool>>,
        audio: &Rc<AudioService>,
        s: &AudioState,
    ) {
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
            let audio = audio.clone();
            w.sink_devices.update(&s.sinks, move |id| {
                audio.send(Command::SetDefaultSink(id));
            });
        }

        // Per-application playback streams
        w.streams_container.set_visible(!s.streams.is_empty());
        Self::rebuild_streams(w, audio, &s.streams);

        // Source section visibility
        let has_source = s.source.is_some();
        w.source_row_container.set_visible(has_source);

        if let Some(ref source_state) = s.source {
            w.source_row.update(source_state, true);
        }

        {
            let audio = audio.clone();
            w.source_devices.update(&s.sources, move |id| {
                audio.send(Command::SetDefaultSource(id));
            });
        }

        *updating.borrow_mut() = false;
    }

    /// Rebuild the per-application mixer rows. Rows are recreated from scratch
    /// on every refresh, so each slider is wired to its stream id directly and
    /// needs no `updating` guard: the initial value is set before the handler
    /// is connected.
    fn rebuild_streams(w: &Rc<Widgets>, audio: &Rc<AudioService>, streams: &[Stream]) {
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

            let mute_btn = gtk4::Button::with_label(volume_icon(&stream.volume, false));
            mute_btn.add_css_class("volume-icon-btn");
            if stream.volume.muted {
                mute_btn.add_css_class("muted");
            }
            {
                // Per-stream mute has no command of its own: the server takes
                // a volume of zero the same way, and one fewer command is one
                // fewer thing to keep in step with the panel.
                let index = stream.index;
                let audio = audio.clone();
                let muted = stream.volume.muted;
                let level = stream.volume.volume;
                mute_btn.connect_clicked(move |_| {
                    audio.send(Command::SetStreamVolume {
                        index,
                        level: if muted { level.max(0.1) } else { 0.0 },
                    });
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
            if stream.volume.volume > 1.0 {
                scale.add_css_class("overamplified");
            }

            let pct_label = gtk4::Label::new(Some(&pct_text(stream.volume.volume)));
            pct_label.add_css_class("volume-pct");
            pct_label.set_width_chars(5);
            pct_label.set_xalign(1.0);

            scale.set_value((stream.volume.volume * 100.0).round());
            {
                let index = stream.index;
                let audio = audio.clone();
                let pct2 = pct_label.clone();
                scale.connect_value_changed(move |scale| {
                    let frac = scale.value() / 100.0;
                    audio.send(Command::SetStreamVolume { index, level: frac });
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

    /// Redraw from the service's current snapshot.
    ///
    /// Kept for the callers that used to force a re-read after changing
    /// something (the OSD's volume keys). It no longer reads anything: the
    /// snapshot is already correct by the time anyone asks.
    pub fn refresh(&self) {
        // The Keys group's boost switch: the rail ends where the keys do.
        let ceiling = crate::settings::store::current().keys().volume_ceiling() * 100.0;
        let scale = self.output_volume_scale();
        if scale.adjustment().upper() != ceiling {
            scale.set_range(0.0, ceiling);
        }
        Self::apply_snapshot(
            &self.widgets,
            &self.updating,
            &self.audio,
            &self.audio.snapshot(),
        );
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    pub fn expand_for_page(&self) {
        self.widgets.detail_revealer.set_reveal_child(true);
        self.widgets.summary_arrow.set_label("▾");
        self.widgets.streams_revealer.set_reveal_child(true);
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
