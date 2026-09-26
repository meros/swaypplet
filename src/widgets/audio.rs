//! Audio section: output and input, their devices, and every application's
//! volume.
//!
//! All state comes from [`crate::services::audio`], which follows the sound
//! server's events. This file draws it, and draws it in place: device rows
//! and application rows are kept per id and updated, and a list is rebuilt
//! only when its members change. It used to rebuild every row on every
//! event, and since moving a slider is itself an event, the application
//! slider under the pointer was replaced mid-drag. A slider being held is
//! also never moved by an incoming snapshot ([`hold_guard`]).
//!
//! The input meter records the microphone, so it runs only when asked
//! ("Test microphone"), for at most [`METER_SECONDS`], and stops when the
//! section leaves the screen. The service leaves its stream out of the
//! recorder list, so the microphone indicator does not report the panel.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use gtk4::prelude::*;

use crate::services::audio::{AudioService, AudioState, Card, Command, Device, DeviceKind, Stream, VolumeState};
use crate::ui;
use crate::ui::icons;

/// How long one microphone test records.
const METER_SECONDS: u32 = 20;

fn volume_icon(state: &VolumeState, is_mic: bool) -> &'static str {
    icons::volume_icon(state.volume, state.muted, is_mic)
}

fn pct_text(vol: f64) -> String {
    format!("{}%", (vol * 100.0).round() as u32)
}

/// The glyph for a device of `kind`.
pub fn device_glyph(kind: DeviceKind) -> &'static str {
    match kind {
        DeviceKind::Speakers => icons::SPEAKERS,
        DeviceKind::Headphones => icons::HEADPHONES,
        DeviceKind::Headset => icons::HEADSET,
        DeviceKind::Hdmi => icons::TV,
        DeviceKind::Bluetooth => icons::BLUETOOTH,
        DeviceKind::Usb => icons::USB,
        DeviceKind::Microphone => icons::MIC,
        DeviceKind::Webcam => icons::WEBCAM,
    }
}

/// A Bluetooth profile in words: what it is for, not its codec.
pub fn profile_label(name: &str, description: &str) -> String {
    let n = name.to_ascii_lowercase();
    if n.starts_with("a2dp") {
        "High quality".into()
    } else if n.contains("headset") || n.contains("handsfree") || n.contains("hfp") || n.contains("hsp") {
        "Headset, with mic".into()
    } else {
        description.to_string()
    }
}

/// A flag that is true while the pointer holds `scale`, so a snapshot never
/// moves a slider out from under a drag.
fn hold_guard(scale: &gtk4::Scale) -> Rc<Cell<bool>> {
    let held = Rc::new(Cell::new(false));
    let press = gtk4::GestureClick::new();
    press.set_button(0);
    press.set_propagation_phase(gtk4::PropagationPhase::Capture);
    {
        let held = held.clone();
        press.connect_pressed(move |_, _, _, _| held.set(true));
    }
    {
        let held = held.clone();
        press.connect_released(move |_, _, _, _| held.set(false));
    }
    {
        let held = held.clone();
        press.connect_stopped(move |_| held.set(false));
    }
    scale.add_controller(press);
    held
}

// ── Volume row ────────────────────────────────────────────────────────────────

struct VolumeRow {
    container: gtk4::Box,
    icon_btn: gtk4::Button,
    icon: gtk4::Label,
    scale: gtk4::Scale,
    pct_label: gtk4::Label,
    held: Rc<Cell<bool>>,
}

impl VolumeRow {
    fn new(is_mic: bool) -> Self {
        // 0–150 %: 150 is the over-amplification ceiling.
        let r = ui::slider_row(if is_mic { icons::MIC } else { icons::SPEAKER_HIGH }, 0.0, 150.0, 1.0);
        ui::glyph::adopt(&r.icon, ui::Text::Title, ui::Tone::Fg);
        let icon_btn = r.icon_button(if is_mic { "Mute input" } else { "Mute output" });
        r.scale.add_mark(100.0, gtk4::PositionType::Bottom, Some("100%"));
        r.value.set_text("0%");
        r.value.set_width_chars(5);
        let held = hold_guard(&r.scale);
        VolumeRow {
            container: r.root,
            icon_btn,
            icon: r.icon,
            scale: r.scale,
            pct_label: r.value,
            held,
        }
    }

