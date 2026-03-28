use std::cell::RefCell;
use std::process::Command;
use std::rc::Rc;

use gtk4::prelude::*;

use crate::spawn::spawn_work;

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
    art_url: Option<String>,
    player_name: Option<String>,
    /// Track length in seconds (from mpris:length, which is in microseconds).
    length_secs: Option<f64>,
    /// Current position in seconds.
    position_secs: Option<f64>,
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
        _ => {
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

    // Album art URL — may be file:///path or https://...
    let art_url = playerctl(&["metadata", "mpris:artUrl"]).filter(|s| !s.is_empty());

    // Player identity (e.g. "Spotify", "firefox")
    let player_name = playerctl(&["metadata", "--format", "{{playerName}}"]).filter(|s| !s.is_empty());

    // Track length (mpris:length is in microseconds)
    let length_secs = playerctl(&["metadata", "mpris:length"])
        .and_then(|s| s.parse::<f64>().ok())
        .map(|us| us / 1_000_000.0)
        .filter(|&s| s > 0.0);

    // Current position in seconds
    let position_secs = playerctl(&["position"])
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|&s| s >= 0.0);

    Some(MediaState {
        status,
        artist,
        title,
        art_url,
        player_name,
        length_secs,
        position_secs,
    })
}

/// Resolve an art URL to a local file path for GTK.
/// - `file:///path` → `/path`
/// - Other URLs are ignored (would need HTTP fetch + cache)
fn resolve_art_path(url: &str) -> Option<String> {
    if let Some(path) = url.strip_prefix("file://") {
        Some(path.to_string())
    } else {
        None
    }
}

fn format_time(secs: f64) -> String {
    let total = secs.round() as u64;
    let m = total / 60;
    let s = total % 60;
    format!("{}:{:02}", m, s)
}

// ── MediaSection ──────────────────────────────────────────────────────────────

struct Widgets {
    title_label: gtk4::Label,
    artist_label: gtk4::Label,
    play_pause_btn: gtk4::Button,
    art_image: gtk4::Picture,
    art_fallback: gtk4::Label,
    player_badge: gtk4::Label,
    progress_bar: gtk4::ProgressBar,
    time_label: gtk4::Label,
}

pub struct MediaSection {
    root: gtk4::Box,
    widgets: Rc<Widgets>,
    state: Rc<RefCell<Option<MediaState>>>,
    progress_timer: Rc<RefCell<Option<glib::SourceId>>>,
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

        // ── Section title row with player badge ──────────────────────────────
        let title_row = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(6)
            .build();

        let section_title = gtk4::Label::builder()
            .label("NOW PLAYING")
            .halign(gtk4::Align::Start)
            .hexpand(true)
            .build();
        section_title.add_css_class("section-title");

        let player_badge = gtk4::Label::builder()
            .label("")
            .halign(gtk4::Align::End)
            .valign(gtk4::Align::Center)
            .visible(false)
            .build();
        player_badge.add_css_class("media-player-badge");

        title_row.append(&section_title);
        title_row.append(&player_badge);
        root.append(&title_row);

        // ── Content row: album art + track info ──────────────────────────────
        let content_row = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(12)
            .build();

        // Album art (picture or fallback icon)
        let art_frame = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .halign(gtk4::Align::Center)
            .valign(gtk4::Align::Center)
            .overflow(gtk4::Overflow::Hidden)
            .build();
        art_frame.add_css_class("media-art-frame");

        let art_image = gtk4::Picture::builder()
            .content_fit(gtk4::ContentFit::Cover)
            .visible(false)
            .build();
        art_image.add_css_class("media-art");

        let art_fallback = gtk4::Label::builder()
            .label("󰎆")
            .halign(gtk4::Align::Center)
            .valign(gtk4::Align::Center)
            .visible(true)
            .build();
        art_fallback.add_css_class("media-art-fallback");

        art_frame.append(&art_image);
        art_frame.append(&art_fallback);
        content_row.append(&art_frame);

        // Track info column
        let info_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(2)
            .hexpand(true)
            .valign(gtk4::Align::Center)
            .build();

        let title_label = gtk4::Label::builder()
            .label("")
            .halign(gtk4::Align::Start)
            .hexpand(true)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .max_width_chars(28)
            .build();
        title_label.add_css_class("media-title");

        let artist_label = gtk4::Label::builder()
            .label("")
            .halign(gtk4::Align::Start)
            .hexpand(true)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .max_width_chars(28)
            .build();
        artist_label.add_css_class("media-artist");

        info_box.append(&title_label);
        info_box.append(&artist_label);
        content_row.append(&info_box);
        root.append(&content_row);

        // ── Progress bar + time ──────────────────────────────────────────────
        let progress_row = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(2)
            .build();

        let progress_bar = gtk4::ProgressBar::builder()
            .hexpand(true)
            .build();
        progress_bar.add_css_class("media-progress");

        let time_label = gtk4::Label::builder()
            .label("")
            .halign(gtk4::Align::End)
            .visible(false)
            .build();
        time_label.add_css_class("media-time");

        progress_row.append(&progress_bar);
        progress_row.append(&time_label);
        root.append(&progress_row);

