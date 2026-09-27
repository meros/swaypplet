//! Quiet by context: popups held while a screen is shared, an output is
//! mirrored, a window is fullscreen on the focused output, or (opt-in) a
//! call is running. ROADMAP "Focus and quiet modes that know the context";
//! prior art in docs/prior-art/notifications-wm/presentation-mode-dnd.md.
//!
//! Four signals, OR-ed, each behind its own switch in the Alerts tab and
//! all of them behind one master switch. The rules while quiet follow the
//! interruption levels (ios-interruption-levels.md) mapped onto
//! freedesktop urgency:
//!
//! - **Critical** breaks through, as it does through Do Not Disturb.
//! - **Normal** is held: it reaches history as always, and no card pops.
//! - **Low** never pops anyway, so it is not held and not counted.
//! - The OSD card is held while the screen is shared, because it would be
//!   in the capture; mirroring and fullscreen leave it alone.
//!
//! Nothing is announced when quiet starts (windows-focus-assist.md: the
//! stand-down notice was louder than what it silenced). When it ends, and
//! only if something was held, one card counts it per sender, with
//! mechanical text, and opens the notification centre. A notification the
//! sender or the user closes in the meantime drops out of the count.
//!
//! The DND tile names the reason and overrides it for the rest of the
//! stretch with one click; the override lapses when the context ends, so
//! the next presentation is quiet again.
//!
//! No timer and no poll: the sway snapshot, the PipeWire graph
//! (`services::capture`), the sound server and the settings file all push,
//! and [`install`] re-evaluates on each push.

use std::cell::RefCell;
use std::rc::Rc;

use super::{Notification, Urgency, group};
use crate::settings::store::Alerts;

/// Why it is quiet. Declared strongest first: when several hold, the tile
/// names the first, and the summary names the strongest of the stretch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Trigger {
    Sharing,
    Mirrored,
    Call,
    Fullscreen,
}

impl Trigger {
    /// The tile's status line while it holds popups.
    pub fn quiet_label(self) -> &'static str {
        match self {
            Trigger::Sharing => "Quiet: screen shared",
            Trigger::Mirrored => "Quiet: display mirrored",
            Trigger::Call => "Quiet: in a call",
            Trigger::Fullscreen => "Quiet: full screen",
        }
    }

    /// The tile's status line after the one-click override.
    pub fn override_label(self) -> &'static str {
        match self {
            Trigger::Sharing => "Screen shared, alerts on",
            Trigger::Mirrored => "Mirrored, alerts on",
            Trigger::Call => "In a call, alerts on",
            Trigger::Fullscreen => "Full screen, alerts on",
        }
    }

    /// The tail of the summary card's title.
    fn while_(self) -> &'static str {
        match self {
            Trigger::Sharing | Trigger::Mirrored => "while you were presenting",
            Trigger::Call => "while you were in a call",
            Trigger::Fullscreen => "while in full screen",
        }
    }
}

/// What the world says right now, before any setting is applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Signals {
    pub sharing: bool,
    pub mirrored: bool,
    pub call: bool,
    pub fullscreen: bool,
}

/// The switches from the Alerts tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Switches {
    pub master: bool,
    pub sharing: bool,
    pub mirrored: bool,
    pub call: bool,
    pub fullscreen: bool,
}

impl Switches {
    pub fn from_alerts(a: &Alerts) -> Self {
        Switches {
            master: a.context_quiet,
            sharing: a.quiet_when_sharing,
            mirrored: a.quiet_when_mirrored,
            call: a.quiet_in_calls,
            fullscreen: a.quiet_when_fullscreen,
        }
    }
}

impl Default for Switches {
    fn default() -> Self {
        Switches::from_alerts(&Alerts::default())
    }
}

/// The count the end card shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    pub reason: Trigger,
    /// Sender label and count, most first; ties keep arrival order.
    pub senders: Vec<(String, usize)>,
}

impl Summary {
    pub fn total(&self) -> usize {
        self.senders.iter().map(|(_, n)| n).sum()
    }

    /// "7 while you were presenting".
    pub fn title(&self) -> String {
        format!("{} {}", self.total(), self.reason.while_())
    }

