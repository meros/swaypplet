//! The Glass tab of the settings pane: the liquid-glass material, edited
//! against the card it is drawn on.
//!
//! Every control here means the same in dark and light (docs/design-system.md
//! §4). The mode owns six values (fill colour and alpha, absorb,
//! photochromic, edge light, frost) and sets them in `glass::for_mode`; the
//! tab moves them only relative to the mode, with Clarity and Frost. The
//! rest is one material in both modes: the profile, refraction, dispersion,
//! the highlight and the bevel. The esoteric numbers (samples, the
//! highlight's exponent, the reflection blur) stay as shipped and have no
//! slider: `glass.nix` and tools/glass-bench in the nixos repo are the bench
//! for those.
//!
//! It edits the compositor live rather than on OK. A material is not a value
//! you can predict from its numbers, so the pane's job is to put the slider
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

use super::form::{self, kind_row, pretty_path, section_box};
use super::glass::{self, SurfaceKind, System, Tuning};
use super::preset;

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

/// One numeric property of the tuning, and how to show it.
struct Knob {
    label: &'static str,
    hint: &'static str,
    min: f64,
    max: f64,
    step: f64,
    decimals: usize,
    get: fn(&Tuning) -> f64,
    set: fn(&mut Tuning, f64),
}

/// Re-reads one control from the tuning. One per widget, so a preset click
/// is a loop over these rather than a list of widget clones `State` would
/// otherwise have to hold by name.
type Sync = Box<dyn Fn(&Tuning)>;

/// The material's sliders, each the same move in both modes. Ranges are
/// what the shader still draws something at; Clarity's ends are safe by
/// construction (`tokens::material_at` keeps the text readable).
static MATERIAL: &[Knob] = &[
    Knob {
        label: "Clarity",
        hint: "How much of the backdrop shows through. 0 is the mode's own body; the text stays readable at either end.",
        min: -1.0,
        max: 1.0,
        step: 0.05,
        decimals: 2,
        get: |t| t.clarity,
        set: |t, v| t.clarity = v,
    },
    Knob {
        label: "Frost",
        hint: "Blur of the backdrop, as a multiple of the mode's own.",
        min: 0.0,
        max: 3.0,
        step: 0.05,
        decimals: 2,
        get: |t| t.frost_scale,
        set: |t, v| t.frost_scale = v,
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
    },
    Knob {
        label: "Highlight",
        hint: "The directional highlight on the top of the card.",
        min: 0.0,
        max: 1.0,
        step: 0.01,
        decimals: 2,
        get: |t| t.material.specular,
        set: |t, v| t.material.specular = v,
    },
    Knob {
        label: "Bevel",
        hint: "Multiplies the bevel's width and depth on every surface, so the slope the light bends on scales with it.",
        min: 0.25,
        max: 3.0,
        step: 0.05,
        decimals: 2,
        get: |t| t.bezel_scale,
        set: |t, v| t.bezel_scale = v,
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
                // Through the mode, like every other path to the
                // compositor: the raw tuning carries the dark material.
                let tuning = glass::for_mode(this.tuning.borrow().clone(), crate::theme::shown());
                this.system.apply(&tuning);
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
        } else {
            self.status
                .set_text("System default, as the sway config ships it");
        }
        form::mark_source(&self.status, !modified);
    }
}

// ── The section ─────────────────────────────────────────────────────────

pub struct GlassPane {
    root: gtk4::Box,
    state: Option<Rc<State>>,
}

