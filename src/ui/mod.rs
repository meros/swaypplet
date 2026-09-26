//! The components of the design system (docs/design-system.md §6).
//!
//! One module per component, each handing back plain GTK widgets with the
//! `ui-*` classes the component stylesheets (`data/css/components/`) style.
//! A surface is assembled from these; its own classes may place things but
//! not colour, size type or round corners, and no `"ui-…"` string appears
//! in Rust outside this directory (design lint `rust-ui-class`).
//!
//! # How the API reads
//!
//! Three shapes, one per job:
//!
//! 1. **A noun constructs.** `ui::button(label, kind)`, `ui::row(..)`,
//!    `ui::section(..)`, `ui::badge(text, tone)`: the component builds its
//!    widget and hands it back (or a struct of its parts).
//! 2. **`ui::<component>::adopt(&w, ..)` styles a widget the caller had to
//!    build**, because GTK builds it (a search entry, a dropdown over a
//!    model, a scale on a shared adjustment) or because the surface owns its
//!    shape (the box that becomes a card): `ui::card::adopt(&b, Card::Thin)`,
//!    `ui::entry::adopt(&e, FieldSize::Large)`, `ui::glyph::adopt(&l, ..)`.
//! 3. **`ui::set_<state>(&w, TypedEnum | bool)` changes state at runtime**:
//!    `ui::set_status(&l, Status::Warning)`, `ui::set_busy(&row, true)`,
//!    `ui::set_face_state(..)`. A state is a typed value, never a class name;
//!    the one class helper (`class::toggle`/`class::swap`) is private here.
//!
//! Layout helpers (`vbox`, `hbox`, `stack`, `pad`) take a step of the space
//! scale, never a literal.

mod avatar;
mod bar;
pub mod button;
pub mod card;
mod chip;
mod class;
mod expander;
mod face;
mod field;
pub mod icons;
mod layout;
mod media;
mod menu;
mod motion;
mod popover;
pub mod progress;
mod row;
pub mod slider;
pub mod surface;
mod text;
mod tile;

pub use avatar::avatar;
pub use bar::{
    BayState, CATEGORIES, Ribbon, Selection, bay, bay_chip, mark, meter, rail, segment, segmented,
    set_bay_state, set_category, set_danger, set_quiet, set_receded, set_ribbon, set_selection,
};
pub use button::{
    Face, Kind, Size, button, button_with, set_armed, set_button_kind, toggle_button,
};
pub use card::{Card, CardTint, group, set_card_tint, set_success, well};
pub use chip::{
    BadgeTone, Handoff, Status, badge, chip, key, set_handoff, set_status, status, toggle_chip,
};
pub use expander::{Disclosure, Section, disclosure, section};
pub use face::{FacePill, FaceState, face_pill, set_face_enter, set_face_state};
pub use field::{FieldSize, FieldState, dropdown, entry, field, set_field_state};
pub use layout::{hbox, pad, pill_group, separator, toolbar, vbox};
pub use media::{choice_grid, lifted, pick_thumb, placeholder, ring, set_pinned, swatch, thumb};
pub use menu::{menu, menu_item};
pub use motion::{page_stack, revealer, set_breathing};
pub use popover::popover;
pub use progress::{progress, set_progress_status};
pub use row::{Row, list, list_row, row, row_button, set_busy, set_instant, set_selected};
pub use slider::{Density, check, set_over_range, slider_row, switch, switch_row};
pub use surface::{canvas, scrim, window};
pub use text::{
    Text, Tone, Weight, glyph, heading, live_caption, on_wallpaper, overline, set_mono,
    set_numeric, set_text_style, set_tone, set_weight, text,
};
pub use tile::{set_loading, tile_toggle};

// ── Cairo colours ───────────────────────────────────────────────────────

/// The token colours for code that paints with Cairo, from the same
/// generator and the same inputs as the stylesheet on screen
/// (`theme::shown`), so a drawn mark and a styled widget cannot disagree and
/// both follow the mode. Nothing reads a colour back out of the CSS. Read it
/// when a surface opens rather than per frame: it is a settings read.
#[derive(Clone, Copy, Debug)]
pub struct Paint {
    /// `--fg`.
    pub fg: crate::tokens::Rgb,
    /// The darkest ground (`--neutral-1`), for a chip drawn over a picture.
    pub ground: crate::tokens::Rgb,
    /// `--accent-bg`: the accent as a line or a fill.
    pub accent: crate::tokens::Rgb,
    pub status: crate::tokens::Status,
    /// `--cat-n`: the readable tone per categorical slot.
    pub categorical: [crate::tokens::Rgb; 6],
}

pub fn paint() -> Paint {
    use crate::tokens;
    let inputs = crate::theme::shown();
    let s = tokens::scales(inputs);
    Paint {
        fg: s.neutral[11],
        ground: s.neutral[0],
        accent: s.accent_bg,
        status: tokens::status(inputs),
        categorical: tokens::categorical(inputs).map(|(text, _)| text),
    }
}

/// Set `c` as the Cairo source, at `alpha`.
pub fn set_source(cr: &gtk4::cairo::Context, c: crate::tokens::Rgb, alpha: f64) {
    cr.set_source_rgba(c.0, c.1, c.2, alpha);
}
