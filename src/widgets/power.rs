//! The panel's power section: the battery, its limits, and the power
//! profile, from `services::battery` (UPower's change signals) and
//! `services::power` (the profile, read when the section opens). Nothing
//! here runs on a timer.
//!
//! `SWAYPPLET_PREVIEW_POWER=charging|low|saver|degraded` draws a fixture
//! instead of this machine, for the render harness: it reads nothing and
//! can change nothing.

use std::cell::Cell;
use std::rc::Rc;

use gtk4::prelude::*;

use crate::services::power::{
    self, BatteryState, ChargeState, Profile, battery_icon, battery_summary_text,
};
use crate::ui;

/// The widgets that follow the battery.
struct BatteryView {
    /// "74 % · 5h 10m remaining", over the bar: the section's summary is
    /// hidden in page mode, and the bar alone says no number.
    charge: gtk4::Label,
    level: gtk4::ProgressBar,
    draw: ui::Row,
    health: ui::Row,
    limit: ui::Row,
}

impl BatteryView {
    fn apply(&self, bat: &BatteryState) {
        self.charge.set_label(&battery_summary_text(bat));
        self.level.set_fraction(f64::from(bat.capacity) / 100.0);
        ui::set_progress_status(&self.level, charge_status(bat));
        match power::watts_text(bat) {
            Some(w) => {
                self.draw.title.set_label(&w);
                set_subtitle(&self.draw, match bat.state {
                    ChargeState::Charging => "Charging at",
                    _ => "Drawing now",
                });
                self.draw.root.set_visible(true);
            }
            None => self.draw.root.set_visible(false),
        }
        match power::health_text(bat) {
            Some(h) => {
                self.health.title.set_label(&h);
                self.health.root.set_visible(true);
            }
            None => self.health.root.set_visible(false),
        }
        self.limit.title.set_label(
            &power::charge_limit_text(bat).unwrap_or_else(|| "Charges to full".to_string()),
        );
    }
}

/// The profile row and, when the daemon lets it be set, the choice.
struct ProfileView {
    row: ui::Row,
    choices: gtk4::Box,
    busy: Cell<bool>,
}

pub struct PowerSection {
    section: ui::Section,
    profile: Rc<ProfileView>,
    fixture: Option<Fixture>,
}

/// A state to draw instead of the machine's, for the harness.
#[derive(Clone, Copy)]
enum Fixture {
    Charging,
    Low,
    Saver,
    Degraded,
}

impl Fixture {
    fn from_env() -> Option<Fixture> {
        match std::env::var("SWAYPPLET_PREVIEW_POWER").ok()?.as_str() {
            "charging" => Some(Fixture::Charging),
            "low" => Some(Fixture::Low),
            "saver" => Some(Fixture::Saver),
            "degraded" => Some(Fixture::Degraded),
            _ => None,
        }
    }

    fn battery(self) -> BatteryState {
        let (capacity, state, to_empty, to_full, watts) = match self {
            Fixture::Charging => (62, ChargeState::Charging, None, Some(48 * 60), 31.0),
            Fixture::Low => (9, ChargeState::Discharging, Some(22 * 60), None, 7.4),
            Fixture::Saver | Fixture::Degraded => {
                (74, ChargeState::Discharging, Some(5 * 3600 + 10 * 60), None, 5.2)
            }
        };
        BatteryState::fixture(capacity, state, watts, to_empty, to_full)
    }

    fn profile(self) -> Profile {
        let choices = vec!["power-saver".into(), "balanced".into(), "performance".into()];
        match self {
            Fixture::Saver | Fixture::Low => Profile::Daemon {
                active: "power-saver".into(),
                choices,
                degraded: None,
                holds: vec![],
            },
            Fixture::Degraded => Profile::Daemon {
                active: "performance".into(),
                choices,
                degraded: Some("high-operating-temperature".into()),
                holds: vec![],
            },
            Fixture::Charging => Profile::Firmware {
                platform: Some("balanced".into()),
                epp: Some("balance_performance".into()),
                governor: Some("powersave".into()),
                owner: Some("auto-cpufreq"),
            },
        }
    }
}

