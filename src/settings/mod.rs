//! The settings pane: a deck page in the Helm card (`panel.rs`), one group per
//! thing that can be configured. Glass is the first.
//!
//! It edits the compositor live rather than on OK. A material is not a value
//! you can predict from its numbers — `glass.nix` is mostly the record of
//! sweeping one knob and looking — so the pane's job is to put the slider
//! under the thing it changes. The card being tuned is the card the sliders
//! are on, which is why this is a page in the panel and not a window of its
//! own.
//!
//! Persistence follows the edit rather than a Save button: the compositor gets
//! it after 40 ms, `~/.config/swaypplet/glass.json` after 800 ms, and
//! `glass::apply_saved` replays that file when the panel next starts. Reset
//! deletes it and puts the system material back, so there is always one way
//! out of a material that turned out to be unreadable.

pub mod glass;
pub mod preset;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4::glib;
use gtk4::prelude::*;

use glass::{GrainKind, Material, SurfaceKind, System};

/// How long after the last slider motion the compositor is told.
///
/// Each push is one `layer_effects` per namespace and each of those re-walks
/// every output's layer surfaces and re-arranges them, so a command per frame
/// is real work. Two to three frames of coalescing costs nothing a hand can
/// feel and cuts the traffic by about that much.
const APPLY_DEBOUNCE_MS: u64 = 40;

/// How long after the last change the override file is written. Long enough
/// that a drag across the whole rail is one write.
const SAVE_DEBOUNCE_MS: u64 = 800;

// ── Knobs ───────────────────────────────────────────────────────────────

/// One numeric material property, and how to show it.
struct Knob {
    label: &'static str,
    hint: &'static str,
    min: f64,
    max: f64,
    step: f64,
    decimals: usize,
    get: fn(&Material) -> f64,
    set: fn(&mut Material, f64),
}

/// Re-reads one control from the material. One per widget, so a preset click
/// is a loop over these rather than a list of widget clones `State` would
/// otherwise have to hold by name.
type Sync = Box<dyn Fn(&Material)>;

/// A titled run of knobs.
struct Group {
    title: &'static str,
    hint: &'static str,
    knobs: &'static [Knob],
}