    /// "Chat 4 · CI 2 · Mail 1": each sender as it named itself, and its
    /// count. Nothing is paraphrased (apple-intelligence-summaries.md).
    pub fn body(&self) -> String {
        self.senders
            .iter()
            .map(|(app, n)| format!("{app} {n}"))
            .collect::<Vec<_>>()
            .join(" · ")
    }
}

/// One held notification: the id, so a close can take it back out, and
/// the sender it counts under.
#[derive(Debug, Clone)]
struct Held {
    id: u32,
    key: String,
    label: String,
}

/// The state machine. Pure: the store owns one, and [`install`] feeds it.
#[derive(Debug, Default)]
pub struct Context {
    signals: Signals,
    switches: Switches,
    /// The tile's override, for this stretch only.
    overridden: bool,
    held: Vec<Held>,
    /// The strongest reason seen while holding, for the summary's words.
    strongest: Option<Trigger>,
}

impl Context {
    /// The strongest trigger that is both signalled and switched on.
    pub fn reason(&self) -> Option<Trigger> {
        let (s, w) = (self.signals, self.switches);
        if !w.master {
            return None;
        }
        [
            (s.sharing && w.sharing, Trigger::Sharing),
            (s.mirrored && w.mirrored, Trigger::Mirrored),
            (s.call && w.call, Trigger::Call),
            (s.fullscreen && w.fullscreen, Trigger::Fullscreen),
        ]
        .into_iter()
        .find_map(|(on, t)| on.then_some(t))
    }

    /// Popups are being held.
    pub fn is_quiet(&self) -> bool {
        self.reason().is_some() && !self.overridden
    }

    pub fn is_overridden(&self) -> bool {
        self.overridden && self.reason().is_some()
    }

    /// The OSD card would be in a capture: hold it. Only sharing does this;
    /// a mirrored or fullscreen screen is the user's own to see.
    pub fn holds_osd(&self) -> bool {
        self.is_quiet() && self.signals.sharing && self.switches.sharing
    }

    /// Should `n`, which would otherwise pop, be held? Counts it if so,
    /// unless it is transient: history does not keep those, so the card
    /// that opens history must not promise them.
    pub fn hold(&mut self, n: &Notification) -> bool {
        if !self.is_quiet() || n.urgency != Urgency::Normal {
            return false;
        }
        if !n.transient && !self.held.iter().any(|h| h.id == n.id) {
            let label = if n.app_name.is_empty() {
                "Other".to_string()
            } else {
                n.app_name.clone()
            };
            let key = group::key(n).unwrap_or_else(|| label.to_lowercase());
            self.held.push(Held {
                id: n.id,
                key,
                label,
            });
        }
        true
    }

    /// A held notification was closed: it no longer counts.
    pub fn forget(&mut self, id: u32) {
        self.held.retain(|h| h.id != id);
    }

    /// New signals. Returns the summary when this ends a quiet stretch
    /// that held something.
    pub fn set_signals(&mut self, signals: Signals) -> Option<Summary> {
        self.transition(|c| c.signals = signals)
    }

    /// New switches, from the Alerts tab.
    pub fn set_switches(&mut self, switches: Switches) -> Option<Summary> {
        self.transition(|c| c.switches = switches)
    }

    /// The tile's one click. Ends the stretch like the context ending would.
    pub fn set_override(&mut self, on: bool) -> Option<Summary> {
        self.transition(|c| c.overridden = on)
    }

    fn transition(&mut self, change: impl FnOnce(&mut Self)) -> Option<Summary> {
        let was = self.is_quiet();
        change(self);
        if self.reason().is_none() {
            // The context is over: the next one starts quiet again.
            self.overridden = false;
        }
        let now = self.is_quiet();
        if now {
            self.strongest = match (self.strongest, self.reason()) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
        }
        if was && !now {
            let reason = self.strongest.take();
            return self.summarize(reason?);
        }
        None
    }

    fn summarize(&mut self, reason: Trigger) -> Option<Summary> {
        let held = std::mem::take(&mut self.held);
        let mut senders: Vec<(String, String, usize)> = Vec::new();
        for h in held {
            match senders.iter_mut().find(|(k, _, _)| *k == h.key) {
                Some(s) => s.2 += 1,
                None => senders.push((h.key, h.label, 1)),
            }
        }
        if senders.is_empty() {
            return None;
        }
        // Stable: equal counts keep the order they arrived in.
        senders.sort_by_key(|s| std::cmp::Reverse(s.2));
        Some(Summary {
            reason,
            senders: senders.into_iter().map(|(_, l, n)| (l, n)).collect(),
        })
    }
}

