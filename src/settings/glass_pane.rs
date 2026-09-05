//! The Glass tab of the settings pane: the liquid-glass material, edited
//! against the card it is drawn on.
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

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4::glib;
use gtk4::prelude::*;

use super::glass::{self, GrainKind, SurfaceKind, System, Tuning};
use super::preset;
use super::ui::{kind_row, pretty_path, section_box};

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
    get: fn(&Tuning) -> f64,
    set: fn(&mut Tuning, f64),
    /// When this knob has anything to say. `None` is always, which is every
    /// knob but the two that describe the grain's frame: an isotropic pattern
    /// has no orientation to turn, and a slider that silently does nothing is
    /// worse than one that says so by going grey.
    live_when: Option<fn(&Tuning) -> bool>,
}

/// Re-reads one control from the tuning. One per widget, so a preset click
/// is a loop over these rather than a list of widget clones `State` would
/// otherwise have to hold by name.
type Sync = Box<dyn Fn(&Tuning)>;

/// A titled run of knobs, and anything in the section that is not one.
struct Group {
    title: &'static str,
    hint: &'static str,
    knobs: &'static [Knob],
    /// Rows appended above the knobs, for a property a slider cannot carry.
    /// Only the fill has one, and only because a colour is not a number.
    extra: Option<fn(&Rc<State>, &gtk4::Box)>,
}

