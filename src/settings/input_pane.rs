//! The Input tab: the keyboard, the touchpad and the mouse, and what the
//! volume and brightness keys do.
//!
//! Two sections: `input`, which `services::input` sends to sway by device
//! type the moment it changes, and `keys`, read per press by the OSD. Every
//! `input` field is optional, and a row nobody touched stays as the sway
//! config sets it: the row shows the value the devices report
//! (`get_inputs`, read when the tab is refreshed), and only an edit makes it
//! an override. Reset drops both sections, and sway reads its config again
//! to put the dropped `input` values back.
//!
//! Devices are listed so per-device rows can come later; today everything
//! applies to every device of a type.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4::prelude::*;

use super::form::{self, dropdown_row, scale_row, section_box, switch_row};
use super::schema::{AccelProfile, ClickMethod, Input, ScrollMethod};
use super::store::{self, Keys};
use super::xkb;
use crate::services::devices::{self, DeviceKey};
use crate::services::input::{self as service, Device};
use crate::ui;

const PROFILES: [(&str, AccelProfile); 2] = [
    ("Adaptive", AccelProfile::Adaptive),
    ("Flat", AccelProfile::Flat),
];

const CLICKS: [(&str, ClickMethod); 3] = [
    ("Button areas", ClickMethod::ButtonAreas),
    ("Fingers", ClickMethod::Clickfinger),
    ("None", ClickMethod::None),
];

const SCROLLS: [(&str, ScrollMethod); 4] = [
    ("Two fingers", ScrollMethod::TwoFinger),
    ("Edge", ScrollMethod::Edge),
    ("Button held", ScrollMethod::OnButtonDown),
    ("None", ScrollMethod::None),
];

/// What Caps Lock can be. A short list on purpose: the xkb catalogue has
/// thirty `caps:` and `ctrl:` options, and these are the ones people ask for.
const CAPS: [(&str, &str); 7] = [
    ("Caps Lock", ""),
    ("Escape", "caps:escape"),
    ("Swap with Escape", "caps:swapescape"),
    ("Ctrl", "ctrl:nocaps"),
    ("Swap with Ctrl", "ctrl:swapcaps"),
    ("Backspace", "caps:backspace"),
    ("Nothing", "caps:none"),
];

/// The layout switches offered when there is no xkb catalogue to list.
const SWITCHES: [(&str, &str); 4] = [
    ("Win+Space", "grp:win_space_toggle"),
    ("Alt+Shift", "grp:alt_shift_toggle"),
    ("Ctrl+Shift", "grp:ctrl_shift_toggle"),
    ("Caps Lock", "grp:caps_toggle"),
];

/// A row's edit: which knob of the section it sets.
type SetKnob<T> = fn(&mut Input, T);

/// How many search hits the picker lists. Past this the query is too short
/// to be worth scrolling.
const PICKER_ROWS: usize = 40;

/// A dropdown over string values, `current` added when the list lacks it
/// (a hand-edited option still shows as itself).
struct Choices {
    values: Vec<String>,
    dropdown: gtk4::DropDown,
}

impl Choices {
    fn new(mut pairs: Vec<(String, String)>, current: Option<&str>) -> Choices {
        if let Some(c) = current
            && !pairs.iter().any(|(_, v)| v == c)
        {
            pairs.push((c.to_string(), c.to_string()));
        }
        let labels: Vec<String> = pairs.iter().map(|(l, _)| l.clone()).collect();
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let dropdown = form::dropdown(&refs);
        let values = pairs.into_iter().map(|(_, v)| v).collect();
        Choices { values, dropdown }
    }

    fn select(&self, value: &str) {
        let at = self.values.iter().position(|v| v == value).unwrap_or(0);
        self.dropdown.set_selected(at as u32);
    }

    fn selected(&self) -> Option<String> {
        self.values.get(self.dropdown.selected() as usize).cloned()
    }
}

fn index_of<T: PartialEq + Copy>(table: &[(&str, T)], value: T) -> u32 {
    table.iter().position(|(_, v)| *v == value).unwrap_or(0) as u32
}

