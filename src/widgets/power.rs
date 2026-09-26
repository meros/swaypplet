use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4::prelude::*;

use crate::services::power::{
    BatteryState, GovernorProfile, battery_icon, battery_summary_text, find_battery_path,
    read_battery, read_governor,
};
use crate::ui;

// ---------------------------------------------------------------------------
// Widget state
// ---------------------------------------------------------------------------

struct PowerState {
    battery: Option<BatteryState>,
    governor: GovernorProfile,
}

impl PowerState {
    fn read(bat_path: Option<&str>) -> Self {
        Self {
            battery: bat_path.and_then(read_battery),
            governor: read_governor(),
        }
    }
}

// ---------------------------------------------------------------------------
// Battery refresh handles — shared between PowerSection and the 30-s timer
// ---------------------------------------------------------------------------

/// Cheap GTK widget handles shared via `Rc` so the periodic timer can push
/// updates without borrowing `PowerSection`.
struct BatteryHandles {
    bat_path: String,
    /// Summary row icon label (always visible).
    summary_icon: gtk4::Label,
    /// Summary row text label (always visible).
    summary_text: gtk4::Label,
    /// Detail: battery level bar (inside revealer).
    health_lbl: gtk4::Label,
    level_bar: gtk4::ProgressBar,
}

impl BatteryHandles {
    fn apply(&self, bat: &BatteryState) {
        // Update summary row.
        self.summary_icon
            .set_label(battery_icon(bat.capacity, bat.charging));
        self.summary_text.set_label(&battery_summary_text(bat));

        // Update detail widgets.
        self.level_bar.set_fraction(bat.capacity as f64 / 100.0);

        if let Some(health) = bat.health_pct {
            self.health_lbl.set_label(&format!("Health: {}%", health));
            self.health_lbl.set_visible(true);
        } else {
            self.health_lbl.set_visible(false);
        }

        ui::set_progress_status(&self.level_bar, charge_status(bat));
    }
}

// ---------------------------------------------------------------------------
// PowerSection
// ---------------------------------------------------------------------------

pub struct PowerSection {
    section: ui::Section,
    /// Cached battery sysfs path (None on desktops without a battery).
    bat_path: Option<String>,
    state: RefCell<PowerState>,

    // Battery widget handles (only present when a battery was found).
    bat_handles: Option<Rc<BatteryHandles>>,

    // Governor info label (inside the section body).
    governor_label: gtk4::Label,
}

impl PowerSection {
    pub fn new() -> Self {
        // ── Discover battery path ────────────────────────────────────────
        let bat_path = find_battery_path();
        if bat_path.is_none() {
            log::info!("No battery found; battery section hidden.");
        }

        // ── Read initial state ────────────────────────────────────────────
        let state = PowerState::read(bat_path.as_deref());

        // Determine initial icon and text for the summary.
        let (initial_icon, initial_text) = if let Some(ref bat) = state.battery {
            (
                battery_icon(bat.capacity, bat.charging).to_owned(),
                battery_summary_text(bat),
            )
        } else {
            ("󰻠".to_owned(), format_governor_info(&state.governor))
        };

        let section = ui::section(&initial_icon, "Power", &initial_text);
        ui::glyph::adopt(&section.icon, ui::Text::Title, ui::Tone::Fg);
        let summary_icon = section.icon.clone();
        let summary_text = section.summary.clone();

        let detail_box = ui::vbox(3);

        // ── Battery detail widgets (conditional) ──────────────────────────
        let bat_handles: Option<Rc<BatteryHandles>> = if let Some(ref bat) = state.battery {
            let bat_detail = ui::vbox(2);
            bat_detail.add_css_class("battery-detail");

            // Health label
            let health_lbl = ui::text("", ui::Text::Caption, ui::Tone::Faint);
            health_lbl.set_visible(false);
            if let Some(health) = bat.health_pct {
                health_lbl.set_label(&format!("Health: {}%", health));
                health_lbl.set_visible(true);
            }

            // Level bar: the accent fill, green while charging and red
            // when low, because those two are status.
            let level_bar = ui::progress(bat.capacity as f64 / 100.0);
            ui::set_progress_status(&level_bar, charge_status(bat));

            bat_detail.append(&level_bar);
            bat_detail.append(&health_lbl);
            detail_box.append(&bat_detail);

            Some(Rc::new(BatteryHandles {
                bat_path: bat_path.as_deref().unwrap_or("").to_owned(),
                summary_icon: summary_icon.clone(),
                summary_text: summary_text.clone(),
                health_lbl,
                level_bar,
            }))
        } else {
            None
        };

        // ── CPU governor info (managed by auto-cpufreq) ─────────────────
        let cpu = ui::row(
            "󰻠",
            &format_governor_info(&state.governor),
            "Managed by auto-cpufreq",
        );
        let governor_label = cpu.title.clone();
        detail_box.append(&cpu.root);

        section.body.append(&detail_box);

        // ── Periodic battery refresh every 30 s ───────────────────────────
        if let Some(ref handles) = bat_handles {
            let handles_weak = Rc::downgrade(handles);
            glib::timeout_add_seconds_local(30, move || {
                let Some(h) = handles_weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                // Only refresh when the widget is visible (mapped to screen).
                // The level bar lives inside the collapsed-by-default detail
                // revealer, which unmaps it; check the always-visible summary
                // icon instead so the badge keeps refreshing while collapsed.
                if !h.summary_icon.is_mapped() {
                    return glib::ControlFlow::Continue;
                }
                if let Some(bat) = read_battery(&h.bat_path) {
                    h.apply(&bat);
                } else {
                    log::error!("Battery info unavailable during periodic refresh.");
                }
                glib::ControlFlow::Continue
            });
        }

        Self {
            section,
            bat_path,
            state: RefCell::new(state),
            bat_handles,
            governor_label,
        }
    }

