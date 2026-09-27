//! The Displays tab: arrange the outputs, set each one's mode, scale,
//! rotation and adaptive sync, apply with a way back, and keep the result
//! as a profile.
//!
//! The outputs come from `services::displays`, which already follows them
//! over `zwlr_output_manager_v1`; this tab edits a draft of them
//! (`arrange::Draft`) and sends the draft through the same service, which
//! tests every configuration before it applies it. An applied layout is on
//! trial for `keep::SECONDS` (`keep.rs`): Keep, or it goes back, and it goes
//! back at once when the page is closed, so a layout that darkens the only
//! screen undoes itself.
//!
//! Event-driven: the service says when the outputs change, and a tab that
//! is not on screen only notes that it is stale and reads them when it is
//! shown again. The canvas is widgets on a `gtk4::Fixed` (a toggle button
//! per output, so selection, focus and the arrow keys are GTK's), not
//! drawing.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4::prelude::*;

use super::arrange::{self, Draft, Fit, Rect};
use super::form::{self, section_box, switch_row};
use super::keep::{Effect, End, Keep, Why};
use crate::services::displays::{self, HeadPlan, HeadState, Outcome};
use crate::ui::{self, Kind, Text, Tone};

/// How close, in canvas pixels, an edge has to come to another to snap.
const SNAP: usize = 4;
/// Logical pixels an arrow key moves an output; with Shift, ten times as far.
const STEP: i32 = 10;

/// What a revert sends: the heads as they were, and the plans that put
/// them back.
type Before = (Vec<HeadState>, Vec<HeadPlan>);

/// A drag in progress: the output, where it started, and the canvas fit
/// frozen at its start (refitting under the pointer would make it swim).
#[derive(Clone, Copy)]
struct Drag {
    index: usize,
    from: (i32, i32),
    fit: Fit,
    moved: bool,
}

struct Details {
    group: gtk4::Box,
    title: gtk4::Label,
    enabled: gtk4::Switch,
    resolution: gtk4::DropDown,
    refresh: gtk4::DropDown,
    scale: gtk4::DropDown,
    rotation: gtk4::DropDown,
    sync_row: gtk4::Box,
    sync: gtk4::Switch,
    /// What the dropdowns' rows stand for, row by row.
    resolutions: RefCell<Vec<(u32, u32)>>,
    refreshes: RefCell<Vec<u32>>,
    scales: RefCell<Vec<f64>>,
}

struct Pane {
    /// Apply and Undo, with their status line; shown only while there is
    /// something to apply.
    footer: gtk4::Box,
    root: gtk4::Box,
    /// Everything that edits; insensitive while a layout is on trial.
    body: gtk4::Box,
    unavailable: gtk4::Label,
    area: gtk4::DrawingArea,
    fixed: gtk4::Fixed,
    /// One per enabled output: the draft's index and its button.
    tiles: RefCell<Vec<(usize, gtk4::ToggleButton)>>,
    chips_box: gtk4::Box,
    chips: RefCell<Vec<gtk4::ToggleButton>>,
    details: Details,
    apply: gtk4::Button,
    undo: gtk4::Button,
    status: gtk4::Label,
    ask: gtk4::Box,
    ask_text: gtk4::Label,
    profiles: gtk4::Box,
    profile_list: gtk4::Box,

    heads: RefCell<Vec<HeadState>>,
    identities: RefCell<Vec<String>>,
    drafts: RefCell<Vec<Draft>>,
    selected: Cell<usize>,
    /// The draft differs from the outputs by an edit here.
    dirty: Cell<bool>,
    /// Widgets are being set from the draft: their handlers stand down.
    updating: Cell<bool>,
    /// The outputs changed while the tab was not on screen.
    stale: Cell<bool>,
    drag: Cell<Option<Drag>>,
    keep: RefCell<Keep<Before>>,
    ticker: RefCell<Option<glib::SourceId>>,
    /// Itself, for the handlers made after it was built.
    me: RefCell<std::rc::Weak<Pane>>,
}

pub struct DisplaysPane {
    pane: Rc<Pane>,
}