/// What the devices report, as an `Input` with every knob set that a
/// device of its type answered for: the first keyboard, touchpad and
/// pointer. Layouts only when every name maps back to a code.
fn reported(devices: &[Device], catalogue: &xkb::Catalogue) -> Input {
    let first = |kind: &str| devices.iter().find(|d| d.kind == kind);
    let mut input = Input::default();
    if let Some(kb) = first("keyboard") {
        let codes: Option<Vec<String>> = kb
            .layouts
            .iter()
            .map(|name| catalogue.code_of(name).map(str::to_string))
            .collect();
        input.layouts = codes.filter(|c| !c.is_empty());
        input.repeat_delay_ms = kb.repeat_delay_ms;
        input.repeat_rate = kb.repeat_rate;
    }
    if let Some(tp) = first("touchpad") {
        input.touchpad_tap = tp.tap;
        input.touchpad_natural_scroll = tp.natural_scroll;
        input.touchpad_speed = tp.speed;
        input.touchpad_accel_profile = tp.accel_profile;
        input.touchpad_dwt = tp.dwt;
        input.touchpad_click_method = tp.click_method;
        input.touchpad_scroll_method = tp.scroll_method;
    }
    if let Some(m) = first("pointer") {
        input.mouse_speed = m.speed;
        input.mouse_accel_profile = m.accel_profile;
        input.mouse_natural_scroll = m.natural_scroll;
    }
    input
}

/// Each knob from `over` where it is set, else from `under`.
fn overlay(over: &Input, under: &Input) -> Input {
    Input {
        layouts: over.layouts.clone().or_else(|| under.layouts.clone()),
        layout_switch: over
            .layout_switch
            .clone()
            .or_else(|| under.layout_switch.clone()),
        caps: over.caps.clone().or_else(|| under.caps.clone()),
        repeat_delay_ms: over.repeat_delay_ms.or(under.repeat_delay_ms),
        repeat_rate: over.repeat_rate.or(under.repeat_rate),
        touchpad_tap: over.touchpad_tap.or(under.touchpad_tap),
        touchpad_natural_scroll: over
            .touchpad_natural_scroll
            .or(under.touchpad_natural_scroll),
        touchpad_speed: over.touchpad_speed.or(under.touchpad_speed),
        touchpad_accel_profile: over.touchpad_accel_profile.or(under.touchpad_accel_profile),
        touchpad_dwt: over.touchpad_dwt.or(under.touchpad_dwt),
        touchpad_click_method: over.touchpad_click_method.or(under.touchpad_click_method),
        touchpad_scroll_method: over.touchpad_scroll_method.or(under.touchpad_scroll_method),
        mouse_speed: over.mouse_speed.or(under.mouse_speed),
        mouse_accel_profile: over.mouse_accel_profile.or(under.mouse_accel_profile),
        mouse_natural_scroll: over.mouse_natural_scroll.or(under.mouse_natural_scroll),
    }
}

fn describe_default(keys: &Keys) -> String {
    format!(
        "System default: the sway config's input blocks; {}% key steps{}",
        keys.volume_step,
        if keys.volume_boost {
            ", boost allowed"
        } else {
            ""
        },
    )
}

struct State {
    catalogue: &'static xkb::Catalogue,
    layouts: gtk4::Box,
    add: gtk4::Button,
    picker: gtk4::Revealer,
    search: gtk4::SearchEntry,
    hits: gtk4::ListBox,
    hit_codes: RefCell<Vec<String>>,
    switch: Choices,
    caps: Choices,
    delay: gtk4::Scale,
    rate: gtk4::Scale,
    tap: gtk4::Switch,
    tp_natural: gtk4::Switch,
    dwt: gtk4::Switch,
    tp_speed: gtk4::Scale,
    tp_profile: gtk4::DropDown,
    click: gtk4::DropDown,
    scroll: gtk4::DropDown,
    m_speed: gtk4::Scale,
    m_profile: gtk4::DropDown,
    m_natural: gtk4::Switch,
    volume_step: gtk4::Scale,
    brightness_step: gtk4::Scale,
    boost: gtk4::Switch,
    devices: gtk4::Box,
    /// The last list sway gave, redrawn when a device is renamed.
    shown: RefCell<Vec<Device>>,
    reported: RefCell<Input>,
    status: gtk4::Label,
    updating: Cell<bool>,
}

impl State {
    fn edit(self: &Rc<Self>, f: impl FnOnce(&mut Input)) {
        if self.updating.get() {
            return;
        }
        store::edit(f);
        self.sync();
    }

    fn edit_keys(self: &Rc<Self>, f: impl FnOnce(&mut Keys)) {
        if self.updating.get() {
            return;
        }
        store::edit(f);
        self.sync();
    }

    /// The layouts in force: the override, else what the keyboard reports.
    fn layout_codes(&self) -> Vec<String> {
        store::current()
            .input()
            .layouts
            .or_else(|| self.reported.borrow().layouts.clone())
            .unwrap_or_default()
    }

