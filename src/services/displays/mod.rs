//! Display profiles, in the panel process (docs/SETTINGS.md, `displays`).
//!
//! Replaces kanshi. The profiles are a settings section, so Nix ships the
//! defaults and the Displays section edits them; the matching is kanshi's
//! (`matcher`); the outputs are driven over `zwlr_output_manager_v1`
//! (`wayland`), which applies every output in one step and answers whether
//! it took. Sway IPC is left for what that protocol cannot say: the
//! workspaces follow their outputs through sway's own `workspace … output`
//! assignments, as they did under kanshi.
//!
//! # When it acts
//!
//! Nothing runs at rest. The compositor ends every change to the outputs
//! with `done`; the profile is matched again only when the *set* of
//! connected outputs changed (the panel's own applies and the Disable
//! button also end in `done`, and must not be undone), when the profiles
//! in the settings changed, and after `swaymsg reload`, which puts the
//! config's `output` lines back. An apply that changes nothing is never
//! sent: it would be a modeset, and a modeset blanks the screen.
//!
//! `failed` leaves the outputs as they were (the compositor stores nothing)
//! and says so in a notification. `cancelled` means the outputs changed
//! while the configuration was on its way; the next `done` matches again,
//! once.

mod matcher;
pub mod naming;
mod wayland;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub use matcher::{HeadPlan, HeadState, Mode, ModeChoice, dpi_scale};
use matcher::PlanError;
pub use wayland::Outcome;

use crate::service::Observed;
use crate::services::notifications::store::{StoreRef, store_add};
use crate::services::notifications::{Notification, Urgency};
use crate::settings::store::{self, DisplayProfile, Displays};

/// What the Displays section shows.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct View {
    /// Whether the compositor offers output management at all.
    pub available: bool,
    pub heads: Vec<HeadState>,
    /// The profile on screen.
    pub current: Option<String>,
    /// Every profile, in order, and whether it fits the outputs now.
    pub profiles: Vec<(String, bool)>,
}

thread_local! {
    static VIEW: Observed<View> = Observed::new(View::default());
    static ENGINE: RefCell<Option<Rc<Engine>>> = const { RefCell::new(None) };
}

/// Read the current view.
pub fn view() -> View {
    VIEW.with(|v| v.with(Clone::clone))
}

/// Run `cb` after the view changes.
pub fn observe(cb: impl Fn() + 'static) {
    VIEW.with(|v| v.connect_change(cb));
}

/// The name a density scale for unnamed outputs is sent under.
const DEFAULT: &str = "(scale from density)";
/// The name a layout from the Displays tab is sent under.
const BY_HAND: &str = "(arranged by hand)";

/// Who asked for a configuration and wants its answer.
type Reply = Box<dyn FnOnce(Outcome)>;

struct Pending {
    id: u64,
    profile: String,
    /// Already retried once after a `cancelled`.
    retried: bool,
    /// A layout from the Displays tab: the answer goes back to it, and
    /// nothing is retried or notified here.
    reply: Option<Reply>,
}

/// Where a configuration goes: the compositor, or a test's recorder.
trait Sink {
    fn apply(&self, id: u64, serial: u32, heads: &[HeadState], plans: &[HeadPlan]);
}

impl Sink for wayland::Outputs {
    fn apply(&self, id: u64, serial: u32, heads: &[HeadState], plans: &[HeadPlan]) {
        wayland::Outputs::apply(self, id, serial, heads, plans);
    }
}

/// The profiles in force, asked for at each match.
type Source = Box<dyn Fn() -> Vec<DisplayProfile>>;
/// A notification: summary and body.
type Notify = Box<dyn Fn(&str, &str)>;

struct Engine {
    sink: Box<dyn Sink>,
    notify: Notify,
    /// The profiles in force: the settings in the panel.
    source: Source,
    heads: RefCell<Vec<HeadState>>,
    serial: Cell<u32>,
    /// The connected outputs' identities at the last match, sorted.
    identities: RefCell<Vec<String>>,
    /// The identities before the change being matched now: an output not in
    /// here is new, and one no profile names gets a scale from its density.
    before: RefCell<Vec<String>>,
    current: RefCell<Option<String>>,
    pending: RefCell<Option<Pending>>,
    /// A `cancelled` asked for a match on the next `done`.
    retry: Cell<bool>,
    /// That match is running now: what it sends is the one retry.
    retrying: Cell<bool>,
    next_id: Cell<u64>,
    /// The profiles at the last match, to notice an edit.
    profiles: RefCell<Displays>,
}

