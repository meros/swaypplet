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
//! a look, it is the contract between a surface's alpha and whether the
//! compositor treats it as a card at all, and it has a band on either side
//! that nothing may land in (see `glass.nix`, "The threshold has two lines").
//! The lock screen's scrim sits 0.08 under the discard line and its card 0.10
//! over the seeding line, so the usable range is about 0.32–0.375 and the
//! failure outside it is a lock screen with no card on it. A slider is the
//! wrong instrument for that.
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
/// it is one of four.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceKind {
    ConvexCircle,
    ConvexSquircle,
    Concave,
    Lip,
    Droplet,
}

impl SurfaceKind {
    pub const ALL: [SurfaceKind; 5] = [
        SurfaceKind::ConvexCircle,
        SurfaceKind::ConvexSquircle,
        SurfaceKind::Concave,
        SurfaceKind::Lip,
        SurfaceKind::Droplet,
    ];

    /// The spelling sway parses.
    pub fn as_str(self) -> &'static str {
        match self {
            SurfaceKind::ConvexCircle => "convex_circle",
            SurfaceKind::ConvexSquircle => "convex_squircle",
            SurfaceKind::Concave => "concave",
            SurfaceKind::Lip => "lip",
            SurfaceKind::Droplet => "droplet",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SurfaceKind::ConvexCircle => "Convex circle",
            SurfaceKind::ConvexSquircle => "Convex squircle",
            SurfaceKind::Concave => "Concave",
            SurfaceKind::Lip => "Lip",
            SurfaceKind::Droplet => "Droplet \u{2014} surface tension",
        }
    }
}

/// The surface at a scale you can resolve, rather than one that integrates
/// into a scattering lobe. See `glass.nix` for what each pattern reads as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrainKind {
    None,
    Seeded,
    Hammered,
    Rippled,
    Reeded,
    CrossReed,
    Prismatic,
    Cathedral,
}

impl GrainKind {
    pub const ALL: [GrainKind; 8] = [
        GrainKind::None,
        GrainKind::Seeded,
        GrainKind::Hammered,
        GrainKind::Rippled,
        GrainKind::Reeded,
        GrainKind::CrossReed,
        GrainKind::Prismatic,
        GrainKind::Cathedral,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            GrainKind::None => "none",
            GrainKind::Seeded => "seeded",
            GrainKind::Hammered => "hammered",
            GrainKind::Rippled => "rippled",
            GrainKind::Reeded => "reeded",
            GrainKind::CrossReed => "cross_reed",
            GrainKind::Prismatic => "prismatic",
            GrainKind::Cathedral => "cathedral",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            GrainKind::None => "None — flat",
            GrainKind::Seeded => "Seeded — cast, bubbles",
            GrainKind::Hammered => "Hammered — dimples",
            GrainKind::Rippled => "Rippled — rolled glass",
            GrainKind::Reeded => "Reeded — flutes",
            GrainKind::CrossReed => "Cross-reed — flutes both ways",
            GrainKind::Prismatic => "Prismatic — cut facets",
            GrainKind::Cathedral => "Cathedral — hand-rolled",
        }
    }

    /// Whether the pattern has an orientation worth turning.
    ///
    /// The three directional ones do; the rest are isotropic by construction,
    /// so a rotation control over them is a slider that changes nothing. The
    /// pane hides it rather than offering it and hoping nobody tries.
    pub fn is_directional(self) -> bool {
        matches!(
            self,
            GrainKind::Reeded | GrainKind::CrossReed | GrainKind::Prismatic
        )
    }
}