    fn set_layouts(self: &Rc<Self>, codes: Vec<String>) {
        if codes.is_empty() {
            return;
        }
        self.edit(|i| i.layouts = Some(codes));
    }

    fn sync(self: &Rc<Self>) {
        self.updating.set(true);
        let settings = store::current();
        let shown = overlay(&settings.input(), &self.reported.borrow());
        let keys = settings.keys();

        self.fill_layouts(&shown.layouts.clone().unwrap_or_default());
        self.switch
            .select(shown.layout_switch.as_deref().unwrap_or(""));
        self.caps.select(shown.caps.as_deref().unwrap_or(""));
        self.delay
            .set_value(f64::from(shown.repeat_delay_ms.unwrap_or(600)));
        self.rate
            .set_value(f64::from(shown.repeat_rate.unwrap_or(25)));

        self.tap.set_active(shown.touchpad_tap.unwrap_or(false));
        self.tp_natural
            .set_active(shown.touchpad_natural_scroll.unwrap_or(false));
        self.dwt.set_active(shown.touchpad_dwt.unwrap_or(true));
        self.tp_speed.set_value(shown.touchpad_speed.unwrap_or(0.0));
        self.tp_profile.set_selected(index_of(
            &PROFILES,
            shown
                .touchpad_accel_profile
                .unwrap_or(AccelProfile::Adaptive),
        ));
        self.click.set_selected(index_of(
            &CLICKS,
            shown
                .touchpad_click_method
                .unwrap_or(ClickMethod::ButtonAreas),
        ));
        self.scroll.set_selected(index_of(
            &SCROLLS,
            shown
                .touchpad_scroll_method
                .unwrap_or(ScrollMethod::TwoFinger),
        ));
        self.m_speed.set_value(shown.mouse_speed.unwrap_or(0.0));
        self.m_profile.set_selected(index_of(
            &PROFILES,
            shown.mouse_accel_profile.unwrap_or(AccelProfile::Adaptive),
        ));
        self.m_natural
            .set_active(shown.mouse_natural_scroll.unwrap_or(false));

        self.volume_step.set_value(f64::from(keys.volume_step));
        self.brightness_step
            .set_value(f64::from(keys.brightness_step));
        self.boost.set_active(keys.volume_boost);

        form::set_source(
            &self.status,
            settings.input.is_some() || settings.keys.is_some(),
            &describe_default(&keys),
        );
        self.updating.set(false);
    }

    /// One row per layout: its name, its code, and up, down and remove.
    fn fill_layouts(self: &Rc<Self>, codes: &[String]) {
        while let Some(child) = self.layouts.first_child() {
            self.layouts.remove(&child);
        }
        if codes.is_empty() {
            let none = ui::text(
                "The sway config's layouts. Add one to take over the list.",
                ui::Text::Caption,
                ui::Tone::Faint,
            );
            none.set_xalign(0.0);
            self.layouts.append(&none);
            return;
        }
        let last = codes.len() - 1;
        for (at, code) in codes.iter().enumerate() {
            let row = form::row();
            // The name in the gutter, its code beside it: what a hand edit
            // of the file or `settings set input.layouts` would spell.
            let name = form::row_label(self.catalogue.describe(code).unwrap_or(code));
            name.set_xalign(0.0);
            row.append(&name);
            let code_label = ui::text(code, ui::Text::Caption, ui::Tone::Faint);
            ui::set_mono(&code_label, true);
            code_label.set_hexpand(true);
            code_label.set_xalign(0.0);
            row.append(&code_label);

            let moves: [(&str, &str, bool, isize); 2] = [
                (ui::icons::RAISE, "Earlier in the switch order", at > 0, -1),
                (ui::icons::LOWER, "Later in the switch order", at < last, 1),
            ];
            for (glyph, tooltip, enabled, by) in moves {
                let b = ui::button_with(
                    ui::Face::Glyph { glyph, tooltip },
                    ui::Kind::Flat,
                    ui::Size::Small,
                );
                b.set_sensitive(enabled);
                let state = self.clone();
                b.connect_clicked(move |_| {
                    let mut codes = state.layout_codes();
                    let to = at.saturating_add_signed(by);
                    if to < codes.len() && at < codes.len() {
                        codes.swap(at, to);
                        state.set_layouts(codes);
                    }
                });
                row.append(&b);
            }
            let remove = ui::button_with(
                ui::Face::Glyph {
                    glyph: ui::icons::CLOSE,
                    tooltip: "Remove this layout",
                },
                ui::Kind::Flat,
                ui::Size::Small,
            );
            remove.set_sensitive(codes.len() > 1);
            {
                let state = self.clone();
                remove.connect_clicked(move |_| {
                    let mut codes = state.layout_codes();
                    if at < codes.len() && codes.len() > 1 {
                        codes.remove(at);
                        state.set_layouts(codes);
                    }
                });
            }
            row.append(&remove);
            self.layouts.append(&row);
        }
    }

