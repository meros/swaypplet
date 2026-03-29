use std::cell::RefCell;
use std::process::Command;
use std::rc::Rc;
use std::time::Duration;

use gtk4::prelude::*;

// ---------------------------------------------------------------------------
// Path helper
// ---------------------------------------------------------------------------

fn make_screenshot_path(ext: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    let dir = format!("{}/Pictures/Screenshots", home);
    let _ = std::fs::create_dir_all(&dir);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{}/screenshot-{}.{}", dir, ts, ext)
}

// ---------------------------------------------------------------------------
// Panel-hide helper
// ---------------------------------------------------------------------------

fn hide_panel_for_widget(widget: &gtk4::Widget) {
    if let Some(root) = widget.root() {
        if let Ok(window) = root.downcast::<gtk4::Window>() {
            window.set_visible(false);
        }
    }
}

// ---------------------------------------------------------------------------
// ScreenshotSection
// ---------------------------------------------------------------------------

pub struct ScreenshotSection {
    root: gtk4::Box,
    #[allow(dead_code)]
    summary_arrow: gtk4::Label,
    #[allow(dead_code)]
    detail_revealer: gtk4::Revealer,
    recording_pid: Rc<RefCell<Option<u32>>>,
}

impl ScreenshotSection {
    pub fn new() -> Self {
        // ── Root section box ────────────────────────────────────────────────
        let root = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(6)
            .build();
        root.add_css_class("section");

        // ── Summary row (always visible) ──────────────────────────────────
        let summary_content = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(8)
            .valign(gtk4::Align::Center)
            .build();

        let summary_icon = gtk4::Label::builder()
            .label("󰹑")
            .halign(gtk4::Align::Start)
            .build();
        summary_icon.add_css_class("section-summary-icon");

        let summary_text = gtk4::Label::builder()
            .label("Screenshot")
            .halign(gtk4::Align::Start)
            .hexpand(true)
            .xalign(0.0)
            .build();
        summary_text.add_css_class("section-summary-label");

        let summary_arrow = gtk4::Label::builder()
            .label("▸")
            .halign(gtk4::Align::End)
            .build();
        summary_arrow.add_css_class("section-expand-arrow");

        summary_content.append(&summary_icon);
        summary_content.append(&summary_text);
        summary_content.append(&summary_arrow);

        let summary_btn = gtk4::Button::builder()
            .child(&summary_content)
            .build();
        summary_btn.add_css_class("section-summary");
        root.append(&summary_btn);

        // ── Detail revealer ───────────────────────────────────────────────
        let detail_revealer = gtk4::Revealer::builder()
            .transition_type(gtk4::RevealerTransitionType::SlideDown)
            .transition_duration(200)
            .reveal_child(false)
            .build();

        // ── Actions row ───────────────────────────────────────────────────
        let actions_row = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(4)
            .homogeneous(true)
            .build();
        actions_row.add_css_class("power-actions-row");

        // Helper: build one icon-button + label column.
        let make_action_btn = |icon: &str, name: &str| {
            let col = gtk4::Box::builder()
                .orientation(gtk4::Orientation::Vertical)
                .spacing(4)
                .halign(gtk4::Align::Center)
                .build();

            let icon_lbl = gtk4::Label::builder().label(icon).build();
            let btn = gtk4::Button::builder().child(&icon_lbl).build();
            btn.add_css_class("toggle-btn");

            let text_lbl = gtk4::Label::builder().label(name).build();
            text_lbl.add_css_class("toggle-label");

            col.append(&btn);
            col.append(&text_lbl);
            (col, btn, icon_lbl, text_lbl)
        };

        // ── Full screenshot ────────────────────────────────────────────────
        let (col_full, btn_full, _, _) = make_action_btn("󰹑", "Full");
        btn_full.connect_clicked(|btn| {
            hide_panel_for_widget(btn.upcast_ref());
            glib::timeout_add_local_once(Duration::from_millis(200), || {
                let filename = make_screenshot_path("png");
                if let Err(e) = Command::new("grim").arg(&filename).spawn() {
                    log::error!("Failed to spawn grim for full screenshot: {}", e);
                }
            });
        });
        actions_row.append(&col_full);

        // ── Area screenshot ────────────────────────────────────────────────
        let (col_area, btn_area, _, _) = make_action_btn("󰩬", "Area");
        btn_area.connect_clicked(|btn| {
            hide_panel_for_widget(btn.upcast_ref());
            glib::timeout_add_local_once(Duration::from_millis(200), || {
                let filename = make_screenshot_path("png");
                let cmd = format!("grim -g \"$(slurp)\" {}", filename);
                if let Err(e) = Command::new("sh").args(["-c", &cmd]).spawn() {
                    log::error!("Failed to spawn slurp/grim for area screenshot: {}", e);
                }
            });
        });
        actions_row.append(&col_area);

        // ── Window screenshot ──────────────────────────────────────────────
        let (col_window, btn_window, _, _) = make_action_btn("󰖲", "Window");
        btn_window.connect_clicked(|btn| {
            hide_panel_for_widget(btn.upcast_ref());
            glib::timeout_add_local_once(Duration::from_millis(200), || {
                let filename = make_screenshot_path("png");
                // Extract focused window geometry from swaymsg, pass to grim -g.
                let cmd = format!(
                    "grim -g \"$(swaymsg -t get_tree | jq -r '.. | select(.focused?) | .rect | \"\\(.x),\\(.y) \\(.width)x\\(.height)\"')\" {}",
                    filename
                );
                if let Err(e) = Command::new("sh").args(["-c", &cmd]).spawn() {
                    log::error!("Failed to spawn grim for window screenshot: {}", e);
                }
            });
        });
        actions_row.append(&col_window);

        // ── Screen recording toggle ────────────────────────────────────────
        let recording_pid: Rc<RefCell<Option<u32>>> = Rc::new(RefCell::new(None));

        let col_record = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(4)
            .halign(gtk4::Align::Center)
            .build();

        let record_icon_lbl = gtk4::Label::builder().label("󰑋").build();
        let btn_record = gtk4::Button::builder()
            .child(&record_icon_lbl)
            .build();
        btn_record.add_css_class("toggle-btn");

        let record_text_lbl = gtk4::Label::builder().label("Record").build();
        record_text_lbl.add_css_class("toggle-label");

        col_record.append(&btn_record);
        col_record.append(&record_text_lbl);

        {
            let recording_pid_c = recording_pid.clone();
            let icon_c = record_icon_lbl.clone();
            let text_c = record_text_lbl.clone();
            btn_record.connect_clicked(move |btn| {
                let current_pid = *recording_pid_c.borrow();
                if let Some(pid) = current_pid {
                    // Stop recording: kill wf-recorder.
                    if let Err(e) = Command::new("kill").arg(pid.to_string()).spawn() {
                        log::error!("Failed to kill wf-recorder (pid {}): {}", pid, e);
                    }
                    *recording_pid_c.borrow_mut() = None;
                    icon_c.set_label("󰑋");
                    text_c.set_label("Record");
                    btn.remove_css_class("active");
                } else {
                    // Start recording.
                    let filename = make_screenshot_path("mp4");
                    match Command::new("wf-recorder").args(["-f", &filename]).spawn() {
                        Ok(child) => {
                            let pid = child.id();
                            *recording_pid_c.borrow_mut() = Some(pid);
                            icon_c.set_label("󰻃");
                            text_c.set_label("Stop");
                            btn.add_css_class("active");
                        }
                        Err(e) => {
                            log::error!("Failed to spawn wf-recorder: {}", e);
                        }
                    }
                }
            });
        }
        actions_row.append(&col_record);

        detail_revealer.set_child(Some(&actions_row));
        root.append(&detail_revealer);

        // ── Toggle detail on summary button click ─────────────────────────
        {
            let revealer_c = detail_revealer.clone();
            let arrow_c = summary_arrow.clone();
            summary_btn.connect_clicked(move |_| {
                let expanded = !revealer_c.reveals_child();
                revealer_c.set_reveal_child(expanded);
                arrow_c.set_label(if expanded { "▾" } else { "▸" });
            });
        }

        Self {
            root,
            summary_arrow,
            detail_revealer,
            recording_pid,
        }
    }

    /// Check recording state — no-op unless we want to verify the process is still alive.
    pub fn refresh(&self) {
        let mut pid_ref = self.recording_pid.borrow_mut();
        if let Some(pid) = *pid_ref {
            // Check if wf-recorder is still running by sending signal 0.
            let still_running = Command::new("kill")
                .args(["-0", &pid.to_string()])
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if !still_running {
                log::info!("wf-recorder (pid {}) has exited; clearing recording state.", pid);
                *pid_ref = None;
            }
        }
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }
}
