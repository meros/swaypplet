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
//! `services::backup`, which watches the status directory, so this segment
//! owns no timer and reads no files.

use std::rc::Rc;

use gtk4::prelude::*;

use crate::services::backup::{BackupStatusService, Tier};
use crate::ui;

/// The glyph's tone per tier. This breaks the battery's rest-is-faint rule
/// on purpose: the glance this instrument exists for is "the nightly jobs
/// ran", so success is the resting state and the segment earns its width
/// by being readable when nothing is wrong. Running is plain ink, a
/// failed or two-night-old run the warning tone. Unknown is faint, because
/// a bar with no status files must claim nothing. The fill, the divider
/// and the ends come from the segment, as for every other instrument.
fn tone(tier: Tier) -> ui::Tone {
    match tier {
        Tier::Ok => ui::Tone::Success,
        Tier::Running => ui::Tone::Fg,
        Tier::Stale | Tier::Failed => ui::Tone::Warning,
        Tier::Unknown => ui::Tone::Faint,
    }
}

pub fn build(service: &Rc<BackupStatusService>) -> gtk4::Box {
    let segment = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .css_classes(["bar-seg"])
        .build();
    ui::segment(&segment, false);

    let glyph = gtk4::Label::new(None);
    segment.append(&glyph);

    let render = {
        let service = service.clone();
        let segment = segment.clone();
        let glyph = glyph.clone();
        move || {
            let snapshot = service.snapshot();
            let tier = snapshot.tier();
            glyph.set_label(tier.icon());
            ui::set_tone(&glyph, tone(tier));
            segment.set_tooltip_text(Some(&snapshot.tooltip()));
        }
    };

    render();
    service.connect_change(render);
    segment
}