impl DisplaysPane {
    pub fn new() -> Self {
        let root = form::pane();

        // ── Keep this layout? ────────────────────────────────────────────
        let ask = ui::group(2);
        ask.add_css_class("settings-group");
        let ask_title = ui::text("Keep this layout?", Text::TitleSm, Tone::Fg);
        ask_title.set_halign(gtk4::Align::Start);
        let ask_text = ui::text("", Text::Caption, Tone::Muted);
        ask_text.set_halign(gtk4::Align::Start);
        ui::set_numeric(&ask_text, true);
        let keep_btn = ui::button("Keep", Kind::Primary);
        let revert_btn = ui::button("Revert", Kind::Secondary);
        let ask_row = ui::hbox(3);
        ask_row.append(&keep_btn);
        ask_row.append(&revert_btn);
        ask.append(&ask_title);
        ask.append(&ask_text);
        ask.append(&ask_row);
        ask.set_visible(false);

        let unavailable = ui::text(
            "This compositor does not offer output management, so the displays cannot be arranged here.",
            Text::Caption,
            Tone::Muted,
        );
        unavailable.set_wrap(true);
        unavailable.set_max_width_chars(form::HINT_CHARS);
        unavailable.set_visible(false);

        let body = ui::vbox(4);

        // ── Arrangement ──────────────────────────────────────────────────
        let arrangement = section_box(
            "Arrangement",
            "Drag a display to move it; it lands against the nearest edge. Arrow keys move the selected one, Shift for larger steps.",
        );
        let canvas = ui::well();
        canvas.add_css_class("displays-canvas");
        let area = gtk4::DrawingArea::new();
        area.set_hexpand(true);
        area.set_vexpand(true);
        let fixed = gtk4::Fixed::new();
        let overlay = gtk4::Overlay::new();
        overlay.set_child(Some(&area));
        overlay.add_overlay(&fixed);
        overlay.set_vexpand(true);
        canvas.append(&overlay);
        arrangement.append(&canvas);
        let chips_box = ui::hbox(2);
        chips_box.add_css_class("displays-chips");
        arrangement.append(&chips_box);

        // ── The selected output ──────────────────────────────────────────
        let group = ui::group(1);
        group.set_hexpand(true);
        group.add_css_class("settings-group");
        let title = ui::overline("", Tone::Muted);
        group.append(&title);
        let (enabled_row, enabled) = switch_row(
            "Enabled",
            "Off, the display goes dark and its workspaces move to the others.",
            true,
        );
        let (res_row, resolution) =
            form::dropdown_row("Resolution", "The modes the display offers.", &[]);
        let (rate_row, refresh) = form::dropdown_row("Refresh rate", "At this resolution.", &[]);
        let (scale_row, scale) = form::dropdown_row(
            "Scale",
            "How large everything is drawn. The suggested one gives about 110 pixels to the logical inch.",
            &[],
        );
        let names: Vec<&str> = arrange::TRANSFORMS.iter().map(|(_, n)| *n).collect();
        let (rot_row, rotation) =
            form::dropdown_row("Rotation", "As the display stands on the desk.", &names);
        let (sync_row, sync) = switch_row(
            "Adaptive sync",
            "Let the refresh follow the frames (FreeSync, VRR). Off when games or video stutter or flicker.",
            false,
        );
        for row in [
            &enabled_row,
            &res_row,
            &rate_row,
            &scale_row,
            &rot_row,
            &sync_row,
        ] {
            group.append(row);
        }

        // ── Apply ────────────────────────────────────────────────────────
        let apply = ui::button("Apply", Kind::Primary);
        let undo = form::action_button("Undo changes", "Back to the layout on screen.");
        let (footer, status) = form::footer(&[&apply, &undo]);

        // ── Profiles ─────────────────────────────────────────────────────
        let profiles = section_box(
            "Profiles",
            "A profile is applied when its displays connect; the most specific wins. Save stores the layout on screen, not unapplied changes.",
        );
        let profile_list = ui::vbox(1);
        profiles.append(&profile_list);
        profiles.append(&crate::widgets::display::save_row("Save as profile"));

        body.append(&arrangement);
        body.append(&group);
        body.append(&footer);
        body.append(&profiles);
        root.append(&ask);
        root.append(&unavailable);
        root.append(&body);

        let pane = Rc::new(Pane {
            root,
            body,
            footer: footer.clone(),
            unavailable,
            area,
            fixed,
            tiles: RefCell::default(),
            chips_box,
            chips: RefCell::default(),
            details: Details {
                group,
                title,
                enabled,
                resolution,
                refresh,
                scale,
                rotation,
                sync_row,
                sync,
                resolutions: RefCell::default(),
                refreshes: RefCell::default(),
                scales: RefCell::default(),
            },
            apply,
            undo,
            status,
            ask,
            ask_text,
            profiles,
            profile_list,
            heads: RefCell::default(),
            identities: RefCell::default(),
            drafts: RefCell::default(),
            selected: Cell::new(0),
            dirty: Cell::new(false),
            updating: Cell::new(false),
            stale: Cell::new(true),
            drag: Cell::new(None),
            keep: RefCell::new(Keep::default()),
            ticker: RefCell::new(None),
            me: RefCell::default(),
        });
        *pane.me.borrow_mut() = Rc::downgrade(&pane);
        Pane::connect(&pane, &keep_btn, &revert_btn);
        DisplaysPane { pane }
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.pane.root
    }

