use std::process::Command;
use std::rc::Rc;
use std::thread;

use gtk4::prelude::*;
use log::{error, warn};

const ICON_CLIPBOARD: &str = "󰅍";
const MAX_ENTRIES: usize = 10;
const PREVIEW_LEN: usize = 60;

// ── Data types ────────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
struct ClipEntry {
    /// The raw line from `cliphist list` (ID\tcontent).
    raw_line: String,
    /// Display preview (truncated, single line).
    preview: String,
}

#[derive(Clone, Debug)]
enum FetchedState {
    Ok(Vec<ClipEntry>),
    Unavailable(String),
}

// ── cliphist helpers (blocking — run on background threads) ──────────────────

fn cliphist_list_blocking() -> FetchedState {
    let out = Command::new("cliphist")
        .arg("list")
        .output()
        .map_err(|e| {
            warn!("cliphist spawn error: {e}");
        });

    let out = match out {
        Ok(o) => o,
        Err(_) => {
            return FetchedState::Unavailable("cliphist not available".to_string());
        }
    };

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        warn!("cliphist list failed ({}): {}", out.status, stderr.trim());
        return FetchedState::Unavailable("cliphist not available".to_string());
    }

    let stdout = String::from_utf8_lossy(&out.stdout);
    let entries: Vec<ClipEntry> = stdout
        .lines()
        .take(MAX_ENTRIES)
        .filter_map(|line| {
            if line.is_empty() {
                return None;
            }
            // Each line: "<id>\t<content_preview>"
            let preview = if let Some(tab_pos) = line.find('\t') {
                let content = &line[tab_pos + 1..];
                make_preview(content)
            } else {
                make_preview(line)
            };
            Some(ClipEntry {
                raw_line: line.to_string(),
                preview,
            })
        })
        .collect();

    FetchedState::Ok(entries)
}

/// Truncate content to a single-line preview of at most PREVIEW_LEN chars.
fn make_preview(content: &str) -> String {
    // Collapse newlines to a space so multi-line content shows on one line.
    let single_line: String = content
        .chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect();

    if single_line.chars().count() <= PREVIEW_LEN {
        single_line
    } else {
        let truncated: String = single_line.chars().take(PREVIEW_LEN).collect();
        format!("{}…", truncated)
    }
}

/// Restore a clipboard entry by piping its raw line through `cliphist decode | wl-copy`.
fn restore_entry_blocking(raw_line: &str) {
    // echo "<line>" | cliphist decode | wl-copy
    use std::io::Write;
    use std::process::Stdio;

    let decode = Command::new("cliphist")
        .arg("decode")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();

    let mut decode = match decode {
        Ok(child) => child,
        Err(e) => {
            error!("cliphist decode spawn error: {e}");
            return;
        }
    };

    if let Some(stdin) = decode.stdin.as_mut() {
        if let Err(e) = stdin.write_all(raw_line.as_bytes()) {
            error!("cliphist decode write error: {e}");
            return;
        }
    }

    let decode_out = match decode.wait_with_output() {
        Ok(o) => o,
        Err(e) => {
            error!("cliphist decode wait error: {e}");
            return;
        }
    };

    if !decode_out.status.success() {
        let stderr = String::from_utf8_lossy(&decode_out.stderr);
        error!("cliphist decode failed: {}", stderr.trim());
        return;
    }

    // Pipe decoded output to wl-copy.
    let wlcopy = Command::new("wl-copy")
        .stdin(Stdio::piped())
        .spawn();

    let mut wlcopy = match wlcopy {
        Ok(child) => child,
        Err(e) => {
            error!("wl-copy spawn error: {e}");
            return;
        }
    };

    if let Some(stdin) = wlcopy.stdin.as_mut() {
        if let Err(e) = stdin.write_all(&decode_out.stdout) {
            error!("wl-copy write error: {e}");
            return;
        }
    }

    if let Err(e) = wlcopy.wait() {
        error!("wl-copy wait error: {e}");
    }
}