    fn update(&self, state: &VolumeState, is_mic: bool) {
        self.icon.set_label(volume_icon(state, is_mic));
        ui::set_tone(&self.icon, if state.muted { ui::Tone::Danger } else { ui::Tone::Fg });
        if self.held.get() {
            return;
        }
        self.scale.set_value((state.volume * 100.0).round());
        self.pct_label.set_text(&if state.muted { "Muted".to_string() } else { pct_text(state.volume) });
        ui::set_over_range(&self.scale, state.volume > 1.0);
    }
}

// ── Device picker ─────────────────────────────────────────────────────────────

/// A list of devices, the default one selected; keyed by id and updated in
/// place. Shared by the section and the media popover's output picker.
pub struct DevicePicker {
    pub container: gtk4::Box,
    rows: RefCell<HashMap<String, (gtk4::Button, ui::Row)>>,
    order: RefCell<Vec<String>>,
    on_select: Rc<dyn Fn(String)>,
}

impl DevicePicker {
    pub fn new(on_select: impl Fn(String) + 'static) -> Rc<Self> {
        Rc::new(DevicePicker {
            container: ui::vbox(1),
            rows: RefCell::default(),
            order: RefCell::default(),
            on_select: Rc::new(on_select),
        })
    }

    pub fn update(&self, devices: &[Device]) {
        let ids: Vec<String> = devices.iter().map(|d| d.id.clone()).collect();
        let mut rows = self.rows.borrow_mut();
        if *self.order.borrow() != ids {
            while let Some(child) = self.container.first_child() {
                self.container.remove(&child);
            }
            rows.retain(|id, _| ids.contains(id));
            for d in devices {
                let (button, row) = rows.entry(d.id.clone()).or_insert_with(|| {
                    let (button, row) = ui::row_button(device_glyph(d.kind), &d.name, &d.detail);
                    let (on_select, id) = (self.on_select.clone(), d.id.clone());
                    button.connect_clicked(move |_| on_select(id.clone()));
                    (button, row)
                });
                let _ = row;
                self.container.append(button);
            }
            *self.order.borrow_mut() = ids;
        }
        for d in devices {
            let Some((button, row)) = rows.get(&d.id) else { continue };
            row.icon.set_label(device_glyph(d.kind));
            row.title.set_label(&d.name);
            let detail = if d.available {
                d.detail.clone()
            } else if d.detail.is_empty() {
                "Not plugged in".to_string()
            } else {
                format!("{} · not plugged in", d.detail)
            };
            row.subtitle.set_label(&detail);
            row.subtitle.set_visible(!detail.is_empty());
            ui::set_selected(button, d.is_default);
            button.set_sensitive(d.available || d.is_default);
            button.set_tooltip_text(Some(if d.is_default { "In use" } else { "Use this device" }));
        }
    }
}

// ── Application mixer ─────────────────────────────────────────────────────────

struct AppRow {
    root: gtk4::Box,
    name: gtk4::Label,
    mute: gtk4::Button,
    glyph: gtk4::Label,
    scale: gtk4::Scale,
    pct: gtk4::Label,
    held: Rc<Cell<bool>>,
    /// The mute button's next action, read by its handler.
    muted: Rc<Cell<bool>>,
    updating: Rc<Cell<bool>>,
}

impl AppRow {
    fn new(audio: &Rc<AudioService>, stream: &Stream) -> Self {
        let r = ui::slider_row(icons::SPEAKER_HIGH, 0.0, 150.0, 1.0);
        ui::glyph::adopt(&r.icon, ui::Text::Title, ui::Tone::Fg);
        let glyph = r.icon.clone();
        let mute = r.icon_button("Mute");
        // The application's own icon beside its name, when the theme has it.
        let identity = ui::hbox(2);
        if let Some(icon) = stream.icon.as_deref().filter(|i| icon_exists(i)) {
            let image = gtk4::Image::from_icon_name(icon);
            image.set_pixel_size(20);
            identity.append(&image);
        }
        let name = ui::text(&stream.name, ui::Text::Body, ui::Tone::Fg);
        name.set_width_chars(9);
        name.set_max_width_chars(12);
        name.set_xalign(0.0);
        name.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        identity.append(&name);
        r.root.insert_child_after(&identity, Some(&mute));
        r.value.set_width_chars(5);
        let held = hold_guard(&r.scale);
        let muted = Rc::new(Cell::new(stream.volume.muted));
        let updating = Rc::new(Cell::new(false));
        {
            let (audio, muted, index) = (audio.clone(), muted.clone(), stream.index);
            mute.connect_clicked(move |_| {
                audio.send(Command::SetStreamMute {
                    index,
                    mute: !muted.get(),
                });
            });
        }
        {
            let (audio, index, pct, updating) = (audio.clone(), stream.index, r.value.clone(), updating.clone());
            r.scale.connect_value_changed(move |scale| {
                if updating.get() {
                    return;
                }
                let level = scale.value() / 100.0;
                audio.send(Command::SetStreamVolume { index, level });
                pct.set_text(&pct_text(level));
                ui::set_over_range(scale, level > 1.0);
            });
        }
        AppRow {
            root: r.root,
            name,
            mute,
            glyph,
            scale: r.scale,
            pct: r.value,
            held,
            muted,
            updating,
        }
    }

