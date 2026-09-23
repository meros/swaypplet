//! Materials to start from.
//!
//! [`ALL`] is the row of buttons on the Glass tab, after the "System" one the
//! pane draws first. System is the shipped material read back from
//! `/etc/swaypplet/glass.json`, which is what Reset returns to; these are
//! points you can go to and come back from.
//!
//! Each is moved as a whole rather than one knob at a time: `glass.nix`'s
//! argument is that a mirror-sharp reflection on a milky surface is a
//! combination nothing physical produces, and a preset that changed only
//! `roughness` would keep walking into exactly that.
//!
//! [`base`] is the shipped material, so every preset here is a departure from
//! what actually ships. It was not, until 2026-09-09: the base sat at
//! `roughness 0.30`, `refraction 1.50`, `frost_radius 20` while the shipped
//! material had been retuned to a wet lens at `roughness 0.01`,
//! `refraction 1.07`, `frost_radius 3`. Sixteen presets were departures from
//! a middle that no longer existed, so the row read as sixteen ways to not
//! look like the desktop you have.
//!
//! Six, not sixteen, and that is also a reversal. The old row was written as
//! a tour: one preset per `SurfaceKind` and per `GrainKind`, on the argument
//! that a profile no preset visits is one nobody finds without reading the
//! dropdown. What a tour actually produced was four presets in the reeded
//! family whose difference is the pitch of a pattern, three ways to be dark,
//! and a row three deep that nobody reads to the end of. The dropdowns below
//! still reach every profile and every grain; the row is now the places worth
//! stopping, and `every_preset_is_visibly_a_different_material` is what keeps
//! them apart.

use super::glass::{GrainKind, Material, SurfaceKind};

/// A named material, and one line on what it is for.
pub struct Preset {
    pub name: &'static str,
    pub hint: &'static str,
    build: fn() -> Material,
}