/// Every `liquid_glass_*` value that describes the material rather than the
/// surface it is drawn on. Field names are the sway spellings, so the writer
/// below is a formatting loop and not a translation table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Material {
    pub roughness: f64,
    pub surface: SurfaceKind,
    pub refraction: f64,
    pub dispersion: f64,
    pub samples: f64,
    pub reflection: f64,
    pub lensing: f64,
    pub frost_radius: f64,
    pub absorb: f64,
    pub absorb_floor: f64,
    pub photochromic: f64,
    pub haze: f64,
    pub specular: f64,
    pub edge_light: f64,
    pub noise: f64,
    pub frost: f64,
    pub shine: f64,
    pub reflect_blur: f64,
    pub grain: GrainKind,
    pub grain_scale: f64,
    pub grain_strength: f64,
    /// The pattern's own frame: degrees clockwise, and a stretch along the
    /// pattern's own x. Neither changes what `grain_strength` means — the
    /// shader divides the stretched slope by the larger scale — so they are
    /// shape and not amount.
    ///
    /// Defaulted, like the fill pair below, so an override written before the
    /// grain rebuild loads as the unrotated, unstretched pattern it described.
    #[serde(default)]
    pub grain_angle: f64,
    #[serde(default = "isotropic")]
    pub grain_aspect: f64,
    pub energy_comp: f64,
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
    /// Artsy reality-bending controls
    #[serde(default)]
    pub iridescence: f64,
    #[serde(default)]
    pub edge_glow: f64,
    #[serde(default = "unset_color")]
    pub edge_glow_color: String,
    #[serde(default)]
    pub wave_amplitude: f64,
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

fn isotropic() -> f64 {
    1.0
}

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
    /// Multiplies each class's shipped crest radius. Deliberately not folded
    /// into `bezel_scale`: the crest radius is pinned to the card's own corner
    /// radius, which does not change when the bevel gets wider. 1 is what the
    /// system config ships.
    #[serde(default = "unit")]
    pub crest_scale: f64,
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
            crest_scale: 1.0,
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
        // The sentinel survives scaling: negative means the shader derives it
        // from the bezel, and a scaled negative is still negative but no
        // longer says so at any particular strength.
        let crest_radius = if shipped.crest_radius >= 0.0 {
            shipped.crest_radius * self.crest_scale
        } else {
            shipped.crest_radius
        };
        Geometry {
            bezel,
            thickness,
            crest_radius,
        }
    }
}