    fn update(&self, stream: &Stream) {
        self.name.set_label(&stream.name);
        self.muted.set(stream.volume.muted);
        self.glyph.set_label(volume_icon(&stream.volume, false));
        ui::set_tone(&self.glyph, if stream.volume.muted { ui::Tone::Danger } else { ui::Tone::Fg });
        self.mute.set_tooltip_text(Some(if stream.volume.muted { "Unmute" } else { "Mute" }));
        ui::set_tone(&self.name, if stream.volume.muted { ui::Tone::Muted } else { ui::Tone::Fg });
        if self.held.get() {
            return;
        }
        self.updating.set(true);
        self.scale.set_value((stream.volume.volume * 100.0).round());
        self.updating.set(false);
        self.pct.set_text(&if stream.volume.muted { "Muted".to_string() } else { pct_text(stream.volume.volume) });
        ui::set_over_range(&self.scale, stream.volume.volume > 1.0);
    }
}

fn icon_exists(name: &str) -> bool {
    gtk4::gdk::Display::default()
        .map(|d| gtk4::IconTheme::for_display(&d).has_icon(name))
        .unwrap_or(false)
}

// ── Section ───────────────────────────────────────────────────────────────────

struct Widgets {
    section: ui::Section,
    content: gtk4::Box,
    unavailable: gtk4::Label,
    sink_row: VolumeRow,
    outputs_head: gtk4::Label,
    outputs: Rc<DevicePicker>,
    profile_box: gtk4::Box,
    profile_chips: RefCell<Vec<(String, gtk4::Button)>>,
    profile_card: Cell<Option<u32>>,
    apps_head: gtk4::Label,
    apps: gtk4::Box,
    app_rows: RefCell<HashMap<u32, AppRow>>,
    app_order: RefCell<Vec<u32>>,
    input_box: gtk4::Box,
    source_row: VolumeRow,
    inputs: Rc<DevicePicker>,
    meter_btn: gtk4::Button,
    meter: gtk4::ProgressBar,
    metering: Cell<bool>,
    meter_timer: RefCell<Option<glib::SourceId>>,
}

pub struct AudioSection {
    widgets: Rc<Widgets>,
    audio: Rc<AudioService>,
    updating: Rc<Cell<bool>>,
}

