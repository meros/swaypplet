//! Update segment: a pre-built NixOS update, waiting, as one glyph on the
//! instrument track. A click on a ready update starts the switch.
//!
//! The state is root's: a service builds the next system ahead of time and
//! reports it in `/run/nx/prebuilt.json`, which
//! `swaypplet_system::nx_prebuilt` parses and watches. This segment owns no
//! timer and reads no file. It is hidden while nothing is waiting and
//! nothing is going on, which is the state of a machine that is up to date,
//! the way the hazard lane is empty when healthy.
//!
//! One glyph per kind of state, and a tone on top, so the bar survives a
//! grayscale filter (BAR_VISION P3): the download arrow for a waiting or a
//! building update (faint while it builds), an alert for a failed one, and a
//! progress arrow while the switch runs. Only a ready update takes a click.
//! The switch is `systemctl start --no-block nx-apply.service`, which the
//! nixos side lets this user start without a password; a refusal is logged
//! and shown as a notification card, the way the Displays profiles report
//! a failed apply.
//!
//! The tooltip is built when it opens, so "built 20 min ago" is current
//! without a timer.

use std::cell::Cell;
use std::rc::Rc;

use gtk4::prelude::*;

use swaypplet_base::spawn::spawn_work;
use swaypplet_model::store;
use swaypplet_notifyd::store::{StoreRef, store_add};
use swaypplet_notifyd::{Notification, Urgency};
use swaypplet_system::nx_prebuilt::{self, Prebuilt, PrebuiltService, Shown};
use swaypplet_ui::ui;

// Nerd Font: md-download / md-alert / md-progress-download.
const ICON_DOWNLOAD: &str = "\u{f01da}";
const ICON_ALERT: &str = "\u{f0026}";
const ICON_SWITCHING: &str = "\u{f0da0}";

fn icon(shown: Shown) -> &'static str {
    match shown {
        Shown::Ready | Shown::Building => ICON_DOWNLOAD,
        Shown::Failed => ICON_ALERT,
        Shown::Switching => ICON_SWITCHING,
    }
}

/// Ready is the one state that asks for something, so it takes the accent;
/// a build in progress is faint, the way a stopped unit is, and a failure
/// is the bar's one amber.
fn tone(shown: Shown) -> ui::Tone {
    match shown {
        Shown::Ready => ui::Tone::Accent,
        Shown::Building => ui::Tone::Faint,
        Shown::Switching => ui::Tone::Muted,
        Shown::Failed => ui::Tone::Warning,
    }
}

/// Whether a click does anything: only a waiting update, and never twice.
fn clickable(shown: Option<Shown>, in_flight: bool) -> bool {
    shown == Some(Shown::Ready) && !in_flight
}

/// The Bar tab's switch for this segment.
fn wanted() -> bool {
    store::with(|s| s.bar().nx_update)
}

/// What the segment draws for the current file: `None` hides it.
fn view(snapshot: &Option<Prebuilt>) -> Option<(&Prebuilt, Shown)> {
    let p = snapshot.as_ref()?;
    Some((p, p.shown()?))
}

fn report_failure(store: Option<&StoreRef>, error: &str) {
    log::warn!("nx update: {error}");
    let Some(store) = store else {
        return;
    };
    store_add(
        store,
        Notification {
            app_name: "Update".into(),
            summary: "Could not start the update".into(),
            body: error.into(),
            urgency: Urgency::Normal,
            expire_timeout: 8000,
            // `Default` leaves the epoch, which the card shows as its age.
            timestamp: std::time::SystemTime::now(),
            ..Default::default()
        },
    );
}

pub fn build(service: &Rc<PrebuiltService>, notifications: Option<StoreRef>) -> gtk4::Button {
    let glyph = gtk4::Label::new(Some(ICON_DOWNLOAD));
    let btn = gtk4::Button::builder()
        .child(&glyph)
        .css_classes(["bar-seg"])
        .has_tooltip(true)
        .visible(false)
        .build();
    ui::segment::adopt(&btn, false);

    // A switch is being started: a second click would queue behind it.
    let in_flight = Rc::new(Cell::new(false));

    let render = {
        let (service, btn, glyph) = (service.clone(), btn.downgrade(), glyph.downgrade());
        Rc::new(move || {
            let (Some(btn), Some(glyph)) = (btn.upgrade(), glyph.upgrade()) else {
                return;
            };
            let snapshot = service.snapshot();
            let shown = view(&snapshot).map(|(_, s)| s);
            btn.set_visible(wanted() && shown.is_some());
            if let Some(shown) = shown {
                glyph.set_label(icon(shown));
                ui::set_tone(&glyph, tone(shown));
            }
        })
    };

    {
        let service = service.clone();
        btn.connect_query_tooltip(move |_, _, _, _, tip| {
            let snapshot = service.snapshot();
            let Some((p, shown)) = view(&snapshot) else {
                return false;
            };
            tip.set_text(Some(&p.tooltip(shown, nx_prebuilt::now_s())));
            true
        });
    }

    {
        let (service, in_flight) = (service.clone(), in_flight.clone());
        btn.connect_clicked(move |_| {
            let snapshot = service.snapshot();
            if !clickable(view(&snapshot).map(|(_, s)| s), in_flight.get()) {
                return;
            }
            in_flight.set(true);
            let (in_flight, notifications) = (in_flight.clone(), notifications.clone());
            spawn_work(nx_prebuilt::start_apply, move |result| {
                in_flight.set(false);
                if let Err(e) = result {
                    report_failure(notifications.as_ref(), &e);
                }
            });
        });
    }

    // The file changes under the service; the Bar tab's switch changes here.
    {
        let render = render.clone();
        service.connect_change(move || render());
    }
    {
        let render = render.clone();
        store::observe(move || render());
    }
    render();

    btn
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_waiting_update_takes_a_click() {
        assert!(clickable(Some(Shown::Ready), false));
        assert!(!clickable(Some(Shown::Ready), true));
        for shown in [Shown::Building, Shown::Switching, Shown::Failed] {
            assert!(!clickable(Some(shown), false), "{shown:?}");
        }
        assert!(!clickable(None, false));
    }

    #[test]
    fn each_kind_of_state_has_its_own_glyph_and_tone() {
        assert_eq!(icon(Shown::Ready), icon(Shown::Building));
        assert_ne!(tone(Shown::Ready), tone(Shown::Building));
        assert_ne!(icon(Shown::Failed), icon(Shown::Ready));
        assert_ne!(icon(Shown::Switching), icon(Shown::Ready));
        assert_eq!(tone(Shown::Failed), ui::Tone::Warning);
    }

    #[test]
    fn no_file_and_nothing_to_do_hide_the_segment() {
        assert!(view(&None).is_none());
        let idle = nx_prebuilt::parse(r#"{"state":"idle","ready":false}"#);
        assert!(view(&idle).is_none());
        let ready = nx_prebuilt::parse(r#"{"state":"ready","ready":true}"#);
        assert_eq!(view(&ready).map(|(_, s)| s), Some(Shown::Ready));
    }
}
