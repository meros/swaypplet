//! Audio section: output and input volume, device pickers, per-app mixer.
//!
//! All of the state comes from [`crate::services::audio`], which holds one connection to
//! the sound server and pushes a snapshot whenever anything changes. This file
//! used to own that too — a `wpctl status` parser, a second `wpctl` call per
//! device for its volume, and a 2-second timer polling for a default-device
//! change — and now owns none of it. The timer is the part worth naming: the
//! section polled twice a second forever so that plugging in headphones would
//! be noticed, and the server had an event for that the whole time.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;

use crate::icons;
use crate::services::audio::{AudioService, AudioState, Command, Device, Stream, VolumeState};
use crate::ui;

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
    /// The glyph inside `icon_btn`, which follows the volume and mute.
    icon: gtk4::Label,
    scale: gtk4::Scale,
    pct_label: gtk4::Label,
}

impl VolumeRow {
    fn new(is_mic: bool) -> Self {
        // Scale range: 0–150, drawn as a percentage of the server's
        // normal volume (1.0), so 150 is the over-amplification ceiling.
        // Marks at 0, 50, 100 and 150. Values >100 are over-amplification.
        let r = ui::slider_row(
            if is_mic {
                icons::MIC
            } else {
                icons::SPEAKER_HIGH
            },
            0.0,
            150.0,
            1.0,
        );
        ui::glyph(&r.icon, ui::Text::Title, ui::Tone::Fg);
        let icon_btn =
            ui::slider_icon_button(&r, if is_mic { "Mute input" } else { "Mute output" });
        r.scale.add_mark(0.0, gtk4::PositionType::Bottom, None);
        r.scale.add_mark(50.0, gtk4::PositionType::Bottom, None);
        r.scale
            .add_mark(100.0, gtk4::PositionType::Bottom, Some("100%"));
        r.scale.add_mark(150.0, gtk4::PositionType::Bottom, None);
        r.value.set_text("0%");
        r.value.set_width_chars(5);

        VolumeRow {
            container: r.root,
            icon_btn,
            icon: r.icon,
            scale: r.scale,
            pct_label: r.value,
        }
    }

    fn update(&self, state: &VolumeState, is_mic: bool) {
        self.icon.set_label(volume_icon(state, is_mic));
        let pct_val = (state.volume * 100.0).round();
        self.scale.set_value(pct_val);
        self.pct_label.set_text(&pct_text(state.volume));

        // Visual cue for over-amplification (> 100 %).
        ui::set_class(&self.scale, "over", state.volume > 1.0);
    }
}

// ── Device list ───────────────────────────────────────────────────────────────

struct DeviceList {
    container: gtk4::Box,
    /// The glyph each row carries: a speaker for outputs, a mic for inputs.
    icon: &'static str,
}

impl DeviceList {
    fn new(icon: &'static str) -> Self {
        DeviceList {
            container: ui::vbox(1),
            icon,
        }
    }

    /// Rebuild the device rows for the given list. The default device is
    /// the selected row.
    fn update(&self, devices: &[Device], on_select: impl Fn(String) + Clone + 'static) {
        // Remove all existing children.
        while let Some(child) = self.container.first_child() {
            self.container.remove(&child);
        }

        for device in devices {
            let (btn, _) = ui::row_button(self.icon, &device.name, "");
            ui::set_selected(&btn, device.is_default);
            let id = device.id.clone();
            let cb = on_select.clone();
            btn.connect_clicked(move |_| cb(id.clone()));
            self.container.append(&btn);
        }
    }
}

// ── Placeholder for a sound server that is not answering ─────────────────────

struct UnavailableBanner {
    label: gtk4::Label,
}

impl UnavailableBanner {
    fn new() -> Self {
        let label = ui::text("WirePlumber not available", ui::Text::Body, ui::Tone::Muted);
        UnavailableBanner { label }
    }
}

// ── AudioSection ──────────────────────────────────────────────────────────────

struct Widgets {
    // The section: its header (always visible) and body
    section: ui::Section,
    // Output (sink)
    sink_row: VolumeRow,
    sink_devices: DeviceList,
    // Per-application playback streams
    streams_container: gtk4::Box, // wraps toggle + revealer, hidden when no streams
    streams: ui::Disclosure,
    // Input (source)
    source_row: VolumeRow,
    source_row_container: gtk4::Box, // wraps source_row + source_devices, shown/hidden
    source_devices: DeviceList,
    // Content containers
    content: gtk4::Box,             // shown when the server is connected
    unavailable: UnavailableBanner, // shown when it is not
}

pub struct AudioSection {
    widgets: Rc<Widgets>,
    audio: Rc<AudioService>,
    /// Guard flag: true while we are programmatically updating the scale value
    /// so we don't feed our own update back as a user gesture.
    updating: Rc<RefCell<bool>>,
}

