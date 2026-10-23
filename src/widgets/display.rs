//! The Helm's displays page: what you do to the displays now. The night
//! light on or off, a saved layout applied, and what is connected. How the
//! displays are arranged, the profiles kept, and how warm the night is are
//! configured once, in settings (Displays, Appearance), which the page's
//! last row opens.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{Box, Label};

use crate::services::cast::{self, backend::Target};
use crate::services::devices::{self, DeviceKey};
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
    /// The automatic name and the key a given one is stored under
    /// (`displays::naming`).
    label: String,
    key: String,
}

impl OutputInfo {
    /// The name the person gave this screen, else the automatic one. Main
    /// thread only.
    fn shown_name(&self) -> String {
        devices::display_name(Some(&DeviceKey::Display(self.key.clone())), &self.label)
    }
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
            label: displays::naming::auto_name(&o.name, &o.make, &o.model),
            key: displays::naming::key(&o.name, &o.make, &o.model, &o.serial),
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

// ── Outputs ───────────────────────────────────────────────────────────────────

/// One output, read only: what it is and what it runs at. Turning one on or
/// off is arranging them, which settings does with the layout in view.
fn output_row(output: &OutputInfo) -> Box {
    // The connector and the product go under the name, where they stay
    // findable.
    let mut subtitle = vec![output.name.clone()];
    subtitle.extend(output.product.iter().map(|p| p.trim().to_string()));
    subtitle.push(describe(output));
    let name = output.shown_name();
    let r = ui::row(icons::DISPLAY, &name, &subtitle.join(" · "));
    let key = DeviceKey::Display(output.key.clone());
    r.end.append(&crate::widgets::rename::button(
        &r.title,
        move || name.clone(),
        &output.label,
        move |name| devices::rename(&key, &name),
    ));
    r.root
}

/// Clear `list` and rebuild it from pre-fetched output data.
fn populate_output_list_with_data(list: &Box, outputs: &[OutputInfo]) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    for output in outputs {
        list.append(&output_row(output));
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
    // A saved layout cannot be put back: the first press asks.
    ui::arm(&delete, "Delete?", move |_| displays::delete(&n));
    r.end.append(&delete);
    r.root
}

/// The profiles a click can put on screen now: those that fit these
/// displays, with Apply, and the one on screen, marked. Ordering, deleting
/// and saving them is the Displays settings pane's (`fill_profiles`). The
/// group hides when there is nothing to pick, or no output management.
fn populate_picks(group: &Box, list: &Box) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    let view = displays::view();
    let mut any = false;
    for (name, fits) in &view.profiles {
        let current = view.current.as_deref() == Some(name.as_str());
        if !(current || *fits) {
            continue;
        }
        any = true;
        let r = ui::row(
            icons::DISPLAY_PROFILE,
            name,
            if current {
                "On screen"
            } else {
                "Fits these displays"
            },
        );
        if !current {
            let apply = ui::button_with(
                ui::Face::Label("Apply"),
                ui::Kind::Secondary,
                ui::Size::Small,
            );
            let n = name.clone();
            apply.connect_clicked(move |_| displays::apply(&n));
            r.end.append(&apply);
        }
        list.append(&r.root);
    }
    group.set_visible(view.available && any);
}

/// Fill `list` with the profiles, one row each with Apply, raise and
/// delete; the Displays settings tab shows the same list. False when the
/// compositor offers no output management, where profiles do nothing.
pub fn fill_profiles(list: &Box) -> bool {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    let view = displays::view();
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
    view.available
}

/// A name entry and "Save current layout": the layout on screen becomes a
/// profile of that name, at the top of the list.
pub fn save_row(label: &str) -> Box {
    let save = ui::hbox(2);
    let name = gtk4::Entry::new();
    name.set_placeholder_text(Some("Profile name"));
    name.set_hexpand(true);
    ui::entry::adopt(&name, ui::FieldSize::Normal);
    let save_btn = ui::button_with(ui::Face::Label(label), ui::Kind::Secondary, ui::Size::Small);
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
    save
}

// ── Extra screen ──────────────────────────────────────────────────────────────

/// A small button that starts or stops the extra screen; it waits while
/// one is starting or stopping.
fn cast_button(label: &str, busy: bool, on_click: impl Fn() + 'static) -> gtk4::Button {
    let b = ui::button_with(ui::Face::Label(label), ui::Kind::Secondary, ui::Size::Small);
    b.set_sensitive(!busy);
    b.connect_clicked(move |_| on_click());
    b
}

