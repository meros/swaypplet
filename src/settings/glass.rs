//! The liquid-glass material, as something that can be read, edited and
//! pushed at a running compositor.
//!
//! Nix owns the shipped material (nixos `users/modules/theme/glass.nix`) and
//! writes it, the four geometries and the namespace table to
//! `/etc/swaypplet/glass.json`. That file is the *system default*: the state a
//! Reset returns to, and the baseline an override is an override of. Its
//! absence means this host does not configure glass, and the pane hides the
//! group rather than editing numbers it had to invent — the same feature-flag
//! shape `switch_user::config()` uses.
//!
//! An edit goes two places. It goes at the compositor immediately, over IPC,
//! because a material you cannot see while you drag the slider is not being
//! tuned; and it goes to `~/.config/swaypplet/glass.json` as a [`Tuning`], which `apply_saved`
//! replays when the panel starts so a session restart does not silently undo
//! it. Nothing here writes to the Nix side — `Material::as_nix` renders the
//! attrset for a keeper to be promoted into `glass.nix` by hand.
//!
//! ## What is deliberately not editable
//!
//! `mask_threshold` is in the system file and is never written back. It is not
//! a look: it decides which of the card's pixels the body tint takes its
//! colour from, against the 0.50 every card paints the fill key at (see
//! `glass.nix`). A slider is the wrong instrument for that.
//!
//! `liquid_glass enable|disable` is not written either, and neither are
//! `blur_ignore_transparent` or `corner_radius`. The first belongs to
//! `anim::set_layer_blur`, which counts a namespace's live surfaces and
//! toggles the material at the population boundaries; the other two belong to
//! the sway config. All three survive a write from here because
//! `layer_criteria_add` clones a namespace's existing effects before parsing
//! the new list — which is also why this module may send a partial material
//! and get a whole one.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Where Nix leaves the shipped material. `SWAYPPLET_GLASS_CONFIG` overrides
/// it, for the render harness and for trying a material without a rebuild.
const SYSTEM_CONFIG: &str = "/etc/swaypplet/glass.json";

// ── The material ────────────────────────────────────────────────────────

/// The height profile of the bevel.
///
/// Named rather than numbered, exactly as the config is: sway rejects a name
/// it does not know and takes the whole `layer_effects` block down with it,
/// so the set is closed here for the same reason `glass-config.nix` asserts
/// it is one of two.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceKind {
    ConvexSquircle,
    Droplet,
}

impl SurfaceKind {
    pub const ALL: [SurfaceKind; 2] = [SurfaceKind::ConvexSquircle, SurfaceKind::Droplet];

    /// The spelling sway parses.
    pub fn as_str(self) -> &'static str {
        match self {
            SurfaceKind::ConvexSquircle => "convex_squircle",
            SurfaceKind::Droplet => "droplet",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SurfaceKind::ConvexSquircle => "Convex squircle",
            SurfaceKind::Droplet => "Droplet \u{2014} surface tension",
        }
    }
}

/// Every `liquid_glass_*` value that describes the material rather than the
/// surface it is drawn on. Field names are the sway spellings, so the writer
/// below is a formatting loop and not a translation table.
///
/// `frost`, `shine` and `reflect_blur` are direct values. They used to be
/// derived from a `roughness` whenever they were 0; that knob is gone, and 0
/// now means what it says (no frost, the broadest highlight, a sharp
/// reflection).
///
/// Unknown fields are ignored rather than refused, which is what keeps an
/// override written before a knob was removed loading: `roughness`, `haze`,
/// `noise`, `energy_comp`, the grain, the thin-film, glow and wave effects
/// `edge_light` and `absorb_floor` all lived here once. The absorption is
/// constant across the card now: the tint is the body's, for legibility, not
/// the slab's thickness, so there is no floor for a thin rim to sit on.
///
/// `contact_angle`, `tail` and `tail_length` shape the droplet's profile and
/// do nothing on the squircle: the edge's slope in degrees, and the weight
/// and length of the long, gentle part that runs out to the flat top. The
/// length is a multiple of the slab's thickness since 2026-10-05 (it was a
/// multiple of the steep edge's, 1 to 12), so it can outrun a thin bar and
/// round it into a dome. Missing from an older override, they load as the
/// shipped droplet, and an old-style length loads as the shipped one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Material {
    pub surface: SurfaceKind,
    pub refraction: f64,
    pub dispersion: f64,
    pub samples: f64,
    pub reflection: f64,
    pub lensing: f64,
    pub frost_radius: f64,
    pub absorb: f64,
    pub photochromic: f64,
    pub specular: f64,
    pub frost: f64,
    pub shine: f64,
    pub reflect_blur: f64,
    #[serde(default = "shipped_contact_angle")]
    pub contact_angle: f64,
    #[serde(default = "shipped_tail")]
    pub tail: f64,
    #[serde(default = "shipped_tail_length")]
    pub tail_length: f64,
    /// The fill the compositor paints under swaypplet's own content, as
    /// `#rrggbb`, or the literal `none` for the card's own colour.
    ///
    /// A `String` rather than an `Option`, because the sway config's spelling
    /// is `none` and the whole point of this struct is that the writer below
    /// is a formatting loop rather than a translation table. `none` also has
    /// to survive the wire: a namespace's effects are parsed on top of what
    /// it already has, so *omitting* the key means "keep the override", which
    /// is the opposite of what clearing one means.
    #[serde(default = "unset_color")]
    pub fill_color: String,
    /// The alpha that fill is painted at. Negative is the card's own.
    ///
    /// Set, it is authoritative in both directions, and that is why it exists
    /// at all: the card's alpha is also the mask, so the stylesheet cannot
    /// turn the fill down without the compositor losing the card. This turns
    /// it down on the far side of the mask instead, so 0 is clear glass under
    /// a card swaypplet is still painting at 0.50. See `glass.nix`.
    #[serde(default = "unset")]
    pub fill_alpha: f64,
}