        // ── Controls row ──────────────────────────────────────────────────────
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
            title_label,
            artist_label,
            play_pause_btn,
            art_image,
            art_fallback,
            player_badge,
            progress_bar,
            time_label,
        });

        let state: Rc<RefCell<Option<MediaState>>> = Rc::new(RefCell::new(None));
        let progress_timer: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));

        // ── Button signals ────────────────────────────────────────────────
        {
            let w = widgets.clone();
            let s = state.clone();
            let root_ref = root.clone();
            let pt = progress_timer.clone();
            prev_btn.connect_clicked(move |_| {
                Self::send_command_and_refresh(
                    "previous",
                    root_ref.clone(),
                    w.clone(),
                    s.clone(),
                    pt.clone(),
                );
            });
        }

        {
            let w = widgets.clone();
            let s = state.clone();
            let root_ref = root.clone();
            let pt = progress_timer.clone();
            play_pause_btn_for_signal.connect_clicked(move |_| {
                Self::send_command_and_refresh(
                    "play-pause",
                    root_ref.clone(),
                    w.clone(),
                    s.clone(),
                    pt.clone(),
                );
            });
        }

        {
            let w = widgets.clone();
            let s = state.clone();
            let root_ref = root.clone();
            let pt = progress_timer.clone();
            next_btn.connect_clicked(move |_| {
                Self::send_command_and_refresh(
                    "next",
                    root_ref.clone(),
                    w.clone(),
                    s.clone(),
                    pt.clone(),
                );
            });
        }

        let section = MediaSection {
            root,
            widgets,
            state,
            progress_timer,
        };
        section.refresh();
        section
    }

    fn schedule_refresh(
        root: gtk4::Box,
        w: Rc<Widgets>,
        state: Rc<RefCell<Option<MediaState>>>,
        progress_timer: Rc<RefCell<Option<glib::SourceId>>>,
    ) {
        spawn_work(read_state, move |new_state| {
            Self::apply_state(&root, &w, &new_state, &progress_timer);
            *state.borrow_mut() = new_state;
        });
    }

    /// Run a playerctl command on a background thread, then refresh state.
    fn send_command_and_refresh(
        command: &'static str,
        root: gtk4::Box,
        w: Rc<Widgets>,
        state: Rc<RefCell<Option<MediaState>>>,
        progress_timer: Rc<RefCell<Option<glib::SourceId>>>,
    ) {
        spawn_work(
            move || {
                playerctl(&[command]);
                read_state()
            },
            move |new_state| {
                Self::apply_state(&root, &w, &new_state, &progress_timer);
                *state.borrow_mut() = new_state;
            },
        );
    }

    fn apply_state(
        root: &gtk4::Box,
        w: &Rc<Widgets>,
        state: &Option<MediaState>,
        progress_timer: &Rc<RefCell<Option<glib::SourceId>>>,
    ) {
        // Cancel existing progress timer
        if let Some(id) = progress_timer.borrow_mut().take() {
            id.remove();
        }

        match state {
            None => {
                root.set_visible(false);
            }
            Some(ms) => {
                root.set_visible(true);

                // Title + artist
                let display_title = if ms.title.is_empty() {
                    "Unknown track"
                } else {
                    &ms.title
                };
                w.title_label.set_label(display_title);
                w.artist_label.set_label(&ms.artist);
                w.artist_label.set_visible(!ms.artist.is_empty());

                // Player badge
                if let Some(ref name) = ms.player_name {
                    let display_name = capitalize(name);
                    w.player_badge.set_label(&display_name);
                    w.player_badge.set_visible(true);
                } else {
                    w.player_badge.set_visible(false);
                }

                // Album art
                let art_shown = if let Some(ref url) = ms.art_url {
                    if let Some(path) = resolve_art_path(url) {
                        let file = gtk4::gio::File::for_path(&path);
                        w.art_image.set_file(Some(&file));
                        w.art_image.set_visible(true);
                        w.art_fallback.set_visible(false);
                        true
                    } else {
                        false
                    }
                } else {
                    false
                };
                if !art_shown {
                    w.art_image.set_visible(false);
                    w.art_fallback.set_visible(true);
                }

                // Progress bar + time
                if let (Some(pos), Some(len)) = (ms.position_secs, ms.length_secs) {
                    let fraction = (pos / len).clamp(0.0, 1.0);
                    w.progress_bar.set_fraction(fraction);
                    w.progress_bar.set_visible(true);
                    w.time_label
                        .set_label(&format!("{} / {}", format_time(pos), format_time(len)));
                    w.time_label.set_visible(true);

                    // Live-update progress while playing
                    if ms.status == PlaybackStatus::Playing {
                        let w_c = w.clone();
                        let len_c = len;
                        let cancelled = Rc::new(std::cell::Cell::new(false));
                        let cancelled_c = cancelled.clone();
                        let id = glib::timeout_add_local(
                            std::time::Duration::from_millis(500),
                            move || {
                                if cancelled_c.get() {
                                    return glib::ControlFlow::Break;
                                }
                                let w_inner = w_c.clone();
                                let cancelled_inner = cancelled_c.clone();
                                spawn_work(
                                    || {
                                        playerctl(&["position"])
                                            .and_then(|s| s.parse::<f64>().ok())
                                    },
                                    move |pos| {
                                        if let Some(pos) = pos {
                                            let frac = (pos / len_c).clamp(0.0, 1.0);
                                            w_inner.progress_bar.set_fraction(frac);
                                            w_inner.time_label.set_label(&format!(
                                                "{} / {}",
                                                format_time(pos),
                                                format_time(len_c)
                                            ));
                                        } else {
                                            cancelled_inner.set(true);
                                        }
                                    },
                                );
                                glib::ControlFlow::Continue
                            },
                        );
                        *progress_timer.borrow_mut() = Some(id);
                    }
                } else {
                    w.progress_bar.set_visible(false);
                    w.time_label.set_visible(false);
                }

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
        Self::schedule_refresh(
            self.root.clone(),
            self.widgets.clone(),
            self.state.clone(),
            self.progress_timer.clone(),
        );
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
    }
}