/// The rows under "Extra screen": the one that is on, with Stop; else the
/// targets, each with Start where a backend reaches it (`cast::backend`).
/// A cast takes seconds to start (the first segments, the TV's player), so
/// a row says it is on its way.
fn fill_extra_screen(list: &Box) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    let v = cast::view();
    if let Some(s) = &v.session {
        let title = match &s.target {
            Target::Device(d) => format!("Extra screen on {}", d.name),
            Target::Viewer => "Extra screen for a VNC viewer".to_string(),
        };
        let r = ui::row(
            icons::TV,
            &title,
            &format!("{} · {} on {}", s.output, s.backend, s.location),
        );
        r.end.append(&cast_button("Stop", v.busy, cast::end));
        list.append(&r.root);
    } else if v.busy {
        let r = ui::row(
            icons::TV,
            "Starting the extra screen…",
            "A TV shows it once the first seconds are encoded",
        );
        list.append(&r.root);
    } else {
        let r = ui::row(
            icons::DISPLAY,
            "VNC viewer",
            "A screen beside these, for a VNC viewer",
        );
        r.end.append(&cast_button("Start", v.busy, || {
            cast::begin(Target::Viewer)
        }));
        list.append(&r.root);
        for d in &v.devices {
            let target = Target::Device(d.clone());
            let reached = cast::backend::for_target(&target).is_some();
            let mut subtitle: Vec<&str> = d.model.iter().map(String::as_str).collect();
            subtitle.push(if reached {
                &d.address
            } else {
                "No address to cast to"
            });
            let r = ui::row(icons::TV, &d.name, &subtitle.join(" · "));
            if reached {
                r.end.append(&cast_button("Start", v.busy, move || {
                    cast::begin(target.clone())
                }));
            }
            list.append(&r.root);
        }
        if v.searching {
            let r = ui::row(
                icons::TV,
                "Looking for TVs…",
                "Cast devices on this network",
            );
            list.append(&r.root);
        } else if v.devices.is_empty() {
            let r = ui::row(
                icons::TV,
                "No TV found",
                "Cast devices with a screen show here",
            );
            r.end.append(&cast_button("Search", false, cast::search));
            list.append(&r.root);
        }
    }
    if let Some(problem) = &v.problem {
        let l = ui::text(problem, ui::Text::Caption, ui::Tone::Danger);
        l.set_halign(gtk4::Align::Start);
        l.set_wrap(true);
        list.append(&l);
    }
}

/// The rows of "Extra screen", for the Helm and for the Displays pane. The
/// network is searched when the rows come on screen, not when the panel opens.
pub fn extra_screen_list() -> Box {
    let list = ui::vbox(1);
    fill_extra_screen(&list);
    list.connect_map(|_| cast::search_if_stale());
    let rows = list.clone();
    cast::observe(move || fill_extra_screen(&rows));
    list
}

/// "Extra screen": a virtual output sent to a TV or a viewer.
fn extra_screen_group() -> Box {
    let group = ui::group(1);
    let head = ui::heading("Extra screen");
    head.set_halign(gtk4::Align::Start);
    head.add_css_class("display-group-head");
    group.append(&head);
    group.append(&extra_screen_list());
    group
}

// ── DisplaySection ────────────────────────────────────────────────────────────

/// What "Arrange displays…" runs, once the Helm has said.
type OnArrange = Rc<RefCell<Option<std::boxed::Box<dyn Fn()>>>>;

pub struct DisplaySection {
    section: ui::Section,
    output_list: Box,
    /// What "Arrange displays…" opens: the Displays settings pane, set by
    /// the Helm (`set_on_arrange`).
    on_arrange: OnArrange,
}