impl PowerSection {
    pub fn new() -> Self {
        let fixture = Fixture::from_env();
        let has_battery = fixture.is_some() || crate::services::battery::start();
        let bat = fixture.map(Fixture::battery).or_else(crate::services::battery::current);

        let (icon, summary) = match &bat {
            Some(b) => (battery_icon(b.capacity, b.charging).to_owned(), battery_summary_text(b)),
            None => ("󰻠".to_owned(), "Power".to_owned()),
        };
        let section = ui::section(&icon, "Power", &summary);
        ui::glyph::adopt(&section.icon, ui::Text::Title, ui::Tone::Fg);
        let body = ui::vbox(2);

        let battery = has_battery.then(|| {
            let charge = ui::text("", ui::Text::Body, ui::Tone::Fg);
            charge.set_xalign(0.0);
            ui::set_weight(&charge, ui::Weight::Strong);
            body.append(&charge);
            let level = ui::progress(0.0);
            body.append(&level);
            let rows = ui::list();
            let draw = ui::row("󱐋", "", "");
            let health = ui::row("󰁹", "", "Of the capacity it was built with");
            let limit = ui::row("󰂄", "", "Set in firmware; changing it needs root");
            for r in [&draw, &health, &limit] {
                rows.append(&r.root);
            }
            body.append(&rows);
            Rc::new(BatteryView {
                charge,
                level,
                draw,
                health,
                limit,
            })
        });

        // The choice sits under the row, not beside it: beside it, the row's
        // reason ("Performance held back: …") was the part that got cut.
        let profile_row = ui::row("󰓅", "…", "");
        let choices = ui::pill_group(1);
        choices.set_halign(gtk4::Align::Start);
        body.append(&profile_row.root);
        body.append(&choices);
        let profile = Rc::new(ProfileView {
            row: profile_row,
            choices,
            busy: Cell::new(false),
        });

        section.body.append(&body);

        if let (Some(view), Some(b)) = (&battery, &bat) {
            view.apply(b);
        }
        // The battery follows UPower; the fixture stands still.
        if fixture.is_none() && battery.is_some() {
            let view = battery.clone().expect("checked");
            let icon = section.icon.clone();
            let summary = section.summary.clone();
            crate::services::battery::observe(move || {
                if let Some(b) = crate::services::battery::current() {
                    view.apply(&b);
                    icon.set_label(battery_icon(b.capacity, b.charging));
                    summary.set_label(&battery_summary_text(&b));
                }
            });
        }

        let me = Self {
            section,
            profile,
            fixture,
        };
        me.refresh();
        me
    }

    /// Read the profile again: when the section opens (the panel calls this
    /// after its enter transition), and after a change made here.
    pub fn refresh(&self) {
        if let Some(f) = self.fixture {
            show_profile(&self.profile, &f.profile(), true);
            return;
        }
        load_profile(self.profile.clone());
    }

    /// Switch into page mode: reveal detail immediately, hide the summary
    /// toggle row.
    pub fn expand_for_page(&self) {
        self.section.show_as_page();
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.section.root
    }
}

fn load_profile(view: Rc<ProfileView>) {
    crate::spawn::spawn_work(power::read_profile, move |p| show_profile(&view, &p, false));
}

/// Draw `p`. The choice exists only when the daemon owns the profile; a
/// fixture's choice is drawn but inert.
fn show_profile(view: &Rc<ProfileView>, p: &Profile, inert: bool) {
    view.row.title.set_label(&p.name());
    set_subtitle(&view.row, &p.detail());
    ui::set_status(&view.row.subtitle, match p {
        Profile::Daemon { degraded: Some(_), .. } => ui::Status::Warning,
        _ => ui::Status::Neutral,
    });
    while let Some(c) = view.choices.first_child() {
        view.choices.remove(&c);
    }
    let Profile::Daemon { active, choices, .. } = p else {
        view.choices.set_visible(false);
        return;
    };
    view.choices.set_visible(true);
    let mut group: Option<gtk4::ToggleButton> = None;
    for choice in choices {
        let label = match choice.as_str() {
            "power-saver" => "Saver",
            "balanced" => "Balanced",
            "performance" => "Performance",
            other => other,
        };
        let b = ui::toggle_button(ui::Face::Label(label), ui::Kind::Flat, ui::Size::Small);
        b.set_active(choice == active);
        if let Some(g) = &group {
            b.set_group(Some(g));
        } else {
            group = Some(b.clone());
        }
        if !inert {
            let view = view.clone();
            let choice = choice.clone();
            b.connect_toggled(move |b| {
                if !b.is_active() || view.busy.replace(true) {
                    return;
                }
                let view = view.clone();
                let choice = choice.clone();
                let asked = choice.clone();
                crate::spawn::spawn_work(
                    move || power::set_profile(&asked),
                    move |ok| {
                        if !ok {
                            log::warn!("power: the daemon refused {choice}");
                        }
                        view.busy.set(false);
                        load_profile(view);
                    },
                );
            });
        }
        view.choices.append(&b);
    }
}

/// A row's subtitle, shown: a row built with an empty one hides it.
fn set_subtitle(row: &ui::Row, text: &str) {
    row.subtitle.set_label(text);
    row.subtitle.set_visible(!text.is_empty());
}

