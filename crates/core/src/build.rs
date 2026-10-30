//! What this binary was built from.
//!
//! Only the binary crate (`crates/swaypplet`) reads the build environment
//! and hands it over here at startup. A new commit then recompiles that one
//! small crate, and every library crate stays in the Nix store.

use std::sync::OnceLock;

static REV: OnceLock<&'static str> = OnceLock::new();

/// The swaypplet commit, from `SWAYPPLET_REV` (the flake sets it; a
/// `cargo build` has none). Called once, first thing in `main`.
pub fn set_rev(rev: Option<&'static str>) {
    let _ = REV.set(rev.unwrap_or_default());
}

/// The commit set by [`set_rev`], or `None` for a `cargo build`.
pub fn rev() -> Option<&'static str> {
    REV.get().copied().filter(|r| !r.is_empty())
}
