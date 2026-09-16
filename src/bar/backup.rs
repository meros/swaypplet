//! Backup segment — right instrument track, between presence and the clock.
//!
//! Deliberately not a hazard: the hazard lane is zero width when healthy
//! (docs/BAR_VISION.md, increment 7), and this instrument is wanted while
//! things are *fine*, as the one glance that says the nightly restic jobs
//! ran. So it lives with the other instruments, quiet and monochrome at
//! rest, amber when a run failed or the newest success is two nights old.
//!
//! One glyph, no text: the numbers belong in the tooltip, which is where
//! the question the glyph provokes gets answered, and in the Helm's power
//! sheet (`widgets/backup.rs`) for the full readout. State comes from
//! `crate::backup`, which watches the status directory, so this segment
//! owns no timer and reads no files.

use std::rc::Rc;

use gtk4::prelude::*;

use crate::backup::BackupStatusService;

/// Every class this segment can wear, cleared before the current one goes
/// on: GTK keeps whatever is not removed, and a segment that went amber
/// once would otherwise stay amber under the green.
const CLASSES: [&str; 4] = [
    "backup-ok",
    "backup-running",
    "backup-warn",
    "backup-unknown",
];

pub fn build(service: &Rc<BackupStatusService>) -> gtk4::Box {
    let segment = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .css_classes(["bar-backup"])
        .build();

    let glyph = gtk4::Label::new(None);
    glyph.add_css_class("bar-backup-glyph");
    segment.append(&glyph);

    let render = {
        let service = service.clone();
        let segment = segment.clone();
        let glyph = glyph.clone();
        move || {
            let snapshot = service.snapshot();
            let tier = snapshot.tier();
            glyph.set_label(tier.icon());
            for class in CLASSES {
                segment.remove_css_class(class);
            }
            segment.add_css_class(tier.css());
            segment.set_tooltip_text(Some(&snapshot.tooltip()));
        }
    };

    render();
    service.connect_change(render);
    segment
}
