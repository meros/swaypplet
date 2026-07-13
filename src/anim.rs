//! Shared motion scale for all animated surfaces.
//!
//! One easing (cubic ease-out) and three durations, so every surface moves
//! the same way:
//! - micro interactions (hover/press color changes): 150ms, CSS only
//! - structural reveal/collapse: 200ms (GtkRevealer `transition_duration`)
//! - enter/move: [`ENTER_MS`]/[`MOVE_MS`]; exit: [`EXIT_MS`]
//!
//! The CSS side of this scale is documented in `data/style.css`.
//!
//! # The glass rule: never fade a blurred surface
//!
//! swayfx composites layer_effects blur at full strength behind any surface
//! pixel with alpha > 0 — scenefx's `blur_ignore_transparent` is a stencil
//! that discards only pixels with alpha exactly 0 (tex.frag:
//! `if (discard_transparent && gl_FragColor.a == 0.0) discard;`), and blur
//! strength never scales with layer-surface alpha. An opacity ramp therefore
//! flashes a fully frosted halo before content is legible (enter) or after
//! it has faded (exit). All enter/exit motion on glass is geometry behind a
//! clip: GtkRevealer SlideUp for the OSD and start menu, full-width slides
//! past the clipped canvas edge for notification cards.

pub const ENTER_MS: f64 = 300.0;
pub const MOVE_MS: f64 = 300.0;
pub const EXIT_MS: f64 = 200.0;

pub fn ease_out_cubic(t: f64) -> f64 {
    1.0 - (1.0 - t).powi(3)
}
