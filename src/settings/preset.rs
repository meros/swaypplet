//! Materials to start from.
//!
//! [`ALL`] is the row of buttons on the Glass tab, after the "System" one the
//! pane draws first. System is the shipped material read back from
//! `/etc/swaypplet/glass.json`, which is what Reset returns to; these are
//! points you can go to and come back from.
//!
//! Each is moved as a whole rather than one knob at a time: `glass.nix`'s
//! argument is that a mirror-sharp reflection on a milky surface is a
//! combination nothing physical produces. So a preset names the surface's
//! roughness once ([`rough`]), and the highlight and the reflection blur both
//! come from it; the material carries the two results, which is all the
//! compositor reads.
//!
//! [`base`] is the shipped material, so every preset here is a departure from
//! what actually ships. It was not, until 2026-09-09: the base sat at
//! `roughness 0.30`, `refraction 1.50`, `frost_radius 20` while the shipped
//! material had been retuned to a wet lens at `roughness 0.01`,
//! `refraction 1.07`, `frost_radius 3`. Sixteen presets were departures from
//! a middle that no longer existed, so the row read as sixteen ways to not
//! look like the desktop you have.
//!
//! Five, not sixteen, and that is also a reversal. The old row was written as
//! a tour: one preset per profile and per grain, on the argument that a
//! profile no preset visits is one nobody finds without reading the dropdown.
//! What a tour actually produced was four presets in the reeded family whose
//! difference is the pitch of a pattern, three ways to be dark, and a row
//! three deep that nobody reads to the end of. The grain has since gone from
//! the material altogether, and with it the last patterned preset. The row is
//! the places worth stopping, and `every_preset_is_visibly_a_different_material`
//! is what keeps them apart.

use super::glass::{Material, SurfaceKind, Tuning};

/// A named material, and one line on what it is for.
pub struct Preset {
    pub name: &'static str,
    pub hint: &'static str,
    build: fn() -> Material,
    /// The body, as the moves a tuning may make on the mode's own values
    /// (`Tuning::clarity`, `Tuning::frost_scale`). A preset cannot set the
    /// fill, absorb, photochromic or frost itself: the mode owns
    /// those, so a smoked preset is a dense clarity rather than an absorb.
    pub clarity: f64,
    pub frost_scale: f64,
}

impl Preset {
    pub fn material(&self) -> Material {
        (self.build)()
    }

    /// The preset as a whole tuning, on the shipped geometry.
    pub fn tuning(&self) -> Tuning {
        Tuning {
            material: self.material(),
            bezel_scale: 1.0,
            thickness_ratio: 0.0,
            clarity: self.clarity,
            frost_scale: self.frost_scale,
        }
    }
}

/// The shipped material, field for field: `theme/glass.nix`'s `material` in
/// the nixos repo, which the compositor is running right now.
///
/// Kept in the binary as well as in Nix on purpose. `System` reads the file
/// and is therefore whatever the last rebuild put there and nothing at all on
/// a host with no file; this is a fixed point that a session can always get
/// back to, and the middle the presets below are written as departures from.
///
/// Its `shine` and `reflect_blur` are `glass.nix`'s own numbers, not what any
/// one roughness gives: the shipped surface is mirror-smooth and its
/// reflection a little soft. That is the one deliberate exception to the rule
/// the rest of this file keeps, and `only_the_shipped_material_splits_the_distribution`
/// is where it is written down.
fn base() -> Material {
    Material {
        surface: SurfaceKind::Droplet,
        refraction: 1.35,
        dispersion: 0.003,
        samples: 4.0,
        reflection: 0.4,
        lensing: 0.15,
        frost_radius: 12.0,
        absorb: 0.0,
        photochromic: 0.89,
        specular: 0.0,
        frost: 0.33,
        shine: 4096.0,
        reflect_blur: 0.06,
        contact_angle: 89.5,
        tail: 0.5,
        tail_length: 0.1,
        // The same in every preset: a preset that set its own fill would be
        // changing swaypplet's colours under the guise of picking a
        // material. Clear (`fill_alpha` 0), as glass.nix ships it since
        // 2026-10-05; the mode replaces it in `glass::for_mode` either way.
        fill_color: "none".to_string(),
        fill_alpha: 0.0,
    }
}

/// [`base`] on a surface of roughness `a`: the highlight's exponent and the
/// reflection's blur as one microfacet distribution gives them, so the two
/// agree about how rough the surface is.
///
/// This is the projection the compositor used to make itself from a
/// `roughness` knob (`liquid_glass_data_derive`): a Blinn-Phong exponent
/// fitted to a Beckmann lobe, and the blur as a^0.9. The knob is gone and the
/// compositor takes the two numbers as they are, so the projection lives with
/// the only thing that still wants it. Frost is not here: the mode owns it.
fn rough(a: f64) -> Material {
    let a = a.clamp(0.0, 1.0);
    Material {
        shine: (2.0 / (a * a).max(1.0e-4) - 2.0).clamp(2.0, 4096.0),
        reflect_blur: a.powf(0.9),
        ..base()
    }
}

