//! Materials to start from.
//!
//! These are not copies of the shipped one — that lives in Nix and arrives as
//! `System::material`, which is what the pane calls "System" and what Reset
//! goes back to. These are other coherent points in the same model, each
//! moved as a whole rather than one knob at a time: `glass.nix`'s argument is
//! that a mirror-sharp reflection on a milky surface is a combination nothing
//! physical produces, and a preset that changed only `roughness` would keep
//! walking into exactly that.
//!
//! The list is ordered by family rather than by when each was written —
//! clear, then frosted, then smoked, then the ones whose whole point is a
//! grain you can resolve — because it is rendered as a row of buttons and a
//! row of buttons is read left to right. Between them the set reaches every
//! `SurfaceKind` and every `GrainKind` the shader has, which the tests at the
//! bottom hold to: the row is the tour of the model, and a profile no preset
//! visits is one nobody finds without reading the dropdown.

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

/// The point every preset below is written as a departure from. Never offered
/// on its own: it is a middle, not a look.
fn base() -> Material {
    Material {
        roughness: 0.30,
        surface: SurfaceKind::ConvexSquircle,
        refraction: 1.50,
        dispersion: 0.004,
        samples: 4.0,
        reflection: 1.0,
        lensing: 0.22,
        frost_radius: 20.0,
        absorb: 1.8,
        absorb_floor: 0.14,
        photochromic: 0.14,
        haze: 0.05,
        specular: 0.12,
        edge_light: 0.08,
        noise: 0.012,
        // Zero lets `roughness` decide all three. Every preset leaves them
        // there; overriding one is what splits the microfacet distribution
        // into three unrelated numbers.
        frost: 0.0,
        shine: 0.0,
        reflect_blur: 0.0,
        grain: GrainKind::None,
        grain_scale: 18.0,
        grain_strength: 0.0,
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
    }
}