    /// For the render harness (`preview.rs`): move the last output below
    /// the first, as a drag would, and apply it, so the question and the
    /// revert can be seen against a nested compositor.
    pub fn demo_apply(&self) {
        let p = &self.pane;
        p.read_service();
        let n = p.drafts.borrow().len();
        if n < 2 {
            return;
        }
        let (rects, k) = p.rects(n - 1);
        let below = (rects[0].x, rects[0].y + rects[0].h);
        let to = arrange::settle(&rects, k, below, 0);
        {
            let mut drafts = p.drafts.borrow_mut();
            drafts[n - 1].position = to;
            arrange::normalize(&mut drafts);
        }
        p.relayout();
        p.edited();
        p.send();
    }

    /// The panel opened: read the outputs when the tab is next on screen.
    pub fn refresh(&self) {
        if self.pane.root.is_mapped() {
            self.pane.read_service();
        } else {
            self.pane.stale.set(true);
        }
    }
}

impl Pane {
    fn rc(&self) -> Rc<Pane> {
        self.me
            .borrow()
            .upgrade()
            .expect("the pane is built as an Rc")
    }

    fn connect(p: &Rc<Pane>, keep_btn: &gtk4::Button, revert_btn: &gtk4::Button) {
        {
            let p = p.clone();
            displays::observe(move || {
                if p.root.is_mapped() {
                    p.read_service();
                } else {
                    p.stale.set(true);
                }
            });
        }
        {
            let q = p.clone();
            p.root.connect_map(move |_| {
                if q.stale.get() {
                    q.read_service();
                }
            });
        }
        {
            // The page went away (the panel closed, another tab or page):
            // a layout still on trial goes back.
            let q = p.clone();
            p.root.connect_unmap(move |_| {
                q.drag.set(None);
                let e = q.keep.borrow_mut().left();
                q.run(e);
            });
        }
        {
            let q = p.clone();
            p.area.connect_resize(move |_, _, _| q.relayout());
        }
        {
            let q = p.clone();
            keep_btn.connect_clicked(move |_| {
                let e = q.keep.borrow_mut().keep();
                q.run(e);
            });
        }
        {
            let q = p.clone();
            revert_btn.connect_clicked(move |_| {
                let e = q.keep.borrow_mut().undo();
                q.run(e);
            });
        }
        {
            let q = p.clone();
            p.apply.connect_clicked(move |_| q.send());
        }
        {
            let q = p.clone();
            p.undo.connect_clicked(move |_| {
                q.reset();
                q.say("", false);
            });
        }
        Pane::connect_details(p);
    }