    /// The picker's hits for what is typed, the layouts already in the
    /// list left out.
    fn fill_hits(&self) {
        while let Some(child) = self.hits.first_child() {
            self.hits.remove(&child);
        }
        let have = self.layout_codes();
        let query = self.search.text();
        let hits: Vec<&xkb::Layout> = self
            .catalogue
            .search(query.as_str())
            .filter(|l| !have.contains(&l.code))
            .take(PICKER_ROWS)
            .collect();
        for hit in &hits {
            let content = ui::hbox(3);
            let name = ui::text(&hit.description, ui::Text::Label, ui::Tone::Fg);
            name.set_xalign(0.0);
            name.set_ellipsize(gtk4::pango::EllipsizeMode::End);
            content.append(&name);
            let code = ui::text(&hit.code, ui::Text::Caption, ui::Tone::Faint);
            ui::set_mono(&code, true);
            code.set_hexpand(true);
            code.set_xalign(0.0);
            content.append(&code);
            self.hits.append(&ui::list_row(&content));
        }
        *self.hit_codes.borrow_mut() = hits.iter().map(|l| l.code.clone()).collect();
    }

    /// The device list, and the values the rows fall back on.
    fn take_devices(self: &Rc<Self>, devices: Vec<Device>) {
        while let Some(child) = self.devices.first_child() {
            self.devices.remove(&child);
        }
        let listed = service::listed(&devices);
        if listed.is_empty() {
            let none = ui::text(
                "Sway reports no keyboard, touchpad or mouse.",
                ui::Text::Caption,
                ui::Tone::Faint,
            );
            none.set_xalign(0.0);
            self.devices.append(&none);
        }
        for d in listed {
            let row = form::row();
            // The kernel's name and sway's identifier stay findable here.
            row.set_tooltip_text(Some(&format!("{} · {}", d.name, d.identifier)));
            let auto = service::auto_name(d);
            let key = DeviceKey::Input(d.identifier.clone());
            let shown = devices::display_name(Some(&key), &auto);
            let name = form::row_label(&shown);
            name.set_hexpand(true);
            name.set_xalign(0.0);
            name.set_ellipsize(gtk4::pango::EllipsizeMode::End);
            row.append(&name);
            row.append(&crate::widgets::rename::button(
                &name,
                move || shown.clone(),
                &auto,
                move |n| devices::rename(&key, &n),
            ));
            let kind = match d.kind.as_str() {
                "pointer" => "Mouse",
                "touchpad" => "Touchpad",
                _ => "Keyboard",
            };
            row.append(&ui::text(kind, ui::Text::Caption, ui::Tone::Muted));
            self.devices.append(&row);
        }
        *self.reported.borrow_mut() = reported(&devices, self.catalogue);
        *self.shown.borrow_mut() = devices;
        self.sync();
    }
}

pub struct InputPane {
    root: gtk4::Box,
    state: Rc<State>,
}

