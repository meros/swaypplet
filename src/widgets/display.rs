use std::cell::Cell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{Box, Label};

use crate::services::displays;
use crate::settings::store::{self, NightLight, NightSchedule};
use crate::spawn::spawn_work;
use crate::ui;
use crate::ui::icons;

// ── Data types ────────────────────────────────────────────────────────────────

/// One output as the section shows it, from sway's `get_outputs` over the
/// app's own IPC connection (no `swaymsg` process per refresh).
#[derive(Debug, Clone, PartialEq)]
struct OutputInfo {
    name: String,
    active: bool,
    /// Width, height and refresh in mHz; absent for disabled outputs.
    mode: Option<(i32, i32, i32)>,
    scale: Option<f64>,
    /// "Make Model", when sway knows them.
    product: Option<String>,
}

// ── Backend helpers ───────────────────────────────────────────────────────────

/// sway's outputs. Blocking (one IPC round trip); call from a worker.
fn get_outputs() -> Vec<OutputInfo> {
    let outputs = match crate::sway::ipc::connect().and_then(|mut c| c.get_outputs()) {
        Ok(o) => o,
        Err(e) => {
            log::warn!("display: get_outputs: {e}");
            return Vec::new();
        }
    };
    outputs
        .into_iter()
        .map(|o| OutputInfo {
            product: Some(format!("{} {}", o.make, o.model))
                .filter(|p| !p.trim().is_empty() && !p.contains("Unknown")),
            mode: o.current_mode.map(|m| (m.width, m.height, m.refresh)),
            scale: o.scale,
            active: o.active,
            name: o.name,
        })
        .collect()
}

/// "2880×1800 @ 60 Hz · scale 2" for an active output, "Off" otherwise.
fn describe(o: &OutputInfo) -> String {
    let Some((w, h, mhz)) = o.mode.filter(|m| m.0 > 0 && m.1 > 0) else {
        return "Off".to_string();
    };
    let mut s = format!("{w}×{h}");
    if mhz > 0 {
        s.push_str(&format!(" @ {} Hz", (mhz + 500) / 1000));
    }
    if let Some(scale) = o.scale.filter(|s| (*s - 1.0).abs() > 1e-6) {
        s.push_str(&format!(" · scale {}", trim(scale)));
    }
    s
}

/// A scale without trailing zeros: 2, 1.5, 1.25.
fn trim(v: f64) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

// ── Row builder ───────────────────────────────────────────────────────────────

/// Build a single output row and return it along with the widget that should be
/// refreshed when the toggle completes (`output_list`).
fn make_output_row(output: &OutputInfo, shared: &Rc<Cell<usize>>, output_list: &Box) -> Box {
    let title = match &output.product {
        Some(p) => format!("{} · {p}", output.name),
        None => output.name.clone(),
    };
    let r = ui::row(icons::DISPLAY, &title, &describe(output));
    let active_count = shared.get();

    // Disable button is suppressed when it would turn off the last active display.
    let can_disable = output.active && active_count > 1;
    let btn_label = if output.active { "Disable" } else { "Enable" };
    let toggle_btn = ui::button_with(
        ui::Face::Label(btn_label),
        ui::Kind::Secondary,
        ui::Size::Small,
    );
    if !can_disable && output.active {
        // Last active display: prevent disabling.
        toggle_btn.set_sensitive(false);
        toggle_btn.set_tooltip_text(Some("Cannot disable the only active display"));
    }

    // ── Toggle handler ────────────────────────────────────────────────────────
    {
        let name = output.name.clone();
        let active = output.active;
        let output_list_c = output_list.clone();
        let count = shared.clone();

        toggle_btn.connect_clicked(move |btn| {
            // The shared count, not the row's copy: two rapid Disable clicks
            // on two active displays must not both pass and leave none. The
            // first one takes its display off the count before sway answers.
            if active && count.get() <= 1 {
                btn.set_tooltip_text(Some("Cannot disable the only active display"));
                return;
            }
            if active {
                count.set(count.get() - 1);
            }
            btn.set_sensitive(false);
            let cmd = format!("output {name} {}", if active { "disable" } else { "enable" });
            let output_list_refresh = output_list_c.clone();
            let count = count.clone();
            crate::sway::ipc::run_command_result(&cmd, move |_ok| {
                refresh_list(&output_list_refresh, &count);
            });
        });
    }

    r.end.append(&toggle_btn);
    r.root
}

// ── List population ───────────────────────────────────────────────────────────

/// Read the outputs on a worker, then rebuild `list`.
fn refresh_list(list: &Box, active: &Rc<Cell<usize>>) {
    let (list, active) = (list.clone(), active.clone());
    spawn_work(get_outputs, move |outputs| populate_output_list_with_data(&list, &outputs, &active));
}