/// Start following the outputs. Call once, in the panel process, on the
/// GTK thread. A compositor without output management leaves the outputs
/// to whatever else sets them.
pub fn start(store: StoreRef) {
    let Some((outputs, rx)) = wayland::Outputs::start() else {
        return;
    };
    let engine = Rc::new(Engine::new(
        Box::new(outputs),
        Box::new(move |summary, body| notify(&store, summary, body)),
        Box::new(|| store::current().displays().profiles),
    ));
    *engine.profiles.borrow_mut() = store::current().displays();
    ENGINE.with(|e| *e.borrow_mut() = Some(engine.clone()));
    {
        let engine = engine.clone();
        glib::spawn_future_local(async move {
            while let Ok(event) = rx.recv().await {
                engine.event(event);
            }
        });
    }
    store::observe(move || {
        let now = store::current().displays();
        if *engine.profiles.borrow() != now {
            *engine.profiles.borrow_mut() = now;
            engine.rematch();
        }
    });
}

/// Sway reloaded its config and put its own `output` lines back: show the
/// profile again.
pub fn on_reload() {
    if let Some(e) = engine() {
        e.rematch();
    }
}

/// Show `name`, when it fits the outputs now. It stays until the outputs
/// change and it no longer fits.
pub fn apply(name: &str) {
    let Some(e) = engine() else { return };
    let profiles = store::current().displays().profiles;
    let heads = e.heads.borrow().clone();
    let Some(profile) = profiles.iter().find(|p| p.name == name) else {
        return;
    };
    let Some(assignment) = matcher::assign(profile, &heads) else {
        return;
    };
    e.show(profile, &assignment, &heads);
}

/// Send `plans`, one per head of `heads` (the view the caller built them
/// from), as one configuration: tested, then applied. `reply` gets the
/// answer, always once and never inside this call. The plans are matched to
/// the outputs as they are now, by connector, so a layout built from an
/// older view (a revert) is judged against what is on screen: one already
/// there answers `Succeeded` without a modeset, and one that enables
/// nothing is refused here as `Failed`. A hand layout is no profile, so the
/// profile on screen is forgotten when it succeeds.
pub fn configure(heads: &[HeadState], plans: Vec<HeadPlan>, reply: impl FnOnce(Outcome) + 'static) {
    let answer = |reply: Reply, outcome| {
        glib::idle_add_local_once(move || reply(outcome));
    };
    let reply: Reply = Box::new(reply);
    let Some(e) = engine() else {
        return answer(reply, Outcome::Failed);
    };
    let now = e.heads.borrow().clone();
    let Some(plans) = onto(heads, &plans, &now) else {
        return answer(reply, Outcome::Failed);
    };
    if matcher::satisfied(&plans, &now) {
        return answer(reply, Outcome::Succeeded);
    }
    e.send_with(BY_HAND, &plans, &now, Some(reply));
}

/// `plans` (one per head of `from`) for the heads in `now`, by connector.
/// A head the plans do not name stays as it is. `None` when the plans do
/// not line up with `from`, or when nothing would be left on.
fn onto(from: &[HeadState], plans: &[HeadPlan], now: &[HeadState]) -> Option<Vec<HeadPlan>> {
    if plans.len() != from.len() {
        return None;
    }
    let out: Vec<HeadPlan> = now
        .iter()
        .map(|h| {
            from.iter()
                .position(|f| f.name == h.name)
                .map(|i| plans[i].clone())
                .unwrap_or(if h.enabled {
                    HeadPlan::Enable {
                        mode: None,
                        position: None,
                        scale: None,
                        transform: None,
                        adaptive_sync: None,
                    }
                } else {
                    HeadPlan::Disable
                })
        })
        .collect();
    (!out.iter().all(|p| *p == HeadPlan::Disable)).then_some(out)
}

/// Save the layout on screen as `name`, replacing a profile of that name
/// or adding one at the top, where it wins over the rest.
pub fn save_current(name: &str) {
    let Some(e) = engine() else { return };
    let name = name.trim();
    if name.is_empty() {
        return;
    }
    let profile = matcher::capture(name, &e.heads.borrow());
    *e.current.borrow_mut() = Some(name.to_string());
    store::edit::<Displays>(|d| match d.profiles.iter().position(|p| p.name == name) {
        Some(i) => d.profiles[i] = profile,
        None => d.profiles.insert(0, profile),
    });
    e.publish();
}

/// Remove the profile `name`.
pub fn delete(name: &str) {
    store::edit::<Displays>(|d| d.profiles.retain(|p| p.name != name));
}