impl DisplaySection {
    pub fn new() -> Self {
        let section = ui::section(icons::DISPLAY, "Displays", "");
        ui::glyph::adopt(&section.icon, ui::Text::Title, ui::Tone::Fg);
        let output_list = ui::vbox(1);

        // ── Night light: on or off now; how warm is Appearance's ──────────────
        let current = store::current().night_light();
        let (on_row, on) = ui::switch_row("Night light", night_when(current.schedule));
        on.set_active(current.enabled);
        on.connect_active_notify(|s| {
            let on = s.is_active();
            store::edit::<NightLight>(|n| n.enabled = on);
        });
        {
            // A change from elsewhere (the deck tile, the settings file, the
            // CLI) moves the switch; the handler above then writes back what
            // is already there, which the store drops.
            let (on, when) = (on.clone(), on_row.subtitle.clone());
            store::observe(move || {
                let n = store::current().night_light();
                if on.is_active() != n.enabled {
                    on.set_active(n.enabled);
                }
                when.set_label(night_when(n.schedule));
            });
        }

        // ── Profiles to apply ─────────────────────────────────────────────────
        let profiles = ui::group(1);
        let profiles_head = ui::heading("Profiles");
        profiles_head.set_halign(gtk4::Align::Start);
        profiles_head.add_css_class("display-group-head");
        profiles.append(&profiles_head);
        let profile_list = ui::vbox(1);
        profiles.append(&profile_list);
        populate_picks(&profiles, &profile_list);

        let outputs = ui::group(1);
        let outputs_head = ui::heading("Connected");
        outputs_head.set_halign(gtk4::Align::Start);
        outputs_head.add_css_class("display-group-head");
        outputs.append(&outputs_head);
        outputs.append(&output_list);

        // ── Arrange displays…: the settings pane ─────────────────────────────
        let on_arrange: OnArrange = Rc::default();
        let arrange = ui::row(
            icons::DISPLAY,
            "Arrange displays…",
            "Layout, scale, profiles and night warmth, in settings",
        );
        let open = ui::button_with(
            ui::Face::Label("Open"),
            ui::Kind::Secondary,
            ui::Size::Small,
        );
        {
            let on_arrange = on_arrange.clone();
            open.connect_clicked(move |_| {
                if let Some(f) = on_arrange.borrow().as_ref() {
                    f();
                }
            });
        }
        arrange.end.append(&open);

        let detail_box = ui::vbox(3);
        detail_box.append(&on_row.root);
        detail_box.append(&profiles);
        detail_box.append(&outputs);
        detail_box.append(&extra_screen_group());
        detail_box.append(&arrange.root);
        section.body.append(&detail_box);

        let display = Self {
            section,
            output_list,
            on_arrange,
        };

        {
            // The outputs or the profiles changed: the list, the rows and the
            // summary follow. Only on a change; nothing polls.
            let output_list = display.output_list.clone();
            let summary = display.section.summary.clone();
            displays::observe(move || {
                populate_picks(&profiles, &profile_list);
                refresh_outputs(&output_list, &summary);
            });
        }
        {
            // The profiles do not see an extra screen come and go
            // (`displays::matcher::screens`); the list of outputs does.
            let output_list = display.output_list.clone();
            let summary = display.section.summary.clone();
            let last = RefCell::new(cast::view().session.map(|s| s.output));
            cast::observe(move || {
                let now = cast::view().session.map(|s| s.output);
                if *last.borrow() != now {
                    *last.borrow_mut() = now;
                    refresh_outputs(&output_list, &summary);
                }
            });
        }
        {
            // A screen renamed here or in settings.
            let output_list = display.output_list.clone();
            let summary = display.section.summary.clone();
            devices::observe(move || refresh_outputs(&output_list, &summary));
        }

        display.refresh();
        display
    }

    /// What "Arrange displays…" does: open the Displays settings pane.
    pub fn set_on_arrange(&self, f: impl Fn() + 'static) {
        *self.on_arrange.borrow_mut() = Some(std::boxed::Box::new(f));
    }

    /// Read the outputs again (on a worker) and rebuild the list and the
    /// summary.
    pub fn refresh(&self) {
        refresh_outputs(&self.output_list, &self.section.summary);
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
fn refresh_outputs(output_list: &Box, summary_text: &Label) {
    let output_list = output_list.clone();
    let summary_text = summary_text.clone();
    spawn_work(get_outputs, move |outputs| {
        populate_output_list_with_data(&output_list, &outputs);

        let active_count = outputs.iter().filter(|o| o.active).count();
        let summary = match active_count {
            0 => "No displays".to_string(),
            1 => outputs
                .iter()
                .find(|o| o.active)
                .map(OutputInfo::shown_name)
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
        OutputInfo {
            name: "eDP-1".into(),
            active: mode.is_some(),
            mode,
            scale,
            product: None,
            label: "Built-in display".into(),
            key: "builtin|eDP-1".into(),
        }
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