    fn connect_details(p: &Rc<Pane>) {
        let d = &p.details;
        {
            let q = p.clone();
            d.enabled.connect_active_notify(move |s| {
                if q.updating.get() {
                    return;
                }
                let on = s.is_active();
                let i = q.selected.get();
                let refused = {
                    let mut drafts = q.drafts.borrow_mut();
                    let others_on = drafts.iter().enumerate().any(|(j, d)| j != i && d.enabled);
                    if !on && !others_on {
                        true
                    } else {
                        drafts[i].enabled = on;
                        if on {
                            if drafts[i].mode.is_none() {
                                drafts[i].mode = arrange::best_mode(&drafts[i].modes);
                            }
                            arrange::place_new(&mut drafts, i);
                        }
                        arrange::close_gaps(&mut drafts);
                        false
                    }
                };
                if refused {
                    q.updating.set(true);
                    s.set_active(true);
                    q.updating.set(false);
                    q.say(&arrange::Problem::NoneEnabled.say(), true);
                    return;
                }
                q.rebuild();
                q.edited();
            });
        }
        {
            let q = p.clone();
            d.resolution.connect_selected_notify(move |dd| {
                if q.updating.get() {
                    return;
                }
                let Some(size) = q
                    .details
                    .resolutions
                    .borrow()
                    .get(dd.selected() as usize)
                    .copied()
                else {
                    return;
                };
                q.resize(|d| {
                    let had = d.mode.map_or(0, |m| m.2);
                    d.mode = Some(arrange::mode_at(&d.modes, d.mode, size, had));
                });
            });
        }
        {
            let q = p.clone();
            d.refresh.connect_selected_notify(move |dd| {
                if q.updating.get() {
                    return;
                }
                let Some(rate) = q
                    .details
                    .refreshes
                    .borrow()
                    .get(dd.selected() as usize)
                    .copied()
                else {
                    return;
                };
                q.resize(|d| {
                    if let Some(m) = &mut d.mode {
                        m.2 = rate;
                    }
                });
            });
        }
        {
            let q = p.clone();
            d.scale.connect_selected_notify(move |dd| {
                if q.updating.get() {
                    return;
                }
                let Some(s) = q
                    .details
                    .scales
                    .borrow()
                    .get(dd.selected() as usize)
                    .copied()
                else {
                    return;
                };
                q.resize(|d| d.scale = s);
            });
        }
        {
            let q = p.clone();
            d.rotation.connect_selected_notify(move |dd| {
                if q.updating.get() {
                    return;
                }
                let Some((t, _)) = arrange::TRANSFORMS.get(dd.selected() as usize).copied() else {
                    return;
                };
                q.resize(|d| d.transform = t);
            });
        }
        {
            let q = p.clone();
            d.sync.connect_active_notify(move |s| {
                if q.updating.get() {
                    return;
                }
                let i = q.selected.get();
                q.drafts.borrow_mut()[i].adaptive_sync = Some(s.is_active());
                q.edited();
            });
        }
    }

    // ── From the service ─────────────────────────────────────────────────

    /// Read the outputs. A new set of outputs replaces the draft; the same
    /// set keeps an unapplied edit.
    fn read_service(&self) {
        self.stale.set(false);
        let view = displays::view();
        self.unavailable.set_visible(!view.available);
        self.body.set_visible(view.available);
        self.profiles.set_visible(view.available);
        crate::widgets::display::fill_profiles(&self.profile_list);
        // Left to right, then top to bottom, the ones switched off last: the
        // order the chips read in. The compositor lists them newest first.
        let mut heads = view.heads;
        heads.sort_by(|a, b| {
            (!a.enabled, a.position.0, a.position.1, &a.name).cmp(&(
                !b.enabled,
                b.position.0,
                b.position.1,
                &b.name,
            ))
        });
        let mut ids: Vec<String> = heads.iter().map(HeadState::identity).collect();
        ids.sort();
        let replugged = *self.identities.borrow() != ids;
        *self.identities.borrow_mut() = ids;
        *self.heads.borrow_mut() = heads;
        if replugged {
            let e = self.keep.borrow_mut().replugged();
            self.run(e);
        }
        if replugged || (!self.dirty.get() && self.keep.borrow().idle()) {
            self.reset();
        }
    }