/// Clear `list` and rebuild it from pre-fetched output data.
fn populate_output_list_with_data(list: &Box, outputs: &[OutputInfo], active: &Rc<Cell<usize>>) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    active.set(outputs.iter().filter(|o| o.active).count());
    for output in outputs {
        list.append(&make_output_row(output, active, list));
    }
}

// ── Profiles ──────────────────────────────────────────────────────────────────

/// One profile: what it is to the displays now, and what can be done to it.
fn profile_row(name: &str, current: bool, fits: bool, first: bool) -> Box {
    let state = if current {
        "On screen"
    } else if fits {
        "Fits these displays"
    } else {
        "For other displays"
    };
    let r = ui::row(icons::DISPLAY_PROFILE, name, state);
    if fits && !current {
        let apply = ui::button_with(
            ui::Face::Label("Apply"),
            ui::Kind::Secondary,
            ui::Size::Small,
        );
        let n = name.to_string();
        apply.connect_clicked(move |_| displays::apply(&n));
        r.end.append(&apply);
    }
    if !first {
        let raise = ui::button_with(
            ui::Face::Glyph {
                glyph: icons::RAISE,
                tooltip: "Match before the profile above",
            },
            ui::Kind::Flat,
            ui::Size::Small,
        );
        let n = name.to_string();
        raise.connect_clicked(move |_| displays::raise(&n));
        r.end.append(&raise);
    }
    let delete = ui::button_with(
        ui::Face::Glyph {
            glyph: icons::CLOSE,
            tooltip: "Delete this profile",
        },
        ui::Kind::Flat,
        ui::Size::Small,
    );
    let n = name.to_string();
    delete.connect_clicked(move |_| displays::delete(&n));
    r.end.append(&delete);
    r.root
}

/// Rebuild the profile list from the service's view.
fn populate_profiles(group: &Box, list: &Box) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    let view = displays::view();
    group.set_visible(view.available);
    if view.profiles.is_empty() {
        let empty = ui::row(
            icons::DISPLAY_PROFILE,
            "No profiles",
            "Save this layout to restore it when these displays connect",
        );
        list.append(&empty.root);
    }
    for (i, (name, fits)) in view.profiles.iter().enumerate() {
        let current = view.current.as_deref() == Some(name.as_str());
        list.append(&profile_row(name, current, *fits, i == 0));
    }
}

// ── DisplaySection ────────────────────────────────────────────────────────────

pub struct DisplaySection {
    section: ui::Section,
    output_list: Box,
    /// Active outputs as last read, shared by every row's Disable.
    active: Rc<Cell<usize>>,
}