fn cliphist_wipe_blocking() {
    let out = Command::new("cliphist").arg("wipe").output();
    match out {
        Ok(o) if !o.status.success() => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            warn!("cliphist wipe failed ({}): {}", o.status, stderr.trim());
        }
        Err(e) => warn!("cliphist wipe spawn error: {e}"),
        _ => {}
    }
}

// ── ClipboardSection ──────────────────────────────────────────────────────────

struct Widgets {
    summary_icon: gtk4::Label,
    summary_text: gtk4::Label,
    summary_arrow: gtk4::Label,
    detail_revealer: gtk4::Revealer,
    entry_list: gtk4::Box,
    clear_btn: gtk4::Button,
}

pub struct ClipboardSection {
    root: gtk4::Box,
    widgets: Rc<Widgets>,
}

impl ClipboardSection {
    pub fn new() -> Self {
        let root = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(6)
            .build();
        root.add_css_class("section");

        // ── Summary row (always visible, toggles detail revealer) ─────────────
        let summary_row = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(6)
            .build();
        summary_row.add_css_class("section-summary");

        let summary_icon = gtk4::Label::new(Some(ICON_CLIPBOARD));
        summary_icon.add_css_class("section-summary-icon");

        let summary_text = gtk4::Label::new(Some("Clipboard"));
        summary_text.add_css_class("section-summary-label");
        summary_text.set_hexpand(true);
        summary_text.set_xalign(0.0);
        summary_text.set_ellipsize(gtk4::pango::EllipsizeMode::End);

        let summary_arrow = gtk4::Label::new(Some("▸"));
        summary_arrow.add_css_class("section-expand-arrow");

        summary_row.append(&summary_icon);
        summary_row.append(&summary_text);
        summary_row.append(&summary_arrow);

        // ── Detail revealer (collapsed by default) ───────────────────────────
        let detail_revealer = gtk4::Revealer::builder()
            .transition_type(gtk4::RevealerTransitionType::SlideDown)
            .transition_duration(200)
            .reveal_child(false)
            .build();

        // Wire summary row click to toggle the revealer.
        {
            let rev = detail_revealer.clone();
            let arrow = summary_arrow.clone();
            let gesture = gtk4::GestureClick::new();
            gesture.connect_released(move |_, _, _, _| {
                let revealed = rev.reveals_child();
                rev.set_reveal_child(!revealed);
                arrow.set_label(if revealed { "▸" } else { "▾" });
            });
            summary_row.add_controller(gesture);
        }

        root.append(&summary_row);
        root.append(&detail_revealer);

        // ── Detail content box ────────────────────────────────────────────────
        let detail_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(4)
            .build();
        detail_revealer.set_child(Some(&detail_box));

        // ── Entry list ────────────────────────────────────────────────────────
        let entry_list = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(2)
            .build();
        entry_list.add_css_class("device-list");
        detail_box.append(&entry_list);

        // ── Clear button ──────────────────────────────────────────────────────
        let clear_btn = gtk4::Button::with_label("Clear History");
        clear_btn.add_css_class("flat");
        detail_box.append(&clear_btn);

        let widgets = Rc::new(Widgets {
            summary_icon,
            summary_text,
            summary_arrow,
            detail_revealer,
            entry_list,
            clear_btn,
        });

        // Wire the Clear button.
        {
            let w = widgets.clone();
            widgets.clear_btn.connect_clicked(move |_| {
                thread::spawn(|| {
                    cliphist_wipe_blocking();
                });
                Self::schedule_refresh(w.clone());
            });
        }

        let section = ClipboardSection { root, widgets };
        section.refresh();
        section
    }

    // ── Async refresh machinery ───────────────────────────────────────────────