impl AudioSection {
    pub fn new(audio: Rc<AudioService>) -> Self {
        let section = ui::section(icons::SPEAKER_HIGH, "Audio", "—");
        ui::glyph::adopt(&section.icon, ui::Text::Title, ui::Tone::Fg);

        let unavailable = ui::text(
            "The sound server is not answering. Audio settings return when it does.",
            ui::Text::Body,
            ui::Tone::Muted,
        );
        unavailable.set_wrap(true);
        unavailable.set_xalign(0.0);
        unavailable.set_visible(false);
        section.body.append(&unavailable);

        let content = ui::vbox(3);

        // ── Output ─────────────────────────────────────────────────────
        let sink_row = VolumeRow::new(false);
        content.append(&sink_row.container);
        let outputs_head = ui::overline("Output", ui::Tone::Muted);
        outputs_head.set_xalign(0.0);
        content.append(&outputs_head);
        let outputs = {
            let audio = audio.clone();
            DevicePicker::new(move |id| audio.send(Command::SetDefaultSink(id)))
        };
        content.append(&outputs.container);
        let profile_box = ui::pill_group(1);
        profile_box.set_halign(gtk4::Align::Start);
        profile_box.set_visible(false);
        content.append(&profile_box);

        // ── Applications ───────────────────────────────────────────────
        let apps_head = ui::overline("Apps", ui::Tone::Muted);
        apps_head.set_xalign(0.0);
        content.append(&apps_head);
        let apps = ui::vbox(1);
        content.append(&apps);

        // ── Input ──────────────────────────────────────────────────────
        let input_box = ui::vbox(2);
        let input_head = ui::overline("Input", ui::Tone::Muted);
        input_head.set_xalign(0.0);
        input_box.append(&input_head);
        let source_row = VolumeRow::new(true);
        input_box.append(&source_row.container);
        let inputs = {
            let audio = audio.clone();
            DevicePicker::new(move |id| audio.send(Command::SetDefaultSource(id)))
        };
        input_box.append(&inputs.container);
        let meter_line = ui::hbox(3);
        let meter_btn = ui::button_with(
            ui::Face::Label("Test microphone"),
            ui::Kind::Secondary,
            ui::Size::Small,
        );
        let meter = ui::progress(0.0);
        meter.set_hexpand(true);
        meter.set_valign(gtk4::Align::Center);
        meter.set_visible(false);
        meter_line.append(&meter_btn);
        meter_line.append(&meter);
        input_box.append(&meter_line);
        content.append(&input_box);

        section.body.append(&content);

        // The full mixer, for routing and per-port settings, when installed.
        let advanced = ["pavucontrol", "helvum"]
            .into_iter()
            .find(|p| glib::find_program_in_path(p).is_some());
        if let Some(program) = advanced {
            let open = ui::button("Sound mixer…", ui::Kind::Secondary);
            open.set_halign(gtk4::Align::Start);
            open.connect_clicked(move |_| {
                let _ = std::process::Command::new(program).spawn();
            });
            section.body.append(&open);
        }

        let widgets = Rc::new(Widgets {
            section,
            content,
            unavailable,
            sink_row,
            outputs_head,
            outputs,
            profile_box,
            profile_chips: RefCell::default(),
            profile_card: Cell::new(None),
            apps_head,
            apps,
            app_rows: RefCell::default(),
            app_order: RefCell::default(),
            input_box,
            source_row,
            inputs,
            meter_btn,
            meter,
            metering: Cell::new(false),
            meter_timer: RefCell::new(None),
        });

        let section = AudioSection {
            widgets: widgets.clone(),
            audio: audio.clone(),
            updating: Rc::new(Cell::new(false)),
        };
        section.connect_signals();
        section.refresh();

        {
            let (w, updating, audio_c) = (widgets.clone(), section.updating.clone(), audio.clone());
            audio.connect_change(move || Self::apply(&w, &updating, &audio_c, &audio_c.snapshot()));
        }
        {
            let (w, audio_c) = (widgets.clone(), audio.clone());
            audio.connect_level(move || {
                if w.metering.get() {
                    w.meter.set_fraction(f64::from(audio_c.level()).sqrt());
                }
            });
        }
        section
    }

    fn connect_signals(&self) {
        let w = self.widgets.clone();
        let audio = self.audio.clone();
        let updating = self.updating.clone();

        {
            let audio = audio.clone();
            w.sink_row.icon_btn.connect_clicked(move |_| audio.send(Command::ToggleSinkMute));
        }
        {
            let (w2, upd, audio) = (w.clone(), updating.clone(), audio.clone());
            w.sink_row.scale.connect_value_changed(move |scale| {
                if upd.get() {
                    return;
                }
                let level = scale.value() / 100.0;
                audio.send(Command::SetSinkVolume(level));
                w2.sink_row.pct_label.set_text(&pct_text(level));
                ui::set_over_range(&w2.sink_row.scale, level > 1.0);
            });
        }
        {
            let audio = audio.clone();
            w.source_row.icon_btn.connect_clicked(move |_| audio.send(Command::ToggleSourceMute));
        }
        {
            let (w2, upd, audio) = (w.clone(), updating.clone(), audio.clone());
            w.source_row.scale.connect_value_changed(move |scale| {
                if upd.get() {
                    return;
                }
                let level = scale.value() / 100.0;
                audio.send(Command::SetSourceVolume(level));
                w2.source_row.pct_label.set_text(&pct_text(level));
            });
        }
        // The microphone test: on for METER_SECONDS, or until pressed again
        // or the section leaves the screen.
        {
            let (w2, audio) = (w.clone(), audio.clone());
            w.meter_btn.connect_clicked(move |_| {
                let on = !w2.metering.get();
                set_meter(&w2, &audio, on);
            });
        }
        {
            let (w2, audio) = (w.clone(), audio.clone());
            w.section.root.connect_unmap(move |_| set_meter(&w2, &audio, false));
        }
    }