pub static ALL: [Preset; 16] = [
    // ── Clear ────────────────────────────────────────────────────────
    Preset {
        name: "Clear",
        hint: "A wet lens. Tight highlight, and the desktop still readable through it.",
        build: || Material {
            roughness: 0.08,
            frost_radius: 12.0,
            absorb: 1.2,
            absorb_floor: 0.10,
            // Carried on purpose by a preset this transparent: with `absorb`
            // this low the ceiling is the only thing holding a white desktop
            // down, and without it the card disappears over one.
            photochromic: 0.18,
            haze: 0.02,
            specular: 0.20,
            edge_light: 0.12,
            lensing: 0.26,
            ..base()
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
            // rather than cut.
            surface: SurfaceKind::Lip,
            // Soda-lime, as shipped in a window. `Clear` reads as a lens
            // because it bends; this reads as glass because it does not.
            refraction: 1.52,
            dispersion: 0.003,
            lensing: 0.10,
            frost_radius: 6.0,
            absorb: 0.8,
            absorb_floor: 0.06,
            photochromic: 0.24,
            haze: 0.0,
            specular: 0.22,
            edge_light: 0.16,
            ..base()
        },
    },
    Preset {
        name: "Aqua",
        hint: "A bead of water: low index, deep bend, no colour of its own.",
        build: || Material {
            roughness: 0.06,
            // The circle leaves the contact line with a vertical tangent,
            // which the shader notes is a droplet only in the instant before
            // it beads up. That is exactly this material: water on something
            // it does not wet, standing on its own surface tension rather
            // than spreading — which is what `Molten`'s droplet profile is.
            surface: SurfaceKind::ConvexCircle,
            refraction: 1.33,
            // Water disperses about a third as much as crown glass, and the
            // bend here is large enough that any more would fringe the text.
            dispersion: 0.002,
            lensing: 0.44,
            frost_radius: 10.0,
            absorb: 1.2,
            absorb_floor: 0.09,
            photochromic: 0.20,
            haze: 0.02,
            specular: 0.20,
            edge_light: 0.14,
            ..base()
        },
    },
    Preset {
        name: "Minimal",
        hint: "Gentle daily driver. Soft edge definition, maximum contrast under text.",
        build: || Material {
            roughness: 0.04,
            refraction: 1.12,
            dispersion: 0.002,
            lensing: 0.12,
            frost_radius: 8.0,
            absorb: 0.9,
            absorb_floor: 0.05,
            photochromic: 0.26,
            haze: 0.0,
            specular: 0.12,
            edge_light: 0.07,
            grain: GrainKind::None,
            grain_scale: 18.0,
            grain_strength: 0.0,
            ..base()
        },
    },
    // ── Scattered ────────────────────────────────────────────────────
    Preset {
        name: "Frosted",
        hint: "Further into the frost. Fine texture goes, the backdrop's colour stays.",
        build: || Material {
            roughness: 0.80,
            frost_radius: 30.0,
            absorb: 2.0,
            haze: 0.06,
            // The lobe is broad at this roughness, so a third of the clear
            // preset's brightness is still a lit top rather than a patch.
            specular: 0.08,
            edge_light: 0.08,
            grain: GrainKind::Rippled,
            grain_scale: 18.0,
            grain_strength: 1.0,
            ..base()
        },
    },
    Preset {
        name: "Mist",
        hint: "Pale fog. Scattered forward rather than absorbed, so it lightens instead of tinting.",
        build: || Material {
            roughness: 0.50,
            // Dished, which is the one profile that turns the interior into
            // the wall and the rim into the flat. Fog in a cast blank rather
            // than fog in the same slab `Frosted` is, and the difference is
            // where the light piles up.
            surface: SurfaceKind::Concave,
            refraction: 1.46,
            dispersion: 0.003,
            lensing: 0.16,
            frost_radius: 30.0,
            // The pair that makes this the light end of the smoke family:
            // haze is light arriving from everywhere, absorb is light not
            // arriving at all, and only the second one darkens.
            absorb: 1.4,
            absorb_floor: 0.18,
            photochromic: 0.28,
            haze: 0.30,
            specular: 0.08,
            edge_light: 0.07,
            grain: GrainKind::Rippled,
            grain_scale: 26.0,
            grain_strength: 0.8,
            ..base()
        },
    },
    Preset {
        name: "Smoked",
        hint: "Deep and turbid: mostly scattered light rather than an image.",
        build: || Material {
            roughness: 0.62,
            frost_radius: 26.0,
            absorb: 2.9,
            absorb_floor: 0.09,
            photochromic: 0.10,
            haze: 0.16,
            specular: 0.07,
            edge_light: 0.06,
            grain: GrainKind::Rippled,
            grain_scale: 22.0,
            grain_strength: 1.0,
            ..base()
        },
    },
    Preset {
        name: "Ash",
        hint: "Smoke you can set text on: even, dark, and with the turbidity taken out.",
        build: || Material {
            roughness: 0.30,
            refraction: 1.50,
            dispersion: 0.003,
            lensing: 0.20,
            frost_radius: 20.0,
            // `Smoked` buys its depth with haze, and haze is light arriving
            // from every direction at once, which is exactly what eats the
            // contrast under a caption. This buys the same darkness with
            // `absorb_floor` instead: the optical path no longer depends on
            // where in the bevel you are, so the tint is flat across the card
            // and the glyph edges keep their contrast against it.
            absorb: 2.5,
            absorb_floor: 0.20,
            photochromic: 0.18,
            haze: 0.06,
            specular: 0.11,
            edge_light: 0.09,
            ..base()
        },
    },
    Preset {
        name: "Obsidian",
        hint: "A dark mirror. Little comes through; most of what you see is reflected.",
        build: || Material {
            roughness: 0.18,
            refraction: 1.58,
            dispersion: 0.004,
            // The one preset that takes `reflection` off physical, and it has
            // to: at this absorption the transmitted term is nearly gone, and
            // a Fresnel weight of 1.0 would leave a card that is merely dark.
            // Pushing it is what makes the light that remains read as a
            // surface rather than as a hole in the desktop.
            reflection: 2.6,
            lensing: 0.20,
            frost_radius: 14.0,
            absorb: 3.5,
            absorb_floor: 0.30,
            // Near zero, unlike every other preset here. The ceiling exists
            // to stop a white desktop coming through too bright, and at this
            // absorption there is no white desktop coming through.
            photochromic: 0.06,
            haze: 0.05,
            specular: 0.30,
            edge_light: 0.18,
            ..base()
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
            photochromic: 0.16,
            haze: 0.03,
            specular: 0.26,
            edge_light: 0.18,
            grain: GrainKind::Seeded,
            grain_scale: 26.0,
            grain_strength: 2.0,
            ..base()
        },
    },
    Preset {
        name: "Prism",
        hint: "Cut crystal. Square facets, each shifting its own rigid copy of the backdrop.",
        build: || Material {
            // Low, and it has to be: the facets are the texture here, and a
            // scattering lobe wide enough to see would blur the arris between
            // them, which is the feature.
            roughness: 0.12,
            refraction: 1.66,
            dispersion: 0.016,
            lensing: 0.32,
            frost_radius: 12.0,
            absorb: 1.4,
            absorb_floor: 0.10,
            photochromic: 0.16,
            haze: 0.02,
            specular: 0.30,
            edge_light: 0.20,
            grain: GrainKind::Prismatic,
            grain_scale: 18.0,
            grain_strength: 3.0,
            ..base()
        },
    },
    Preset {
        name: "Molten",
        hint: "Young-Laplace droplet curvature. Liquid meniscus edge with organic bubbles.",
        build: || Material {
            roughness: 0.05,
            surface: SurfaceKind::Droplet,
            refraction: 1.45,
            dispersion: 0.006,
            lensing: 0.30,
            frost_radius: 10.0,
            absorb: 1.1,
            photochromic: 0.22,
            haze: 0.01,
            specular: 0.24,
            edge_light: 0.15,
            grain: GrainKind::Seeded,
            grain_scale: 20.0,
            grain_strength: 1.5,
            ..base()
        },
    },
    Preset {
        name: "Fluted",
        hint: "Art-deco reeded flutes: parallel ribbed cylindrical lenses.",
        build: || Material {
            roughness: 0.22,
            refraction: 1.52,
            dispersion: 0.005,
            lensing: 0.28,
            frost_radius: 18.0,
            absorb: 1.7,
            photochromic: 0.14,
            haze: 0.04,
            specular: 0.18,
            edge_light: 0.11,
            grain: GrainKind::Reeded,
            grain_scale: 14.0,
            grain_strength: 3.0,
            ..base()
        },
    },
    Preset {
        name: "Cross-reed",
        hint: "Flutes running both ways: a grid of pillow lenses, each its own little window.",
        build: || Material {
            roughness: 0.20,
            refraction: 1.52,
            dispersion: 0.005,
            lensing: 0.26,
            frost_radius: 16.0,
            absorb: 1.7,
            absorb_floor: 0.13,
            photochromic: 0.14,
            haze: 0.04,
            specular: 0.17,
            edge_light: 0.11,
            grain: GrainKind::CrossReed,
            // Finer than `Fluted`, because the pattern's peak sits on the
            // diagonal where both flutes are at full slope: at one pitch the
            // pillows read as coarse squares rather than as reeding.
            grain_scale: 12.0,
            grain_strength: 2.4,
            ..base()
        },
    },
    Preset {
        name: "Hammered",
        hint: "Peened dents tiling the plane. Every dimple is a lens with a bottom.",
        build: || Material {
            roughness: 0.26,
            refraction: 1.52,
            dispersion: 0.005,
            lensing: 0.24,
            frost_radius: 16.0,
            absorb: 1.8,
            absorb_floor: 0.14,
            photochromic: 0.14,
            haze: 0.04,
            specular: 0.16,
            edge_light: 0.10,
            // The dents tile the plane with no flat glass between them, so
            // the strength is the highest here of anything that is not a
            // straight facet: there is no untextured ground for it to stand
            // out against, and an amount that reads on `Crystal`'s sparse
            // bubbles disappears into a surface that is all rim.
            grain: GrainKind::Hammered,
            grain_scale: 13.0,
            grain_strength: 3.4,
            ..base()
        },
    },
    Preset {
        name: "Cathedral",
        hint: "Hand-rolled sheet: coarse, poured, and never repeating.",
        build: || Material {
            roughness: 0.34,
            refraction: 1.50,
            dispersion: 0.004,
            lensing: 0.22,
            frost_radius: 22.0,
            absorb: 1.9,
            absorb_floor: 0.15,
            photochromic: 0.16,
            haze: 0.06,
            specular: 0.13,
            edge_light: 0.09,
            grain: GrainKind::Cathedral,
            // Coarse on purpose. The pattern is five waves at incommensurate
            // angles, so what makes it read as poured rather than as noise is
            // the beat between them, and a beat needs a card wide enough to
            // hold more than one of it.
            grain_scale: 34.0,
            grain_strength: 2.8,
            ..base()
        },
    },
];