impl GlassPane {
    pub fn new() -> Self {
        let root = form::pane();

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

        // Capped like every other wrapping label in the pane (`form::HINT_CHARS`,
        // for what an uncapped one does to the whole tab's width). This tab
        // builds its own status rather than taking `form::footer`'s, because it
        // also carries the Undo button.
        let status = form::status_label();

        let undo_btn = form::action_button("Undo", "Revert the last tuning or preset change.");
        undo_btn.set_sensitive(false);

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

        let material = section_box(
            "Material",
            "The same in dark and light. Clarity and Frost move the mode's own body; the rest is the glass itself.",
        );
        for knob in MATERIAL {
            material.append(&build_knob(&state, knob));
        }
        root.append(&material);

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
    let note = crate::ui::group(2);
    note.add_css_class("settings-empty");

    let title = crate::ui::text(
        "No glass configuration on this host",
        crate::ui::Text::Body,
        crate::ui::Tone::Fg,
    );
    crate::ui::set_weight(&title, crate::ui::Weight::Strong);

    let body = crate::ui::text(
        "The material and the surfaces it applies to come from \
         /etc/swaypplet/glass.json, written by the NixOS side \
         (users/modules/theme/glass-config.nix). Without it there is no \
         baseline to edit against.",
        crate::ui::Text::Caption,
        crate::ui::Tone::Muted,
    );
    body.set_wrap(true);
    body.set_max_width_chars(form::HINT_CHARS);

    note.append(&title);
    note.append(&body);
    note
}

fn build_presets(state: &Rc<State>) -> gtk4::Box {
    let group = section_box(
        "Presets",
        "The shipped material, and other coherent glass. Each one means the same in dark and light.",
    );

    // A grid of three columns rather than a FlowBox, because a FlowBox is a
    // height-for-width widget and GTK could not get a consistent answer out
    // of this one inside the pane's fixed column: "min width of 898 for
    // height of 581, but min height of 581 for width of 572", after which it
    // took the 898 and the whole tab hung off the right of the card. The
    // number of presets is known and small, so nothing here needs to reflow.
    let row = gtk4::Grid::builder()
        .row_spacing(crate::tokens::space(2))
        .column_spacing(crate::tokens::space(2))
        .column_homogeneous(true)
        .build();
    row.add_css_class("settings-presets");
    let mut slot = 0i32;
    let mut place = |btn: &gtk4::Button| {
        row.attach(btn, slot % 3, slot / 3, 1, 1);
        slot += 1;
    };

    let system_btn = form::preset_button("System");
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
    place(&system_btn);

    for p in &preset::ALL {
        let btn = form::preset_button(p.name);
        btn.set_tooltip_text(Some(p.hint));
        let state = state.clone();
        // A preset is a whole tuning, geometry and clarity included: keeping
        // a bevel scale from whatever was being tried before would make the
        // same preset land differently depending on what preceded it.
        btn.connect_clicked(move |_| state.replace(p.tuning(), true));
        place(&btn);
    }

    group.append(&row);
    group
}

/// The named property. A dropdown rather than a slider because sway takes it
/// as a name, and an unknown one costs the whole `layer_effects` block.
fn build_kinds(state: &Rc<State>) -> gtk4::Box {
    let group = section_box("Profile", "The bevel's height profile.");

    let surface_labels: Vec<&str> = SurfaceKind::ALL.iter().map(|k| k.label()).collect();
    let surface = form::dropdown(&surface_labels);
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
    group
}

fn build_knob(state: &Rc<State>, knob: &'static Knob) -> gtk4::Box {
    let row = form::row();
    row.set_tooltip_text(Some(knob.hint));

    row.append(&form::row_label(knob.label));

    let scale = form::scale(knob.min, knob.max, knob.step);
    let value = form::value_label();

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
    let reset = super::form::action_button(
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
    let copy = super::form::copy_button(
        status,
        "The material as a glass.nix attrset body, for promoting a keeper into the Nix side by hand.",
        "Copied — paste into material = { … } in theme/glass.nix",
        {
            let state = state.clone();
            move || Some(state.tuning.borrow().as_nix(&state.system))
        },
    );

    let (footer, shared_status) = super::form::footer(&[undo_btn, &reset, &copy]);
    footer.remove(&shared_status);
    footer.append(status);
    footer
}

// ── Search ──────────────────────────────────────────────────────────────

use super::search::{Entry, row};

/// This tab's rows as the launcher finds them (`search.rs`). A row added
/// to the tab gets a line here; the test there fails until it does.
#[rustfmt::skip]
pub(super) const SEARCH: &[Entry] = &[
    row("Presets", "", "The shipped glass and other coherent ones", &["preset", "glass style", "material"]),
    row("Material", "Clarity", "How much of the backdrop shows through", &["transparency", "transparent", "opacity", "see through", "translucent"]),
    row("Material", "Frost", "Blur of the backdrop", &["blur", "frosted", "frosting", "blur radius"]),
    row("Material", "Refraction", "How far the slab bends the backdrop", &["bend", "distortion", "lens", "index"]),
    row("Material", "Dispersion", "Colour split through the bevel", &["chromatic aberration", "rainbow", "prism"]),
    row("Material", "Highlight", "The light on the top of the card", &["specular", "shine", "gloss"]),
    row("Material", "Bevel", "The width and depth of the edge", &["edge", "bezel", "depth", "border"]),
    row("Profile", "Surface", "The bevel's height profile", &["shape", "curve"]),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Every knob on the tab.
    fn knobs() -> impl Iterator<Item = &'static Knob> {
        MATERIAL.iter()
    }

    /// What a knob's setter moves, named, so two knobs on one number show.
    fn moved(a: &Tuning, b: &Tuning) -> Vec<&'static str> {
        let mut changed: Vec<&str> = a
            .material
            .numbers()
            .iter()
            .zip(b.material.numbers().iter())
            .filter(|((_, x), (_, y))| x != y)
            .map(|((name, _), _)| *name)
            .collect();
        for (name, x, y) in [
            ("bezel_scale", a.bezel_scale, b.bezel_scale),
            ("thickness_ratio", a.thickness_ratio, b.thickness_ratio),
            ("clarity", a.clarity, b.clarity),
            ("frost_scale", a.frost_scale, b.frost_scale),
        ] {
            if x != y {
                changed.push(name);
            }
        }
        changed
    }