/// Ranges are what the shader will accept and still draw something, not what
/// is tasteful — the point of a live pane is that taste is decided by looking.
/// The exception is anything whose bad value is not "ugly" but "gone": see the
/// `glass` module header for why `mask_threshold` has no knob at all.
static GROUPS: &[Group] = &[
    Group {
        title: "Optics & Surface",
        hint: "Microfacet refraction, dispersion and scattering. Roughness controls transmission frost, specular lobe and reflection blur together.",
        knobs: &[
            Knob {
                label: "Roughness",
                hint: "Primary microfacet roughness. 0 is mirror-wet, 1 is fully frosted.",
                min: 0.0,
                max: 1.0,
                step: 0.01,
                decimals: 2,
                get: |t| t.material.roughness,
                set: |t, v| t.material.roughness = v,
                live_when: None,
            },
            Knob {
                label: "Refraction",
                hint: "Index of the slab. 1.5 is soda-lime glass; 1.0 bends nothing.",
                min: 1.0,
                max: 2.0,
                step: 0.01,
                decimals: 2,
                get: |t| t.material.refraction,
                set: |t, v| t.material.refraction = v,
                live_when: None,
            },
            Knob {
                label: "Dispersion",
                hint: "Channel split through the bevel. Small on purpose: text sits on these surfaces.",
                min: 0.0,
                max: 0.05,
                step: 0.001,
                decimals: 3,
                get: |t| t.material.dispersion,
                set: |t, v| t.material.dispersion = v,
                live_when: None,
            },
            Knob {
                label: "Lensing",
                hint: "How far the bevel displaces what is behind it.",
                min: 0.0,
                max: 1.0,
                step: 0.01,
                decimals: 2,
                get: |t| t.material.lensing,
                set: |t, v| t.material.lensing = v,
                live_when: None,
            },
            Knob {
                label: "Reflection",
                hint: "Weight on the Fresnel term. 1.0 is physical.",
                min: 0.0,
                max: 4.0,
                step: 0.01,
                decimals: 2,
                get: |t| t.material.reflection,
                set: |t, v| t.material.reflection = v,
                live_when: None,
            },
            Knob {
                label: "Frost",
                hint: "Override the scatter Roughness would have derived. Zero lets the physics decide.",
                min: 0.0,
                max: 1.0,
                step: 0.01,
                decimals: 2,
                get: |t| t.material.frost,
                set: |t, v| t.material.frost = v,
                live_when: None,
            },
            Knob {
                label: "Frost radius",
                hint: "How far scattering spreads at full roughness, in pixels.",
                min: 0.0,
                max: 64.0,
                step: 1.0,
                decimals: 0,
                get: |t| t.material.frost_radius,
                set: |t, v| t.material.frost_radius = v,
                live_when: None,
            },
            Knob {
                label: "Reflect blur",
                hint: "Override the reflection blur Roughness would have derived. Zero lets the physics decide.",
                min: 0.0,
                max: 1.0,
                step: 0.01,
                decimals: 2,
                get: |t| t.material.reflect_blur,
                set: |t, v| t.material.reflect_blur = v,
                live_when: None,
            },
            Knob {
                label: "Samples",
                hint: "Spectral dispersion taps per fragment (1..8). More is smoother.",
                min: 1.0,
                max: 8.0,
                step: 1.0,
                decimals: 0,
                get: |t| t.material.samples,
                set: |t, v| t.material.samples = v,
                live_when: None,
            },
            Knob {
                label: "Energy comp",
                hint: "Multi-scatter energy preservation. 1 preserves brightness; 0 is single-scatter.",
                min: 0.0,
                max: 1.0,
                step: 0.01,
                decimals: 2,
                get: |t| t.material.energy_comp,
                set: |t, v| t.material.energy_comp = v,
                live_when: None,
            },
        ],
        extra: None,
    },
    Group {
        title: "Tone & Density",
        hint: "Absorption and tinting: Beer-Lambert volume attenuation, photochromic glare compression, and card body fill.",
        knobs: &[
            Knob {
                label: "Absorb",
                hint: "Beer-Lambert volumetric darkening through the thickness.",
                min: 0.0,
                max: 4.0,
                step: 0.05,
                decimals: 2,
                get: |t| t.material.absorb,
                set: |t, v| t.material.absorb = v,
                live_when: None,
            },
            Knob {
                label: "Absorb floor",
                hint: "Minimum transmission through thin rim boundaries.",
                min: 0.0,
                max: 0.5,
                step: 0.01,
                decimals: 2,
                get: |t| t.material.absorb_floor,
                set: |t, v| t.material.absorb_floor = v,
                live_when: None,
            },
            Knob {
                label: "Photochromic",
                hint: "Adaptive tone ceiling so bright backdrops compress smoothly.",
                min: 0.0,
                max: 1.0,
                step: 0.01,
                decimals: 2,
                get: |t| t.material.photochromic,
                set: |t, v| t.material.photochromic = v,
                live_when: None,
            },
            Knob {
                label: "Haze",
                hint: "Turbidity: how much of the result is scattered light rather than image.",
                min: 0.0,
                max: 0.5,
                step: 0.01,
                decimals: 2,
                get: |t| t.material.haze,
                set: |t, v| t.material.haze = v,
                live_when: None,
            },
            Knob {
                label: "Fill alpha",
                hint: "Body tint opacity. 0 is clear glass, 1 is solid tint.",
                min: 0.0,
                max: 1.0,
                step: 0.01,
                decimals: 2,
                get: |t| t.material.fill_alpha.max(0.0),
                set: |t, v| t.material.fill_alpha = v,
                live_when: None,
            },
        ],
        extra: Some(build_fill_controls),
    },
    Group {
        title: "Highlights & Artsy Effects",
        hint: "Additive highlights, thin-film iridescence, neon rim glow, fluidic surface waves, and dithering.",
        knobs: &[
            Knob {
                label: "Specular",
                hint: "Blinn-Phong directional highlight intensity.",
                min: 0.0,
                max: 1.0,
                step: 0.01,
                decimals: 2,
                get: |t| t.material.specular,
                set: |t, v| t.material.specular = v,
                live_when: None,
            },
            Knob {
                label: "Shine",
                hint: "Override specular exponent (2/a² − 2). Zero lets Roughness decide.",
                min: 0.0,
                max: 256.0,
                step: 1.0,
                decimals: 0,
                get: |t| t.material.shine,
                set: |t, v| t.material.shine = v,
                live_when: None,
            },
            Knob {
                label: "Edge light",
                hint: "Rim glow just inside the boundary.",
                min: 0.0,
                max: 0.5,
                step: 0.01,
                decimals: 2,
                get: |t| t.material.edge_light,
                set: |t, v| t.material.edge_light = v,
                live_when: None,
            },
            Knob {
                label: "Iridescence",
                hint: "Thin-film interference: soap bubble / pearl / oil-slick sheen on bevels.",
                min: 0.0,
                max: 1.0,
                step: 0.01,
                decimals: 2,
                get: |t| t.material.iridescence,
                set: |t, v| t.material.iridescence = v,
                live_when: None,
            },
            Knob {
                label: "Edge glow",
                hint: "Bioluminescent / neon rim emission trapped along the bevel.",
                min: 0.0,
                max: 2.0,
                step: 0.01,
                decimals: 2,
                get: |t| t.material.edge_glow,
                set: |t, v| t.material.edge_glow = v,
                live_when: None,
            },
            Knob {
                label: "Wave amplitude",
                hint: "Fluidic wave ripples and caustic surface displacement.",
                min: 0.0,
                max: 2.0,
                step: 0.01,
                decimals: 2,
                get: |t| t.material.wave_amplitude,
                set: |t, v| t.material.wave_amplitude = v,
                live_when: None,
            },
            Knob {
                label: "Noise",
                hint: "Spatial dither to eliminate gradient banding.",
                min: 0.0,
                max: 0.05,
                step: 0.001,
                decimals: 3,
                get: |t| t.material.noise,
                set: |t, v| t.material.noise = v,
                live_when: None,
            },
        ],
        extra: None,
    },
    Group {
        title: "Geometry",
        hint: "Slab bevel width, thickness, and crest rounding across all surfaces.",
        knobs: &[
            Knob {
                label: "Bevel scale",
                hint: "Multiplies bezel and thickness for all four classes.",
                min: 0.25,
                max: 3.0,
                step: 0.05,
                decimals: 2,
                get: |t| t.bezel_scale,
                set: |t, v| t.bezel_scale = v,
                live_when: None,
            },
            Knob {
                label: "Thickness ratio",
                hint: "Thickness as a multiple of the scaled bezel (0 keeps class ratio).",
                min: 0.0,
                max: 8.0,
                step: 0.1,
                decimals: 1,
                get: |t| t.thickness_ratio,
                set: |t, v| t.thickness_ratio = v,
                live_when: None,
            },
            Knob {
                label: "Crest radius",
                hint: "Scales how wide the crest rounds where edges compete.",
                min: 0.25,
                max: 3.0,
                step: 0.05,
                decimals: 2,
                get: |t| t.crest_scale,
                set: |t, v| t.crest_scale = v,
                live_when: None,
            },
        ],
        extra: None,
    },
    Group {
        title: "Surface Grain",
        hint: "Resolvable surface relief structures (fluting, peened dimples, hammered, cathedral).",
        knobs: &[
            Knob {
                label: "Grain scale",
                hint: "Cell, flute or wave pitch in pixels.",
                min: 4.0,
                max: 96.0,
                step: 1.0,
                decimals: 0,
                get: |t| t.material.grain_scale,
                set: |t, v| t.material.grain_scale = v,
                live_when: None,
            },
            Knob {
                label: "Grain strength",
                hint: "Peak lateral displacement in pixels.",
                min: 0.0,
                max: 8.0,
                step: 0.1,
                decimals: 1,
                get: |t| t.material.grain_strength,
                set: |t, v| t.material.grain_strength = v,
                live_when: None,
            },
            Knob {
                label: "Grain angle",
                hint: "Rotation in degrees clockwise for directional flutes.",
                min: 0.0,
                max: 180.0,
                step: 1.0,
                decimals: 0,
                get: |t| t.material.grain_angle,
                set: |t, v| t.material.grain_angle = v,
                live_when: Some(|t| t.material.grain.is_directional()),
            },
            Knob {
                label: "Grain aspect",
                hint: "Anisotropic stretch factor along the pattern's axis.",
                min: 0.25,
                max: 4.0,
                step: 0.05,
                decimals: 2,
                get: |t| t.material.grain_aspect,
                set: |t, v| t.material.grain_aspect = v,
                live_when: None,
            },
        ],
        extra: None,
    },
];