/// The sentinels. Both are also the `#[serde(default)]`s, so an override
/// written before these fields existed loads as "the card decides", which is
/// what it meant.
fn unset() -> f64 {
    -1.0
}

fn unset_color() -> String {
    "none".to_string()
}

fn shipped_contact_angle() -> f64 {
    89.5
}

fn shipped_tail() -> f64 {
    0.5
}

fn shipped_tail_length() -> f64 {
    0.1
}

/// The longest tail the length's new meaning allows; anything past it in a
/// saved override is the old meaning (1 to 12 steep edges).
const TAIL_LENGTH_MAX: f64 = 1.5;

impl Material {
    /// The fill colour as the compositor wants it, or `None` when the card
    /// decides. Anything unparseable is treated as unset rather than as an
    /// error: this value reaches here from a file a human may have edited,
    /// and the card's own colour is always a safe answer.
    pub fn fill_rgb(&self) -> Option<(f64, f64, f64)> {
        let hex = self
            .fill_color
            .strip_prefix('#')
            .unwrap_or(&self.fill_color);
        if hex.len() != 6 {
            return None;
        }
        let v = u32::from_str_radix(hex, 16).ok()?;
        Some((
            ((v >> 16) & 0xff) as f64 / 255.0,
            ((v >> 8) & 0xff) as f64 / 255.0,
            (v & 0xff) as f64 / 255.0,
        ))
    }

    /// Set it from a colour, or clear it back to the card's own.
    pub fn set_fill_rgb(&mut self, rgb: Option<(f64, f64, f64)>) {
        self.fill_color = match rgb {
            Some((r, g, b)) => format!(
                "#{:02x}{:02x}{:02x}",
                (r.clamp(0.0, 1.0) * 255.0).round() as u8,
                (g.clamp(0.0, 1.0) * 255.0).round() as u8,
                (b.clamp(0.0, 1.0) * 255.0).round() as u8
            ),
            None => unset_color(),
        };
    }
}

/// The material plus what the pane is allowed to do to a surface's geometry.
///
/// Geometry is not material — `glass.nix` gives each class of surface its own
/// bezel and thickness, and the pane has no business inventing a fifth class.
/// What it can do is scale the four it was given, which is the one geometry
/// move that measurably shows: thickness alone is nearly invisible (the shader
/// normalises every depth-driven term by it, and on a flat top the normal is
/// vertical so no amount of thickness bends a ray), and bezel alone only
/// changes how wide the band that shows any of this is. Together they change
/// the bevel's *slope*, which is what decides how the light bends and is why
/// `glass.nix` ties the two at a fixed ratio in the first place.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tuning {
    pub material: Material,
    /// Multiplies both bezel and thickness, every class, so the classes keep
    /// their relationship to each other and each keeps its own ratio. 1 is
    /// what the system config ships.
    #[serde(default = "unit")]
    pub bezel_scale: f64,
    /// Thickness as a multiple of the scaled bezel. Zero keeps each class's
    /// own shipped ratio, which is the only value that leaves a bar and a lock
    /// card reading as one material rather than two thicknesses of it — so
    /// this is the knob for deliberately breaking that, not for setting it.
    #[serde(default)]
    pub thickness_ratio: f64,
    /// How much of the backdrop shows through, relative to the mode: 0 is
    /// the mode's own body fill, +1 thins it, −1 thickens it
    /// (`tokens::material_at`, which also keeps the text readable). The one
    /// move on the five values the mode owns, and the same move in both.
    #[serde(default)]
    pub clarity: f64,
    /// Multiplies the mode's frost. 1 is what the mode ships.
    #[serde(default = "unit")]
    pub frost_scale: f64,
}