    fn schedule_refresh(w: Rc<Widgets>) {
        let (tx, rx) = std::sync::mpsc::channel::<FetchedState>();

        thread::spawn(move || {
            let state = cliphist_list_blocking();
            let _ = tx.send(state);
        });

        glib::idle_add_local_once(move || match rx.try_recv() {
            Ok(fetched) => Self::apply_fetched(&w, fetched),
            Err(_) => Self::poll_until_ready(w, rx),
        });
    }

    fn poll_until_ready(w: Rc<Widgets>, rx: std::sync::mpsc::Receiver<FetchedState>) {
        glib::idle_add_local_once(move || match rx.try_recv() {
            Ok(fetched) => Self::apply_fetched(&w, fetched),
            Err(std::sync::mpsc::TryRecvError::Empty) => Self::poll_until_ready(w, rx),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                error!("clipboard state background thread disconnected unexpectedly");
            }
        });
    }

    fn apply_fetched(w: &Rc<Widgets>, fetched: FetchedState) {
        match fetched {
            FetchedState::Unavailable(msg) => {
                error!("Clipboard section: {msg}");

                // Replace entry list content with an unavailable notice.
                while let Some(child) = w.entry_list.first_child() {
                    w.entry_list.remove(&child);
                }
                let notice = gtk4::Label::new(Some("Clipboard manager not available"));
                notice.set_xalign(0.0);
                notice.add_css_class("device-row");
                w.entry_list.append(&notice);

                w.clear_btn.set_sensitive(false);
                w.summary_text.set_label("Unavailable");

                // Disable the expand gesture by preventing the arrow from changing.
                w.summary_arrow.set_label("▸");
                w.detail_revealer.set_reveal_child(false);
                w.detail_revealer.set_sensitive(false);
            }
            FetchedState::Ok(entries) => {
                w.clear_btn.set_sensitive(true);
                w.detail_revealer.set_sensitive(true);

                // Update summary text.
                if entries.is_empty() {
                    w.summary_text.set_label("Clipboard");
                } else {
                    let count = entries.len();
                    w.summary_text
                        .set_label(&format!("Clipboard · {} item{}", count, if count == 1 { "" } else { "s" }));
                }

                // Rebuild entry rows.
                while let Some(child) = w.entry_list.first_child() {
                    w.entry_list.remove(&child);
                }

                if entries.is_empty() {
                    let empty_label = gtk4::Label::new(Some("No clipboard history"));
                    empty_label.set_xalign(0.0);
                    empty_label.add_css_class("device-row");
                    w.entry_list.append(&empty_label);
                } else {
                    for entry in entries {
                        let row = Self::build_entry_row(entry, w.clone());
                        w.entry_list.append(&row);
                    }
                }
            }
        }
    }

    fn build_entry_row(entry: ClipEntry, w: Rc<Widgets>) -> gtk4::Box {
        let row = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(8)
            .build();
        row.add_css_class("device-row");
        row.set_focusable(true);
        row.set_can_focus(true);

        let preview_label = gtk4::Label::new(Some(&entry.preview));
        preview_label.set_hexpand(true);
        preview_label.set_xalign(0.0);
        preview_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        row.append(&preview_label);

        // Clicking restores this entry and collapses the panel.
        let raw_line = entry.raw_line.clone();
        let gesture = gtk4::GestureClick::new();
        gesture.connect_released(move |_, _, _, _| {
            let line = raw_line.clone();
            let w2 = w.clone();
            thread::spawn(move || {
                restore_entry_blocking(&line);
            });
            // Collapse the revealer after selection.
            w2.detail_revealer.set_reveal_child(false);
            w2.summary_arrow.set_label("▸");
        });
        row.add_controller(gesture);

        row
    }

    // ── Public API ────────────────────────────────────────────────────────────

    /// Kick off a non-blocking state refresh. The UI is updated asynchronously
    /// via the GLib main loop once the background thread completes.
    pub fn refresh(&self) {
        Self::schedule_refresh(self.widgets.clone());
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }
}