// ── Wiring ──────────────────────────────────────────────────────────────

use super::store::{StoreRef, store_add};

// The summary card's id, so its click can be told from any other's; and
// what that click does.
thread_local! {
    static SUMMARY: RefCell<Option<u32>> = const { RefCell::new(None) };
    static OPEN_CENTRE: RefCell<Option<Rc<dyn Fn()>>> = const { RefCell::new(None) };
}

/// Post the end card. Transient: it is a count of what history already
/// holds, so it does not go there itself.
fn post(store: &StoreRef, summary: Summary) {
    log::info!("quiet: over, {} held ({})", summary.total(), summary.body());
    let id = store_add(
        store,
        Notification {
            app_name: "Notifications".into(),
            summary: summary.title(),
            body: summary.body(),
            urgency: Urgency::Normal,
            actions: vec![
                ("default".into(), "Show".into()),
                ("show".into(), "Show all".into()),
            ],
            transient: true,
            // Stays until clicked or dismissed. It stands in for every card
            // that did not pop, and the user may not be looking the moment
            // the presentation ends; one quiet card is a fair price.
            expire_timeout: 0,
            timestamp: std::time::SystemTime::now(),
            ..Default::default()
        },
    );
    SUMMARY.with(|s| *s.borrow_mut() = Some(id));
}

/// Apply `change` to the store's context and post the card it returns.
pub fn update(store: &StoreRef, change: impl FnOnce(&mut Context) -> Option<Summary>) {
    let summary = change(store.borrow_mut().context_mut());
    if let Some(summary) = summary {
        post(store, summary);
    }
}

/// The tile's override, from `widgets::tiles`.
pub fn set_override(store: &StoreRef, on: bool) {
    update(store, |c| c.set_override(on));
}

/// Where the signals come from. Each is optional: a process without a bar
/// has no sway model, and a machine without `pw-dump` has no capture feed.
pub struct Sources {
    pub sway: Option<Rc<crate::sway::ipc::SwayService>>,
    pub capture: Option<Rc<crate::services::capture::CaptureService>>,
    pub audio: Option<Rc<crate::services::audio::AudioService>>,
    /// Opens the panel on the notification centre.
    pub open_centre: Rc<dyn Fn()>,
}

/// Follow the sources and the settings for the life of the process.
pub fn install(store: StoreRef, sources: Sources) {
    OPEN_CENTRE.with(|o| *o.borrow_mut() = Some(sources.open_centre.clone()));

    // The summary card's click and its button both open the centre.
    store.borrow_mut().connect_action(move |id, key| {
        if SUMMARY.with(|s| *s.borrow()) == Some(id)
            && (key == "default" || key == "show")
            && let Some(open) = OPEN_CENTRE.with(|o| o.borrow().clone())
        {
            open();
        }
    });

    let sway = sources.sway.clone();
    let capture = sources.capture.clone();
    let audio = sources.audio.clone();
    let read = Rc::new(move || {
        let (fullscreen, mirrored) = sway
            .as_ref()
            .map(|s| s.snapshot())
            .map_or((false, false), |s| {
                (s.fullscreen_on_focused_output, s.mirrored)
            });
        let (sharing, camera) = capture
            .as_ref()
            .map(|c| c.snapshot())
            .map_or((false, false), |c| (c.screen, c.camera));
        let call_mic = audio
            .as_ref()
            .is_some_and(|a| a.snapshot().recorder_names().iter().any(|n| is_call_app(n)));
        Signals {
            sharing,
            mirrored,
            call: camera || call_mic,
            fullscreen,
        }
    });

    let evaluate = {
        let store = store.clone();
        let read = read.clone();
        Rc::new(move || {
            let signals = read();
            update(&store, |c| {
                let was = c.reason();
                let summary = c.set_signals(signals);
                if c.reason() != was {
                    log::info!("quiet: context {:?} → {:?}", was, c.reason());
                }
                summary
            });
        })
    };

    if let Some(s) = &sources.sway {
        let evaluate = evaluate.clone();
        s.connect_change(move || evaluate());
    }
    if let Some(c) = &sources.capture {
        let evaluate = evaluate.clone();
        c.connect_change(move || evaluate());
    }
    if let Some(a) = &sources.audio {
        let evaluate = evaluate.clone();
        a.connect_change(move || evaluate());
    }
    {
        let store = store.clone();
        let apply = move || {
            let switches = Switches::from_alerts(&crate::settings::store::current().alerts());
            update(&store, |c| c.set_switches(switches));
        };
        apply();
        crate::settings::store::observe(apply);
    }
    evaluate();
}

