use std::cell::RefCell;

use gtk4::prelude::*;

use crate::spawn::spawn_work;

pub struct QuickInfoStrip {
    root: gtk4::Box,
    clock_label: gtk4::Label,
    battery_label: gtk4::Label,
    volume_label: gtk4::Label,
    network_label: gtk4::Label,
    media_label: gtk4::Label,
    #[allow(dead_code)]
    pills_box: gtk4::Box,
    search_entry: gtk4::SearchEntry,
    refresh_id: RefCell<Option<glib::SourceId>>,
}

// ── Data reading helpers (blocking — run on background thread) ────────────────

fn read_clock() -> String {
    // Use glib::DateTime for formatting without chrono dependency
    if let Ok(now) = glib::DateTime::now_local() {
        now.format("%H:%M").unwrap_or_else(|_| glib::GString::from("--:--")).to_string()
    } else {
        "--:--".to_string()
    }
}

fn read_battery() -> Option<String> {
    // Find first BAT* entry under /sys/class/power_supply
    let entries = std::fs::read_dir("/sys/class/power_supply").ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !name_str.starts_with("BAT") {
            continue;
        }
        let base = entry.path();
        let capacity_str = std::fs::read_to_string(base.join("capacity")).ok()?;
        let capacity: u32 = capacity_str.trim().parse().ok()?;
        let status = std::fs::read_to_string(base.join("status"))
            .unwrap_or_default();
        let status = status.trim();
        let label = if status == "Charging" || status == "Full" {
            format!("{}% ⚡", capacity)
        } else {
            format!("{}%", capacity)
        };
        return Some(label);
    }
    None
}

fn read_volume() -> Option<String> {
    // Check mute state first
    let mute_out = std::process::Command::new("pactl")
        .args(["get-sink-mute", "@DEFAULT_SINK@"])
        .output()
        .ok()?;
    let mute_str = String::from_utf8_lossy(&mute_out.stdout);
    if mute_str.contains("yes") {
        return Some("🔇".to_string());
    }

    let vol_out = std::process::Command::new("pactl")
        .args(["get-sink-volume", "@DEFAULT_SINK@"])
        .output()
        .ok()?;
    if !vol_out.status.success() {
        return None;
    }
    let vol_str = String::from_utf8_lossy(&vol_out.stdout);
    // Parse first percentage like "65%"
    for part in vol_str.split_whitespace() {
        if let Some(pct) = part.strip_suffix('%') {
            if pct.parse::<u32>().is_ok() {
                return Some(format!("{}%", pct));
            }
        }
    }
    None
}

fn read_network() -> String {
    let out = std::process::Command::new("nmcli")
        .args(["-t", "-f", "NAME,TYPE", "c", "show", "--active"])
        .output();
    let Ok(out) = out else {
        return "—".to_string();
    };
    if !out.status.success() {
        return "—".to_string();
    }
    let text = String::from_utf8_lossy(&out.stdout);
    // Prefer wifi, fall back to ethernet, else "—"
    let mut ethernet_name: Option<String> = None;
    for line in text.lines() {
        let parts: Vec<&str> = line.splitn(2, ':').collect();
        if parts.len() < 2 {
            continue;
        }
        let name = parts[0].trim();
        let conn_type = parts[1].trim();
        if conn_type.contains("wireless") || conn_type.contains("wifi") {
            return name.to_string();
        }
        if conn_type.contains("ethernet") && ethernet_name.is_none() {
            ethernet_name = Some("Wired".to_string());
        }
    }
    ethernet_name.unwrap_or_else(|| "—".to_string())
}

