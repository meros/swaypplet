//! The progress bar, plain or saying a status.
//!
//! `ui::progress(fraction)`, `ui::progress::adopt(&bar)`;
//! `ui::set_progress_status` at runtime.

use gtk4::prelude::*;

use super::Status;
use super::class::swap;

pub fn progress(fraction: f64) -> gtk4::ProgressBar {
    let p = gtk4::ProgressBar::new();
    adopt(&p);
    p.set_fraction(fraction);
    p
}

/// Style a progress bar the caller built.
pub fn adopt(p: &gtk4::ProgressBar) {
    p.add_css_class("ui-progress");
}

/// A progress bar whose fill says a status (charging, low) rather than a
/// plain level; `None` (or `Neutral`) is the accent fill.
pub fn set_progress_status(p: &gtk4::ProgressBar, status: Option<Status>) {
    let one = status.filter(|s| *s != Status::Neutral).map(Status::class);
    swap(p, Status::ALL.map(Status::class), one);
}
