//! The progress bar, plain or saying a status.

use gtk4::prelude::*;

use super::class::swap;
use super::*;

pub fn progress(fraction: f64) -> gtk4::ProgressBar {
    let p = gtk4::ProgressBar::new();
    p.add_css_class("ui-progress");
    p.set_fraction(fraction);
    p
}

pub fn make_progress(p: &gtk4::ProgressBar) {
    p.add_css_class("ui-progress");
}

/// A progress bar whose fill says a status (charging, low) rather than a
/// plain level; `None` is the accent fill.
pub fn set_progress_status(p: &gtk4::ProgressBar, status: Option<Status>) {
    let one = status.filter(|s| *s != Status::Neutral).map(Status::class);
    swap(p, Status::ALL.map(Status::class), one);
}