// ── State ───────────────────────────────────────────────────────────────

struct State {
    /// The shipped material and the namespace table. Loaded once: a host
    /// without one never builds a `State` at all, it gets the note instead.
    system: System,
    tuning: RefCell<Tuning>,
    history: RefCell<Vec<Tuning>>,
    /// Filled in as the controls are built; see [`Sync`].
    sync: RefCell<Vec<Sync>>,
    /// True while `sync` is driving the widgets, so their change handlers do
    /// not treat a programmatic write as an edit and push it straight back.
    updating: Cell<bool>,
    apply_timer: RefCell<Option<glib::SourceId>>,
    save_timer: RefCell<Option<glib::SourceId>>,
    status: gtk4::Label,
    undo_btn: gtk4::Button,
}

impl State {
    fn push_history(&self, t: Tuning) {
        let mut hist = self.history.borrow_mut();
        if hist.last() != Some(&t) {
            hist.push(t);
            if hist.len() > 50 {
                hist.remove(0);
            }
        }
        self.undo_btn.set_sensitive(!hist.is_empty());
    }

    fn undo(self: &Rc<Self>) {
        let prev = self.history.borrow_mut().pop();
        if let Some(tuning) = prev {
            self.replace_internal(tuning, true, true);
        }
        self.undo_btn
            .set_sensitive(!self.history.borrow().is_empty());
    }

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

