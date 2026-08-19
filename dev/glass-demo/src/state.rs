//! Everything the shader is driven by, and the presets that put it somewhere
//! interesting.

use std::collections::VecDeque;

pub const MAX_SHAPES: usize = 8;
pub const MAX_RIPPLES: usize = 4;

#[derive(Clone, Copy, Debug)]
pub struct Shape {
    /// Centre in UI pixels: origin top-left, y down, matching GTK events.
    pub pos: [f32; 2],
    pub half: [f32; 2],
    pub radius: f32,
    pub rot: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Ripple {
    pub pos: [f32; 2],
    pub born: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct Params {
    pub merge: f32,
    pub bezel: f32,
    pub thickness: f32,
    pub profile: i32,
    pub ior: f32,
    pub dispersion: f32,
    pub samples: i32,
    pub frost: f32,
    pub specular: f32,
    pub shine: f32,
    pub fresnel: f32,
    pub lens_gain: f32,
    pub tint: [f32; 4],
    pub shadow: f32,
    pub refract: f32,
    pub edge_light: f32,
    pub noise: f32,
    pub ripple_amp: f32,
}

impl Default for Params {
    fn default() -> Self {
        PRESETS[0].1
    }
}

/// A preset is a look plus a layout: the parameters alone do not tell the
/// story if the shapes are in the wrong place for them (merge means nothing
/// with one shape, dispersion means little on a thin bezel).
#[derive(Clone, Copy)]
pub enum Layout {
    /// One large card, centred. The reference case.
    Card,
    /// Three pills close enough to fuse under a large merge radius.
    Blobs,
    /// A row of chips over the test card, the bar/notification case.
    Chips,
}

pub const PRESETS: [(&str, Params, Layout); 6] = [
    (
        // Apple's "Regular": adaptive, legible over anything, moderate lensing.
        "Regular",
        Params {
            merge: 0.0,
            bezel: 46.0,
            thickness: 95.0,
            profile: 1,
            ior: 1.5,
            dispersion: 0.004,
            samples: 4,
            frost: 3.0,
            specular: 0.30,
            shine: 44.0,
            fresnel: 0.45,
            lens_gain: 0.55,
            tint: [1.0, 1.0, 1.0, 0.07],
            shadow: 0.42,
            refract: 1.0,
            edge_light: 0.45,
            noise: 0.012,
            ripple_amp: 26.0,
        },
        Layout::Card,
    ),
    (
        // "Clear": permanently transparent, so the optics carry it alone.
        "Clear",
        Params {
            merge: 0.0,
            bezel: 54.0,
            thickness: 130.0,
            profile: 0,
            ior: 1.52,
            dispersion: 0.012,
            samples: 8,
            frost: 0.0,
            specular: 0.55,
            shine: 90.0,
            fresnel: 0.85,
            lens_gain: 0.9,
            tint: [1.0, 1.0, 1.0, 0.0],
            shadow: 0.30,
            refract: 1.0,
            edge_light: 0.75,
            noise: 0.008,
            ripple_amp: 34.0,
        },
        Layout::Card,
    ),
    (
        // The spectral pass at full strength: a real prism edge, 16 taps.
        "Prism",
        Params {
            merge: 0.0,
            bezel: 78.0,
            thickness: 165.0,
            profile: 0,
            ior: 1.62,
            dispersion: 0.055,
            samples: 16,
            frost: 0.0,
            specular: 0.45,
            shine: 70.0,
            fresnel: 0.55,
            lens_gain: 1.1,
            tint: [1.0, 1.0, 1.0, 0.0],
            shadow: 0.34,
            refract: 1.25,
            edge_light: 0.6,
            noise: 0.010,
            ripple_amp: 40.0,
        },
        Layout::Card,
    ),
    (
        // The liquid in liquid glass: a large smooth-union radius, so shapes
        // fuse through a neck and refract as one body.
        "Liquid",
        Params {
            merge: 95.0,
            bezel: 52.0,
            thickness: 120.0,
            profile: 3,
            ior: 1.48,
            dispersion: 0.008,
            samples: 6,
            frost: 1.6,
            specular: 0.42,
            shine: 55.0,
            fresnel: 0.55,
            lens_gain: 0.85,
            tint: [0.85, 0.93, 1.0, 0.05],
            shadow: 0.40,
            refract: 1.1,
            edge_light: 0.7,
            noise: 0.010,
            ripple_amp: 48.0,
        },
        Layout::Blobs,
    ),
    (
        // Concave: the surface dips, so the lens throws light outward and the
        // sign of every optical term flips. Useful as a control.
        "Concave",
        Params {
            merge: 0.0,
            bezel: 60.0,
            thickness: 110.0,
            profile: 2,
            ior: 1.5,
            dispersion: 0.010,
            samples: 6,
            frost: 0.6,
            specular: 0.35,
            shine: 50.0,
            fresnel: 0.5,
            lens_gain: 0.8,
            tint: [1.0, 1.0, 1.0, 0.03],
            shadow: 0.35,
            refract: 1.0,
            edge_light: 0.5,
            noise: 0.010,
            ripple_amp: 30.0,
        },
        Layout::Card,
    ),
    (
        // What swaypplet would actually ship: bar-sized chips, thin bezel,
        // heavy frost, cheap enough to run on every frame of a 40 px bar.
        "Bar chips",
        Params {
            merge: 22.0,
            bezel: 16.0,
            thickness: 30.0,
            profile: 1,
            ior: 1.46,
            dispersion: 0.003,
            samples: 3,
            frost: 4.2,
            specular: 0.28,
            shine: 60.0,
            fresnel: 0.35,
            lens_gain: 0.45,
            tint: [1.0, 1.0, 1.0, 0.10],
            shadow: 0.30,
            refract: 1.0,
            edge_light: 0.40,
            noise: 0.014,
            ripple_amp: 12.0,
        },
        Layout::Chips,
    ),
];

pub struct State {
    pub shapes: Vec<Shape>,
    pub ripples: VecDeque<Ripple>,
    pub params: Params,
    pub preset: usize,
    /// Seconds since start, driven by the frame clock.
    pub time: f64,
    pub light: [f32; 3],
    pub test_card: f32,
    pub debug: i32,
    /// Device pixels per logical pixel. Shapes and pointer coordinates are
    /// both kept in device pixels so the shader never has to know about it.
    pub scale: f32,
    pub drag: Option<Drag>,
    pub paused: bool,
    /// Last pointer position in device pixels. The scroll controller does not
    /// carry a position, so resize-under-cursor needs this remembered.
    pub pointer: [f32; 2],
}

pub struct Drag {
    pub shape: usize,
    pub grab: [f32; 2],
}

impl State {
    pub fn new() -> Self {
        State {
            shapes: Vec::new(),
            ripples: VecDeque::new(),
            params: PRESETS[0].1,
            preset: 0,
            time: 0.0,
            light: [-0.45, -0.65, 0.62],
            // Mixed by default: the wallpaper shows what the effect looks
            // like on real content, the grid over it is the only honest
            // witness to whether the lens actually bends anything.
            // GLASS_DEMO_TESTCARD overrides for capture runs.
            test_card: std::env::var("GLASS_DEMO_TESTCARD")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.45),
            // Env override so capture runs can produce the debug views too.
            debug: std::env::var("GLASS_DEMO_DEBUG")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
            scale: 1.0,
            drag: None,
            paused: false,
            pointer: [0.0, 0.0],
        }
    }

    /// Apply a preset and rebuild its layout for the current surface size.
    pub fn apply_preset(&mut self, index: usize, w: f32, h: f32) {
        let index = index.min(PRESETS.len() - 1);
        self.preset = index;
        self.params = PRESETS[index].1;
        self.relayout(w, h);
    }

    pub fn relayout(&mut self, w: f32, h: f32) {
        if w < 1.0 || h < 1.0 {
            return;
        }
        let (cx, cy) = (w * 0.5, h * 0.5);
        self.shapes = match PRESETS[self.preset].2 {
            Layout::Card => {
                let hw = (w * 0.24).clamp(140.0, 420.0);
                let hh = (h * 0.24).clamp(110.0, 300.0);
                vec![Shape {
                    pos: [cx, cy],
                    half: [hw, hh],
                    radius: hh.min(hw) * 0.55,
                    rot: 0.0,
                }]
            }
            Layout::Blobs => {
                let r = (h * 0.14).clamp(70.0, 170.0);
                let spread = r * 1.9;
                vec![
                    Shape {
                        pos: [cx - spread, cy + r * 0.35],
                        half: [r, r],
                        radius: r,
                        rot: 0.0,
                    },
                    Shape {
                        pos: [cx, cy - r * 0.55],
                        half: [r * 1.25, r * 0.8],
                        radius: r * 0.8,
                        rot: 0.0,
                    },
                    Shape {
                        pos: [cx + spread, cy + r * 0.35],
                        half: [r, r],
                        radius: r,
                        rot: 0.0,
                    },
                ]
            }
            Layout::Chips => {
                let hh = (h * 0.045).clamp(22.0, 46.0);
                let gap = hh * 3.4;
                (0..5)
                    .map(|i| {
                        let hw = hh * if i % 2 == 0 { 2.6 } else { 1.4 };
                        Shape {
                            pos: [cx + (i as f32 - 2.0) * gap * 1.5, cy],
                            half: [hw, hh],
                            radius: hh,
                            rot: 0.0,
                        }
                    })
                    .collect()
            }
        };
    }

    /// Topmost shape whose interior contains `p`, ignoring the merge radius:
    /// grabbing should follow the drawn shape, not the fused blob.
    pub fn pick(&self, p: [f32; 2]) -> Option<usize> {
        self.shapes
            .iter()
            .enumerate()
            .rev()
            .find(|(_, s)| sd_round_rect(p, s) <= 0.0)
            .map(|(i, _)| i)
    }

    pub fn push_ripple(&mut self, p: [f32; 2]) {
        while self.ripples.len() >= MAX_RIPPLES {
            self.ripples.pop_front();
        }
        self.ripples.push_back(Ripple {
            pos: p,
            born: self.time,
        });
    }

    pub fn add_shape(&mut self, p: [f32; 2]) {
        if self.shapes.len() >= MAX_SHAPES {
            self.shapes.remove(0);
        }
        let r = 90.0 * self.scale;
        self.shapes.push(Shape {
            pos: p,
            half: [r, r],
            radius: r,
            rot: 0.0,
        });
    }
}

/// Mirror of `sdRoundRect` in the shader, for hit-testing on the CPU.
fn sd_round_rect(p: [f32; 2], s: &Shape) -> f32 {
    let (c, sn) = (s.rot.cos(), s.rot.sin());
    let d = [p[0] - s.pos[0], p[1] - s.pos[1]];
    let q = [c * d[0] + sn * d[1], -sn * d[0] + c * d[1]];
    let r = s.radius.min(s.half[0]).min(s.half[1]);
    let a = [q[0].abs() - s.half[0] + r, q[1].abs() - s.half[1] + r];
    let outside = [a[0].max(0.0), a[1].max(0.0)];
    a[0].max(a[1]).min(0.0) + (outside[0] * outside[0] + outside[1] * outside[1]).sqrt() - r
}