impl AudioSection {
    pub fn new(audio: Rc<AudioService>) -> Self {
        let section = ui::section(icons::SPEAKER_HIGH, "Audio", "—");
        ui::glyph(&section.icon, ui::Text::Title, ui::Tone::Fg);
        let detail_box = &section.body;

        // ── Unavailable banner (hidden by default) ───────────────────────────
        let unavailable = UnavailableBanner::new();
        unavailable.label.set_visible(false);
        detail_box.append(&unavailable.label);

        // ── Content box (all normal UI lives here) ───────────────────────────
        let content = ui::vbox(2);

        // ── Output volume row ────────────────────────────────────────────────
        let sink_row = VolumeRow::new(false);
        content.append(&sink_row.container);

        // ── Output device list (collapsible) ─────────────────────────────────
        let sink_devices = DeviceList::new(icons::SPEAKER_HIGH);
        let sinks = ui::disclosure("Output Devices");
        sinks.body.append(&sink_devices.container);
        content.append(&sinks.root);

        // ── Per-application streams (collapsible, hidden when empty) ─────────
        let streams = ui::disclosure("Applications");
        streams.root.set_visible(false);
        let streams_container = streams.root.clone();
        content.append(&streams.root);

        // ── Input section (conditionally visible) ────────────────────────────
        let source_row_container = ui::vbox(2);
        source_row_container.set_visible(false);

        let source_row = VolumeRow::new(true);
        source_row_container.append(&source_row.container);

        let source_devices = DeviceList::new(icons::MIC);
        let sources = ui::disclosure("Input Devices");
        sources.body.append(&source_devices.container);
        source_row_container.append(&sources.root);

        content.append(&source_row_container);
        detail_box.append(&content);

        // ── Advanced Audio Settings (pavucontrol / helvum) ───────────────────
        let adv_btn = ui::button(
            "󰕾  Advanced Audio Control (pavucontrol / helvum)",
            ui::Kind::Secondary,
        );
        adv_btn.add_css_class("section-launch-btn");
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
            section,
            sink_row,
            sink_devices,
            streams_container,
            streams,
            source_row,
            source_row_container,
            source_devices,
            content,
            unavailable,
        });

        let updating = Rc::new(RefCell::new(false));

        let section = AudioSection {
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
                ui::set_class(&w2.sink_row.scale, "over", vol_fraction > 1.0);
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
                ui::set_class(&w2.source_row.scale, "over", vol_fraction > 1.0);
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
            w.section.icon.set_label(icons::SPEAKER_MUTED);
            w.section.summary.set_label("Unavailable");
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
            w.section.icon.set_label(volume_icon(sink_state, false));
            w.section
                .summary
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
        let list = &w.streams.body;
        while let Some(child) = list.first_child() {
            list.remove(&child);
        }

        for stream in streams {
            let r = ui::slider_row(volume_icon(&stream.volume, false), 0.0, 150.0, 1.0);
            let mute_btn = ui::slider_icon_button(&r, "Mute");
            // Muted says so in the danger tone, the one place a stream row
            // carries colour.
            let tone = if stream.volume.muted {
                ui::Tone::Danger
            } else {
                ui::Tone::Fg
            };
            ui::glyph(&r.icon, ui::Text::Title, tone);
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

            let name = ui::text(&stream.name, ui::Text::Body, ui::Tone::Muted);
            name.set_width_chars(10);
            name.set_max_width_chars(14);
            name.set_ellipsize(gtk4::pango::EllipsizeMode::End);
            r.root.insert_child_after(&name, Some(&mute_btn));

            let scale = r.scale.clone();
            ui::set_class(&scale, "over", stream.volume.volume > 1.0);

            let pct_label = r.value.clone();
            pct_label.set_text(&pct_text(stream.volume.volume));
            pct_label.set_width_chars(5);

            scale.set_value((stream.volume.volume * 100.0).round());
            {
                let index = stream.index;
                let audio = audio.clone();
                let pct2 = pct_label.clone();
                scale.connect_value_changed(move |scale| {
                    let frac = scale.value() / 100.0;
                    audio.send(Command::SetStreamVolume { index, level: frac });
                    pct2.set_text(&pct_text(frac));
                    ui::set_class(scale, "over", frac > 1.0);
                });
            }

            list.append(&r.root);
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
        &self.widgets.section.root
    }

    /// The Helm page keeps the header (it carries the output's name and
    /// level) and opens the body and the mixer under it.
    pub fn expand_for_page(&self) {
        self.widgets.section.set_open(true);
        self.widgets.streams.set_open(true);
    }

    /// Dev-preview helper: reveal the detail pane and the mixer rows so a
    /// single headless screenshot shows them (see src/preview.rs).
    pub fn expand_for_preview(&self) {
        self.expand_for_page();
    }

    /// Clone of the output (sink) volume `gtk4::Scale` (range 0–150) so it can
    /// be hoisted to the start-menu top level. The clone shares the same
    /// underlying `GtkAdjustment`, so it stays in sync with this section.
    pub fn output_volume_scale(&self) -> gtk4::Scale {
        self.widgets.sink_row.scale.clone()
    }
}