/// A sound-server stream name that belongs to a call. Browsers count:
/// a browser holding the microphone is a meeting far more often than not,
/// and the trigger is off by default for that reason.
fn is_call_app(name: &str) -> bool {
    const CALL_APPS: &[&str] = &[
        "zoom", "teams", "slack", "discord", "webex", "skype", "jitsi", "element", "signal",
        "telegram", "mumble", "firefox", "chrom", "brave",
    ];
    let name = name.to_lowercase();
    CALL_APPS.iter().any(|app| name.contains(app))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(id: u32, app: &str, urgency: Urgency) -> Notification {
        Notification {
            id,
            app_name: app.into(),
            urgency,
            ..Default::default()
        }
    }

    fn sharing() -> Signals {
        Signals {
            sharing: true,
            ..Default::default()
        }
    }

    #[test]
    fn any_one_trigger_is_enough_and_the_strongest_names_it() {
        let mut c = Context::default();
        assert_eq!(c.reason(), None);
        c.set_signals(Signals {
            fullscreen: true,
            ..Default::default()
        });
        assert_eq!(c.reason(), Some(Trigger::Fullscreen));
        c.set_signals(Signals {
            fullscreen: true,
            mirrored: true,
            ..Default::default()
        });
        assert_eq!(c.reason(), Some(Trigger::Mirrored));
        c.set_signals(Signals {
            fullscreen: true,
            mirrored: true,
            sharing: true,
            call: false,
        });
        assert_eq!(c.reason(), Some(Trigger::Sharing));
        assert!(c.is_quiet());
    }

    #[test]
    fn a_switched_off_trigger_does_not_count_and_the_master_turns_all_off() {
        let mut c = Context::default();
        // Calls are off by default.
        c.set_signals(Signals {
            call: true,
            ..Default::default()
        });
        assert_eq!(c.reason(), None);
        c.set_switches(Switches {
            call: true,
            ..Switches::default()
        });
        assert_eq!(c.reason(), Some(Trigger::Call));

        c.set_signals(Signals {
            fullscreen: true,
            ..Default::default()
        });
        c.set_switches(Switches {
            fullscreen: false,
            ..Switches::default()
        });
        assert_eq!(c.reason(), None);

        c.set_switches(Switches::default());
        assert!(c.is_quiet());
        c.set_switches(Switches {
            master: false,
            ..Switches::default()
        });
        assert!(!c.is_quiet());
    }

    #[test]
    fn critical_breaks_through_and_low_is_not_counted() {
        let mut c = Context::default();
        c.set_signals(sharing());
        assert!(!c.hold(&n(1, "Disk", Urgency::Critical)));
        assert!(!c.hold(&n(2, "Music", Urgency::Low)));
        assert!(c.hold(&n(3, "Chat", Urgency::Normal)));
        let transient = Notification {
            transient: true,
            ..n(4, "Volume", Urgency::Normal)
        };
        assert!(c.hold(&transient), "held, since it would be in the capture");
        let summary = c.set_signals(Signals::default()).unwrap();
        assert_eq!(
            summary.total(),
            1,
            "but not counted: history has no row for it"
        );
        assert_eq!(summary.body(), "Chat 1");
    }

    #[test]
    fn the_summary_counts_per_sender_and_says_why() {
        let mut c = Context::default();
        c.set_signals(sharing());
        for (id, app) in [
            (1, "Chat"),
            (2, "CI"),
            (3, "chat"),
            (4, "Chat"),
            (5, "Mail"),
        ] {
            assert!(c.hold(&n(id, app, Urgency::Normal)));
        }
        // A replacement of a held one is the same notification.
        assert!(c.hold(&n(2, "CI", Urgency::Normal)));
        // Nothing is announced while it lasts.
        assert_eq!(
            c.set_signals(Signals {
                sharing: true,
                fullscreen: true,
                ..Default::default()
            }),
            None
        );
        let summary = c.set_signals(Signals::default()).unwrap();
        assert_eq!(summary.title(), "5 while you were presenting");
        assert_eq!(summary.body(), "Chat 3 · CI 1 · Mail 1");
        // Counted once: the next stretch starts from nothing.
        c.set_signals(sharing());
        assert_eq!(c.set_signals(Signals::default()), None);
    }

    #[test]
    fn nothing_held_means_no_card() {
        let mut c = Context::default();
        c.set_signals(sharing());
        assert_eq!(c.set_signals(Signals::default()), None);
    }

    #[test]
    fn a_closed_notification_drops_out_of_the_count() {
        let mut c = Context::default();
        c.set_signals(sharing());
        c.hold(&n(1, "Chat", Urgency::Normal));
        c.hold(&n(2, "Chat", Urgency::Normal));
        c.forget(1);
        assert_eq!(c.set_signals(Signals::default()).unwrap().total(), 1);

        c.set_signals(sharing());
        c.hold(&n(3, "Chat", Urgency::Normal));
        c.forget(3);
        assert_eq!(c.set_signals(Signals::default()), None);
    }

    #[test]
    fn the_override_ends_the_stretch_and_lapses_with_the_context() {
        let mut c = Context::default();
        c.set_signals(sharing());
        c.hold(&n(1, "Chat", Urgency::Normal));
        let summary = c.set_override(true).unwrap();
        assert_eq!(summary.total(), 1);
        assert!(!c.is_quiet());
        assert!(c.is_overridden());
        assert!(!c.hold(&n(2, "Chat", Urgency::Normal)), "overridden pops");
        assert!(!c.holds_osd());

        // Still sharing, now fullscreen too: the override stands.
        c.set_signals(Signals {
            sharing: true,
            fullscreen: true,
            ..Default::default()
        });
        assert!(!c.is_quiet());

        // The context ends, and the next one is quiet again.
        assert_eq!(c.set_signals(Signals::default()), None);
        assert!(!c.is_overridden());
        c.set_signals(sharing());
        assert!(c.is_quiet());

        // Taking the override back is quiet again at once.
        c.set_override(true);
        c.set_override(false);
        assert!(c.is_quiet());
    }

    #[test]
    fn the_osd_is_held_only_while_sharing() {
        let mut c = Context::default();
        c.set_signals(Signals {
            fullscreen: true,
            mirrored: true,
            ..Default::default()
        });
        assert!(c.is_quiet() && !c.holds_osd());
        c.set_signals(sharing());
        assert!(c.holds_osd());
        c.set_switches(Switches {
            sharing: false,
            ..Switches::default()
        });
        assert!(!c.holds_osd());
    }

    #[test]
    fn the_summary_names_the_strongest_reason_of_the_stretch() {
        let mut c = Context::default();
        c.set_signals(Signals {
            fullscreen: true,
            ..Default::default()
        });
        c.hold(&n(1, "Chat", Urgency::Normal));
        c.set_signals(Signals {
            fullscreen: true,
            sharing: true,
            ..Default::default()
        });
        c.set_signals(Signals {
            fullscreen: true,
            ..Default::default()
        });
        let summary = c.set_signals(Signals::default()).unwrap();
        assert_eq!(summary.reason, Trigger::Sharing);

        c.set_signals(Signals {
            fullscreen: true,
            ..Default::default()
        });
        c.hold(&n(2, "Chat", Urgency::Normal));
        let summary = c.set_signals(Signals::default()).unwrap();
        assert_eq!(summary.title(), "1 while in full screen");
    }

    #[test]
    fn switching_the_master_off_mid_stretch_delivers_the_count() {
        let mut c = Context::default();
        c.set_signals(sharing());
        c.hold(&n(1, "Chat", Urgency::Normal));
        let summary = c.set_switches(Switches {
            master: false,
            ..Switches::default()
        });
        assert_eq!(summary.map(|s| s.total()), Some(1));
    }

    #[test]
    fn call_apps_are_matched_by_name() {
        assert!(is_call_app("ZOOM VoiceEngine"));
        assert!(is_call_app("Chromium input"));
        assert!(!is_call_app("arecord"));
    }
}