fn unit() -> f64 {
    1.0
}

impl Tuning {
    /// The system material, untouched.
    pub fn system(system: &System) -> Tuning {
        Tuning {
            material: system.material.clone(),
            bezel_scale: 1.0,
            thickness_ratio: 0.0,
            clarity: 0.0,
            frost_scale: 1.0,
        }
    }

    /// What this tuning makes of one class's shipped geometry.
    pub fn geometry(&self, shipped: Geometry) -> Geometry {
        let bezel = shipped.bezel * self.bezel_scale;
        let thickness = if self.thickness_ratio > 0.0 {
            bezel * self.thickness_ratio
        } else {
            shipped.thickness * self.bezel_scale
        };
        Geometry { bezel, thickness }
    }
}

// ── The system's copy ───────────────────────────────────────────────────

/// One class's bevel. A system config written while `crest_radius` was still
/// in it loads too: the field is ignored, like any other unknown one.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Geometry {
    pub bezel: f64,
    pub thickness: f64,
}

/// `/etc/swaypplet/glass.json`, whole.
#[derive(Debug, Clone, Deserialize)]
pub struct System {
    /// What the sway config ships. Reset goes back to exactly this.
    pub material: Material,
    /// Read and re-emitted untouched; see the module header for why it is not
    /// a setting.
    pub mask_threshold: f64,
    /// Bezel and thickness per class.
    pub geometries: BTreeMap<String, Geometry>,
    /// Layer-shell namespace to geometry class. Generated from the same table
    /// the sway config's `layer_effects` blocks are, so a live edit reaches
    /// exactly the surfaces the config configures.
    pub surfaces: BTreeMap<String, String>,
}

impl System {
    /// The shipped material and its surfaces, or `None` on a host that does
    /// not configure glass.
    pub fn load() -> Option<System> {
        let path =
            std::env::var("SWAYPPLET_GLASS_CONFIG").unwrap_or_else(|_| SYSTEM_CONFIG.to_string());
        let raw = std::fs::read(&path).ok()?;
        match serde_json::from_slice::<System>(&raw) {
            Ok(system) => {
                for line in system.drift() {
                    log::warn!("glass: {path}: {line}");
                }
                Some(system)
            }
            Err(e) => {
                log::warn!("glass: bad system config at {path}: {e}");
                None
            }
        }
    }

    /// Where the shipped `surfaces` table and [`Namespace::glass`] disagree,
    /// one line each; empty when they agree.
    ///
    /// Both are hand-kept copies of one fact, which surfaces get which glass,
    /// held on two sides of a repository boundary (the nixos repo's
    /// `sessionSurfaces`, and the code that names the surfaces). A namespace
    /// in the file that no surface asks for is a row the pane pushes edits
    /// at for nothing (a renamed or deleted surface); a glass namespace the
    /// file lacks is a surface the compositor leaves bare and the pane never
    /// reaches; a different class is a card whose bezel disagrees with its
    /// kind.
    ///
    /// [`Namespace::glass`]: crate::shell::Namespace::glass
    pub fn drift(&self) -> Vec<String> {
        use crate::shell::Namespace;
        let mut out = Vec::new();
        for (name, class) in &self.surfaces {
            match Namespace::parse(name) {
                None => out.push(format!("`{name}` is no swaypplet namespace")),
                Some(ns) => match ns.glass() {
                    None => out.push(format!("`{name}` is not meant to have glass")),
                    Some(want) if want != class => out.push(format!(
                        "`{name}` has geometry `{class}`, the code expects `{want}`"
                    )),
                    Some(_) => {}
                },
            }
        }
        for ns in Namespace::ALL {
            if ns.glass().is_some() && !self.surfaces.contains_key(ns.as_str()) {
                out.push(format!("`{ns}` has glass in the code but no row here"));
            }
        }
        out
    }