fn read_media() -> Option<String> {
    let out = std::process::Command::new("playerctl")
        .args(["metadata", "--format", "{{artist}} - {{title}}"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    // Also check if playing (don't show if paused)
    let status_out = std::process::Command::new("playerctl")
        .args(["status"])
        .output()
        .ok()?;
    let status = String::from_utf8_lossy(&status_out.stdout).trim().to_string();
    if status != "Playing" {
        return None;
    }
    let raw = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if raw.is_empty() || raw == " - " {
        return None;
    }
    // Truncate to 30 chars
    let truncated = if raw.chars().count() > 30 {
        let s: String = raw.chars().take(27).collect();
        format!("{}…", s)
    } else {
        raw
    };
    Some(truncated)
}

// ── QuickInfoStrip ────────────────────────────────────────────────────────────

impl QuickInfoStrip {
    pub fn new() -> Self {
        let root = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(0)
            .hexpand(true)
            .build();
        root.add_css_class("dispatch-quickinfo");

        // Pills container (left-aligned, grows)
        let pills_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(4)
            .hexpand(true)
            .valign(gtk4::Align::Center)
            .build();

        let make_pill = |extra_class: &str| -> gtk4::Label {
            let lbl = gtk4::Label::builder()
                .build();
            lbl.add_css_class("qi-pill");
            lbl.add_css_class(extra_class);
            lbl
        };

        let clock_label   = make_pill("qi-clock");
        let battery_label = make_pill("qi-battery");
        let volume_label  = make_pill("qi-volume");
        let network_label = make_pill("qi-network");
        let media_label   = make_pill("qi-media");

        pills_box.append(&clock_label);
        pills_box.append(&battery_label);
        pills_box.append(&volume_label);
        pills_box.append(&network_label);
        pills_box.append(&media_label);

        // Search entry (hidden until search mode)
        let search_entry = gtk4::SearchEntry::builder()
            .hexpand(true)
            .visible(false)
            .placeholder_text("Search lanes…")
            .build();
        search_entry.add_css_class("qi-search-entry");

        // Search trigger button (right-aligned)
        let search_btn = gtk4::Button::builder()
            .label("/")
            .valign(gtk4::Align::Center)
            .build();
        search_btn.add_css_class("qi-search-btn");

        root.append(&pills_box);
        root.append(&search_entry);
        root.append(&search_btn);

        let strip = Self {
            root: root.clone(),
            clock_label,
            battery_label,
            volume_label,
            network_label,
            media_label,
            pills_box,
            search_entry: search_entry.clone(),
            refresh_id: RefCell::new(None),
        };

        // Wire search button to toggle search mode
        {
            let root_clone = root.clone();
            let entry_clone = search_entry.clone();
            search_btn.connect_clicked(move |_| {
                toggle_search_mode(&root_clone, &entry_clone, true);
            });
        }

        // Start 1Hz refresh timer
        strip.start_timer();

        // Kick off first refresh immediately
        strip.refresh();

        strip
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    pub fn refresh(&self) {
        // Clone handles for closures
        let clock_lbl   = self.clock_label.clone();
        let battery_lbl = self.battery_label.clone();
        let volume_lbl  = self.volume_label.clone();
        let network_lbl = self.network_label.clone();
        let media_lbl   = self.media_label.clone();

        // Clock — cheap enough to do synchronously
        clock_lbl.set_label(&read_clock());

        // Battery
        let battery_lbl2 = battery_lbl.clone();
        spawn_work(
            read_battery,
            move |result| {
                match result {
                    Some(text) => {
                        battery_lbl2.set_label(&text);
                        battery_lbl2.set_visible(true);
                    }
                    None => {
                        battery_lbl2.set_visible(false);
                    }
                }
            },
        );

        // Volume
        let volume_lbl2 = volume_lbl.clone();
        spawn_work(
            read_volume,
            move |result| {
                match result {
                    Some(text) => {
                        volume_lbl2.set_label(&text);
                        volume_lbl2.set_visible(true);
                    }
                    None => {
                        volume_lbl2.set_visible(false);
                    }
                }
            },
        );

        // Network
        let network_lbl2 = network_lbl.clone();
        spawn_work(
            || read_network(),
            move |text| {
                network_lbl2.set_label(&text);
            },
        );

        // Media
        let media_lbl2 = media_lbl.clone();
        spawn_work(
            read_media,
            move |result| {
                match result {
                    Some(text) => {
                        media_lbl2.set_label(&text);
                        media_lbl2.set_visible(true);
                    }
                    None => {
                        media_lbl2.set_visible(false);
                    }
                }
            },
        );
    }

    pub fn enter_search_mode(&self) {
        toggle_search_mode(&self.root, &self.search_entry, true);
    }

    pub fn exit_search_mode(&self) {
        toggle_search_mode(&self.root, &self.search_entry, false);
    }

    pub fn search_entry(&self) -> &gtk4::SearchEntry {
        &self.search_entry
    }

    fn start_timer(&self) {
        let root = self.root.clone();
        let clock_lbl   = self.clock_label.clone();
        let battery_lbl = self.battery_label.clone();
        let volume_lbl  = self.volume_label.clone();
        let network_lbl = self.network_label.clone();
        let media_lbl   = self.media_label.clone();

        let id = glib::timeout_add_local(std::time::Duration::from_secs(1), move || {
            // Stop if widget no longer has a parent (panel destroyed)
            if !root.is_visible() && root.parent().is_none() {
                return glib::ControlFlow::Break;
            }

            // Clock — synchronous
            clock_lbl.set_label(&read_clock());

            // Battery
            let b = battery_lbl.clone();
            spawn_work(read_battery, move |r| {
                match r {
                    Some(t) => { b.set_label(&t); b.set_visible(true); }
                    None    => { b.set_visible(false); }
                }
            });

            // Volume
            let v = volume_lbl.clone();
            spawn_work(read_volume, move |r| {
                match r {
                    Some(t) => { v.set_label(&t); v.set_visible(true); }
                    None    => { v.set_visible(false); }
                }
            });

            // Network
            let n = network_lbl.clone();
            spawn_work(|| read_network(), move |t| { n.set_label(&t); });

            // Media
            let m = media_lbl.clone();
            spawn_work(read_media, move |r| {
                match r {
                    Some(t) => { m.set_label(&t); m.set_visible(true); }
                    None    => { m.set_visible(false); }
                }
            });

            glib::ControlFlow::Continue
        });

        *self.refresh_id.borrow_mut() = Some(id);
    }
}

fn toggle_search_mode(root: &gtk4::Box, entry: &gtk4::SearchEntry, on: bool) {
    if on {
        root.add_css_class("qi-search-active");
        entry.set_visible(true);
        entry.grab_focus();
    } else {
        root.remove_css_class("qi-search-active");
        entry.set_visible(false);
        entry.set_text("");
    }
}