/// Move the profile `name` one place up, ahead of the one before it in the
/// matching order.
pub fn raise(name: &str) {
    store::edit::<Displays>(|d| {
        if let Some(i) = d.profiles.iter().position(|p| p.name == name)
            && i > 0
        {
            d.profiles.swap(i, i - 1);
        }
    });
}

fn engine() -> Option<Rc<Engine>> {
    ENGINE.with(|e| e.borrow().clone())
}

fn notify(store: &StoreRef, summary: &str, body: &str) {
    store_add(
        store,
        Notification {
            app_name: "Displays".into(),
            summary: summary.into(),
            body: body.into(),
            urgency: Urgency::Normal,
            expire_timeout: 8000,
            // `Default` leaves the epoch, which the card shows as its age.
            timestamp: std::time::SystemTime::now(),
            ..Default::default()
        },
    );
}

impl Engine {
    fn new(sink: Box<dyn Sink>, notify: Notify, source: Source) -> Engine {
        Engine {
            sink,
            notify,
            source,
            heads: RefCell::default(),
            serial: Cell::new(0),
            identities: RefCell::default(),
            before: RefCell::default(),
            current: RefCell::new(None),
            pending: RefCell::new(None),
            retry: Cell::new(false),
            retrying: Cell::new(false),
            next_id: Cell::new(1),
            profiles: RefCell::default(),
        }
    }

    fn event(&self, event: wayland::Event) {
        match event {
            wayland::Event::Heads { serial, heads } => {
                self.serial.set(serial);
                let mut ids: Vec<String> = heads.iter().map(HeadState::identity).collect();
                ids.sort();
                *self.heads.borrow_mut() = heads;
                let changed = *self.identities.borrow() != ids;
                if changed {
                    let old = self.identities.replace(ids);
                    *self.before.borrow_mut() = old;
                }
                let retry = self.retry.replace(false);
                if changed || retry {
                    self.retrying.set(retry);
                    self.rematch();
                    self.retrying.set(false);
                } else {
                    self.publish();
                }
            }
            wayland::Event::Outcome { id, outcome } => self.outcome(id, outcome),
        }
    }

    fn outcome(&self, id: u64, outcome: wayland::Outcome) {
        let Some(mut pending) = self.pending.borrow_mut().take_if(|p| p.id == id) else {
            return;
        };
        if let Some(reply) = pending.reply.take() {
            log::info!("displays: layout by hand: {outcome:?}");
            if outcome == Outcome::Succeeded {
                *self.current.borrow_mut() = None;
            }
            reply(outcome);
            self.publish();
            return;
        }
        match outcome {
            wayland::Outcome::Succeeded => {
                log::info!("displays: profile {} applied", pending.profile);
                if pending.profile != DEFAULT {
                    *self.current.borrow_mut() = Some(pending.profile);
                }
            }
            wayland::Outcome::Failed => {
                log::warn!("displays: profile {} refused", pending.profile);
                (self.notify)(
                    &format!("Display profile “{}” could not be applied", pending.profile),
                    "The compositor refused it. The displays are as they were.",
                );
            }
            wayland::Outcome::Cancelled if !pending.retried => {
                log::info!(
                    "displays: {} cancelled, outputs changed; again",
                    pending.profile
                );
                self.retry.set(true);
            }
            wayland::Outcome::Cancelled => {
                log::warn!("displays: {} cancelled twice; left", pending.profile);
            }
        }
        self.publish();
    }

    /// Match the profiles against the outputs and show the winner.
    fn rematch(&self) {
        let profiles = (self.source)();
        let heads = self.heads.borrow().clone();
        let current = self.current.borrow().clone();
        match matcher::choose(&profiles, &heads, current.as_deref()) {
            Some((i, assignment)) => self.show(&profiles[i], &assignment, &heads),
            None => {
                *self.current.borrow_mut() = None;
                // No profile names these outputs: a new one the compositor
                // left at scale 1 gets one from its pixel density.
                let before = self.before.borrow();
                let new: Vec<bool> = heads
                    .iter()
                    .map(|h| !before.contains(&h.identity()))
                    .collect();
                drop(before);
                if let Some(plans) = matcher::default_plan(&heads, &new)
                    && !matcher::satisfied(&plans, &heads)
                {
                    self.send(DEFAULT, &plans, &heads);
                }
                self.publish();
            }
        }
    }