/// Ranges are what the shader will accept and still draw something, not what
/// is tasteful — the point of a live pane is that taste is decided by looking.
/// The exception is anything whose bad value is not "ugly" but "gone": see the
/// `glass` module header for why `mask_threshold` has no knob at all.
static GROUPS: &[Group] = &[
    Group {
        title: "Surface",
        hint: "Roughness is the one knob the physics has: it widens the specular lobe, scatters transmission and blurs the reflection together.",
        knobs: &[
            Knob {
                label: "Roughness",
                hint: "Near zero is the wet look. Toward 1 is frosted; nothing else needs to change.",
                min: 0.0,
                max: 1.0,
                step: 0.01,
                decimals: 2,
                get: |m| m.roughness,
                set: |m, v| m.roughness = v,
            },
            Knob {
                label: "Refraction",
                hint: "Index of the slab. 1.5 is soda-lime glass; 1.0 bends nothing.",
                min: 1.0,
                max: 2.0,
                step: 0.01,
                decimals: 2,
                get: |m| m.refraction,
                set: |m, v| m.refraction = v,
            },
            Knob {
                label: "Lensing",
                hint: "How far the bevel displaces what is behind it.",
                min: 0.0,
                max: 1.0,
                step: 0.01,
                decimals: 2,
                get: |m| m.lensing,
                set: |m, v| m.lensing = v,
            },
            Knob {
                label: "Reflection",
                hint: "Fresnel strength. 1.0 is physical — Schlick's F confines it to the rim unaided.",
                min: 0.0,
                max: 2.0,
                step: 0.01,
                decimals: 2,
                get: |m| m.reflection,
                set: |m, v| m.reflection = v,
            },
            Knob {
                label: "Dispersion",
                hint: "Channel split through the bevel. Small on purpose: text sits on these surfaces.",
                min: 0.0,
                max: 0.05,
                step: 0.001,
                decimals: 3,
                get: |m| m.dispersion,
                set: |m, v| m.dispersion = v,
            },
        ],
    },
    Group {
        title: "Depth",
        hint: "Clear to smoke is three independent knobs: absorb is dark-but-clear, haze is turbid, and frost (below) is scattered-but-bright.",
        knobs: &[
            Knob {
                label: "Absorb",
                hint: "Beer-Lambert through the thickness, neutral. This is the tone under text.",
                min: 0.0,
                max: 4.0,
                step: 0.05,
                decimals: 2,
                get: |m| m.absorb,
                set: |m, v| m.absorb = v,
            },
            Knob {
                label: "Absorb floor",
                hint: "How much always gets through, however long the path.",
                min: 0.0,
                max: 0.5,
                step: 0.01,
                decimals: 2,
                get: |m| m.absorb_floor,
                set: |m, v| m.absorb_floor = v,
            },
            Knob {
                label: "Photochromic",
                hint: "Ceiling on transmitted luminance, so a bright desktop saturates instead of scaling every mid-tone down with it. Zero is off.",
                min: 0.0,
                max: 0.5,
                step: 0.01,
                decimals: 2,
                get: |m| m.photochromic,
                set: |m, v| m.photochromic = v,
            },
            Knob {
                label: "Haze",
                hint: "Turbidity: how much of the result is scattered light rather than image.",
                min: 0.0,
                max: 0.5,
                step: 0.01,
                decimals: 2,
                get: |m| m.haze,
                set: |m, v| m.haze = v,
            },
        ],
    },
    Group {
        title: "Scatter",
        hint: "Blur removes texture, not colour. A card that looks busy with colour wants Absorb, not these.",
        knobs: &[
            Knob {
                label: "Frost radius",
                hint: "How far scattering spreads at full roughness, in pixels. The blur actually run is this times Frost.",
                min: 0.0,
                max: 64.0,
                step: 1.0,
                decimals: 0,
                get: |m| m.frost_radius,
                set: |m, v| m.frost_radius = v,
            },
            Knob {
                label: "Frost",
                hint: "Override the scatter Roughness would have derived. Zero lets the physics decide.",
                min: 0.0,
                max: 1.0,
                step: 0.01,
                decimals: 2,
                get: |m| m.frost,
                set: |m, v| m.frost = v,
            },
            Knob {
                label: "Reflect blur",
                hint: "Override the reflection blur Roughness would have derived. Zero lets the physics decide.",
                min: 0.0,
                max: 1.0,
                step: 0.01,
                decimals: 2,
                get: |m| m.reflect_blur,
                set: |m, v| m.reflect_blur = v,
            },
        ],
    },
    Group {
        title: "Highlights",
        hint: "The two additive terms. Neither is sampled from anything, so these are the ones that can look invented.",
        knobs: &[
            Knob {
                label: "Specular",
                hint: "A Blinn-Phong lobe around a fixed light. Broad at high roughness, so a large value is a large bright patch.",
                min: 0.0,
                max: 1.0,
                step: 0.01,
                decimals: 2,
                get: |m| m.specular,
                set: |m, v| m.specular = v,
            },
            Knob {
                label: "Edge light",
                hint: "Rim glow just inside the boundary. This is what draws the card's outline.",
                min: 0.0,
                max: 0.5,
                step: 0.01,
                decimals: 2,
                get: |m| m.edge_light,
                set: |m, v| m.edge_light = v,
            },
            Knob {
                label: "Shine",
                hint: "Override the specular exponent Roughness would have derived (2/a² − 2). Zero lets the physics decide.",
                min: 0.0,
                max: 256.0,
                step: 1.0,
                decimals: 0,
                get: |m| m.shine,
                set: |m, v| m.shine = v,
            },
            Knob {
                label: "Noise",
                hint: "Dither over the result, against banding in the gradients.",
                min: 0.0,
                max: 0.05,
                step: 0.001,
                decimals: 3,
                get: |m| m.noise,
                set: |m, v| m.noise = v,
            },
        ],
    },
    Group {
        title: "Grain",
        hint: "The same surface as Roughness, at a scale you can resolve: above a pixel the facets refract individually instead of integrating into a lobe.",
        knobs: &[
            Knob {
                label: "Grain scale",
                hint: "Pitch in pixels. Much below the panel's own text size it stops reading as an uneven surface and becomes noise.",
                min: 4.0,
                max: 96.0,
                step: 1.0,
                decimals: 0,
                get: |m| m.grain_scale,
                set: |m, v| m.grain_scale = v,
            },
            Knob {
                label: "Grain strength",
                hint: "Peak lateral displacement in pixels. It is strength/scale — the slope — that the highlight terms react to, so move the two together.",
                min: 0.0,
                max: 8.0,
                step: 0.1,
                decimals: 1,
                get: |m| m.grain_strength,
                set: |m, v| m.grain_strength = v,
            },
        ],
    },
    Group {
        title: "Cost",
        hint: "What the pass spends, and the term that keeps frosting from dimming for a reason unrelated to Absorb.",
        knobs: &[
            Knob {
                label: "Samples",
                hint: "Taps per fragment. More is smoother dispersion and a slower pass.",
                min: 1.0,
                max: 8.0,
                step: 1.0,
                decimals: 0,
                get: |m| m.samples,
                set: |m, v| m.samples = v,
            },
            Knob {
                label: "Energy comp",
                hint: "How much of the energy single scattering drops to put back. 1 preserves energy; 0 is what the material did before the term existed.",
                min: 0.0,
                max: 1.0,
                step: 0.01,
                decimals: 2,
                get: |m| m.energy_comp,
                set: |m, v| m.energy_comp = v,
            },
        ],
    },
];

