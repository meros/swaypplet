//! The design tokens, generated from the theme inputs (docs/design-system.md).
//!
//! Every colour swaypplet shows comes from here: two 12-step scales per mode
//! (neutral and accent), a fixed status set, and the semantic tier the
//! stylesheet is allowed to use, emitted as CSS custom properties on
//! `:root`. The same numbers are available to Rust (Cairo drawing, the
//! glass material values), so nothing reads a colour back out of the CSS.
//!
//! Pure: values in, values out. No I/O and no GTK; resolving the inputs at
//! runtime and loading the stylesheet is `crate::theme`.
//!
//! | file | holds |
//! |---|---|
//! | `color.rs` | [`Rgb`], [`Oklch`], the OKLCH interpolation |
//! | `inputs.rs` | [`Mode`], [`Accent`], [`Neutral`], [`Contrast`], [`Inputs`] |
//! | `tint.rs` | [`Tint`], the wallpaper's hues as an input (§2.2) |
//! | `scales.rs` | [`scales`]: neutral and accent, 12 steps each (§3.1) |
//! | `semantic.rs` | [`status`], [`categorical`], [`levels`] (§3.2, §3.3) |
//! | `backdrop.rs` | [`Backdrop`], [`on_wallpaper`]: ink and halo for text on bare wallpaper |
//! | `material.rs` | [`material`] per mode and `glass_body` (§4) |
//! | `apca.rs` | [`apca`], and the §5 contrast tests |
//! | `fixed.rs` | space, type, radius, durations, the fill key, component sizes |
//! | `motion.rs` | the seven motions by meaning (§3.8) |
//! | `emit.rs` | [`css`]: `tokens.css` |

mod apca;
mod backdrop;
mod color;
mod emit;
mod fixed;
mod inputs;
pub(crate) mod material;
pub mod motion;
mod scales;
mod semantic;
pub mod tint;

pub use apca::apca;
pub use backdrop::{Backdrop, on_wallpaper};
pub use color::{Oklch, Rgb};
pub use emit::css;
pub use fixed::{COMPONENT, DURATION, RADIUS, SPACE, SURFACE_KEY, TYPE, space};
#[cfg(test)]
use inputs::every;
pub use inputs::{Accent, Contrast, Inputs, Mode, Neutral};
pub use material::{material, material_at};
pub use scales::scales;
pub use semantic::{ON_STATUS, Status, categorical, levels, status};
pub use tint::{Palette, Tint};