pub static ALL: [Preset; 5] = [
    // ── The shipped material, and the two nearest ways off it ────────
    Preset {
        name: "Bubble",
        clarity: 0.0,
        frost_scale: 1.0,
        hint: "The material this desktop ships: a wet lens, tight highlight, the wallpaper still readable through it.",
        build: base,
    },
    Preset {
        name: "Liquid",
        // The clearest the text allows. Every other preset here smokes or
        // frosts the backdrop; this one transmits it, which is the whole
        // material: the wallpaper through the card, the event at the rim.
        clarity: 0.8,
        frost_scale: 0.3,
        hint: "Optical glass: near-zero absorption, airy transmission, and a bright specular rim.",
        build: || Material {
            surface: SurfaceKind::ConvexSquircle,
            refraction: 1.12,
            dispersion: 0.008,
            lensing: 0.24,
            frost_radius: 4.0,
            specular: 0.28,
            ..rough(0.01)
        },
    },
    // ── Scattered ────────────────────────────────────────────────────
    Preset {
        name: "Frosted",
        clarity: 0.0,
        frost_scale: 2.5,
        hint: "The wet lens taken all the way into the frost. Fine texture goes, the backdrop's colour stays.",
        build: || Material {
            // A real index, unlike the shipped 1.07. That number buys a card
            // you can read a wallpaper through, and at this roughness there
            // is no image left to protect.
            refraction: 1.50,
            dispersion: 0.004,
            lensing: 0.22,
            frost_radius: 30.0,
            // The lobe is broad at this roughness, so most of the shipped
            // highlight is still a lit top rather than a patch.
            specular: 0.08,
            ..rough(0.80)
        },
    },
    Preset {
        name: "Smoked",
        // Dense, where it used to be an absorb of 2.9: the mode owns the
        // absorb now, and a dense body is the same look in both modes.
        clarity: -0.8,
        frost_scale: 2.0,
        hint: "Deep and dense: the backdrop as light rather than as an image. The dark end of the frost.",
        build: || Material {
            refraction: 1.50,
            dispersion: 0.004,
            lensing: 0.20,
            frost_radius: 26.0,
            specular: 0.07,
            ..rough(0.62)
        },
    },
    // ── Lit ──────────────────────────────────────────────────────────
    Preset {
        name: "Crystal",
        clarity: 0.2,
        frost_scale: 1.0,
        hint: "Sharp, dispersive, lit. A show piece — text sits on it less comfortably.",
        build: || Material {
            refraction: 1.62,
            dispersion: 0.012,
            lensing: 0.36,
            frost_radius: 16.0,
            specular: 0.26,
            ..rough(0.16)
        },
    },
];