    /// One namespace's `liquid_glass_*` list, geometry folded in.
    ///
    /// A namespace whose geometry class is missing from `geometries` is
    /// skipped rather than sent without a bezel: sway would take the block
    /// and draw a slab with no bevel, which looks like a rendering bug rather
    /// than like a malformed config.
    fn effects(&self, namespace: &str, tuning: &Tuning) -> Option<String> {
        let class = self.surfaces.get(namespace)?;
        let shipped = self.geometries.get(class).copied().or_else(|| {
            log::warn!("glass: {namespace} wants geometry `{class}`, which the system config does not define");
            None
        })?;
        let geometry = tuning.geometry(shipped);

        let m = &tuning.material;
        let mut out = String::with_capacity(512);
        // Named rather than looped over a serialised map: the order is stable,
        // the enum is spelled by hand anyway, and a field added to
        // `Material` should fail to compile here rather than silently stop
        // being sent.
        for (name, value) in [
            ("refraction", m.refraction),
            ("dispersion", m.dispersion),
            ("samples", m.samples),
            ("reflection", m.reflection),
            ("lensing", m.lensing),
            ("frost_radius", m.frost_radius),
            ("absorb", m.absorb),
            ("photochromic", m.photochromic),
            ("specular", m.specular),
            ("frost", m.frost),
            ("shine", m.shine),
            ("reflect_blur", m.reflect_blur),
            ("contact_angle", m.contact_angle),
            ("tail", m.tail),
            ("tail_length", m.tail_length),
            ("fill_alpha", m.fill_alpha),
            ("bezel", geometry.bezel),
            ("thickness", geometry.thickness),
            ("mask_threshold", self.mask_threshold),
        ] {
            let _ = write!(out, "liquid_glass_{name} {value:.6}; ");
        }
        // The ones that are words rather than numbers.
        let _ = write!(
            out,
            "liquid_glass_surface {}; liquid_glass_fill_color {}",
            m.surface.as_str(),
            m.fill_color
        );
        Some(out)
    }

    /// The whole push, as one sway command sequence.
    ///
    /// One `layer_effects` per namespace, each effect list double-quoted.
    /// Both halves of that matter. Unquoted, sway's command splitter takes the
    /// first `;` as the end of the command and only one effect ever lands;
    /// quoted, `argsep` tracks the quote and `layer_criteria_parse` does the
    /// splitting instead. And one command per namespace rather than one effect
    /// per command, because a parse failure destroys the whole replacement
    /// criteria and leaves the surface with no material — so the smaller the
    /// number of commands that can fail independently, the better.
    pub fn command(&self, tuning: &Tuning) -> String {
        self.surfaces
            .keys()
            .filter_map(|ns| {
                self.effects(ns, tuning)
                    .map(|effects| format!("layer_effects \"{ns}\" \"{effects}\""))
            })
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// Push `material` at the running compositor, stopping a fade in
    /// progress. Sway answers `CMD_SUCCESS` even when a criteria failed to
    /// parse (`cmd_layer_effects` ignores a NULL result), so there is no reply
    /// worth reading; the push only waits for the one before it, so two can
    /// never land in the wrong order (`glass_fade`).
    pub fn apply(&self, tuning: &Tuning) {
        super::glass_fade::direct(self, tuning);
    }
}

// ── The user's override ─────────────────────────────────────────────────

/// `~/.config/swaypplet/glass.json`, absent until something is changed.
pub fn override_path() -> PathBuf {
    let dir = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    dir.join("swaypplet").join("glass.json")
}

/// The saved override, or `None` when there is none (or it no longer parses,
/// which is treated the same way — the system material is always a safe
/// answer and a stale file is not worth failing the panel over).
pub fn load_override() -> Option<Tuning> {
    let path = override_path();
    let raw = std::fs::read(&path).ok()?;
    match serde_json::from_slice::<Tuning>(&raw) {
        Ok(mut tuning) => {
            if tuning.material.tail_length > TAIL_LENGTH_MAX {
                log::info!(
                    "glass: tail_length {} in {} is the old relative length; using {}",
                    tuning.material.tail_length,
                    path.display(),
                    shipped_tail_length()
                );
                tuning.material.tail_length = shipped_tail_length();
            }
            Some(tuning)
        }
        Err(e) => {
            log::warn!(
                "glass: ignoring unreadable override at {}: {e}",
                path.display()
            );
            None
        }
    }
}

pub fn save_override(tuning: &Tuning) {
    let path = override_path();
    if let Some(parent) = path.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        log::warn!("glass: cannot create {}: {e}", parent.display());
        return;
    }
    match serde_json::to_vec_pretty(tuning) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                log::warn!("glass: cannot write {}: {e}", path.display());
            }
        }
        Err(e) => log::warn!("glass: cannot serialise tuning: {e}"),
    }
}

/// Drop the override, so the next start uses the system material again.
pub fn clear_override() {
    let path = override_path();
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => log::warn!("glass: cannot remove {}: {e}", path.display()),
    }
}

/// Replay the saved override at startup.
///
/// The sway config has already put the system material on every namespace by
/// the time anything here runs, so this is a no-op on a session that has never
/// been tuned — which is why it is a plain call in `app::run` rather than
/// something the panel has to remember to do. The same holds after a
/// `swaymsg reload`, the other caller: the config's material is back on.
pub fn apply_saved() {
    replay(crate::theme::inputs(), true);
}

/// [`apply_saved`] for `inputs` already resolved: what `theme::watch` hands
/// its material callback when the mode, the contrast or a full tint moved
/// the material, so the replay matches the stylesheet just loaded instead of
/// resolving the theme a second time.
///
/// Always sends. The compositor holds whatever the last switch sent, not the
/// config's material, so a switch back to the default look has to send it:
/// skipping it here is what left light glass under a dark stylesheet.
pub fn apply_saved_for(inputs: crate::tokens::Inputs) {
    replay(inputs, false);
}