impl DisplaySection {
    pub fn new() -> Self {
        let section = ui::section(icons::DISPLAY, "Displays", "");
        ui::glyph::adopt(&section.icon, ui::Text::Title, ui::Tone::Fg);
        let output_list = ui::vbox(1);

        // ── Night light warmth, above the outputs ─────────────────────────────
        let night = ui::group(1);
        night.add_css_class("display-night");
        night.append(&ui::heading("Night Light"));
        // On or off, and when: the tile's switch and its schedule, here with
        // the warmth they apply.
        let current = store::current().night_light();
        let (on_row, on) = ui::switch_row("Night light", night_when(current.schedule));
        on.set_active(current.enabled);
        on.connect_active_notify(|s| {
            let on = s.is_active();
            store::edit::<NightLight>(|n| n.enabled = on);
        });
        night.append(&on_row.root);
        let night_row = ui::slider_row(
            "󰖔",
            f64::from(NightLight::MIN_K),
            f64::from(NightLight::DAY_K),
            100.0,
        );
        ui::glyph::adopt(&night_row.icon, ui::Text::Title, ui::Tone::Fg);
        night_row.scale.adjustment().set_page_increment(500.0);
        // The night's temperature (`night_light.night_k`): the night light
        // ramps to it over a second as the slider moves, and the settings
        // file is written once the drag rests.
        let night_k = store::current().night_light().night_k;
        night_row.scale.set_value(f64::from(night_k));
        night_row.value.set_label(&format!("{night_k}K"));
        {
            let val_lbl = night_row.value.clone();
            night_row.scale.connect_value_changed(move |s| {
                let temp = s.value().round() as u32;
                val_lbl.set_label(&format!("{temp}K"));
                store::edit::<NightLight>(|n| n.night_k = temp);
            });
        }
        {
            // A change from elsewhere (the settings file, the CLI) moves the
            // slider; the handler above then writes back what is already
            // there, which the store drops.
            let scale = night_row.scale.clone();
            let (on, when) = (on.clone(), on_row.subtitle.clone());
            store::observe(move || {
                let n = store::current().night_light();
                let k = f64::from(n.night_k);
                if (scale.value() - k).abs() >= 1.0 {
                    scale.set_value(k);
                }
                if on.is_active() != n.enabled {
                    on.set_active(n.enabled);
                }
                when.set_label(night_when(n.schedule));
            });
        }
        night.append(&night_row.root);

        // ── Profiles: kanshi's job (`services::displays`) ─────────────────────
        let profiles = ui::group(1);
        profiles.append(&ui::heading("Profiles"));
        let profile_list = ui::vbox(1);
        profiles.append(&profile_list);
        let save = ui::hbox(2);
        let name = gtk4::Entry::new();
        name.set_placeholder_text(Some("Profile name"));
        name.set_hexpand(true);
        ui::entry::adopt(&name, ui::FieldSize::Normal);
        let save_btn = ui::button_with(
            ui::Face::Label("Save current layout"),
            ui::Kind::Secondary,
            ui::Size::Small,
        );
        {
            let name_c = name.clone();
            let save_now = move || {
                let text = name_c.text();
                if !text.trim().is_empty() {
                    displays::save_current(&text);
                    name_c.set_text("");
                }
            };
            let s = save_now.clone();
            save_btn.connect_clicked(move |_| s());
            name.connect_activate(move |_| save_now());
        }
        save.append(&name);
        save.append(&save_btn);
        profiles.append(&save);
        populate_profiles(&profiles, &profile_list);

        let detail_box = ui::vbox(3);
        detail_box.append(&night);
        detail_box.append(&profiles);
        detail_box.append(&output_list);
        section.body.append(&detail_box);

        let display = Self {
            section,
            output_list,
            active: Rc::new(Cell::new(0)),
        };

        {
            // The outputs or the profiles changed: the list, the rows and the
            // summary follow. Only on a change; nothing polls.
            let output_list = display.output_list.clone();
            let summary = display.section.summary.clone();
            let active = display.active.clone();
            displays::observe(move || {
                populate_profiles(&profiles, &profile_list);
                refresh_outputs(&output_list, &summary, &active);
            });
        }

        display.refresh();
        display
    }

    /// Read the outputs again (on a worker) and rebuild the list and the
    /// summary.
    pub fn refresh(&self) {
        refresh_outputs(&self.output_list, &self.section.summary, &self.active);
    }

    /// Switch into page mode: the body alone, open at once.
    pub fn expand_for_page(&self) {
        self.section.show_as_page();
    }

    /// Return a reference to the root widget for embedding in the panel.
    pub fn widget(&self) -> &Box {
        &self.section.root
    }
}

/// Re-query the outputs off the main thread, then rebuild the list and the
/// summary: the profile on screen, and how many displays.
fn refresh_outputs(output_list: &Box, summary_text: &Label, active: &Rc<Cell<usize>>) {
    let output_list = output_list.clone();
    let summary_text = summary_text.clone();
    let active = active.clone();
    {
        spawn_work(get_outputs, move |outputs| {
            populate_output_list_with_data(&output_list, &outputs, &active);

            let active_count = outputs.iter().filter(|o| o.active).count();
            let summary = match active_count {
                0 => "No displays".to_string(),
                1 => outputs
                    .iter()
                    .find(|o| o.active)
                    .map(|o| o.name.clone())
                    .unwrap_or_default(),
                n => format!("{n} displays"),
            };
            let summary = match displays::view().current {
                Some(profile) => format!("{profile} · {summary}"),
                None => summary,
            };
            summary_text.set_label(&summary);
        });
    }
}

/// The night light's schedule, in words.
fn night_when(schedule: NightSchedule) -> &'static str {
    match schedule {
        NightSchedule::Sun => "Warm from dusk to dawn",
        NightSchedule::Always => "Warm all day",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn out(mode: Option<(i32, i32, i32)>, scale: Option<f64>) -> OutputInfo {
        OutputInfo { name: "eDP-1".into(), active: mode.is_some(), mode, scale, product: None }
    }

    #[test]
    fn an_output_is_described_with_its_scale_only_when_scaled() {
        assert_eq!(describe(&out(Some((2880, 1800, 60001)), Some(2.0))), "2880×1800 @ 60 Hz · scale 2");
        assert_eq!(describe(&out(Some((3840, 2160, 60000)), Some(1.5))), "3840×2160 @ 60 Hz · scale 1.5");
        assert_eq!(describe(&out(Some((1920, 1080, 144000)), Some(1.0))), "1920×1080 @ 144 Hz");
        assert_eq!(describe(&out(None, None)), "Off");
        // A headless output reports no refresh: no "@ 0 Hz".
        assert_eq!(describe(&out(Some((1000, 900, 0)), Some(1.0))), "1000×900");
    }
}
