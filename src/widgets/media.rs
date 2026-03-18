use std::cell::RefCell;
use std::process::Command;
use std::rc::Rc;

use gtk4::prelude::*;

// ── Nerd Font icons ───────────────────────────────────────────────────────────
const ICON_PREV: &str = "󰒮";
const ICON_PLAY: &str = "󰐊";
const ICON_PAUSE: &str = "󰏤";
const ICON_NEXT: &str = "󰒭";

// ── Backend ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum PlaybackStatus {
    Playing,
    Paused,
}

#[derive(Debug, Clone)]
struct MediaState {
    status: PlaybackStatus,
    artist: String,
    title: String,
}

fn playerctl(args: &[&str]) -> Option<String> {
    let out = Command::new("playerctl").args(args).output().ok()?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    } else {
        None
    }
}

fn read_state() -> Option<MediaState> {
    let status_str = playerctl(&["status"])?;

    let status = match status_str.as_str() {
        "Playing" => PlaybackStatus::Playing,
        "Paused" => PlaybackStatus::Paused,
        // "Stopped" or anything else: only show if there is still metadata
        _ => {
            // Try to get metadata; if absent, treat as inactive
            let artist = playerctl(&["metadata", "artist"]).unwrap_or_default();
            let title = playerctl(&["metadata", "title"]).unwrap_or_default();
            if artist.is_empty() && title.is_empty() {
                return None;
            }
            PlaybackStatus::Paused
        }
    };

    let artist = playerctl(&["metadata", "artist"]).unwrap_or_default();
    let title = playerctl(&["metadata", "title"]).unwrap_or_default();

    Some(MediaState { status, artist, title })
}

// ── MediaSection ──────────────────────────────────────────────────────────────

struct Widgets {
    info_label: gtk4::Label,
    play_pause_btn: gtk4::Button,
}

pub struct MediaSection {
    root: gtk4::Box,
    widgets: Rc<Widgets>,
    state: Rc<RefCell<Option<MediaState>>>,
}

impl MediaSection {
    pub fn new() -> Self {
        // ── Root container ───────────────────────────────────────────────────
        let root = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(6)
            .visible(false)
            .build();
        root.add_css_class("section");

        // ── Section title ─────────────────────────────────────────────────
        let title = gtk4::Label::builder()
            .label("NOW PLAYING")
            .halign(gtk4::Align::Start)
            .build();
        title.add_css_class("section-title");
        root.append(&title);

        // ── Info row (artist — title) ─────────────────────────────────────
        let info_label = gtk4::Label::builder()
            .label("")
            .halign(gtk4::Align::Start)
            .hexpand(true)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .max_width_chars(35)
            .build();
        info_label.add_css_class("media-info");
        root.append(&info_label);

        // ── Controls row ──────────────────────────────────────────────────
        let controls = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(8)
            .halign(gtk4::Align::Center)
            .build();
        controls.add_css_class("media-controls");

        let prev_btn = gtk4::Button::with_label(ICON_PREV);
        prev_btn.add_css_class("media-btn");

        let play_pause_btn = gtk4::Button::with_label(ICON_PLAY);
        play_pause_btn.add_css_class("media-btn");
        play_pause_btn.add_css_class("media-play-pause");

        let next_btn = gtk4::Button::with_label(ICON_NEXT);
        next_btn.add_css_class("media-btn");

        controls.append(&prev_btn);
        controls.append(&play_pause_btn);
        controls.append(&next_btn);
        root.append(&controls);

        let play_pause_btn_for_signal = play_pause_btn.clone();

        let widgets = Rc::new(Widgets {
            info_label,
            play_pause_btn,
        });

        let state: Rc<RefCell<Option<MediaState>>> = Rc::new(RefCell::new(None));

        // ── Button signals ────────────────────────────────────────────────
        {
            let w = widgets.clone();
            let s = state.clone();
            let root_ref = root.clone();
            prev_btn.connect_clicked(move |_| {
                playerctl(&["previous"]);
                Self::schedule_refresh(root_ref.clone(), w.clone(), s.clone());
            });
        }

        {
            let w = widgets.clone();
            let s = state.clone();
            let root_ref = root.clone();
            play_pause_btn_for_signal.connect_clicked(move |_| {
                playerctl(&["play-pause"]);
                Self::schedule_refresh(root_ref.clone(), w.clone(), s.clone());
            });
        }

        {
            let w = widgets.clone();
            let s = state.clone();
            let root_ref = root.clone();
            next_btn.connect_clicked(move |_| {
                playerctl(&["next"]);
                Self::schedule_refresh(root_ref.clone(), w.clone(), s.clone());
            });
        }

        let section = MediaSection { root, widgets, state };
        section.refresh();
        section
    }

    fn schedule_refresh(root: gtk4::Box, w: Rc<Widgets>, state: Rc<RefCell<Option<MediaState>>>) {
        glib::idle_add_local_once(move || {
            let new_state = read_state();
            Self::apply_state(&root, &w, &new_state);
            *state.borrow_mut() = new_state;
        });
    }

    fn apply_state(root: &gtk4::Box, w: &Rc<Widgets>, state: &Option<MediaState>) {
        match state {
            None => {
                root.set_visible(false);
            }
            Some(ms) => {
                root.set_visible(true);

                // Info label
                let info = if ms.artist.is_empty() {
                    ms.title.clone()
                } else if ms.title.is_empty() {
                    ms.artist.clone()
                } else {
                    format!("{} \u{2014} {}", ms.artist, ms.title)
                };
                w.info_label.set_label(&info);

                // Play/pause button icon + "suggested" class
                if ms.status == PlaybackStatus::Playing {
                    w.play_pause_btn.set_label(ICON_PAUSE);
                    w.play_pause_btn.add_css_class("suggested");
                } else {
                    w.play_pause_btn.set_label(ICON_PLAY);
                    w.play_pause_btn.remove_css_class("suggested");
                }
            }
        }
    }

    // ── Public API ────────────────────────────────────────────────────────────

    pub fn refresh(&self) {
        let new_state = read_state();
        Self::apply_state(&self.root, &self.widgets, &new_state);
        *self.state.borrow_mut() = new_state;
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }
}