// ── State ───────────────────────────────────────────────────────────────

struct State {
    /// The shipped material and the namespace table. Loaded once: a host
    /// without one never builds a `State` at all, it gets the note instead.
    system: System,
    material: RefCell<Material>,
    /// Filled in as the controls are built; see [`Sync`].
    sync: RefCell<Vec<Sync>>,
    /// True while `sync` is driving the widgets, so their change handlers do
    /// not treat a programmatic write as an edit and push it straight back.
    updating: Cell<bool>,
    apply_timer: RefCell<Option<glib::SourceId>>,
    save_timer: RefCell<Option<glib::SourceId>>,
    status: gtk4::Label,
}

impl State {
    /// Take an edit: update the widgets that did not make it, and start both
    /// clocks.
    fn edited(self: &Rc<Self>) {
        if self.updating.get() {
            return;
        }
        self.sync_controls();
        self.schedule_apply();
        self.schedule_save();
        self.set_status(true);
    }

    /// Replace the whole material — a preset, or Reset.
    fn replace(self: &Rc<Self>, material: Material, modified: bool) {
        *self.material.borrow_mut() = material;
        self.sync_controls();
        self.schedule_apply();
        if modified {
            self.schedule_save();
        } else {
            // Reset is the one path that removes the file rather than writing
            // one. A pending save from the edits being discarded would put it
            // straight back, so it has to be cancelled, not just skipped.
            if let Some(id) = self.save_timer.replace(None) {
                crate::spawn::remove_source(id);
            }
            glass::clear_override();
        }
        self.set_status(modified);
    }

    fn sync_controls(&self) {
        self.updating.set(true);
        let material = self.material.borrow();
        for sync in self.sync.borrow().iter() {
            sync(&material);
        }
        self.updating.set(false);
    }

    fn schedule_apply(self: &Rc<Self>) {
        if let Some(id) = self.apply_timer.replace(None) {
            crate::spawn::remove_source(id);
        }
        let this = self.clone();
        let id = glib::timeout_add_local_once(
            std::time::Duration::from_millis(APPLY_DEBOUNCE_MS),
            move || {
                this.apply_timer.replace(None);
                this.system.apply(&this.material.borrow());
            },
        );
        self.apply_timer.replace(Some(id));
    }

    fn schedule_save(self: &Rc<Self>) {
        if let Some(id) = self.save_timer.replace(None) {
            crate::spawn::remove_source(id);
        }
        let this = self.clone();
        let id = glib::timeout_add_local_once(
            std::time::Duration::from_millis(SAVE_DEBOUNCE_MS),
            move || {
                this.save_timer.replace(None);
                glass::save_override(&this.material.borrow());
            },
        );
        self.save_timer.replace(Some(id));
    }

    fn set_status(&self, modified: bool) {
        if modified {
            self.status.set_text(&format!(
                "Overridden — saved to {}",
                pretty_path(&glass::override_path())
            ));
            self.status.remove_css_class("settings-status-system");
        } else {
            self.status
                .set_text("System default, as the sway config ships it");
            self.status.add_css_class("settings-status-system");
        }
    }
}

/// `~/.config/…` rather than the whole home path, which is noise in a label.
fn pretty_path(path: &std::path::Path) -> String {
    let shown = path.display().to_string();
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() => shown.replace(&home, "~"),
        _ => shown,
    }
}