    /// The draft back to the outputs on screen.
    fn reset(&self) {
        *self.drafts.borrow_mut() = arrange::drafts(&self.heads.borrow());
        self.dirty.set(false);
        let n = self.drafts.borrow().len();
        if self.selected.get() >= n {
            self.selected.set(0);
        }
        self.rebuild();
        self.update_actions();
    }

    // ── Editing ──────────────────────────────────────────────────────────

    /// Change the selected output's size (mode, scale, rotation), keeping
    /// its neighbours against it.
    fn resize(&self, change: impl FnOnce(&mut Draft)) {
        let i = self.selected.get();
        {
            let mut drafts = self.drafts.borrow_mut();
            let old = drafts[i].rect();
            change(&mut drafts[i]);
            if drafts[i].enabled {
                arrange::follow_resize(&mut drafts, i, old);
            }
        }
        self.show_details();
        self.relayout();
        self.edited();
    }

    fn edited(&self) {
        self.dirty.set(true);
        self.update_actions();
    }

    fn update_actions(&self) {
        let idle = self.keep.borrow().idle();
        self.body.set_sensitive(idle);
        let problem = arrange::check(&self.drafts.borrow()).err();
        self.apply
            .set_sensitive(idle && self.dirty.get() && problem.is_none());
        self.undo.set_sensitive(idle && self.dirty.get());
        // Nothing to apply: no row of disabled buttons floating under the
        // group. It comes back with the first change.
        self.footer.set_visible(self.dirty.get() || !idle);
        if idle && self.dirty.get() {
            match problem {
                Some(p) => self.say(&p.say(), true),
                None => self.say("Not applied yet.", false),
            }
        }
    }

    fn say(&self, text: &str, bad: bool) {
        self.status.set_text(text);
        ui::set_tone(&self.status, if bad { Tone::Danger } else { Tone::Muted });
    }

    fn select(&self, i: usize) {
        if self.updating.get() {
            return;
        }
        self.selected.set(i);
        self.updating.set(true);
        for (j, tile) in self.tiles.borrow().iter() {
            if *j == i {
                tile.set_active(true);
            }
        }
        if let Some(chip) = self.chips.borrow().get(i) {
            chip.set_active(true);
        }
        self.updating.set(false);
        self.show_details();
    }

    // ── Apply, keep, revert ──────────────────────────────────────────────

    fn send(&self) {
        let drafts = self.drafts.borrow().clone();
        if arrange::check(&drafts).is_err() {
            return;
        }
        let heads = self.heads.borrow().clone();
        let before = arrange::plans(&arrange::drafts(&heads));
        if !self.keep.borrow_mut().sent((heads.clone(), before)) {
            return;
        }
        self.update_actions();
        self.say("Trying the layout…", false);
        let q = self.rc();
        displays::configure(&heads, arrange::plans(&drafts), move |o| q.answered(o));
    }

    fn answered(&self, outcome: Outcome) {
        let e = self.keep.borrow_mut().answered(outcome);
        self.run(e);
    }

    /// Do what the keep-or-revert machine asks.
    fn run(&self, effect: Effect<Before>) {
        match effect {
            Effect::Nothing => {}
            Effect::Ask(left) => {
                self.ask.set_visible(true);
                self.ask_text
                    .set_text(&format!("The previous layout comes back in {left} s."));
                self.update_actions();
                self.say("", false);
                self.start_ticker();
            }
            Effect::Revert((heads, plans)) => {
                self.stop_ticker();
                self.ask.set_visible(false);
                self.say("Putting the previous layout back…", false);
                let q = self.rc();
                displays::configure(&heads, plans, move |o| q.answered(o));
            }
            Effect::Done(end) => {
                self.stop_ticker();
                self.ask.set_visible(false);
                self.dirty.set(false);
                self.reset();
                let (text, bad) = match end {
                    End::Kept => ("Layout kept.", false),
                    End::Refused => ("The compositor refused this layout. Nothing changed.", true),
                    End::Stale => (
                        "The displays changed while the layout was on its way. Nothing changed.",
                        true,
                    ),
                    End::Reverted(Why::Asked) => ("The previous layout is back.", false),
                    End::Reverted(Why::TimedOut) => {
                        ("Not kept in time: the previous layout is back.", false)
                    }
                    End::Reverted(Why::Left) => (
                        "The page closed before Keep: the previous layout is back.",
                        false,
                    ),
                    End::RevertFailed => ("The previous layout could not be put back.", true),
                    End::Replugged => (
                        "A display was plugged in or out; the profiles decide the layout now.",
                        false,
                    ),
                };
                self.say(text, bad);
            }
        }
    }

