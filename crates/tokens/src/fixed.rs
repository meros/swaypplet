//! The tokens that do not move with the inputs: space, type, radius, the
//! base durations, the fill key and the component sizes (§3.4–§3.9).

use super::Rgb;

/// `--space-1` … `--space-7`. Box spacing and margins in Rust use these.
pub const SPACE: [i32; 7] = [2, 4, 8, 12, 16, 24, 32];

/// `--space-n`, 1-based as in the stylesheet; 0 is no space at all.
pub const fn space(n: usize) -> i32 {
    if n == 0 { 0 } else { SPACE[n - 1] }
}

pub const TYPE: [(&str, u32); 8] = [
    ("caption", 11),
    ("label", 12),
    ("body", 13),
    ("title-sm", 15),
    ("title", 18),
    ("display-sm", 28),
    ("display", 36),
    ("hero", 136),
];

pub const RADIUS: [(&str, u32); 5] = [
    ("control", 6),
    ("tile", 10),
    ("thin", 14),
    ("card", 18),
    ("pill", 999),
];

pub const DURATION: [(&str, u32); 5] = [
    ("fast", 150),
    ("standard", 200),
    ("emphasis", 300),
    ("spatial", 400),
    ("long", 500),
];

/// The fill key: what a glass card paints so the compositor finds its
/// shape, and drops. `glass.nix` `fillKey`; the cross-repo guard checks it.
pub const SURFACE_KEY: (Rgb, f64) = (Rgb::hex(0x32302f), 0.5);

/// Component tokens (§3.9): `--<name>` in px.
pub const COMPONENT: [(&str, u32); 9] = [
    ("control-height", 30),
    ("control-height-small", 24),
    ("field-height", 34),
    ("chip-height", 26),
    ("menu-item-height", 32),
    ("row-height", 40),
    ("row-height-dense", 32),
    ("tile-height", 52),
    ("track-height", 6),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn space_zero_is_nothing() {
        assert_eq!(space(0), 0);
        assert_eq!(space(1), 2);
        assert_eq!(space(7), 32);
    }
}