    #[test]
    fn every_knob_range_contains_what_the_presets_ask_for() {
        // A preset outside a knob's range is a preset the slider silently
        // clamps, so the pane would show a different material than the one it
        // just pushed at the compositor.
        for p in &preset::ALL {
            let t = p.tuning();
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
    fn each_knob_moves_one_number_and_no_two_share_one() {
        let base = preset::ALL[0].tuning();
        let mut seen = Vec::new();
        for knob in knobs() {
            let mut t = base.clone();
            (knob.set)(&mut t, (knob.get)(&base) + knob.step);
            let changed = moved(&base, &t);
            assert_eq!(changed.len(), 1, "{} moved {changed:?}", knob.label);
            assert!(!seen.contains(&changed[0]), "two knobs on {}", changed[0]);
            seen.push(changed[0]);
        }
    }

    /// The tab never offers a raw handle on what the mode owns: those go
    /// through Clarity and Frost, the same move in both modes.
    #[test]
    fn no_knob_sets_what_the_mode_owns() {
        let base = preset::ALL[0].tuning();
        for knob in knobs() {
            let mut t = base.clone();
            (knob.set)(&mut t, (knob.get)(&base) + knob.step);
            for owned in [
                "absorb",
                "photochromic",
                "edge_light",
                "frost",
                "fill_alpha",
            ] {
                assert!(
                    !moved(&base, &t).contains(&owned),
                    "{} sets {owned}",
                    knob.label
                );
            }
        }
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
    fn the_geometry_starts_where_the_system_config_does() {
        // The defaults have to be the identity, or opening the pane would
        // move the geometry before anything was touched.
        let t = preset::ALL[0].tuning();
        let shipped = glass::Geometry {
            bezel: 10.0,
            thickness: 39.0,
        };
        let got = t.geometry(shipped);
        assert_eq!(got.bezel, 10.0);
        assert_eq!(got.thickness, 39.0);
    }
}