    /// Re-read sysfs and update all widgets.
    pub fn refresh(&self) {
        let new_state = PowerState::read(self.bat_path.as_deref());

        if let (Some(bat), Some(handles)) = (&new_state.battery, &self.bat_handles) {
            // BatteryHandles::apply updates summary_icon and summary_text as well.
            handles.apply(bat);
        } else if self.bat_handles.is_none() {
            // Desktop without battery: show governor in summary.
            self.section.icon.set_label("󰻠");
            self.section
                .summary
                .set_label(&format_governor_info(&new_state.governor));
        }
        // else: machine has a battery but this read failed transiently —
        // keep the last-known battery display rather than clobbering it.

        self.governor_label
            .set_label(&format_governor_info(&new_state.governor));

        *self.state.borrow_mut() = new_state;
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
    if let Some(root) = widget.root() {
        if let Ok(window) = root.downcast::<gtk4::Window>() {
            window.set_visible(false);
        }
    }
}

fn format_governor_info(gov: &GovernorProfile) -> String {
    match gov {
        GovernorProfile::Performance => "Performance".to_owned(),
        GovernorProfile::Balanced => "Balanced".to_owned(),
        GovernorProfile::Powersave => "Powersave".to_owned(),
        GovernorProfile::Other(s) => s.clone(),
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
        spawn_session_cmd("loginctl", &["lock-session"]);
    });
    row.append(&lock);

    let suspend = rail_btn("󰤄", "Suspend", false);
    suspend.connect_clicked(|b| {
        hide_panel_for_widget(b.upcast_ref());
        spawn_session_cmd("systemctl", &["suspend"]);
    });
    row.append(&suspend);

    let logout = rail_btn("󰍃", "Logout", false);
    logout.connect_clicked(|_| spawn_session_cmd("swaymsg", &["exit"]));
    row.append(&logout);

    let reboot = rail_btn("󰜉", "Reboot", true);
    wire_confirm(&reboot, "Reboot", || {
        spawn_session_cmd("systemctl", &["reboot"])
    });
    row.append(&reboot);

    let shutdown = rail_btn("󰐥", "Shutdown", true);
    wire_confirm(&shutdown, "Shutdown", || {
        spawn_session_cmd("systemctl", &["poweroff"])
    });
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

fn spawn_session_cmd(cmd: &str, args: &[&str]) {
    if let Err(e) = std::process::Command::new(cmd).args(args).spawn() {
        log::error!("Failed to spawn {} {:?}: {}", cmd, args, e);
    }
}