    fn apply(w: &Rc<Widgets>, updating: &Rc<Cell<bool>>, audio: &Rc<AudioService>, s: &AudioState) {
        if !s.connected {
            w.content.set_visible(false);
            w.unavailable.set_visible(true);
            w.section.icon.set_label(icons::SPEAKER_MUTED);
            w.section.summary.set_label("Unavailable");
            return;
        }
        w.unavailable.set_visible(false);
        w.content.set_visible(true);
        updating.set(true);

        let default_sink = s.sinks.iter().find(|d| d.is_default);
        if let Some(state) = &s.sink {
            w.sink_row.update(state, false);
            w.section.icon.set_label(volume_icon(state, false));
            let name = default_sink.map_or("Output", |d| d.name.as_str());
            let level = if state.muted { "Muted".to_string() } else { pct_text(state.volume) };
            w.section.summary.set_label(&format!("{level} · {name}"));
        }
        w.outputs.update(&s.sinks);
        w.outputs_head.set_visible(s.sinks.len() > 1);
        w.outputs.container.set_visible(s.sinks.len() > 1);

        let card = default_sink
            .and_then(|d| d.card)
            .and_then(|c| s.cards.iter().find(|card| card.index == c));
        Self::apply_profiles(w, audio, card);

        Self::apply_apps(w, audio, &s.streams);

        w.input_box.set_visible(s.source.is_some());
        if let Some(state) = &s.source {
            w.source_row.update(state, true);
        }
        w.inputs.update(&s.sources);
        w.inputs.container.set_visible(s.sources.len() > 1);

        updating.set(false);
    }

    /// The Bluetooth profile switch, for a Bluetooth output with more than
    /// one usable profile.
    fn apply_profiles(w: &Rc<Widgets>, audio: &Rc<AudioService>, card: Option<&Card>) {
        let usable: Vec<_> = card
            .map(|c| c.profiles.iter().filter(|p| p.available).collect())
            .unwrap_or_default();
        let Some(card) = card.filter(|_| usable.len() > 1) else {
            w.profile_box.set_visible(false);
            return;
        };
        w.profile_box.set_visible(true);
        let names: Vec<String> = usable.iter().map(|p| p.name.clone()).collect();
        let same = w.profile_card.get() == Some(card.index)
            && w.profile_chips.borrow().iter().map(|(n, _)| n.clone()).collect::<Vec<_>>() == names;
        if !same {
            while let Some(child) = w.profile_box.first_child() {
                w.profile_box.remove(&child);
            }
            let mut chips = Vec::new();
            for p in &usable {
                let label = profile_label(&p.name, &p.description);
                let chip = ui::chip(ui::Face::Label(&label));
                chip.set_tooltip_text(Some(&p.description));
                let (audio, card, profile) = (audio.clone(), card.index, p.name.clone());
                chip.connect_clicked(move |_| {
                    audio.send(Command::SetCardProfile {
                        card,
                        profile: profile.clone(),
                    })
                });
                w.profile_box.append(&chip);
                chips.push((p.name.clone(), chip));
            }
            *w.profile_chips.borrow_mut() = chips;
            w.profile_card.set(Some(card.index));
        }
        for (name, chip) in w.profile_chips.borrow().iter() {
            ui::set_selected(chip, card.active.as_deref() == Some(name.as_str()));
        }
    }

    fn apply_apps(w: &Rc<Widgets>, audio: &Rc<AudioService>, streams: &[Stream]) {
        let order: Vec<u32> = streams.iter().map(|s| s.index).collect();
        w.apps_head.set_visible(!streams.is_empty());
        let mut rows = w.app_rows.borrow_mut();
        if *w.app_order.borrow() != order {
            while let Some(child) = w.apps.first_child() {
                w.apps.remove(&child);
            }
            rows.retain(|index, _| order.contains(index));
            for stream in streams {
                let row = rows.entry(stream.index).or_insert_with(|| AppRow::new(audio, stream));
                w.apps.append(&row.root);
            }
            *w.app_order.borrow_mut() = order;
        }
        for stream in streams {
            if let Some(row) = rows.get(&stream.index) {
                row.update(stream);
            }
        }
    }