// ── The section ─────────────────────────────────────────────────────────

pub struct SettingsSection {
    root: gtk4::Box,
    state: Option<Rc<State>>,
}

impl SettingsSection {
    pub fn new() -> Self {
        let root = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(14)
            .build();
        root.add_css_class("settings-pane");

        let Some(system) = System::load() else {
            root.append(&unconfigured_note());
            return SettingsSection { root, state: None };
        };

        // The override is what the compositor is already showing, because
        // `app::run` replayed it at startup. Starting from the system material
        // instead would put the sliders somewhere the screen is not.
        let saved = glass::load_override();
        let modified = saved.is_some();
        let material = saved.unwrap_or_else(|| system.material.clone());

        let status = gtk4::Label::builder().xalign(0.0).wrap(true).build();
        status.add_css_class("settings-status");

        let state = Rc::new(State {
            system,
            material: RefCell::new(material),
            sync: RefCell::new(Vec::new()),
            updating: Cell::new(false),
            apply_timer: RefCell::new(None),
            save_timer: RefCell::new(None),
            status: status.clone(),
        });

        root.append(&build_presets(&state));
        root.append(&build_kinds(&state));
        for group in GROUPS {
            root.append(&build_group(&state, group));
        }
        root.append(&build_footer(&state, &status));

        state.sync_controls();
        state.set_status(modified);

        SettingsSection {
            root,
            state: Some(state),
        }
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    /// Re-read the controls from the material the pane holds.
    ///
    /// The panel refreshes every section when it opens. There is nothing to
    /// re-read from the system here — this process is the thing that changed
    /// the material — so this only repairs widgets, which costs nothing and
    /// means the pane cannot be caught showing a stale slider.
    pub fn refresh(&self) {
        if let Some(state) = &self.state {
            state.sync_controls();
        }
    }
}

/// What the pane says on a host that does not configure glass.
fn unconfigured_note() -> gtk4::Box {
    let note = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(6)
        .build();
    note.add_css_class("settings-empty");

    let title = gtk4::Label::builder()
        .label("No glass configuration on this host")
        .xalign(0.0)
        .build();
    title.add_css_class("settings-empty-title");

    let body = gtk4::Label::builder()
        .label(
            "The material and the surfaces it applies to come from \
             /etc/swaypplet/glass.json, written by the NixOS side \
             (users/modules/theme/glass-config.nix). Without it there is no \
             baseline to edit against.",
        )
        .xalign(0.0)
        .wrap(true)
        .build();
    body.add_css_class("settings-empty-body");

    note.append(&title);
    note.append(&body);
    note
}

fn build_presets(state: &Rc<State>) -> gtk4::Box {
    let group = section_box(
        "Presets",
        "The shipped material, and four other coherent points in the same model.",
    );

    let row = gtk4::FlowBox::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .selection_mode(gtk4::SelectionMode::None)
        .min_children_per_line(2)
        .max_children_per_line(5)
        .row_spacing(6)
        .column_spacing(6)
        .build();
    row.add_css_class("settings-presets");

    let system_btn = gtk4::Button::with_label("System");
    system_btn.add_css_class("settings-preset-btn");
    system_btn.set_tooltip_text(Some(
        "The material users/modules/theme/glass.nix ships. Also what Reset returns to.",
    ));
    {
        let state = state.clone();
        system_btn.connect_clicked(move |_| {
            let material = state.system.material.clone();
            state.replace(material, true);
        });
    }
    row.append(&system_btn);

    for p in &preset::ALL {
        let btn = gtk4::Button::with_label(p.name);
        btn.add_css_class("settings-preset-btn");
        btn.set_tooltip_text(Some(p.hint));
        let state = state.clone();
        btn.connect_clicked(move |_| state.replace(p.material(), true));
        row.append(&btn);
    }

    group.append(&row);
    group
}

/// The two named properties. Dropdowns rather than sliders because sway takes
/// them as names, and an unknown one costs the whole `layer_effects` block.
fn build_kinds(state: &Rc<State>) -> gtk4::Box {
    let group = section_box(
        "Profile",
        "The bevel's height profile, and the sub-pixel structure laid over it.",
    );

    let surface_labels: Vec<&str> = SurfaceKind::ALL.iter().map(|k| k.label()).collect();
    let surface = gtk4::DropDown::from_strings(&surface_labels);
    surface.add_css_class("settings-dropdown");
    {
        let state = state.clone();
        surface.connect_selected_notify(move |d| {
            // Before the borrow, not after: `sync_controls` sets the selection
            // while holding `material`, and a `borrow_mut` under that is a
            // panic rather than a wasted round trip.
            if state.updating.get() {
                return;
            }
            let Some(kind) = SurfaceKind::ALL.get(d.selected() as usize).copied() else {
                return;
            };
            state.material.borrow_mut().surface = kind;
            state.edited();
        });
    }
    {
        let surface = surface.clone();
        state.sync.borrow_mut().push(Box::new(move |m| {
            let index = SurfaceKind::ALL.iter().position(|k| *k == m.surface);
            surface.set_selected(index.unwrap_or(0) as u32);
        }));
    }
    group.append(&kind_row("Surface", &surface));

    let grain_labels: Vec<&str> = GrainKind::ALL.iter().map(|k| k.label()).collect();
    let grain = gtk4::DropDown::from_strings(&grain_labels);
    grain.add_css_class("settings-dropdown");
    {
        let state = state.clone();
        grain.connect_selected_notify(move |d| {
            if state.updating.get() {
                return;
            }
            let Some(kind) = GrainKind::ALL.get(d.selected() as usize).copied() else {
                return;
            };
            state.material.borrow_mut().grain = kind;
            state.edited();
        });
    }
    {
        let grain = grain.clone();
        state.sync.borrow_mut().push(Box::new(move |m| {
            let index = GrainKind::ALL.iter().position(|k| *k == m.grain);
            grain.set_selected(index.unwrap_or(0) as u32);
        }));
    }
    group.append(&kind_row("Grain", &grain));

    group
}

fn kind_row(label: &str, control: &impl IsA<gtk4::Widget>) -> gtk4::Box {
    let row = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(10)
        .build();
    row.add_css_class("settings-row");

    let name = gtk4::Label::builder().label(label).xalign(0.0).build();
    name.add_css_class("settings-row-label");
    row.append(&name);

    let control = control.as_ref();
    control.set_hexpand(true);
    row.append(control);
    row
}

fn build_group(state: &Rc<State>, group: &'static Group) -> gtk4::Box {
    let container = section_box(group.title, group.hint);
    for knob in group.knobs {
        container.append(&build_knob(state, knob));
    }
    container
}

fn build_knob(state: &Rc<State>, knob: &'static Knob) -> gtk4::Box {
    let row = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(10)
        .build();
    row.add_css_class("settings-row");
    row.set_tooltip_text(Some(knob.hint));

    let name = gtk4::Label::builder().label(knob.label).xalign(0.0).build();
    name.add_css_class("settings-row-label");
    row.append(&name);

    let scale =
        gtk4::Scale::with_range(gtk4::Orientation::Horizontal, knob.min, knob.max, knob.step);
    scale.set_draw_value(false);
    scale.set_hexpand(true);
    scale.add_css_class("settings-scale");

    let value = gtk4::Label::builder()
        .label("")
        .xalign(1.0)
        .width_chars(6)
        .build();
    value.add_css_class("settings-row-value");

    {
        let state = state.clone();
        let value = value.clone();
        scale.connect_value_changed(move |s| {
            // Snapping here rather than trusting the adjustment: a Scale's
            // step only governs the keyboard and the scroll wheel, so a drag
            // hands back a continuous value and `absorb` would land on
            // 1.8734921 in the exported Nix.
            let raw = (s.value() / knob.step).round() * knob.step;
            value.set_text(&format!("{raw:.prec$}", prec = knob.decimals));
            if state.updating.get() {
                return;
            }
            (knob.set)(&mut state.material.borrow_mut(), raw);
            state.edited();
        });
    }
    {
        let scale = scale.clone();
        let value = value.clone();
        state.sync.borrow_mut().push(Box::new(move |m| {
            let v = (knob.get)(m);
            scale.set_value(v);
            // Written here as well as in the handler above, because
            // `set_value` only emits `value-changed` when the value actually
            // moves. A knob whose material value equals the adjustment's
            // starting 0 — `frost`, `shine` and `reflect_blur` all default
            // there — would otherwise keep an empty label for the lifetime of
            // the pane, which reads as "no value" rather than as zero.
            value.set_text(&format!("{v:.prec$}", prec = knob.decimals));
        }));
    }

    row.append(&scale);
    row.append(&value);
    row
}

fn build_footer(state: &Rc<State>, status: &gtk4::Label) -> gtk4::Box {
    let footer = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(8)
        .build();
    footer.add_css_class("settings-footer");

    let buttons = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(8)
        .build();

    let reset = gtk4::Button::with_label("Reset to system");
    reset.add_css_class("settings-action");
    reset.set_tooltip_text(Some(
        "Put the shipped material back and delete the override file.",
    ));
    {
        let state = state.clone();
        reset.connect_clicked(move |_| {
            let material = state.system.material.clone();
            state.replace(material, false);
        });
    }
    buttons.append(&reset);

    let copy = gtk4::Button::with_label("Copy as Nix");
    copy.add_css_class("settings-action");
    copy.set_tooltip_text(Some(
        "The material as a glass.nix attrset body, for promoting a keeper into the Nix side by hand.",
    ));
    {
        let state = state.clone();
        let status = status.clone();
        copy.connect_clicked(move |_| {
            let nix = state.material.borrow().as_nix();
            match gtk4::gdk::Display::default() {
                Some(display) => {
                    display.clipboard().set_text(&nix);
                    status.set_text("Copied — paste into material = { … } in theme/glass.nix");
                }
                None => log::warn!("glass: no display, cannot reach the clipboard"),
            }
        });
    }
    buttons.append(&copy);

    footer.append(&buttons);
    footer.append(status);
    footer
}

fn section_box(title: &str, hint: &str) -> gtk4::Box {
    let container = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(4)
        .build();
    container.add_css_class("settings-group");

    let heading = gtk4::Label::builder().label(title).xalign(0.0).build();
    heading.add_css_class("settings-group-title");
    container.append(&heading);

    let sub = gtk4::Label::builder()
        .label(hint)
        .xalign(0.0)
        .wrap(true)
        .build();
    sub.add_css_class("settings-group-hint");
    container.append(&sub);

    container
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_knob_range_contains_what_the_presets_ask_for() {
        // A preset outside a knob's range is a preset the slider silently
        // clamps, so the pane would show a different material than the one it
        // just pushed at the compositor.
        for p in &preset::ALL {
            let m = p.material();
            for group in GROUPS {
                for knob in group.knobs {
                    let v = (knob.get)(&m);
                    assert!(
                        v >= knob.min && v <= knob.max,
                        "{}: {} = {v} outside {}..{}",
                        p.name,
                        knob.label,
                        knob.min,
                        knob.max
                    );
                }
            }
        }
    }

    #[test]
    fn every_numeric_field_has_exactly_one_knob() {
        // The knob table and `Material` are edited in different places; a
        // field with no knob is invisible in the pane and a field with two is
        // two sliders fighting over one number.
        let probe = preset::clear();
        let mut seen = Vec::new();
        for group in GROUPS {
            for knob in group.knobs {
                // Identify a knob by which field its setter moves.
                let mut m = probe.clone();
                let bumped = (knob.get)(&probe) + knob.step;
                (knob.set)(&mut m, bumped);
                let changed: Vec<&str> = probe
                    .numbers()
                    .iter()
                    .zip(m.numbers().iter())
                    .filter(|((_, a), (_, b))| a != b)
                    .map(|((name, _), _)| *name)
                    .collect();
                assert_eq!(changed.len(), 1, "{} moved {changed:?}", knob.label);
                seen.push(changed[0]);
            }
        }
        seen.sort_unstable();
        let mut all: Vec<&str> = probe.numbers().iter().map(|(n, _)| *n).collect();
        all.sort_unstable();
        assert_eq!(seen, all, "knob table and Material disagree");
    }

    #[test]
    fn a_knobs_step_divides_its_range() {
        // Otherwise the rail's top end is unreachable: the snap in
        // `build_knob` rounds to a multiple of `step`, and a max that is not
        // one can never be selected.
        for group in GROUPS {
            for knob in group.knobs {
                let steps = (knob.max - knob.min) / knob.step;
                assert!(
                    (steps - steps.round()).abs() < 1e-6,
                    "{}: {}..{} is not a whole number of {} steps",
                    knob.label,
                    knob.min,
                    knob.max,
                    knob.step
                );
            }
        }
    }
}