    fn show(&self, profile: &DisplayProfile, assignment: &[usize], heads: &[HeadState]) {
        let plans: Vec<HeadPlan> = match matcher::plan(profile, assignment, heads) {
            Ok(p) => p,
            Err(PlanError::NoneEnabled) => {
                (self.notify)(
                    &format!("Display profile “{}” not applied", profile.name),
                    "It turns every display off.",
                );
                return;
            }
        };
        if matcher::satisfied(&plans, heads) {
            *self.current.borrow_mut() = Some(profile.name.clone());
            self.publish();
            return;
        }
        self.send(&profile.name, &plans, heads);
    }

    fn send(&self, name: &str, plans: &[HeadPlan], heads: &[HeadState]) {
        self.send_with(name, plans, heads, None);
    }

    fn send_with(&self, name: &str, plans: &[HeadPlan], heads: &[HeadState], reply: Option<Reply>) {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        let before = self.pending.replace(Some(Pending {
            id,
            profile: name.to_string(),
            retried: self.retrying.get(),
            reply,
        }));
        // The one before is superseded and its answer will be dropped; a
        // caller waiting on it hears so now rather than never.
        if let Some(reply) = before.and_then(|p| p.reply) {
            glib::idle_add_local_once(move || reply(Outcome::Cancelled));
        }
        log::info!("displays: applying profile {name}");
        self.sink.apply(id, self.serial.get(), heads, plans);
    }

    fn publish(&self) {
        let heads = self.heads.borrow().clone();
        let profiles = (self.source)()
            .iter()
            .map(|p| (p.name.clone(), matcher::assign(p, &heads).is_some()))
            .collect();
        let v = View {
            available: true,
            heads,
            current: self.current.borrow().clone(),
            profiles,
        };
        VIEW.with(|o| o.set_if_changed(v));
    }
}

#[cfg(test)]
mod tests {
    use super::matcher::tests::head;
    use super::*;
    use crate::settings::store::{DisplayOutput, OutputMatch};

    #[derive(Default)]
    struct Recorder {
        sent: RefCell<Vec<(u64, u32, Vec<HeadPlan>)>>,
    }

    impl Sink for Rc<Recorder> {
        fn apply(&self, id: u64, serial: u32, _: &[HeadState], plans: &[HeadPlan]) {
            self.sent.borrow_mut().push((id, serial, plans.to_vec()));
        }
    }

    fn profile(name: &str, scale: f64) -> DisplayProfile {
        DisplayProfile {
            name: name.into(),
            outputs: vec![DisplayOutput {
                criteria: OutputMatch {
                    name: Some("DP-1".into()),
                    ..OutputMatch::default()
                },
                enabled: true,
                mode: None,
                position: None,
                scale: Some(scale),
                transform: None,
                adaptive_sync: None,
            }],
        }
    }

    fn engine(profiles: Vec<DisplayProfile>) -> (Engine, Rc<Recorder>, Rc<RefCell<Vec<String>>>) {
        let rec = Rc::new(Recorder::default());
        let said = Rc::new(RefCell::new(Vec::new()));
        let s = said.clone();
        let e = Engine::new(
            Box::new(rec.clone()),
            Box::new(move |summary, _| s.borrow_mut().push(summary.to_string())),
            Box::new(move || profiles.clone()),
        );
        (e, rec, said)
    }

    fn heads(serial: u32, scale: f64) -> wayland::Event {
        let mut h = head("DP-1", "X", "Y");
        h.scale = scale;
        wayland::Event::Heads {
            serial,
            heads: vec![h],
        }
    }

    #[test]
    fn a_new_output_set_is_matched_and_applied_once() {
        let (e, rec, _) = engine(vec![profile("big", 2.0)]);
        e.event(heads(7, 1.0));
        let sent = rec.sent.borrow().clone();
        assert_eq!(sent.len(), 1);
        assert_eq!(
            sent[0].1, 7,
            "the configuration carries the serial it was built on"
        );
        e.event(wayland::Event::Outcome {
            id: sent[0].0,
            outcome: wayland::Outcome::Succeeded,
        });
        assert_eq!(e.current.borrow().as_deref(), Some("big"));
        // The apply's own `done`: same outputs, now at 2.0. Nothing is sent.
        e.event(heads(8, 2.0));
        // A change by hand (the same outputs at another scale) is not undone.
        e.event(heads(9, 1.5));
        assert_eq!(rec.sent.borrow().len(), 1);
    }