    /// Replace the whole tuning — a preset, or Reset.
    fn replace(self: &Rc<Self>, tuning: Tuning, modified: bool) {
        self.push_history(self.tuning.borrow().clone());
        self.replace_internal(tuning, modified, true);
    }

    fn replace_internal(self: &Rc<Self>, tuning: Tuning, modified: bool, save: bool) {
        *self.tuning.borrow_mut() = tuning;
        self.sync_controls();
        self.schedule_apply();
        if modified && save {
            self.schedule_save();
        } else if !modified {
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
        let tuning = self.tuning.borrow();
        for sync in self.sync.borrow().iter() {
            sync(&tuning);
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
                this.system.apply(&this.tuning.borrow());
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
                glass::save_override(&this.tuning.borrow());
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

// ── The section ─────────────────────────────────────────────────────────

pub struct GlassPane {
    root: gtk4::Box,
    state: Option<Rc<State>>,
}

impl GlassPane {
    pub fn new() -> Self {
        let root = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(14)
            .build();
        root.add_css_class("settings-pane");

        let Some(system) = System::load() else {
            root.append(&unconfigured_note());
            return GlassPane { root, state: None };
        };

        // The override is what the compositor is already showing, because
        // `app::run` replayed it at startup. Starting from the system material
        // instead would put the sliders somewhere the screen is not.
        let saved = glass::load_override();
        let modified = saved.is_some();
        let tuning = saved.unwrap_or_else(|| Tuning::system(&system));

        let status = gtk4::Label::builder().xalign(0.0).wrap(true).build();
        status.add_css_class("settings-status");

        let undo_btn = gtk4::Button::with_label("Undo");
        undo_btn.add_css_class("settings-action");
        undo_btn.set_sensitive(false);
        undo_btn.set_tooltip_text(Some("Revert the last tuning or preset change."));

        let state = Rc::new(State {
            system,
            tuning: RefCell::new(tuning),
            history: RefCell::new(Vec::new()),
            sync: RefCell::new(Vec::new()),
            updating: Cell::new(false),
            apply_timer: RefCell::new(None),
            save_timer: RefCell::new(None),
            status: status.clone(),
            undo_btn: undo_btn.clone(),
        });

        {
            let state = state.clone();
            undo_btn.connect_clicked(move |_| {
                state.undo();
            });
        }

        root.append(&build_presets(&state));
        root.append(&build_kinds(&state));
        for group in GROUPS {
            root.append(&build_group(&state, group));
        }
        root.append(&build_footer(&state, &status, &undo_btn));

        state.sync_controls();
        state.set_status(modified);

        GlassPane {
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
        "The shipped material, and other coherent physical glass presets (click any to preview).",
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
            let tuning = Tuning::system(&state.system);
            state.replace(tuning, true);
        });
    }
    row.append(&system_btn);

    for p in &preset::ALL {
        let btn = gtk4::Button::with_label(p.name);
        btn.add_css_class("settings-preset-btn");
        btn.set_tooltip_text(Some(p.hint));
        let state = state.clone();
        // A preset is a material, and it resets the geometry with it: keeping
        // a bevel scale from whatever was being tried before would make the
        // same preset land differently depending on what preceded it.
        btn.connect_clicked(move |_| {
            state.replace(
                Tuning {
                    material: p.material(),
                    ..Tuning::system(&state.system)
                },
                true,
            )
        });
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
            state.tuning.borrow_mut().material.surface = kind;
            state.edited();
        });
    }
    {
        let surface = surface.clone();
        state.sync.borrow_mut().push(Box::new(move |t| {
            let index = SurfaceKind::ALL
                .iter()
                .position(|k| *k == t.material.surface);
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
            state.tuning.borrow_mut().material.grain = kind;
            state.edited();
        });
    }
    {
        let grain = grain.clone();
        state.sync.borrow_mut().push(Box::new(move |t| {
            let index = GrainKind::ALL.iter().position(|k| *k == t.material.grain);
            grain.set_selected(index.unwrap_or(0) as u32);
        }));
    }
    group.append(&kind_row("Grain", &grain));

    group
}

fn build_group(state: &Rc<State>, group: &'static Group) -> gtk4::Box {
    let container = section_box(group.title, group.hint);
    // Above the knobs, because the fill's colour is the question its alpha is
    // an answer about: reading "card's own / #32302f / 0.50" downward is the
    // sentence, and the reverse order is not.
    if let Some(extra) = group.extra {
        extra(state, &container);
    }
    for knob in group.knobs {
        container.append(&build_knob(state, knob));
    }
    container
}

/// The fill's colour, and the two checks that hand either half of the fill
/// back to the card.
///
/// Two rather than one, because the halves are independent: a tint over the
/// card's own alpha and the card's own colour at an alpha of your choosing are
/// both things to want. Neither check is the only way to set its half - moving
/// the colour button or the alpha slider takes that half over on its own, and
/// the check follows - so what they are really for is the way back.
fn build_fill_controls(state: &Rc<State>, container: &gtk4::Box) {
    let button = gtk4::Button::builder()
        .has_frame(false)
        .css_classes(["settings-preset-btn"])
        .build();

    let swatch_box = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(8)
        .build();

    let swatch = gtk4::DrawingArea::builder()
        .content_width(28)
        .content_height(16)
        .build();

    let current_rgb = Rc::new(Cell::new((0.196, 0.188, 0.184)));
    {
        let current_rgb = current_rgb.clone();
        swatch.set_draw_func(move |_, cr, _w, _h| {
            let (r, g, b) = current_rgb.get();
            cr.set_source_rgb(r, g, b);
            let _ = cr.paint();
        });
    }

    let hex_label = gtk4::Label::builder()
        .label("card default")
        .css_classes(["settings-row-value"])
        .build();

    swatch_box.append(&swatch);
    swatch_box.append(&hex_label);
    button.set_child(Some(&swatch_box));

    let popover = gtk4::Popover::builder()
        .position(gtk4::PositionType::Bottom)
        .has_arrow(true)
        .build();
    popover.set_parent(&button);

    let pop_body = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(8)
        .margin_start(10)
        .margin_end(10)
        .margin_top(10)
        .margin_bottom(10)
        .build();

    let pal_label = gtk4::Label::builder()
        .label("PALETTE SWATCHES")
        .xalign(0.0)
        .css_classes(["settings-group-title"])
        .build();
    pop_body.append(&pal_label);

    let pal_grid = gtk4::FlowBox::builder()
        .max_children_per_line(6)
        .selection_mode(gtk4::SelectionMode::None)
        .css_classes(["settings-presets"])
        .build();

    let swatches = [
        ("#32302f", "Gruvbox Soft"),
        ("#1d2021", "Gruvbox Dark"),
        ("#282828", "Dark Neutral"),
        ("#689d6a", "Aqua / Teal"),
        ("#458588", "Blue"),
        ("#b8bb26", "Green"),
        ("#fabd2f", "Yellow"),
        ("#fe8019", "Orange"),
        ("#fb4934", "Red"),
        ("#d3869b", "Purple"),
        ("#70c0ba", "Ice Cyan"),
        ("#ebdbb2", "Light Cream"),
    ];

    let hex_entry = gtk4::Entry::builder()
        .text("#32302f")
        .max_length(7)
        .width_chars(8)
        .css_classes(["settings-row-value"])
        .build();

    let r_scale = gtk4::Scale::with_range(gtk4::Orientation::Horizontal, 0.0, 255.0, 1.0);
    r_scale.set_hexpand(true);
    r_scale.add_css_class("settings-scale");
    let g_scale = gtk4::Scale::with_range(gtk4::Orientation::Horizontal, 0.0, 255.0, 1.0);
    g_scale.set_hexpand(true);
    g_scale.add_css_class("settings-scale");
    let b_scale = gtk4::Scale::with_range(gtk4::Orientation::Horizontal, 0.0, 255.0, 1.0);
    b_scale.set_hexpand(true);
    b_scale.add_css_class("settings-scale");

    for (hex, name) in swatches {
        let btn = gtk4::Button::builder()
            .tooltip_text(name)
            .css_classes(["settings-preset-btn"])
            .build();
        let swatch_da = gtk4::DrawingArea::builder()
            .content_width(20)
            .content_height(14)
            .build();
        let v = u32::from_str_radix(&hex[1..], 16).unwrap_or(0);
        let (sr, sg, sb) = (
            ((v >> 16) & 0xff) as f64 / 255.0,
            ((v >> 8) & 0xff) as f64 / 255.0,
            (v & 0xff) as f64 / 255.0,
        );
        swatch_da.set_draw_func(move |_, cr, _, _| {
            cr.set_source_rgb(sr, sg, sb);
            let _ = cr.paint();
        });
        btn.set_child(Some(&swatch_da));

        {
            let state = state.clone();
            btn.connect_clicked(move |_| {
                state
                    .tuning
                    .borrow_mut()
                    .material
                    .set_fill_rgb(Some((sr, sg, sb)));
                state.edited();
            });
        }
        pal_grid.append(&btn);
    }
    pop_body.append(&pal_grid);

    let rgb_box = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(4)
        .build();

    let make_channel_row = |name: &str, scale: &gtk4::Scale| -> gtk4::Box {
        let row = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(8)
            .build();
        let lbl = gtk4::Label::builder()
            .label(name)
            .width_chars(2)
            .css_classes(["settings-row-label"])
            .build();
        row.append(&lbl);
        row.append(scale);
        row
    };

    rgb_box.append(&make_channel_row("R", &r_scale));
    rgb_box.append(&make_channel_row("G", &g_scale));
    rgb_box.append(&make_channel_row("B", &b_scale));
    pop_body.append(&rgb_box);

    let hex_row = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(8)
        .build();
    let hex_lbl = gtk4::Label::builder()
        .label("Hex:")
        .css_classes(["settings-row-label"])
        .build();
    hex_row.append(&hex_lbl);
    hex_row.append(&hex_entry);
    pop_body.append(&hex_row);

    popover.set_child(Some(&pop_body));

    {
        let popover = popover.clone();
        button.connect_clicked(move |_| {
            popover.popup();
        });
    }

    {
        let state = state.clone();
        let rs = r_scale.clone();
        let gs = g_scale.clone();
        let bs = b_scale.clone();
        let update_from_scales = move || {
            if state.updating.get() {
                return;
            }
            let r = rs.value() / 255.0;
            let g = gs.value() / 255.0;
            let b = bs.value() / 255.0;
            state
                .tuning
                .borrow_mut()
                .material
                .set_fill_rgb(Some((r, g, b)));
            state.edited();
        };

        let u1 = update_from_scales.clone();
        r_scale.connect_value_changed(move |_| u1());
        let u2 = update_from_scales.clone();
        g_scale.connect_value_changed(move |_| u2());
        let u3 = update_from_scales.clone();
        b_scale.connect_value_changed(move |_| u3());
    }

    {
        let state = state.clone();
        hex_entry.connect_text_notify(move |e| {
            if state.updating.get() {
                return;
            }
            let text = e.text();
            let hex = text.strip_prefix('#').unwrap_or(&text);
            if hex.len() == 6
                && let Ok(v) = u32::from_str_radix(hex, 16)
            {
                let r = ((v >> 16) & 0xff) as f64 / 255.0;
                let g = ((v >> 8) & 0xff) as f64 / 255.0;
                let b = (v & 0xff) as f64 / 255.0;
                state
                    .tuning
                    .borrow_mut()
                    .material
                    .set_fill_rgb(Some((r, g, b)));
                state.edited();
            }
        });
    }

    let own_color = gtk4::CheckButton::with_label("Card's own colour");
    own_color.set_tooltip_text(Some(
        "Take the fill's colour from swaypplet's stylesheet, as the material did before this knob existed.",
    ));
    {
        let state = state.clone();
        own_color.connect_toggled(move |c| {
            if state.updating.get() {
                return;
            }
            if c.is_active() {
                state.tuning.borrow_mut().material.set_fill_rgb(None);
                state.edited();
            }
            // Unchecking on its own says nothing about which colour is wanted,
            // and the button beside it is already showing one. Picking from it
            // is what turns the override on, and that clears this.
        });
    }

    let own_alpha = gtk4::CheckButton::with_label("Card's own alpha");
    own_alpha.set_tooltip_text(Some(
        "Take the fill's alpha from swaypplet's stylesheet. Unchecked, the slider below is authoritative and 0 is clear glass.",
    ));
    {
        let state = state.clone();
        own_alpha.connect_toggled(move |c| {
            if state.updating.get() {
                return;
            }
            if c.is_active() {
                state.tuning.borrow_mut().material.fill_alpha = -1.0;
                state.edited();
            }
        });
    }

    {
        let swatch = swatch.clone();
        let hex_label = hex_label.clone();
        let hex_entry = hex_entry.clone();
        let r_scale = r_scale.clone();
        let g_scale = g_scale.clone();
        let b_scale = b_scale.clone();
        let own_color = own_color.clone();
        let own_alpha = own_alpha.clone();
        let current_rgb = current_rgb.clone();

        state.sync.borrow_mut().push(Box::new(move |t| {
            let rgb = t.material.fill_rgb();
            own_color.set_active(rgb.is_none());
            own_alpha.set_active(t.material.fill_alpha < 0.0);
            if let Some((r, g, b)) = rgb {
                current_rgb.set((r, g, b));
                swatch.queue_draw();
                let hex = format!(
                    "#{:02x}{:02x}{:02x}",
                    (r * 255.0).round() as u8,
                    (g * 255.0).round() as u8,
                    (b * 255.0).round() as u8
                );
                hex_label.set_text(&hex);
                hex_entry.set_text(&hex);
                r_scale.set_value(r * 255.0);
                g_scale.set_value(g * 255.0);
                b_scale.set_value(b * 255.0);
            } else {
                current_rgb.set((0.196, 0.188, 0.184)); // default @surface (#32302f)
                swatch.queue_draw();
                hex_label.set_text("card default");
            }
        }));
    }

    container.append(&kind_row("Fill colour", &button));
    container.append(&own_color);
    container.append(&own_alpha);
}

fn build_knob(state: &Rc<State>, knob: &'static Knob) -> gtk4::Box {
    let row = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(10)
        .build();
    row.add_css_class("settings-row");
    row.set_tooltip_text(Some(knob.hint));
    if let Some(live_when) = knob.live_when {
        let row = row.clone();
        state
            .sync
            .borrow_mut()
            .push(Box::new(move |t| row.set_sensitive(live_when(t))));
    }

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
            (knob.set)(&mut state.tuning.borrow_mut(), raw);
            state.edited();
        });
    }
    {
        let scale = scale.clone();
        let value = value.clone();
        state.sync.borrow_mut().push(Box::new(move |t| {
            let v = (knob.get)(t);
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

/// The glass footer keeps its own status label (the pane's `State` holds
/// it), so the shared footer's is swapped for it.
fn build_footer(state: &Rc<State>, status: &gtk4::Label, undo_btn: &gtk4::Button) -> gtk4::Box {
    let reset = super::ui::action_button(
        "Reset to system",
        "Put the shipped material back and delete the override file.",
    );
    {
        let state = state.clone();
        reset.connect_clicked(move |_| {
            let tuning = Tuning::system(&state.system);
            state.replace(tuning, false);
        });
    }
    let copy = super::ui::copy_button(
        status,
        "The material as a glass.nix attrset body, for promoting a keeper into the Nix side by hand.",
        "Copied — paste into material = { … } in theme/glass.nix",
        {
            let state = state.clone();
            move || Some(state.tuning.borrow().as_nix(&state.system))
        },
    );

    let (footer, shared_status) = super::ui::footer(&[undo_btn, &reset, &copy]);
    footer.remove(&shared_status);
    footer.append(status);
    footer
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tuning built on one preset, with the geometry left where the system
    /// config would have put it.
    fn probe() -> Tuning {
        Tuning {
            material: preset::clear(),
            bezel_scale: 1.0,
            thickness_ratio: 0.0,
            crest_scale: 1.0,
        }
    }

    /// Every knob, flattened out of the groups.
    fn knobs() -> impl Iterator<Item = &'static Knob> {
        GROUPS.iter().flat_map(|g| g.knobs)
    }

    #[test]
    fn every_knob_range_contains_what_the_presets_ask_for() {
        // A preset outside a knob's range is a preset the slider silently
        // clamps, so the pane would show a different material than the one it
        // just pushed at the compositor.
        for p in &preset::ALL {
            let t = Tuning {
                material: p.material(),
                ..probe()
            };
            for knob in knobs() {
                let v = (knob.get)(&t);
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

    #[test]
    fn every_numeric_field_has_exactly_one_knob() {
        // The knob table and `Tuning` are edited in different places; a field
        // with no knob is invisible in the pane and a field with two is two
        // sliders fighting over one number. Identify a knob by what its setter
        // moves — the twenty material numbers, plus the two geometry ones that
        // are not in `Material` at all.
        let base = probe();
        let mut seen = Vec::new();
        for knob in knobs() {
            let mut t = base.clone();
            (knob.set)(&mut t, (knob.get)(&base) + knob.step);

            let mut changed: Vec<&str> = base
                .material
                .numbers()
                .iter()
                .zip(t.material.numbers().iter())
                .filter(|((_, a), (_, b))| a != b)
                .map(|((name, _), _)| *name)
                .collect();
            if t.bezel_scale != base.bezel_scale {
                changed.push("bezel_scale");
            }
            if t.thickness_ratio != base.thickness_ratio {
                changed.push("thickness_ratio");
            }
            if t.crest_scale != base.crest_scale {
                changed.push("crest_scale");
            }
            assert_eq!(changed.len(), 1, "{} moved {changed:?}", knob.label);
            seen.push(changed[0]);
        }

        seen.sort_unstable();
        let mut all: Vec<&str> = base.material.numbers().iter().map(|(n, _)| *n).collect();
        all.push("bezel_scale");
        all.push("thickness_ratio");
        all.push("crest_scale");
        all.sort_unstable();
        assert_eq!(seen, all, "knob table and Tuning disagree");
    }

    #[test]
    fn a_knobs_step_divides_its_range() {
        // Otherwise the rail's top end is unreachable: the snap in
        // `build_knob` rounds to a multiple of `step`, and a max that is not
        // one can never be selected.
        for knob in knobs() {
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

    #[test]
    fn the_geometry_knobs_start_where_the_system_config_does() {
        // Both defaults have to be the identity, or opening the pane would
        // move the geometry before anything was touched.
        let t = probe();
        let shipped = glass::Geometry {
            bezel: 10.0,
            thickness: 39.0,
            crest_radius: 14.0,
        };
        let got = t.geometry(shipped);
        assert_eq!(got.bezel, 10.0);
        assert_eq!(got.thickness, 39.0);
        assert_eq!(got.crest_radius, 14.0);
    }
}