    /// Tick once a second while the question is asked.
    fn start_ticker(&self) {
        if self.ticker.borrow().is_some() {
            return;
        }
        let q = self.rc();
        let id = glib::timeout_add_seconds_local(1, move || {
            let e = q.keep.borrow_mut().tick();
            let go_on = matches!(e, Effect::Ask(_));
            if !go_on {
                // This source ends by returning Break; nobody removes it.
                q.ticker.borrow_mut().take();
            }
            q.run(e);
            if go_on {
                glib::ControlFlow::Continue
            } else {
                glib::ControlFlow::Break
            }
        });
        *self.ticker.borrow_mut() = Some(id);
    }

    fn stop_ticker(&self) {
        if let Some(id) = self.ticker.borrow_mut().take() {
            id.remove();
        }
    }

    // ── The canvas and the rows ──────────────────────────────────────────

    /// Build the tiles and the chips again: the outputs, or which of them
    /// are on, changed.
    fn rebuild(&self) {
        let me = self.rc();
        let refocus = self.tiles.borrow().iter().any(|(_, t)| t.has_focus());
        for (_, t) in self.tiles.borrow_mut().drain(..) {
            self.fixed.remove(&t);
        }
        for c in self.chips.borrow_mut().drain(..) {
            self.chips_box.remove(&c);
        }
        let drafts = self.drafts.borrow().clone();
        let mut sel = self.selected.get();
        if drafts.get(sel).is_none() {
            sel = 0;
        }
        let mut first_tile: Option<gtk4::ToggleButton> = None;
        let mut first_chip: Option<gtk4::ToggleButton> = None;
        for (i, d) in drafts.iter().enumerate() {
            let chip_label = if d.enabled {
                d.name.clone()
            } else {
                format!("{} (off)", d.name)
            };
            let chip = ui::toggle_chip(&chip_label);
            match &first_chip {
                Some(f) => chip.set_group(Some(f)),
                None => first_chip = Some(chip.clone()),
            }
            {
                let q = me.clone();
                chip.connect_toggled(move |c| {
                    if c.is_active() {
                        q.select(i);
                    }
                });
            }
            self.chips_box.append(&chip);
            self.chips.borrow_mut().push(chip);
            if !d.enabled {
                continue;
            }
            let tile = me.tile(i, d);
            match &first_tile {
                Some(f) => tile.set_group(Some(f)),
                None => first_tile = Some(tile.clone()),
            }
            self.fixed.put(&tile, 0.0, 0.0);
            self.tiles.borrow_mut().push((i, tile));
        }
        self.updating.set(false);
        self.selected.set(usize::MAX);
        self.select(sel);
        self.relayout();
        if refocus && let Some((_, t)) = self.tiles.borrow().iter().find(|(i, _)| *i == sel) {
            t.grab_focus();
        }
    }