    #[test]
    fn a_refused_profile_leaves_the_outputs_and_says_so() {
        let (e, rec, said) = engine(vec![profile("big", 2.0)]);
        e.event(heads(1, 1.0));
        let id = rec.sent.borrow()[0].0;
        e.event(wayland::Event::Outcome {
            id,
            outcome: wayland::Outcome::Failed,
        });
        assert_eq!(said.borrow().len(), 1, "{:?}", said.borrow());
        assert!(e.current.borrow().is_none());
        assert_eq!(rec.sent.borrow().len(), 1, "no retry after failed");
    }

    #[test]
    fn a_cancelled_apply_is_retried_once_on_the_next_done() {
        let (e, rec, _) = engine(vec![profile("big", 2.0)]);
        e.event(heads(1, 1.0));
        let cancel = |e: &Engine, rec: &Recorder| {
            let id = rec.sent.borrow().last().unwrap().0;
            e.event(wayland::Event::Outcome {
                id,
                outcome: wayland::Outcome::Cancelled,
            });
        };
        cancel(&e, &rec);
        e.event(heads(2, 1.0));
        assert_eq!(rec.sent.borrow().len(), 2);
        assert_eq!(
            rec.sent.borrow()[1].1,
            2,
            "the retry carries the new serial"
        );
        cancel(&e, &rec);
        e.event(heads(3, 1.0));
        assert_eq!(rec.sent.borrow().len(), 2, "only one retry");
    }

    #[test]
    fn a_layout_by_hand_answers_its_caller_and_forgets_the_profile() {
        let (e, rec, said) = engine(vec![profile("big", 2.0)]);
        e.event(heads(1, 1.0));
        let id = rec.sent.borrow()[0].0;
        e.event(wayland::Event::Outcome {
            id,
            outcome: wayland::Outcome::Succeeded,
        });
        assert_eq!(e.current.borrow().as_deref(), Some("big"));
        let got = Rc::new(Cell::new(None));
        let g = got.clone();
        let h = e.heads.borrow().clone();
        let plan = vec![HeadPlan::Enable {
            mode: None,
            position: Some((10, 0)),
            scale: None,
            transform: None,
            adaptive_sync: None,
        }];
        e.send_with(BY_HAND, &plan, &h, Some(Box::new(move |o| g.set(Some(o)))));
        let id = rec.sent.borrow()[1].0;
        e.event(wayland::Event::Outcome {
            id,
            outcome: wayland::Outcome::Failed,
        });
        assert_eq!(got.get(), Some(Outcome::Failed));
        assert!(said.borrow().is_empty(), "the tab says it, not a notification");
        assert_eq!(e.current.borrow().as_deref(), Some("big"), "a refusal changes nothing");
        let g = got.clone();
        e.send_with(BY_HAND, &plan, &h, Some(Box::new(move |o| g.set(Some(o)))));
        let id = rec.sent.borrow()[2].0;
        e.event(wayland::Event::Outcome {
            id,
            outcome: wayland::Outcome::Succeeded,
        });
        assert_eq!(got.get(), Some(Outcome::Succeeded));
        assert!(e.current.borrow().is_none(), "a hand layout is no profile");
    }

    #[test]
    fn a_revert_is_judged_against_the_outputs_now() {
        // The plan puts DP-1 back at scale 1, built from the view before an
        // apply moved it to 2: against that old view it is "already there",
        // against the outputs now it has to be sent.
        let mut before = head("DP-1", "X", "Y");
        before.scale = 1.0;
        let mut now = before.clone();
        now.scale = 2.0;
        let back = vec![HeadPlan::Enable {
            mode: None,
            position: None,
            scale: Some(1.0),
            transform: None,
            adaptive_sync: None,
        }];
        let plans = onto(&[before.clone()], &back, &[now.clone()]).unwrap();
        assert!(matcher::satisfied(&plans, &[before.clone()]));
        assert!(!matcher::satisfied(&plans, &[now.clone()]));
        // A head the plans do not name stays as it is; nothing on is refused.
        let other = head("HDMI-A-1", "S", "TV");
        let plans = onto(&[before.clone()], &back, &[now.clone(), other]).unwrap();
        assert!(matches!(plans[1], HeadPlan::Enable { scale: None, .. }));
        assert!(onto(&[before], &[HeadPlan::Disable], &[now]).is_none());
    }

    #[test]
    fn nothing_matching_sends_nothing() {
        let (e, rec, said) = engine(vec![profile("big", 2.0)]);
        let mut other = head("HDMI-A-1", "S", "TV");
        other.scale = 1.0;
        e.event(wayland::Event::Heads {
            serial: 1,
            heads: vec![other],
        });
        assert!(rec.sent.borrow().is_empty() && said.borrow().is_empty());
    }
}