impl Preset {
    pub fn material(&self) -> Material {
        (self.build)()
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
/// It carries the two overrides `glass.nix` documents at length: `frost` and
/// `reflect_blur` sit a few multiples above what `roughness 0.01` would
/// derive, so the surface is mirror-smooth and its transmission is not quite.
/// That is the one deliberate exception to the rule the rest of this file
/// keeps, and `only_the_shipped_material_splits_the_distribution` is where it
/// is written down.
fn base() -> Material {
    Material {
        roughness: 0.01,
        surface: SurfaceKind::ConvexSquircle,
        refraction: 1.07,
        dispersion: 0.003,
        samples: 4.0,
        reflection: 1.0,
        lensing: 0.15,
        frost_radius: 3.0,
        absorb: 1.0,
        absorb_floor: 0.07,
        photochromic: 0.28,
        haze: 0.0,
        specular: 0.10,
        edge_light: 0.09,
        noise: 0.007,
        frost: 0.05,
        shine: 0.0,
        reflect_blur: 0.10,
        grain: GrainKind::Seeded,
        grain_scale: 10.0,
        grain_strength: 1.0,
        // Unrotated and unstretched. A preset that turned the pattern would
        // be picking an orientation for a card whose long axis it does not
        // know: the bar runs one way and a notification the other.
        grain_angle: 0.0,
        grain_aspect: 1.0,
        energy_comp: 1.0,
        // Unset, in every preset: the fill is the card's, and a preset that
        // took it over would be changing swaypplet's own colours under the
        // guise of picking a material.
        fill_color: "none".to_string(),
        fill_alpha: -1.0,
        iridescence: 0.0,
        edge_glow: 0.0,
        edge_glow_color: "none".to_string(),
        wave_amplitude: 0.0,
    }
}

/// Everything a departure has to say to stop deriving the wet lens's
/// transmission. [`base`] carries `frost` and `reflect_blur` by hand because
/// the shipped material wants a smooth surface with a slightly soft image;
/// any preset that moves `roughness` wants all three to follow it, and zero
/// is what tells the shader to derive them.
fn derived() -> Material {
    Material {
        frost: 0.0,
        shine: 0.0,
        reflect_blur: 0.0,
        ..base()
    }
}

pub static ALL: [Preset; 7] = [
    // ── The shipped material, and the two nearest ways off it ────────
    Preset {
        name: "Bubble",
        hint: "The material this desktop ships: a wet lens, tight highlight, the wallpaper still readable through it.",
        build: base,
    },
    Preset {
        name: "Liquid",
        hint: "Optical glass: near-zero absorption, airy transmission, and a bright specular rim.",
        build: || Material {
            roughness: 0.01,
            surface: SurfaceKind::ConvexSquircle,
            refraction: 1.12,
            dispersion: 0.008,
            lensing: 0.24,
            frost_radius: 4.0,
            // Near nothing. Every other preset here smokes or frosts the
            // backdrop; this one transmits it, which is the whole material —
            // a card the wallpaper shows through at its own brightness, with
            // the event at the rim.
            absorb: 0.02,
            absorb_floor: 0.0,
            photochromic: 0.0,
            // A whisper of forward scatter, so the card has a body over a
            // busy wallpaper without going milky.
            haze: 0.015,
            specular: 0.28,
            edge_light: 0.28,
            grain: GrainKind::None,
            grain_strength: 0.0,
            ..derived()
        },
    },
    Preset {
        name: "Sheet",
        hint: "Plate glass. Flat through the middle, and all of the event at the rim.",
        build: || Material {
            roughness: 0.03,
            // The only profile that leaves the bevel flat-tangent at both
            // ends, so there is no crease anywhere and no dome in the middle.
            // That is the whole material: a pane, and an edge that was rolled
            // rather than cut — which is the one structural departure in this
            // row, everything else here being domed like the shipped card.
            surface: SurfaceKind::Lip,
            // Soda-lime, as shipped in a window. `Bubble` reads as a lens
            // because it bends; this reads as glass because it does not.
            refraction: 1.52,
            lensing: 0.10,
            frost_radius: 6.0,
            absorb: 0.8,
            absorb_floor: 0.06,
            photochromic: 0.24,
            specular: 0.22,
            edge_light: 0.16,
            grain: GrainKind::None,
            grain_strength: 0.0,
            ..derived()
        },
    },
    // ── Scattered ────────────────────────────────────────────────────
    Preset {
        name: "Frosted",
        hint: "The wet lens taken all the way into the frost. Fine texture goes, the backdrop's colour stays.",
        build: || Material {
            roughness: 0.80,
            // A real index, unlike the shipped 1.07. That number buys a card
            // you can read a wallpaper through, and at this roughness there
            // is no image left to protect.
            refraction: 1.50,
            dispersion: 0.004,
            lensing: 0.22,
            frost_radius: 30.0,
            absorb: 2.0,
            absorb_floor: 0.14,
            haze: 0.06,
            // The lobe is broad at this roughness, so most of the shipped
            // highlight is still a lit top rather than a patch.
            specular: 0.08,
            edge_light: 0.08,
            grain: GrainKind::Rippled,
            grain_scale: 18.0,
            grain_strength: 1.0,
            ..derived()
        },
    },
    Preset {
        name: "Smoked",
        hint: "Deep and turbid: mostly scattered light rather than an image. The dark end of the frost.",
        build: || Material {
            roughness: 0.62,
            refraction: 1.50,
            dispersion: 0.004,
            lensing: 0.20,
            frost_radius: 26.0,
            absorb: 2.9,
            absorb_floor: 0.09,
            // Near the floor, unlike everything else here. The ceiling exists
            // to stop a white desktop coming through too bright, and at this
            // absorption there is no white desktop coming through.
            photochromic: 0.10,
            haze: 0.16,
            specular: 0.07,
            edge_light: 0.06,
            grain: GrainKind::Rippled,
            // Coarser than `Frosted` at a lower roughness, which is what
            // separates them by more than darkness: this one you can resolve
            // the scattering of, and that one you cannot.
            grain_scale: 22.0,
            grain_strength: 1.6,
            ..derived()
        },
    },
    // ── Resolved grain ───────────────────────────────────────────────
    Preset {
        name: "Crystal",
        hint: "Sharp, dispersive, lit. A show piece — text sits on it less comfortably.",
        build: || Material {
            roughness: 0.16,
            refraction: 1.62,
            dispersion: 0.012,
            lensing: 0.36,
            frost_radius: 16.0,
            absorb: 1.5,
            absorb_floor: 0.14,
            photochromic: 0.16,
            haze: 0.03,
            specular: 0.26,
            edge_light: 0.18,
            // The same seeded pattern the shipped material carries, four
            // times the depth and two and a half times the pitch: this is
            // what `Bubble`'s grain looks like when you are meant to see it.
            grain: GrainKind::Seeded,
            grain_scale: 26.0,
            grain_strength: 2.0,
            ..derived()
        },
    },
    Preset {
        name: "Reeded",
        hint: "Art-deco flutes: parallel cylindrical lenses, each carrying its own strip of the backdrop.",
        build: || Material {
            roughness: 0.22,
            refraction: 1.52,
            dispersion: 0.005,
            lensing: 0.28,
            frost_radius: 18.0,
            absorb: 1.7,
            absorb_floor: 0.14,
            photochromic: 0.14,
            haze: 0.04,
            specular: 0.18,
            edge_light: 0.11,
            // One of the four patterned presets this row used to carry. The
            // other three — cross-reed, hammered, cathedral — differ from
            // this one in the pitch and the axis count of the same idea, and
            // all three are a `grain` dropdown away for anyone who wants
            // them. A button row is for choosing between materials, not
            // between pitches.
            grain: GrainKind::Reeded,
            grain_scale: 14.0,
            grain_strength: 3.0,
            ..derived()
        },
    },
];

/// The shipped material, for tests that want a plain one. Was `clear()`, back
/// when the row opened with a preset called Clear.
#[cfg(test)]
pub fn plain() -> Material {
    ALL[0].material()
}

/// A preset whose grain is not the shipped seeded one — the fields `plain()`
/// leaves where the shipped material has them. By predicate rather than by
/// index: the list is ordered for the button row and gets reordered when a
/// preset is added, and a test pinned to a position quietly stops testing
/// what it was written for.
/// A preset with no grain at all, for the export test that checks how the
/// grainless spelling comes out. Predicate rather than index, for the reason
/// `textured` gives.
#[cfg(test)]
pub fn grainless() -> Material {
    ALL.iter()
        .map(Preset::material)
        .find(|m| m.grain == GrainKind::None)
        .expect("no preset is grainless")
}

#[cfg(test)]
pub fn textured() -> Material {
    ALL.iter()
        .map(Preset::material)
        .find(|m| !matches!(m.grain, GrainKind::None | GrainKind::Seeded))
        .expect("no preset carries a grain of its own")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_shipped_material_splits_the_distribution() {
        // `frost`, `shine` and `reflect_blur` at zero mean "derive from
        // roughness". A preset that sets one is describing a material whose
        // transmission, specular lobe and reflection blur disagree about how
        // rough the surface is.
        //
        // `Bubble` is the exception because the shipped material is: it holds
        // `frost` and `reflect_blur` a few multiples above what `roughness
        // 0.01` derives, so the surface is mirror-smooth and its transmission
        // is not quite (theme/glass.nix in the nixos repo says why, at
        // length). This preset exists to be that material, so it has to carry
        // the exception with it — and nothing else here may.
        for p in &ALL {
            let m = p.material();
            assert_eq!(m.shine, 0.0, "{} sets shine", p.name);
            if p.name == "Bubble" {
                assert_eq!(m.frost, 0.05, "Bubble no longer matches theme/glass.nix");
                assert_eq!(
                    m.reflect_blur, 0.10,
                    "Bubble no longer matches theme/glass.nix"
                );
            } else {
                assert_eq!(m.frost, 0.0, "{} sets frost", p.name);
                assert_eq!(m.reflect_blur, 0.0, "{} sets reflect_blur", p.name);
            }
        }
    }

    #[test]
    fn the_row_opens_with_the_shipped_material() {
        // Not merely present: first. It is the one every other preset here is
        // written as a departure from, and the one a session comes back to
        // after trying the rest.
        assert_eq!(ALL[0].name, "Bubble");
        assert_eq!(ALL[0].material(), base());
    }

    /// The look-defining fields, in the sense that moving one changes what a
    /// card looks like rather than how it is arrived at. `samples`,
    /// `energy_comp`, the grain frame and the fill pair are all absent: they
    /// are cost, normalisation, orientation and colour, and two presets that
    /// differed only there would be the same material twice.
    fn differences(a: &Material, b: &Material) -> Vec<&'static str> {
        let mut moved = Vec::new();
        let mut f = |name, x: f64, y: f64| {
            if (x - y).abs() > 1e-9 {
                moved.push(name);
            }
        };
        f("roughness", a.roughness, b.roughness);
        f("refraction", a.refraction, b.refraction);
        f("dispersion", a.dispersion, b.dispersion);
        f("lensing", a.lensing, b.lensing);
        f("frost_radius", a.frost_radius, b.frost_radius);
        f("absorb", a.absorb, b.absorb);
        f("absorb_floor", a.absorb_floor, b.absorb_floor);
        f("photochromic", a.photochromic, b.photochromic);
        f("haze", a.haze, b.haze);
        f("specular", a.specular, b.specular);
        f("edge_light", a.edge_light, b.edge_light);
        f("grain_scale", a.grain_scale, b.grain_scale);
        f("grain_strength", a.grain_strength, b.grain_strength);
        if a.surface != b.surface {
            moved.push("surface");
        }
        if a.grain != b.grain {
            moved.push("grain");
        }
        moved
    }

    #[test]
    fn every_preset_is_visibly_a_different_material() {
        // What replaced the coverage test the old sixteen-preset row carried.
        // That one asked whether the row reached every profile and every
        // grain, which is a question about the shader; this asks whether two
        // buttons are worth pressing separately, which is the question a row
        // of six is for.
        //
        // Three fields is the bar because two is reachable by a pitch change:
        // the reeded family used to fill four buttons on `grain_scale` and
        // `grain_strength` alone.
        for (i, a) in ALL.iter().enumerate() {
            for b in &ALL[i + 1..] {
                let moved = differences(&a.material(), &b.material());
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
            let tuning = super::super::glass::Tuning {
                material: p.material(),
                bezel_scale: 1.0,
                thickness_ratio: 0.0,
                crest_scale: 1.0,
            };
            let path = std::path::Path::new(&dir).join(format!("{}.json", p.name.to_lowercase()));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(&path, serde_json::to_string_pretty(&tuning).unwrap()).unwrap();
            println!("wrote {}", path.display());
        }
    }

    #[test]
    fn grain_strength_and_type_agree() {
        // Strength is a peak lateral displacement in pixels and zero is off
        // whatever the type says, so a named pattern at zero strength is a
        // preset that claims a texture it does not draw.
        for p in &ALL {
            let m = p.material();
            assert_eq!(
                m.grain == GrainKind::None,
                m.grain_strength == 0.0,
                "{} names {:?} at strength {}",
                p.name,
                m.grain,
                m.grain_strength
            );
        }
    }
}
