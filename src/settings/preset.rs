//! A few materials to start from.
//!
//! These are not copies of the shipped one — that lives in Nix and arrives as
//! `System::material`, which is what the pane calls "System" and what Reset
//! goes back to. These are four other coherent points in the same model, each
//! moved as a whole rather than one knob at a time: `glass.nix`'s argument is
//! that a mirror-sharp reflection on a milky surface is a combination nothing
//! physical produces, and a preset that changed only `roughness` would keep
//! walking into exactly that.

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

pub static ALL: [Preset; 7] = [
    Preset {
        name: "Clear",
        hint: "A wet lens. Tight highlight, and the desktop still readable through it.",
        build: || Material {
            roughness: 0.08,
            frost_radius: 12.0,
            absorb: 1.2,
            absorb_floor: 0.10,
            // Higher than the others on purpose: with `absorb` this low the
            // ceiling is the only thing holding a white desktop down.
            photochromic: 0.18,
            haze: 0.02,
            specular: 0.20,
            edge_light: 0.12,
            lensing: 0.26,
            ..base()
        },
    },
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
];

/// The one the tests reach for; also the gentlest thing to land on.
#[cfg(test)]
pub fn clear() -> Material {
    ALL[0].material()
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