impl InputPane {
    pub fn new() -> Self {
        let root = form::pane();
        let catalogue = xkb::catalogue();
        let settings = store::current();
        let input = settings.input();
        let keys = settings.keys();

        // ── Keyboard ──
        let keyboard = section_box(
            "Keyboard",
            "Every keyboard. A row you leave alone stays as the sway config sets it.",
        );
        let layouts = ui::vbox(1);
        keyboard.append(&layouts);

        let add = form::action_button(
            "Add layout",
            "Search the xkb layouts and variants by name or code.",
        );
        let add_row = form::row();
        add_row.set_halign(gtk4::Align::Start);
        add_row.append(&add);
        keyboard.append(&add_row);

        let search = gtk4::SearchEntry::builder()
            .placeholder_text("Swedish, dvorak, us(intl)")
            .hexpand(true)
            .build();
        ui::entry::adopt(&search, ui::FieldSize::Normal);
        let hits = ui::list();
        hits.add_css_class("settings-picker");
        let hits_scroll = gtk4::ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .max_content_height(200)
            .propagate_natural_height(true)
            .child(&hits)
            .build();
        let picker_body = ui::vbox(2);
        picker_body.add_css_class("settings-picker-box");
        picker_body.append(&search);
        picker_body.append(&hits_scroll);
        let picker = ui::revealer(
            gtk4::RevealerTransitionType::SlideDown,
            crate::tokens::motion::EXPAND,
        );
        picker.set_child(Some(&picker_body));
        keyboard.append(&picker);

        let mut switch_pairs = vec![("None".to_string(), String::new())];
        let listed: Vec<(String, String)> = catalogue
            .options_in("grp")
            .map(|o| (o.description.clone(), o.name.clone()))
            .collect();
        if listed.is_empty() {
            switch_pairs.extend(SWITCHES.iter().map(|(l, v)| (l.to_string(), v.to_string())));
        } else {
            switch_pairs.extend(listed);
        }
        let switch = Choices::new(switch_pairs, input.layout_switch.as_deref());
        let row_switch = form::kind_row("Switch layout", &switch.dropdown);
        row_switch.set_tooltip_text(Some(
            "The keys that move to the next layout. Needs two layouts to do anything.",
        ));
        keyboard.append(&row_switch);

        let caps_pairs = CAPS
            .iter()
            .map(|(l, v)| (l.to_string(), v.to_string()))
            .collect();
        let caps = Choices::new(caps_pairs, input.caps.as_deref());
        let row_caps = form::kind_row("Caps Lock", &caps.dropdown);
        row_caps.set_tooltip_text(Some(
            "What the Caps Lock key does. Shares one xkb_options line with the layout switch.",
        ));
        keyboard.append(&row_caps);

        let (row_delay, delay) = scale_row(
            "Repeat delay",
            "How long a key is held before it repeats.",
            (
                f64::from(Input::REPEAT_DELAY_MS.0),
                f64::from(Input::REPEAT_DELAY_MS.1),
                10.0,
            ),
            |v| format!("{v:.0} ms"),
        );
        keyboard.append(&row_delay);
        let (row_rate, rate) = scale_row(
            "Repeat rate",
            "Characters per second while a key is held.",
            (10.0, 80.0, 1.0),
            |v| format!("{v:.0}/s"),
        );
        keyboard.append(&row_rate);

        // ── Touchpad ──
        let touchpad = section_box(
            "Touchpad",
            "Every touchpad. Applied the moment it changes, and to one plugged in later.",
        );
        let (row_tap, tap) = switch_row(
            "Tap to click",
            "A tap is a click; two fingers right-click.",
            false,
        );
        touchpad.append(&row_tap);
        let (row_tpn, tp_natural) = switch_row(
            "Natural scrolling",
            "The content follows the fingers, as on a phone.",
            false,
        );
        touchpad.append(&row_tpn);
        let (row_dwt, dwt) = switch_row(
            "Off while typing",
            "Ignore the touchpad for a moment after a key press, so a palm does not move the cursor.",
            true,
        );
        touchpad.append(&row_dwt);
        let (row_tps, tp_speed) = scale_row(
            "Speed",
            "libinput's pointer speed, from −1 to 1.",
            (-1.0, 1.0, 0.05),
            |v| format!("{v:+.2}"),
        );
        touchpad.append(&row_tps);
        let profile_labels: Vec<&str> = PROFILES.iter().map(|(l, _)| *l).collect();
        let (row_tpp, tp_profile) = dropdown_row(
            "Acceleration",
            "Adaptive speeds up a fast movement; flat moves the cursor as far as the finger.",
            &profile_labels,
        );
        touchpad.append(&row_tpp);
        let click_labels: Vec<&str> = CLICKS.iter().map(|(l, _)| *l).collect();
        let (row_click, click) = dropdown_row(
            "Right click",
            "Button areas: the pad's lower right corner. Fingers: two fingers anywhere.",
            &click_labels,
        );
        touchpad.append(&row_click);
        let scroll_labels: Vec<&str> = SCROLLS.iter().map(|(l, _)| *l).collect();
        let (row_scroll, scroll) =
            dropdown_row("Scrolling", "How the touchpad scrolls.", &scroll_labels);
        touchpad.append(&row_scroll);

        // ── Mouse ──
        let mouse = section_box("Mouse", "Every mouse and trackball, the touchpads apart.");
        let (row_ms, m_speed) = scale_row(
            "Speed",
            "libinput's pointer speed, from −1 to 1.",
            (-1.0, 1.0, 0.05),
            |v| format!("{v:+.2}"),
        );
        mouse.append(&row_ms);
        let (row_mp, m_profile) = dropdown_row(
            "Acceleration",
            "Adaptive speeds up a fast movement; flat moves the cursor as far as the hand.",
            &profile_labels,
        );
        mouse.append(&row_mp);
        let (row_mn, m_natural) = switch_row(
            "Natural scrolling",
            "The wheel moves the content rather than the view.",
            false,
        );
        mouse.append(&row_mn);

        // ── Keys ──
        let keys_group = section_box(
            "Keys",
            "The volume and brightness keys: how far a press goes.",
        );
        let (row_vol, volume_step) = scale_row(
            "Volume step",
            "Percent per press of a volume key.",
            (1.0, 25.0, 1.0),
            |v| format!("{v:.0}%"),
        );
        keys_group.append(&row_vol);
        let (row_bri, brightness_step) = scale_row(
            "Brightness step",
            "Percent per press of a brightness key.",
            (1.0, 25.0, 1.0),
            |v| format!("{v:.0}%"),
        );
        keys_group.append(&row_bri);
        let (row_boost, boost) = switch_row(
            "Volume past 100 %",
            "Let the keys and the panel's rail go to the 150 % the sound server allows.",
            keys.volume_boost,
        );
        keys_group.append(&row_boost);

        // ── Devices ──
        let devices_group = section_box(
            "Devices",
            "Connected now. The rows above apply to every device of a type.",
        );
        let devices = ui::vbox(1);
        devices_group.append(&devices);

        let reset = form::action_button(
            "Reset to system",
            "Drop the input and keys sections from the settings file. Sway reads its config again to put its own input values back.",
        );
        let (footer, status) = form::footer(&[&reset]);
        let copy = form::copy_nix_button(
            &status,
            "The input and keys sections as theme/settings.nix holds them.",
            || {
                let s = store::current();
                Some(format!(
                    "{}{}",
                    s.section_as_nix("input")?,
                    s.section_as_nix("keys")?
                ))
            },
        );
        if let Some(row) = reset.parent().and_downcast::<gtk4::Box>() {
            row.append(&copy);
        }

        let state = Rc::new(State {
            catalogue,
            layouts,
            add: add.clone(),
            picker: picker.clone(),
            search: search.clone(),
            hits: hits.clone(),
            hit_codes: RefCell::new(Vec::new()),
            switch,
            caps,
            delay: delay.clone(),
            rate: rate.clone(),
            tap: tap.clone(),
            tp_natural: tp_natural.clone(),
            dwt: dwt.clone(),
            tp_speed: tp_speed.clone(),
            tp_profile: tp_profile.clone(),
            click: click.clone(),
            scroll: scroll.clone(),
            m_speed: m_speed.clone(),
            m_profile: m_profile.clone(),
            m_natural: m_natural.clone(),
            volume_step: volume_step.clone(),
            brightness_step: brightness_step.clone(),
            boost: boost.clone(),
            devices,
            shown: RefCell::default(),
            reported: RefCell::new(Input::default()),
            status,
            updating: Cell::new(false),
        });

        {
            let state = state.clone();
            add.connect_clicked(move |_| {
                let open = !state.picker.reveals_child();
                state.picker.set_reveal_child(open);
                state
                    .add
                    .set_label(if open { "Done" } else { "Add layout" });
                if open {
                    state.fill_hits();
                    state.search.grab_focus();
                }
            });
        }
        {
            let state = state.clone();
            search.connect_search_changed(move |_| state.fill_hits());
        }
        {
            let state = state.clone();
            hits.connect_row_activated(move |_, row| {
                let Some(code) = state.hit_codes.borrow().get(row.index() as usize).cloned() else {
                    return;
                };
                let mut codes = state.layout_codes();
                if !codes.contains(&code) {
                    codes.push(code);
                }
                state.picker.set_reveal_child(false);
                state.add.set_label("Add layout");
                state.search.set_text("");
                state.set_layouts(codes);
            });
        }
        {
            let state_c = state.clone();
            state.switch.dropdown.connect_selected_notify(move |_| {
                if let Some(v) = state_c.switch.selected() {
                    state_c.edit(|i| i.layout_switch = Some(v));
                }
            });
        }
        {
            let state_c = state.clone();
            state.caps.dropdown.connect_selected_notify(move |_| {
                if let Some(v) = state_c.caps.selected() {
                    state_c.edit(|i| i.caps = Some(v));
                }
            });
        }
        {
            let state = state.clone();
            delay.connect_value_changed(move |s| {
                let ms = ((s.value() / 10.0).round() * 10.0) as u32;
                state.edit(|i| i.repeat_delay_ms = Some(ms));
            });
        }
        {
            let state = state.clone();
            rate.connect_value_changed(move |s| {
                let r = s.value().round() as u32;
                state.edit(|i| i.repeat_rate = Some(r));
            });
        }
        let switches: [(&gtk4::Switch, SetKnob<bool>); 4] = [
            (&tap, |i, v| i.touchpad_tap = Some(v)),
            (&tp_natural, |i, v| i.touchpad_natural_scroll = Some(v)),
            (&dwt, |i, v| i.touchpad_dwt = Some(v)),
            (&m_natural, |i, v| i.mouse_natural_scroll = Some(v)),
        ];
        for (switch, set) in switches {
            let state = state.clone();
            switch.connect_active_notify(move |s| {
                let on = s.is_active();
                state.edit(|i| set(i, on));
            });
        }
        let speeds: [(&gtk4::Scale, SetKnob<f64>); 2] = [
            (&tp_speed, |i, v| i.touchpad_speed = Some(v)),
            (&m_speed, |i, v| i.mouse_speed = Some(v)),
        ];
        for (scale, set) in speeds {
            let state = state.clone();
            scale.connect_value_changed(move |s| {
                let v = (s.value() / 0.05).round() * 0.05;
                state.edit(|i| set(i, v));
            });
        }
        let profiles: [(&gtk4::DropDown, SetKnob<AccelProfile>); 2] = [
            (&tp_profile, |i, v| i.touchpad_accel_profile = Some(v)),
            (&m_profile, |i, v| i.mouse_accel_profile = Some(v)),
        ];
        for (dropdown, set) in profiles {
            let state = state.clone();
            dropdown.connect_selected_notify(move |d| {
                if let Some((_, p)) = PROFILES.get(d.selected() as usize) {
                    state.edit(|i| set(i, *p));
                }
            });
        }
        {
            let state = state.clone();
            click.connect_selected_notify(move |d| {
                if let Some((_, m)) = CLICKS.get(d.selected() as usize) {
                    state.edit(|i| i.touchpad_click_method = Some(*m));
                }
            });
        }
        {
            let state = state.clone();
            scroll.connect_selected_notify(move |d| {
                if let Some((_, m)) = SCROLLS.get(d.selected() as usize) {
                    state.edit(|i| i.touchpad_scroll_method = Some(*m));
                }
            });
        }
        {
            let state = state.clone();
            volume_step.connect_value_changed(move |s| {
                let step = s.value().round() as u8;
                state.edit_keys(|k| k.volume_step = step);
            });
        }
        {
            let state = state.clone();
            brightness_step.connect_value_changed(move |s| {
                let step = s.value().round() as u8;
                state.edit_keys(|k| k.brightness_step = step);
            });
        }
        {
            let state = state.clone();
            boost.connect_active_notify(move |s| {
                let on = s.is_active();
                state.edit_keys(|k| k.volume_boost = on);
            });
        }
        {
            let state = state.clone();
            reset.connect_clicked(move |_| {
                if state.updating.get() {
                    return;
                }
                store::reset::<Input>();
                store::reset::<Keys>();
                state.sync();
                // The reload the reset causes changes what the devices say.
                let state = state.clone();
                glib::timeout_add_local_once(std::time::Duration::from_secs(1), move || {
                    InputPane::read_devices(&state);
                });
            });
        }

        root.append(&keyboard);
        root.append(&touchpad);
        root.append(&mouse);
        root.append(&keys_group);
        root.append(&devices_group);
        root.append(&footer);
        state.sync();
        InputPane::follow_names(&state);

        InputPane { root, state }
    }