    /// One output on the canvas: its connector and product, draggable, and
    /// moved by the arrow keys when it has the focus.
    fn tile(self: &Rc<Pane>, i: usize, d: &Draft) -> gtk4::ToggleButton {
        let text = if d.product.is_empty() {
            d.name.clone()
        } else {
            format!("{}\n{}", d.name, d.product)
        };
        let label = gtk4::Label::new(Some(&text));
        label.set_justify(gtk4::Justification::Center);
        label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        let tile = ui::toggle_button(
            ui::Face::Child(label.upcast_ref()),
            Kind::Secondary,
            ui::Size::Small,
        );
        tile.set_tooltip_text(Some(&text.replace('\n', " · ")));
        {
            let q = self.clone();
            tile.connect_toggled(move |t| {
                if t.is_active() {
                    q.select(i);
                }
            });
        }
        let drag = gtk4::GestureDrag::new();
        drag.set_propagation_phase(gtk4::PropagationPhase::Capture);
        {
            let q = self.clone();
            drag.connect_drag_begin(move |_, _, _| {
                let fit = q.fit();
                let from = q.drafts.borrow()[i].position;
                if let Some(fit) = fit {
                    q.drag.set(Some(Drag {
                        index: i,
                        from,
                        fit,
                        moved: false,
                    }));
                }
            });
        }
        {
            let q = self.clone();
            drag.connect_drag_update(move |g, dx, dy| {
                let Some(mut d) = q.drag.get() else { return };
                if !d.moved {
                    // A press that barely moves is a click: it selects.
                    if dx.hypot(dy) < f64::from(crate::tokens::space(2)) {
                        return;
                    }
                    g.set_state(gtk4::EventSequenceState::Claimed);
                    d.moved = true;
                    q.drag.set(Some(d));
                    q.select(i);
                }
                q.drag_to(d, dx, dy);
            });
        }
        {
            let q = self.clone();
            drag.connect_drag_end(move |_, _, _| {
                let Some(d) = q.drag.take() else { return };
                if d.moved {
                    arrange::normalize(&mut q.drafts.borrow_mut());
                    q.relayout();
                    q.edited();
                }
            });
        }
        tile.add_controller(drag);
        let keys = gtk4::EventControllerKey::new();
        {
            let q = self.clone();
            keys.connect_key_pressed(move |_, key, _, mods| {
                use gtk4::gdk::Key;
                let dir = match key {
                    Key::Left => (-1, 0),
                    Key::Right => (1, 0),
                    Key::Up => (0, -1),
                    Key::Down => (0, 1),
                    _ => return glib::Propagation::Proceed,
                };
                let step = if mods.contains(gtk4::gdk::ModifierType::SHIFT_MASK) {
                    STEP * 10
                } else {
                    STEP
                };
                q.nudge(i, dir, step);
                glib::Propagation::Stop
            });
        }
        tile.add_controller(keys);
        tile
    }

    /// The enabled outputs' rectangles, and where draft `i` is among them.
    fn rects(&self, i: usize) -> (Vec<Rect>, usize) {
        let drafts = self.drafts.borrow();
        let mut at = 0;
        let rects = drafts
            .iter()
            .enumerate()
            .filter(|(_, d)| d.enabled)
            .enumerate()
            .map(|(k, (j, d))| {
                if j == i {
                    at = k;
                }
                d.rect()
            })
            .collect();
        (rects, at)
    }

    fn drag_to(&self, d: Drag, dx: f64, dy: f64) {
        let want = (
            d.from.0 + d.fit.to_layout(dx),
            d.from.1 + d.fit.to_layout(dy),
        );
        let (rects, k) = self.rects(d.index);
        let snap = d.fit.to_layout(f64::from(crate::tokens::space(SNAP)));
        let p = arrange::settle(&rects, k, want, snap);
        self.drafts.borrow_mut()[d.index].position = p;
        self.relayout();
    }

    fn nudge(&self, i: usize, dir: (i32, i32), step: i32) {
        if !self.keep.borrow().idle() {
            return;
        }
        let (rects, k) = self.rects(i);
        let p = arrange::nudge(&rects, k, dir, step);
        if p == (rects[k].x, rects[k].y) {
            return;
        }
        {
            let mut drafts = self.drafts.borrow_mut();
            drafts[i].position = p;
            arrange::normalize(&mut drafts);
        }
        self.relayout();
        self.edited();
    }