/// Whether the compositor already shows what a replay would send: only when
/// it holds the sway config's material (`on_config`), nothing overrides it,
/// and the look is the one the config ships (dark, standard contrast, no
/// neutral cast).
fn config_already_shows(on_config: bool, overridden: bool, inputs: crate::tokens::Inputs) -> bool {
    on_config
        && !overridden
        && inputs.mode == crate::tokens::Mode::Dark
        && inputs.contrast == crate::tokens::Contrast::Standard
        && !inputs.tint.casts_neutral()
}

fn replay(inputs: crate::tokens::Inputs, on_config: bool) {
    let Some(system) = System::load() else {
        return;
    };
    let saved = load_override();
    if config_already_shows(on_config, saved.is_some(), inputs) {
        // Nothing to send, but a later mode switch fades from it.
        super::glass_fade::assume(for_mode(Tuning::system(&system), inputs));
        return;
    }
    if saved.is_some() {
        log::info!(
            "glass: replaying override from {}",
            override_path().display()
        );
    }
    let tuning = saved.unwrap_or_else(|| Tuning::system(&system));
    // Over `page`, with the stylesheet (`theme::fade`); at once when this is
    // the first push of the process.
    super::glass_fade::send(&system, for_mode(tuning, inputs));
}

/// Put the mode's glass on the greeter's own compositor.
///
/// The greeter's sway config ships the lock's dark material on
/// `swaypplet-greeter`, and nothing else sends to that compositor, so the
/// greeter sends it itself when its mode is light or changes. The system
/// material, never an override: the greeter user has none, and a login
/// screen is the machine's, not a person's.
pub fn apply_greeter(inputs: crate::tokens::Inputs) {
    let Some(mut system) = System::load() else {
        return;
    };
    system.surfaces = BTreeMap::from([(
        crate::shell::Namespace::Greeter.as_str().to_string(),
        "lock".to_string(),
    )]);
    super::glass_fade::send(&system, for_mode(Tuning::system(&system), inputs));
}

/// The material as the theme's mode tunes it (docs/design-system.md §4).
///
/// The mode owns five values (fill colour and alpha, absorb, photochromic,
/// frost) and always sets them, dark at standard contrast
/// included, so a tuning made in one mode means the same thing in the
/// other. What the tuning adds is relative: `clarity` moves the body fill
/// (`tokens::material_at`), `frost_scale` multiplies the mode's frost.
/// Everything else in the material (profile, refraction, dispersion,
/// highlight, geometry) is one material in both modes and passes through as
/// tuned.
///
/// The lock's glass follows the mode like every other namespace.
pub fn for_mode(mut tuning: Tuning, inputs: crate::tokens::Inputs) -> Tuning {
    tuning.material = mode_material(&tuning, inputs);
    tuning
}

/// `tuning`'s material with the five values `inputs`' mode owns.
fn mode_material(tuning: &Tuning, inputs: crate::tokens::Inputs) -> Material {
    let m = crate::tokens::material_at(inputs, tuning.clarity);
    Material {
        fill_color: m.fill_color.css(),
        fill_alpha: m.fill_alpha,
        absorb: m.absorb,
        photochromic: m.photochromic,
        frost: m.frost * tuning.frost_scale.max(0.0),
        ..tuning.material.clone()
    }
}

// ── Export ──────────────────────────────────────────────────────────────

impl Material {
    /// The material as the `glass.nix` attrset body, for promoting a keeper
    /// into the Nix side by hand.
    ///
    /// Deliberately not a whole file: `glass.nix` is mostly the argument for
    /// each number, and a generator that overwrote it would throw that away.
    /// This is what goes *inside* `material = { … }`, comments and all left
    /// where they are.
    pub fn as_nix(&self) -> String {
        let mut out = String::from(
            "# swaypplet settings pane, live values.\n# Into material = { \u{2026} }:\n",
        );
        for (name, value) in self.numbers() {
            let _ = writeln!(out, "{name} = {};", trim_float(value));
        }
        let _ = writeln!(out, "surface = \"{}\";", self.surface.as_str());
        let _ = writeln!(out, "fill_color = \"{}\";", self.fill_color);
        out
    }