/// The one the tests reach for; also the gentlest thing to land on.
#[cfg(test)]
pub fn clear() -> Material {
    ALL[0].material()
}

/// A preset that moves the fields `clear()` leaves at their defaults — a
/// grain, a strength, a scale. By predicate rather than by index: the list is
/// ordered for the button row and gets reordered when a preset is added, and
/// a test pinned to a position quietly stops testing what it was written for.
#[cfg(test)]
pub fn textured() -> Material {
    ALL.iter()
        .map(Preset::material)
        .find(|m| m.grain != GrainKind::None)
        .expect("no preset carries a grain")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_preset_splits_the_distribution_by_hand() {
        // `frost`, `shine` and `reflect_blur` at zero mean "derive from
        // roughness". A preset that set one of them would be describing a
        // material whose transmission, specular lobe and reflection blur
        // disagree about how rough the surface is.
        for p in &ALL {
            let m = p.material();
            assert_eq!(m.frost, 0.0, "{} sets frost", p.name);
            assert_eq!(m.shine, 0.0, "{} sets shine", p.name);
            assert_eq!(m.reflect_blur, 0.0, "{} sets reflect_blur", p.name);
        }
    }

    #[test]
    fn every_preset_is_a_distinct_material() {
        for (i, a) in ALL.iter().enumerate() {
            for b in &ALL[i + 1..] {
                assert_ne!(a.material(), b.material(), "{} == {}", a.name, b.name);
            }
        }
    }

    #[test]
    fn the_row_visits_every_profile_and_every_grain() {
        // Both sets are closed and both are offered as dropdowns, which is
        // the worst place to discover a look: a name in a list says nothing
        // about what it does to a card. A preset per entry means every one of
        // them is one click away from being seen, and it means a profile
        // added to the shader without a preset fails here rather than
        // shipping as a name nobody tries.
        for kind in SurfaceKind::ALL {
            assert!(
                ALL.iter().any(|p| p.material().surface == kind),
                "no preset uses {kind:?}"
            );
        }
        for grain in GrainKind::ALL {
            assert!(
                ALL.iter().any(|p| p.material().grain == grain),
                "no preset uses {grain:?}"
            );
        }
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