// ── The system's copy ───────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Geometry {
    pub bezel: f64,
    pub thickness: f64,
    /// How wide the crest rounds where the card's edges compete. Negative
    /// derives it from the bezel, which is what the shader did before the
    /// field existed, so a system config without it still works.
    #[serde(default = "unset")]
    pub crest_radius: f64,
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
            Ok(system) => Some(system),
            Err(e) => {
                log::warn!("glass: bad system config at {path}: {e}");
                None
            }
        }
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
        // the two enums are spelled by hand anyway, and a field added to
        // `Material` should fail to compile here rather than silently stop
        // being sent.
        for (name, value) in [
            ("roughness", m.roughness),
            ("refraction", m.refraction),
            ("dispersion", m.dispersion),
            ("samples", m.samples),
            ("reflection", m.reflection),
            ("lensing", m.lensing),
            ("frost_radius", m.frost_radius),
            ("absorb", m.absorb),
            ("absorb_floor", m.absorb_floor),
            ("photochromic", m.photochromic),
            ("haze", m.haze),
            ("specular", m.specular),
            ("edge_light", m.edge_light),
            ("noise", m.noise),
            ("frost", m.frost),
            ("shine", m.shine),
            ("reflect_blur", m.reflect_blur),
            ("grain_scale", m.grain_scale),
            ("grain_strength", m.grain_strength),
            ("grain_angle", m.grain_angle),
            ("grain_aspect", m.grain_aspect),
            ("energy_comp", m.energy_comp),
            ("fill_alpha", m.fill_alpha),
            ("iridescence", m.iridescence),
            ("edge_glow", m.edge_glow),
            ("wave_amplitude", m.wave_amplitude),
            ("bezel", geometry.bezel),
            ("thickness", geometry.thickness),
            ("crest_radius", geometry.crest_radius),
            ("mask_threshold", self.mask_threshold),
        ] {
            let _ = write!(out, "liquid_glass_{name} {value:.6}; ");
        }
        // The ones that are words rather than numbers.
        let _ = write!(
            out,
            "liquid_glass_surface {}; liquid_glass_grain {}; liquid_glass_fill_color {}; liquid_glass_edge_glow_color {}",
            m.surface.as_str(),
            m.grain.as_str(),
            m.fill_color,
            m.edge_glow_color
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
                let effects = self.effects(ns, tuning)?;
                Some(format!("layer_effects \"{ns}\" \"{effects}\""))
            })
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// Push `material` at the running compositor. Fire and forget: sway
    /// answers `CMD_SUCCESS` even when a criteria failed to parse
    /// (`cmd_layer_effects` ignores a NULL result), so there is no reply worth
    /// waiting on.
    pub fn apply(&self, tuning: &Tuning) {
        let cmd = self.command(tuning);
        if !cmd.is_empty() {
            crate::sway_ipc::run_command(&cmd);
        }
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
        Ok(tuning) => Some(tuning),
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
/// something the panel has to remember to do.
pub fn apply_saved() {
    let (Some(system), Some(tuning)) = (System::load(), load_override()) else {
        return;
    };
    log::info!(
        "glass: replaying override from {}",
        override_path().display()
    );
    system.apply(&tuning);
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
        let _ = writeln!(out, "grain = \"{}\";", self.grain.as_str());
        let _ = writeln!(out, "fill_color = \"{}\";", self.fill_color);
        let _ = writeln!(out, "edge_glow_color = \"{}\";", self.edge_glow_color);
        out
    }

    /// Every numeric field, in the order the export prints them.
    pub(super) fn numbers(&self) -> [(&'static str, f64); 26] {
        [
            ("roughness", self.roughness),
            ("refraction", self.refraction),
            ("dispersion", self.dispersion),
            ("samples", self.samples),
            ("reflection", self.reflection),
            ("lensing", self.lensing),
            ("frost_radius", self.frost_radius),
            ("absorb", self.absorb),
            ("absorb_floor", self.absorb_floor),
            ("photochromic", self.photochromic),
            ("haze", self.haze),
            ("specular", self.specular),
            ("edge_light", self.edge_light),
            ("noise", self.noise),
            ("frost", self.frost),
            ("shine", self.shine),
            ("reflect_blur", self.reflect_blur),
            ("grain_scale", self.grain_scale),
            ("grain_strength", self.grain_strength),
            ("grain_angle", self.grain_angle),
            ("grain_aspect", self.grain_aspect),
            ("energy_comp", self.energy_comp),
            ("fill_alpha", self.fill_alpha),
            ("iridescence", self.iridescence),
            ("edge_glow", self.edge_glow),
            ("wave_amplitude", self.wave_amplitude),
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
        if self.bezel_scale == 1.0 && self.thickness_ratio == 0.0 && self.crest_scale == 1.0 {
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
                "{class} = {{ bezel = {}; thickness = {}; crest_radius = {}; }};",
                trim_float(g.bezel),
                trim_float(g.thickness),
                trim_float(g.crest_radius)
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

    fn system() -> System {
        System {
            material: preset::clear(),
            mask_threshold: 0.40,
            geometries: BTreeMap::from([
                (
                    "thin".into(),
                    Geometry {
                        bezel: 10.0,
                        thickness: 39.0,
                        crest_radius: 14.0,
                    },
                ),
                (
                    "panel".into(),
                    Geometry {
                        bezel: 18.0,
                        thickness: 70.0,
                        crest_radius: 14.0,
                    },
                ),
            ]),
            surfaces: BTreeMap::from([
                ("swaypplet-bar".into(), "thin".into()),
                ("swaypplet".into(), "panel".into()),
            ]),
        }
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
    fn enums_round_trip_through_their_sway_spelling() {
        let json = serde_json::to_string(&SurfaceKind::ConvexSquircle).unwrap();
        assert_eq!(json, "\"convex_squircle\"");
        assert_eq!(
            serde_json::from_str::<GrainKind>("\"rippled\"").unwrap(),
            GrainKind::Rippled
        );
        // The multi-word one is where serde's rename and `as_str` could
        // disagree without anything noticing: sway rejects a name it does not
        // know by discarding the whole layer_effects block, so a surface with
        // this grain selected would lose its material entirely.
        assert_eq!(
            serde_json::to_string(&GrainKind::CrossReed).unwrap(),
            "\"cross_reed\""
        );
    }

    #[test]
    fn every_grain_spells_itself_the_same_way_both_directions() {
        for kind in GrainKind::ALL {
            let json = serde_json::to_string(&kind).unwrap();
            assert_eq!(
                json,
                format!("\"{}\"", kind.as_str()),
                "serde and as_str disagree about {kind:?}"
            );
            assert_eq!(serde_json::from_str::<GrainKind>(&json).unwrap(), kind);
            assert!(!kind.label().is_empty());
        }
    }

    #[test]
    fn only_the_patterns_with_an_orientation_are_directional() {
        // The angle knob is greyed out for the rest. An isotropic pattern
        // gaining a rotation control is a slider that changes nothing, which
        // reads as a broken knob rather than as an inapplicable one.
        assert!(GrainKind::Reeded.is_directional());
        assert!(GrainKind::CrossReed.is_directional());
        assert!(GrainKind::Prismatic.is_directional());
        assert!(!GrainKind::None.is_directional());
        assert!(!GrainKind::Seeded.is_directional());
        assert!(!GrainKind::Hammered.is_directional());
        assert!(!GrainKind::Rippled.is_directional());
        assert!(!GrainKind::Cathedral.is_directional());
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
            crest_radius: 14.0,
        };
        let t = Tuning {
            bezel_scale: 2.0,
            ..Tuning {
                material: preset::clear(),
                bezel_scale: 1.0,
                thickness_ratio: 0.0,
                crest_scale: 1.0,
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
            crest_radius: 14.0,
        };
        let t = Tuning {
            material: preset::clear(),
            bezel_scale: 1.5,
            thickness_ratio: 2.0,
            crest_scale: 1.0,
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
            nix.contains("thin = { bezel = 15; thickness = 58.5; crest_radius = 14; };"),
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
            material: preset::textured(),
            bezel_scale: 1.35,
            thickness_ratio: 4.2,
            crest_scale: 1.0,
        };
        let json = serde_json::to_vec_pretty(&before).unwrap();
        let after: Tuning = serde_json::from_slice(&json).unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn the_nix_export_does_not_print_integers_as_floats() {
        let nix = preset::clear().as_nix();
        assert!(nix.contains("samples = 4;"), "{nix}");
        assert!(nix.contains("surface = \"convex_squircle\";"));
        assert!(nix.contains("grain = \"none\";"));
    }

    #[test]
    fn an_int_valued_field_survives_json_written_by_nix() {
        // `builtins.toJSON` emits `4`, not `4.0`, for an integer.
        let raw = r#"{"roughness":0.55,"surface":"convex_squircle","refraction":1.5,
            "dispersion":0.004,"samples":4,"reflection":1.0,"lensing":0.22,
            "frost_radius":22,"absorb":2.0,"absorb_floor":0.14,"photochromic":0.14,
            "haze":0.05,"specular":0.1,"edge_light":0.08,"noise":0.012,"frost":0,
            "shine":0,"reflect_blur":0,"grain":"rippled","grain_scale":18,
            "grain_strength":1,"energy_comp":1.0}"#;
        let m: Material = serde_json::from_str(raw).unwrap();
        assert_eq!(m.samples, 4.0);
        assert_eq!(m.frost_radius, 22.0);
        assert_eq!(m.grain, GrainKind::Rippled);
        // The same JSON is an override written before the grain frame
        // existed. It has to load as the pattern it described, which is the
        // unrotated, unstretched one — an aspect defaulting to serde's 0
        // would divide the pitch by zero.
        assert_eq!(m.grain_angle, 0.0);
        assert_eq!(m.grain_aspect, 1.0);
    }
}