/// The shipped material, for tests that want a plain one. Was `clear()`, back
/// when the row opened with a preset called Clear.
#[cfg(test)]
pub fn plain() -> Material {
    ALL[0].material()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_shipped_material_splits_the_distribution() {
        // Every preset but the shipped one has a highlight and a reflection
        // blur that agree about one roughness: recover it from the blur and
        // the highlight has to be what that roughness gives. One set by hand
        // would describe a specular lobe and a reflection from two different
        // surfaces.
        //
        // `Bubble` is the exception because the shipped material is
        // (theme/glass.nix in the nixos repo says why). `frost` is not in
        // this rule: the mode owns it, and a preset moves it with
        // `frost_scale`.
        for p in &ALL {
            let m = p.material();
            if p.name == "Bubble" {
                assert_eq!(
                    (m.shine, m.reflect_blur),
                    (4096.0, 0.06),
                    "Bubble no longer matches theme/glass.nix"
                );
                continue;
            }
            let a = m.reflect_blur.powf(1.0 / 0.9);
            let want = rough(a);
            assert!(
                (m.shine - want.shine).abs() < 1e-6 * want.shine.max(1.0),
                "{}: shine {} is not what roughness {a:.3} gives ({})",
                p.name,
                m.shine,
                want.shine
            );
        }
    }

    /// No preset carries a value of its own for the five the mode owns: they
    /// would be overwritten in `glass::for_mode` anyway, and a preset that
    /// looked like it set them would lie about what the button does.
    #[test]
    fn no_preset_sets_what_the_mode_owns() {
        let b = base();
        for p in &ALL {
            let m = p.material();
            assert_eq!(
                (m.absorb, m.photochromic, m.frost),
                (b.absorb, b.photochromic, b.frost),
                "{}",
                p.name
            );
            assert_eq!(
                (m.fill_color.as_str(), m.fill_alpha),
                (b.fill_color.as_str(), b.fill_alpha),
                "{}",
                p.name
            );
        }
    }

    #[test]
    fn the_row_opens_with_the_shipped_material() {
        // Not merely present: first. It is the one every other preset here is
        // written as a departure from, and the one a session comes back to
        // after trying the rest.
        assert_eq!(ALL[0].name, "Bubble");
        assert_eq!(ALL[0].material(), base());
        assert_eq!((ALL[0].clarity, ALL[0].frost_scale), (0.0, 1.0));
    }

    /// The look-defining fields (the mode-owned ones excepted: no preset
    /// sets them), in the sense that moving one changes what a card looks
    /// like rather than how it is arrived at. `samples` and the fill pair are
    /// absent: they are cost and colour, and two presets that differed only
    /// there would be the same material twice. The highlight and the
    /// reflection blur count as one: both are the surface's roughness.
    fn differences(a: &Preset, b: &Preset) -> Vec<&'static str> {
        let (pa, pb) = (a, b);
        let (a, b) = (&pa.material(), &pb.material());
        let mut moved = Vec::new();
        let mut f = |name, x: f64, y: f64| {
            if (x - y).abs() > 1e-9 {
                moved.push(name);
            }
        };
        f("roughness", a.reflect_blur, b.reflect_blur);
        f("refraction", a.refraction, b.refraction);
        f("dispersion", a.dispersion, b.dispersion);
        f("lensing", a.lensing, b.lensing);
        f("frost_radius", a.frost_radius, b.frost_radius);
        f("specular", a.specular, b.specular);
        f("clarity", pa.clarity, pb.clarity);
        f("frost_scale", pa.frost_scale, pb.frost_scale);
        if a.surface != b.surface {
            moved.push("surface");
        }
        moved
    }

    #[test]
    fn every_preset_is_visibly_a_different_material() {
        // What replaced the coverage test the old sixteen-preset row carried.
        // That one asked whether the row reached every profile and every
        // grain, which is a question about the shader; this asks whether two
        // buttons are worth pressing separately, which is the question a row
        // of five is for.
        //
        // Three fields is the bar because two is reachable by a pitch change:
        // the reeded family once filled four buttons on a grain's pitch and
        // strength alone.
        for (i, a) in ALL.iter().enumerate() {
            for b in &ALL[i + 1..] {
                let moved = differences(a, b);
                assert!(
                    moved.len() >= 3,
                    "{} and {} differ only in {moved:?} — same material, two buttons",
                    a.name,
                    b.name
                );
            }
        }
    }

    #[test]
    fn the_row_stays_a_row() {
        // The pane draws System and then these, in one wrapping flow. Past
        // about eight the row becomes a grid nobody reads to the end of,
        // which is the thing the sixteen-preset version got wrong.
        assert!(ALL.len() <= 7, "{} presets is a grid, not a row", ALL.len());
    }

    #[test]
    fn every_preset_is_named_once() {
        // The name is what the button says and what a tooltip is looked up
        // by, so two of them is an ambiguity the pane cannot resolve.
        for (i, a) in ALL.iter().enumerate() {
            for b in &ALL[i + 1..] {
                assert_ne!(a.name, b.name, "two presets named {}", a.name);
            }
        }
    }

    /// `Bubble` as JSON, which `cross-repo-guard.nix` in the nixos repo
    /// checks `theme/glass.nix`'s `material` against.
    fn shipped_json() -> String {
        let mut json = serde_json::to_string_pretty(&ALL[0].material()).unwrap();
        json.push('\n');
        json
    }

    #[test]
    fn the_shipped_material_file_matches_the_bubble_preset() {
        assert_eq!(
            include_str!("../../data/shipped-material.json"),
            shipped_json(),
            "data/shipped-material.json is stale: \
             cargo test -- --ignored write_shipped_material"
        );
    }

    #[test]
    #[ignore]
    fn write_shipped_material() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/data/shipped-material.json");
        std::fs::write(path, shipped_json()).unwrap();
    }

    /// One `Tuning` override file per preset, for `dev/preset-sheet.sh` to
    /// point a nested session at. Not a shipped artifact and not checked by
    /// anything: the row is judged by looking at it, and this is what makes
    /// looking at it one command.
    ///
    ///   SWPP_PRESET_OUT=/tmp/p cargo test -- --ignored write_preset_tunings
    #[test]
    #[ignore]
    fn write_preset_tunings() {
        let dir = std::env::var("SWPP_PRESET_OUT")
            .expect("set SWPP_PRESET_OUT to the directory to write into");
        for p in &ALL {
            let tuning = p.tuning();
            let path = std::path::Path::new(&dir).join(format!("{}.json", p.name.to_lowercase()));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(&path, serde_json::to_string_pretty(&tuning).unwrap()).unwrap();
            println!("wrote {}", path.display());
        }
    }
}