/// The level bar's status: charging is healthy, under 20 % is not, and
/// anything between is just a level.
fn charge_status(bat: &BatteryState) -> Option<ui::Status> {
    if bat.charging {
        Some(ui::Status::Success)
    } else if bat.capacity < 20 {
        Some(ui::Status::Danger)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Standalone helpers
// ---------------------------------------------------------------------------

/// Walk up the widget hierarchy to find the containing `gtk4::Window` and
/// hide it. Used by Lock and Suspend to close the panel before acting.
pub(crate) fn hide_panel_for_widget(widget: &gtk4::Widget) {
    if let Some(root) = widget.root()
        && let Ok(window) = root.downcast::<gtk4::Window>() {
            window.set_visible(false);
        }
}

// ---------------------------------------------------------------------------
// Session rail — icon-only Lock / Suspend / Logout / Reboot / Shutdown
// ---------------------------------------------------------------------------

/// Horizontal flight deck session cluster: Lock, Suspend, Logout, Reboot, Shutdown.
pub fn build_session_row() -> gtk4::Box {
    let row = ui::hbox(3);
    row.add_css_class("deck-session");

    let lock = rail_btn("󰌾", "Lock", false);
    lock.connect_clicked(|b| {
        hide_panel_for_widget(b.upcast_ref());
        Session::Lock.run();
    });
    row.append(&lock);

    let suspend = rail_btn("󰤄", "Suspend", false);
    suspend.connect_clicked(|b| {
        hide_panel_for_widget(b.upcast_ref());
        Session::Suspend.run();
    });
    row.append(&suspend);

    let logout = rail_btn("󰍃", "Logout", false);
    logout.connect_clicked(|_| Session::Logout.run());
    row.append(&logout);

    let reboot = rail_btn("󰜉", "Reboot", true);
    wire_confirm(&reboot, "Reboot", || Session::Reboot.run());
    row.append(&reboot);

    let shutdown = rail_btn("󰐥", "Shutdown", true);
    wire_confirm(&shutdown, "Shutdown", || Session::Poweroff.run());
    row.append(&shutdown);

    row
}

/// One icon-only button on the Helm's action deck: the same glyph button
/// as the deck's own (panel.rs), the glyph in the danger tone for the
/// destructive actions.
fn rail_btn(icon: &str, tooltip: &str, danger: bool) -> gtk4::Button {
    let lbl = gtk4::Label::new(Some(icon));
    ui::glyph::adopt(
        &lbl,
        ui::Text::Title,
        if danger {
            ui::Tone::Danger
        } else {
            ui::Tone::Fg
        },
    );
    let btn = ui::button_with(
        ui::Face::Icon {
            child: lbl.upcast_ref(),
            tooltip,
        },
        ui::Kind::Secondary,
        ui::Size::Normal,
    );
    btn.add_css_class("deck-btn");
    btn
}

/// Arm a two-click confirmation on `btn`: the first click starts a 3 s window
/// (the armed pulse + "Click again…" tooltip); a second click inside the
/// window runs `exec`. The window auto-clears after 3 s.
fn wire_confirm<F: Fn() + 'static>(btn: &gtk4::Button, verb: &'static str, exec: F) {
    let pending = Rc::new(Cell::new(false));
    btn.connect_clicked(move |b| {
        if pending.get() {
            exec();
            return;
        }
        pending.set(true);
        ui::set_armed(b, true);
        b.set_tooltip_text(Some(&format!("Click again to {}", verb.to_lowercase())));

        let pending_c = pending.clone();
        let b_c = b.clone();
        glib::timeout_add_seconds_local(3, move || {
            pending_c.set(false);
            ui::set_armed(&b_c, false);
            b_c.set_tooltip_text(Some(verb));
            glib::ControlFlow::Break
        });
    });
}

/// What the session rail does, and the Helm's typed rows the same way
/// (`panel.rs`): one command each, whichever door it came through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Session {
    Lock,
    Suspend,
    Logout,
    Reboot,
    Poweroff,
}

impl Session {
    pub(crate) fn run(self) {
        // The render harness (dev/render.sh) sets this: its nested session
        // runs the real binary as the real user, and a typed "restart" there
        // must not restart the machine it runs on.
        if std::env::var_os("SWAYPPLET_DRY_SESSION").is_some() {
            log::info!("session: {self:?} (dry run)");
            return;
        }
        match self {
            Session::Lock => spawn_session_cmd("loginctl", &["lock-session"]),
            Session::Suspend => spawn_session_cmd("systemctl", &["suspend"]),
            Session::Logout => spawn_session_cmd("swaymsg", &["exit"]),
            Session::Reboot => spawn_session_cmd("systemctl", &["reboot"]),
            Session::Poweroff => spawn_session_cmd("systemctl", &["poweroff"]),
        }
    }
}

fn spawn_session_cmd(cmd: &str, args: &[&str]) {
    if let Err(e) = std::process::Command::new(cmd).args(args).spawn() {
        log::error!("Failed to spawn {} {:?}: {}", cmd, args, e);
    }
}