    /// Redraw from the service's current snapshot.
    pub fn refresh(&self) {
        // The Keys group's boost switch: the rail ends where the keys do.
        let ceiling = crate::settings::store::current().keys().volume_ceiling() * 100.0;
        let scale = self.output_volume_scale();
        if scale.adjustment().upper() != ceiling {
            scale.set_range(0.0, ceiling);
        }
        Self::apply(&self.widgets, &self.updating, &self.audio, &self.audio.snapshot());
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.widgets.section.root
    }

    /// The Helm page keeps the header (it carries the output's name and
    /// level) and opens the body under it.
    pub fn expand_for_page(&self) {
        self.widgets.section.set_open(true);
    }

    /// Dev preview: the open body, and the meter as if a test were running.
    pub fn expand_for_preview(&self) {
        self.expand_for_page();
        let w = &self.widgets;
        w.metering.set(true);
        w.meter.set_visible(true);
        w.meter.set_fraction(f64::from(self.audio.level()).sqrt());
        w.meter_btn.set_label("Stop test");
    }

    /// The output volume scale, hoisted to the start menu's top level. The
    /// clone shares the adjustment, so both stay in step.
    pub fn output_volume_scale(&self) -> gtk4::Scale {
        self.widgets.sink_row.scale.clone()
    }
}

/// Start or stop the microphone test.
fn set_meter(w: &Rc<Widgets>, audio: &Rc<AudioService>, on: bool) {
    if w.metering.replace(on) == on {
        return;
    }
    audio.send(Command::Meter(on));
    w.meter.set_visible(on);
    w.meter.set_fraction(0.0);
    w.meter_btn.set_label(if on { "Stop test" } else { "Test microphone" });
    if let Some(id) = w.meter_timer.borrow_mut().take() {
        id.remove();
    }
    if on {
        let (w2, audio) = (w.clone(), audio.clone());
        let id = glib::timeout_add_seconds_local_once(METER_SECONDS, move || {
            w2.meter_timer.borrow_mut().take();
            set_meter(&w2, &audio, false);
        });
        *w.meter_timer.borrow_mut() = Some(id);
    }
}

/// "Play on": the outputs as a picker, for the media popover. Hidden when
/// there is only one output to choose.
pub fn output_picker(audio: &Rc<AudioService>) -> gtk4::Box {
    let root = ui::vbox(1);
    let head = ui::overline("Play on", ui::Tone::Muted);
    head.set_xalign(0.0);
    root.append(&head);
    let picker = {
        let audio = audio.clone();
        DevicePicker::new(move |id| audio.send(Command::SetDefaultSink(id)))
    };
    root.append(&picker.container);
    let draw = {
        let (root, picker, audio) = (root.clone(), picker.clone(), audio.clone());
        move || {
            let s = audio.snapshot();
            root.set_visible(s.connected && s.sinks.len() > 1);
            picker.update(&s.sinks);
        }
    };
    draw();
    audio.connect_change(draw);
    root
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bluetooth_profiles_say_what_they_are_for() {
        assert_eq!(profile_label("a2dp-sink-aac", "High Fidelity Playback (A2DP Sink, codec AAC)"), "High quality");
        assert_eq!(profile_label("headset-head-unit", "Headset Head Unit (HSP/HFP)"), "Headset, with mic");
        assert_eq!(profile_label("handsfree_head_unit", "x"), "Headset, with mic");
        assert_eq!(profile_label("pro-audio", "Pro Audio"), "Pro Audio");
    }

    #[test]
    fn every_kind_has_a_glyph_of_its_own() {
        let kinds = [
            DeviceKind::Speakers,
            DeviceKind::Headphones,
            DeviceKind::Headset,
            DeviceKind::Hdmi,
            DeviceKind::Bluetooth,
            DeviceKind::Usb,
            DeviceKind::Microphone,
            DeviceKind::Webcam,
        ];
        let glyphs: std::collections::HashSet<_> = kinds.iter().map(|k| device_glyph(*k)).collect();
        assert_eq!(glyphs.len(), kinds.len());
    }
}