    /// How the layout fits the canvas now: frozen during a drag.
    fn fit(&self) -> Option<Fit> {
        if let Some(d) = self.drag.get() {
            return Some(d.fit);
        }
        let (w, h) = (self.area.width(), self.area.height());
        if w <= 0 || h <= 0 {
            return None;
        }
        let rects: Vec<Rect> = self
            .drafts
            .borrow()
            .iter()
            .filter(|d| d.enabled)
            .map(Draft::rect)
            .collect();
        Some(Fit::new(
            &rects,
            f64::from(w),
            f64::from(h),
            f64::from(crate::tokens::space(4)),
        ))
    }

    /// Put every tile where its output is, at its size.
    fn relayout(&self) {
        let Some(fit) = self.fit() else { return };
        // Each tile stands a little inside its rectangle, so two outputs
        // that touch read as two.
        let inset = f64::from(crate::tokens::space(1));
        let drafts = self.drafts.borrow();
        for (i, tile) in self.tiles.borrow().iter() {
            let r = drafts[*i].rect();
            let (x, y) = fit.to_canvas((r.x, r.y));
            let w = (f64::from(r.w) * fit.scale - 2.0 * inset).round().max(1.0) as i32;
            let h = (f64::from(r.h) * fit.scale - 2.0 * inset).round().max(1.0) as i32;
            tile.set_size_request(w, h);
            self.fixed
                .move_(tile, (x + inset).round(), (y + inset).round());
        }
    }

    /// Fill the rows from the selected output.
    fn show_details(&self) {
        let drafts = self.drafts.borrow();
        let Some(d) = drafts.get(self.selected.get()) else {
            self.details.group.set_visible(false);
            return;
        };
        let dt = &self.details;
        dt.group.set_visible(true);
        self.updating.set(true);
        let title = if d.product.is_empty() {
            d.name.clone()
        } else {
            format!("{} · {}", d.name, d.product)
        };
        dt.title.set_text(&title);
        dt.enabled.set_active(d.enabled);

        let sizes = arrange::resolutions(&d.modes, d.mode);
        let size = d.mode.map(|m| (m.0, m.1));
        set_choices(
            &dt.resolution,
            &sizes
                .iter()
                .map(|s| arrange::resolution_label(*s))
                .collect::<Vec<_>>(),
            size.and_then(|s| sizes.iter().position(|x| *x == s)),
        );
        let rates = size
            .map(|s| arrange::refreshes(&d.modes, d.mode, s))
            .unwrap_or_default();
        set_choices(
            &dt.refresh,
            &rates
                .iter()
                .map(|r| arrange::refresh_label(*r))
                .collect::<Vec<_>>(),
            d.mode.and_then(|m| rates.iter().position(|r| *r == m.2)),
        );
        let scales = arrange::scales(d.scale, d.suggested);
        set_choices(
            &dt.scale,
            &scales
                .iter()
                .map(|s| arrange::scale_label(*s, d.suggested))
                .collect::<Vec<_>>(),
            scales.iter().position(|s| (s - d.scale).abs() < 1e-3),
        );
        dt.rotation.set_selected(
            arrange::TRANSFORMS
                .iter()
                .position(|(t, _)| *t == d.transform)
                .unwrap_or(0) as u32,
        );
        dt.sync_row.set_visible(d.adaptive_sync.is_some());
        dt.sync.set_active(d.adaptive_sync.unwrap_or(false));
        for w in [&dt.resolution, &dt.refresh, &dt.scale, &dt.rotation] {
            w.set_sensitive(d.enabled);
        }
        dt.refresh.set_sensitive(d.enabled && rates.len() > 1);
        dt.sync.set_sensitive(d.enabled);
        *dt.resolutions.borrow_mut() = sizes;
        *dt.refreshes.borrow_mut() = rates;
        *dt.scales.borrow_mut() = scales;
        self.updating.set(false);
    }
}

/// Replace a dropdown's rows and pick one.
fn set_choices(d: &gtk4::DropDown, labels: &[String], selected: Option<usize>) {
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    d.set_model(Some(&gtk4::StringList::new(&refs)));
    d.set_selected(selected.map_or(gtk4::INVALID_LIST_POSITION, |i| i as u32));
}