    /// Every numeric field, in the order the export prints them.
    pub(super) fn numbers(&self) -> [(&'static str, f64); 16] {
        [
            ("refraction", self.refraction),
            ("dispersion", self.dispersion),
            ("samples", self.samples),
            ("reflection", self.reflection),
            ("lensing", self.lensing),
            ("frost_radius", self.frost_radius),
            ("absorb", self.absorb),
            ("photochromic", self.photochromic),
            ("specular", self.specular),
            ("frost", self.frost),
            ("shine", self.shine),
            ("reflect_blur", self.reflect_blur),
            ("contact_angle", self.contact_angle),
            ("tail", self.tail),
            ("tail_length", self.tail_length),
            ("fill_alpha", self.fill_alpha),
        ]
    }
}

impl Tuning {
    /// The whole tuning as `glass.nix` would hold it: the material body, and —
    /// only when the geometry was actually scaled — the four class attrsets
    /// that sit beside it at the file's top level rather than inside
    /// `material`. Untouched geometry prints nothing, so the usual export
    /// stays one pasteable block.
    pub fn as_nix(&self, system: &System) -> String {
        let mut out = self.material.as_nix();
        if self.clarity != 0.0 || self.frost_scale != 1.0 {
            let _ = write!(
                out,
                "\n# Clarity {} and frost \u{d7}{} are moves on the mode's own values, \
                 which glass.nix does not carry; tokens/material.rs owns those.\n",
                trim_float(self.clarity),
                trim_float(self.frost_scale)
            );
        }
        if self.bezel_scale == 1.0 && self.thickness_ratio == 0.0 {
            return out;
        }
        let ratio = if self.thickness_ratio > 0.0 {
            format!("ratio {}", trim_float(self.thickness_ratio))
        } else {
            "each class's own ratio kept".to_string()
        };
        let _ = write!(
            out,
            "\n# Geometry, at bezel scale {} ({ratio}). Top level, beside `material`:\n",
            trim_float(self.bezel_scale)
        );
        // Sorted, so two exports of the same tuning are the same text.
        for (class, shipped) in &system.geometries {
            let g = self.geometry(*shipped);
            let _ = writeln!(
                out,
                "{class} = {{ bezel = {}; thickness = {}; }};",
                trim_float(g.bezel),
                trim_float(g.thickness)
            );
        }
        out
    }
}

/// A float as Nix would have it written: no trailing zeros, and an integral
/// value stays an integer, because `samples = 4.000000;` is noise in a file
/// whose whole point is being read.
fn trim_float(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e9 {
        return format!("{}", value as i64);
    }
    let mut s = format!("{value:.4}");
    while s.ends_with('0') {
        s.pop();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::preset;
    use crate::shell::Namespace;

    /// A switch back to the default dark look sends the glass: the
    /// compositor holds the light material the last switch sent. Only a
    /// compositor fresh from its config may skip it.
    #[test]
    fn a_switch_to_dark_sends_the_glass() {
        let dark = crate::tokens::Inputs::default();
        assert_eq!(dark.mode, crate::tokens::Mode::Dark);
        assert!(!config_already_shows(false, false, dark));
        let light = crate::tokens::Inputs {
            mode: crate::tokens::Mode::Light,
            ..dark
        };
        assert!(!config_already_shows(true, false, light));
        assert!(!config_already_shows(true, true, dark));
        assert!(config_already_shows(true, false, dark));
    }

    fn system() -> System {
        System {
            material: preset::plain(),
            mask_threshold: 0.40,
            geometries: BTreeMap::from([
                (
                    "thin".into(),
                    Geometry {
                        bezel: 10.0,
                        thickness: 39.0,
                    },
                ),
                (
                    "panel".into(),
                    Geometry {
                        bezel: 18.0,
                        thickness: 70.0,
                    },
                ),
            ]),
            surfaces: BTreeMap::from([
                (Namespace::Bar.as_str().into(), "thin".into()),
                (Namespace::Panel.as_str().into(), "panel".into()),
            ]),
        }
    }

    /// The table as the code describes it: what `/etc/swaypplet/glass.json`
    /// holds on a host whose Nix side agrees with this repository.
    fn every_glass_surface() -> System {
        let mut sys = system();
        sys.geometries.insert(
            "lock".into(),
            Geometry {
                bezel: 18.0,
                thickness: 70.0,
            },
        );
        sys.surfaces = Namespace::ALL
            .into_iter()
            .filter_map(|ns| Some((ns.as_str().to_string(), ns.glass()?.to_string())))
            .collect();
        sys
    }

    #[test]
    fn the_surface_table_and_the_glass_namespaces_agree() {
        let sys = every_glass_surface();
        assert_eq!(sys.drift(), Vec::<String>::new());
        // Every glass class the code names is one the table defines, so no
        // surface is skipped as having no geometry.
        for class in sys.surfaces.values() {
            assert!(sys.geometries.contains_key(class), "no geometry `{class}`");
        }
        // The pane reaches every glass surface, and only those.
        let cmd = sys.command(&Tuning::system(&sys));
        for ns in Namespace::ALL {
            assert_eq!(
                cmd.contains(&format!("layer_effects \"{ns}\" ")),
                ns.glass().is_some(),
                "{ns}"
            );
        }
    }

    #[test]
    fn drift_names_a_stale_row_a_missing_one_and_a_wrong_class() {
        let mut sys = every_glass_surface();
        sys.surfaces
            .insert("swaypplet-switcher".into(), "panel".into());
        sys.surfaces.remove(Namespace::Keybinds.as_str());
        sys.surfaces
            .insert(Namespace::Osd.as_str().into(), "panel".into());
        sys.surfaces
            .insert(Namespace::Screenshot.as_str().into(), "panel".into());
        let drift = sys.drift();
        assert_eq!(drift.len(), 4, "{drift:#?}");
        assert!(drift.iter().any(|l| l.contains("swaypplet-switcher")));
        assert!(drift.iter().any(|l| l.contains("swaypplet-keybinds")));
        assert!(drift.iter().any(|l| l.contains("swaypplet-osd")));
        assert!(drift.iter().any(|l| l.contains("swaypplet-screenshot")));
    }

    #[test]
    fn every_effect_list_is_quoted_as_one_argument() {
        let sys = system();
        let cmd = sys.command(&Tuning::system(&sys));
        // Two commands, joined outside the quotes.
        assert_eq!(cmd.matches("layer_effects").count(), 2);
        // Every `;` that separates effects has to sit inside a quoted run,
        // which is what stops sway's splitter from ending the command at it.
        assert_eq!(cmd.matches('"').count(), 8);
    }

    #[test]
    fn geometry_reaches_the_namespace_that_asked_for_it() {
        let sys = system();
        let bar = sys.effects("swaypplet-bar", &Tuning::system(&sys)).unwrap();
        let panel = sys.effects("swaypplet", &Tuning::system(&sys)).unwrap();
        assert!(bar.contains("liquid_glass_bezel 10.000000;"));
        assert!(bar.contains("liquid_glass_thickness 39.000000;"));
        assert!(panel.contains("liquid_glass_bezel 18.000000;"));
        assert!(panel.contains("liquid_glass_thickness 70.000000;"));
    }

    #[test]
    fn the_pane_never_writes_what_anim_and_the_config_own() {
        let sys = system();
        let cmd = sys.command(&Tuning::system(&sys));
        for owned in [
            "liquid_glass enable",
            "liquid_glass disable",
            "blur_ignore_transparent",
            "corner_radius",
            "reset",
        ] {
            assert!(!cmd.contains(owned), "pane must not send `{owned}`");
        }
    }

    #[test]
    fn a_namespace_with_no_geometry_is_skipped_not_sent_flat() {
        let mut sys = system();
        sys.surfaces
            .insert("swaypplet-osd".into(), "nonexistent".into());
        let cmd = sys.command(&Tuning::system(&sys));
        assert!(!cmd.contains("swaypplet-osd"));
        assert!(cmd.contains("swaypplet-bar"));
    }

    #[test]
    fn scaling_the_bevel_moves_both_numbers_together() {
        // The whole reason this is one knob and not two: it is the slope the
        // light bends on, so a scale that changed only one of them would be
        // the "same material, two thicknesses" glass.nix ties the ratio to
        // prevent.
        let shipped = Geometry {
            bezel: 10.0,
            thickness: 39.0,
        };
        let t = Tuning {
            bezel_scale: 2.0,
            ..Tuning {
                material: preset::plain(),
                bezel_scale: 1.0,
                thickness_ratio: 0.0,
                clarity: 0.0,
                frost_scale: 1.0,
            }
        };
        let g = t.geometry(shipped);
        assert_eq!(g.bezel, 20.0);
        assert_eq!(g.thickness, 78.0);
        assert_eq!(g.thickness / g.bezel, shipped.thickness / shipped.bezel);
    }

    #[test]
    fn a_thickness_ratio_overrides_the_class_ratio_but_not_the_scale() {
        let shipped = Geometry {
            bezel: 10.0,
            thickness: 39.0,
        };
        let t = Tuning {
            material: preset::plain(),
            bezel_scale: 1.5,
            thickness_ratio: 2.0,
            clarity: 0.0,
            frost_scale: 1.0,
        };
        let g = t.geometry(shipped);
        assert_eq!(g.bezel, 15.0);
        assert_eq!(g.thickness, 30.0, "thickness follows the scaled bezel");
    }

    #[test]
    fn the_export_only_prints_geometry_that_moved() {
        let sys = system();
        let untouched = Tuning::system(&sys);
        assert!(!untouched.as_nix(&sys).contains("Geometry"));

        let scaled = Tuning {
            bezel_scale: 1.5,
            ..Tuning::system(&sys)
        };
        let nix = scaled.as_nix(&sys);
        assert!(nix.contains("Geometry, at bezel scale 1.5"), "{nix}");
        // thin ships 10/39, so 1.5x is 15/58.5 and the ratio is untouched.
        assert!(
            nix.contains("thin = { bezel = 15; thickness = 58.5; };"),
            "{nix}"
        );
    }

    #[test]
    fn the_override_file_round_trips() {
        // What `save_override` writes is what `load_override` reads back, and
        // `apply_saved` replays at startup. A field that serialises but does
        // not deserialise would be a material that silently reverts one knob
        // per session restart.
        let before = Tuning {
            material: preset::ALL[preset::ALL.len() - 1].material(),
            bezel_scale: 1.35,
            thickness_ratio: 4.2,
            clarity: 0.0,
            frost_scale: 1.0,
        };
        let json = serde_json::to_vec_pretty(&before).unwrap();
        let after: Tuning = serde_json::from_slice(&json).unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn the_nix_export_does_not_print_integers_as_floats() {
        let nix = preset::plain().as_nix();
        assert!(nix.contains("samples = 4;"), "{nix}");
        assert!(nix.contains("surface = \"droplet\";"));
    }

    /// The lock card's glass follows the mode like every other card.
    #[test]
    fn the_lock_card_follows_the_mode() {
        let system = every_glass_surface();
        let light = crate::tokens::Inputs {
            mode: crate::tokens::Mode::Light,
            ..crate::tokens::Inputs::default()
        };
        let dark = crate::tokens::Inputs {
            mode: crate::tokens::Mode::Dark,
            ..light
        };
        let light_cmd = system.command(&for_mode(Tuning::system(&system), light));
        let dark_cmd = system.command(&for_mode(Tuning::system(&system), dark));
        // One namespace's `layer_effects "ns" "…"`, up to the next one.
        let block = |cmd: &str, ns: &str| {
            let start = cmd.find(&format!("layer_effects \"{ns}\" ")).unwrap();
            let rest = &cmd[start + 1..];
            let end = rest
                .find("layer_effects")
                .map_or(cmd.len(), |i| start + 1 + i);
            cmd[start..end].to_string()
        };
        for ns in ["session-lock", "swaypplet-bar"] {
            assert_ne!(block(&light_cmd, ns), block(&dark_cmd, ns), "{ns}");
        }
    }

    #[test]
    fn an_int_valued_field_survives_json_written_by_nix() {
        // `builtins.toJSON` emits `4`, not `4.0`, for an integer.
        let raw = r#"{"surface":"convex_squircle","refraction":1.5,
            "dispersion":0.004,"samples":4,"reflection":1.0,"lensing":0.22,
            "frost_radius":22,"absorb":2.0,"absorb_floor":0.14,"photochromic":0.14,
            "specular":0.1,"frost":0,"shine":0,"reflect_blur":0}"#;
        let m: Material = serde_json::from_str(raw).unwrap();
        assert_eq!(m.samples, 4.0);
        assert_eq!(m.frost_radius, 22.0);
    }

    /// An override saved before the knobs went still loads, and what it is
    /// sent as never names one: sway rejects a whole `layer_effects` list
    /// for one key it does not know.
    #[test]
    fn an_override_from_before_the_removed_knobs_loads_and_sends_none_of_them() {
        let raw = r#"{"material":{"roughness":0.55,"surface":"convex_squircle",
            "refraction":1.5,"dispersion":0.004,"samples":4,"reflection":1.0,
            "lensing":0.22,"frost_radius":22,"absorb":2.0,"absorb_floor":0.14,
            "photochromic":0.14,"haze":0.05,"specular":0.1,"edge_light":0.08,
            "noise":0.012,"frost":0,"shine":0,"reflect_blur":0,"grain":"rippled",
            "grain_scale":18,"grain_strength":1,"grain_angle":0,"grain_aspect":1,
            "energy_comp":1.0,"iridescence":0,"edge_glow":0,"edge_glow_color":"none",
            "wave_amplitude":0},"bezel_scale":1.0,"crest_scale":1.2}"#;
        let t: Tuning = serde_json::from_str(raw).unwrap();
        let mut sys = system();
        // And a system file that still carries a crest radius.
        let geometry: Geometry =
            serde_json::from_str(r#"{"bezel":10,"thickness":39,"crest_radius":14}"#).unwrap();
        sys.geometries.insert("thin".into(), geometry);
        let cmd = sys.command(&t);
        for gone in [
            "roughness",
            "haze",
            "noise",
            "energy_comp",
            "grain",
            "iridescence",
            "edge_glow",
            "wave_amplitude",
            "edge_light",
            "absorb_floor",
            "crest_radius",
        ] {
            assert!(
                !cmd.contains(&format!("liquid_glass_{gone}")),
                "sends {gone}: {cmd}"
            );
        }
    }
}