    fn follow_names(state: &Rc<State>) {
        let state = state.clone();
        devices::observe(move || {
            let shown = state.shown.borrow().clone();
            state.take_devices(shown);
        });
    }

    fn read_devices(state: &Rc<State>) {
        let state = state.clone();
        service::devices(move |devices| state.take_devices(devices));
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    /// The layout picker open on `query`, for the render harness.
    pub fn demo_pick(&self, query: &str) {
        self.state.picker.set_reveal_child(true);
        self.state.add.set_label("Done");
        self.state.search.set_text(query);
        self.state.fill_hits();
    }

    /// Re-read the settings, and ask sway for the devices: the one IPC
    /// round trip this tab makes, when the panel opens.
    pub fn refresh(&self) {
        self.state.sync();
        Self::read_devices(&self.state);
    }
}

// The rows the launcher's settings search finds (`settings::search`).
use super::search::{Entry, row};

pub(super) const SEARCH: &[Entry] = &[
    row("Keyboard", "", "Keyboard layouts, in switching order", &["layout", "keyboard layout", "language", "xkb", "dvorak", "qwerty", "input method"]).keys(&["input.layouts"]),
    row("Keyboard", "Switch layout", "The keys that change layout", &["layout shortcut", "switch language", "keyboard switch"]).keys(&["input.layout_switch"]),
    row("Keyboard", "Caps Lock", "What the Caps Lock key does", &["capslock", "caps", "escape", "ctrl", "control"]).keys(&["input.caps"]),
    row("Keyboard", "Repeat delay", "How long a held key waits before repeating", &["key repeat", "typematic", "delay"]).keys(&["input.repeat_delay_ms"]),
    row("Keyboard", "Repeat rate", "How fast a held key repeats", &["key repeat", "typematic", "rate"]).keys(&["input.repeat_rate"]),
    row("Touchpad", "Tap to click", "A tap is a click", &["tap", "trackpad", "touchpad click"]).keys(&["input.touchpad_tap"]),
    row("Touchpad", "Natural scrolling", "Content follows the fingers", &["reverse scroll", "invert scroll", "trackpad scroll"]).keys(&["input.touchpad_natural_scroll"]),
    row("Touchpad", "Off while typing", "Ignore the touchpad while keys are pressed", &["disable while typing", "dwt", "palm"]).keys(&["input.touchpad_dwt"]),
    row("Touchpad", "Speed", "Touchpad pointer speed", &["trackpad speed", "sensitivity", "cursor speed"]).keys(&["input.touchpad_speed"]),
    row("Touchpad", "Acceleration", "Touchpad acceleration profile", &["trackpad acceleration", "accel", "pointer acceleration"]).keys(&["input.touchpad_accel_profile"]),
    row("Touchpad", "Right click", "How a right click is made", &["secondary click", "two finger click", "click method", "clickfinger"]).keys(&["input.touchpad_click_method"]),
    row("Touchpad", "Scrolling", "Two fingers or the edge", &["scroll method", "two finger scroll", "edge scroll"]).keys(&["input.touchpad_scroll_method"]),
    row("Mouse", "Speed", "Mouse pointer speed", &["mouse speed", "sensitivity", "cursor speed", "trackball"]).keys(&["input.mouse_speed"]),
    row("Mouse", "Acceleration", "Mouse acceleration profile", &["mouse acceleration", "accel", "pointer acceleration"]).keys(&["input.mouse_accel_profile"]),
    row("Mouse", "Natural scrolling", "The wheel scrolls the content's way", &["reverse scroll", "invert wheel", "mouse scroll"]).keys(&["input.mouse_natural_scroll"]),
    row("Keys", "Volume step", "Percent per volume key press", &["volume", "increment", "sound"]).keys(&["keys.volume_step"]),
    row("Keys", "Brightness step", "Percent per brightness key press", &["brightness", "backlight", "increment"]).keys(&["keys.brightness_step"]),
    row("Keys", "Volume past 100 %", "Let the volume go to 150 %", &["boost", "overamplification", "loud", "louder", "amplify"]).keys(&["keys.volume_boost"]),
    row("Devices", "", "The keyboards, touchpads and mice connected now", &["devices", "connected", "input devices"]),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_override_wins_and_the_devices_fill_the_rest() {
        let over = Input {
            touchpad_tap: Some(true),
            ..Input::default()
        };
        let under = Input {
            touchpad_tap: Some(false),
            touchpad_dwt: Some(false),
            ..Input::default()
        };
        let shown = overlay(&over, &under);
        assert_eq!(shown.touchpad_tap, Some(true));
        assert_eq!(shown.touchpad_dwt, Some(false));
        assert_eq!(shown.mouse_speed, None);
    }

    #[test]
    fn reported_layouts_map_back_to_codes_or_not_at_all() {
        let catalogue = xkb::parse("! layout\n  se  Swedish\n  us  English (US)\n");
        let kb = |names: &[&str]| Device {
            kind: "keyboard".into(),
            layouts: names.iter().map(|n| n.to_string()).collect(),
            ..Device::default()
        };
        let got = reported(&[kb(&["Swedish", "English (US)"])], &catalogue);
        assert_eq!(got.layouts, Some(vec!["se".to_string(), "us".to_string()]));
        // One name the catalogue lacks: the list cannot be rebuilt, so the
        // rows show the config's list as unknown rather than a wrong one.
        let got = reported(&[kb(&["Swedish", "Klingon"])], &catalogue);
        assert_eq!(got.layouts, None);
    }
}
