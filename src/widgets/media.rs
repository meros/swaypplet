use std::cell::RefCell;
use std::process::Command;
use std::rc::Rc;

use gtk4::prelude::*;

use crate::spawn::spawn_work;
use crate::ui;
use crate::ui::icons;

// ── Backend ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PlaybackStatus {
    Playing,
    Paused,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MediaState {
    pub(crate) status: PlaybackStatus,
    pub(crate) artist: String,
    pub(crate) title: String,
    art_url: Option<String>,
    player_name: Option<String>,
    /// Track length in seconds (from mpris:length, which is in microseconds).
    length_secs: Option<f64>,
    /// Current position in seconds.
    position_secs: Option<f64>,
}

impl MediaState {
    /// A state read over D-Bus (`crate::mpris`) rather than from playerctl.
    /// No position: MPRIS does not signal it, and the bar does not show it.
    pub(crate) fn from_mpris(
        status: PlaybackStatus,
        artist: String,
        title: String,
        art_url: Option<String>,
        player_name: Option<String>,
        length_secs: Option<f64>,
    ) -> MediaState {
        MediaState {
            status,
            artist,
            title,
            art_url,
            player_name,
            length_secs,
            position_secs: None,
        }
    }

    /// Local album-art path for the bar media popover; remote URLs are
    /// skipped (would need fetch + cache), same rule as the panel section.
    pub(crate) fn art_path(&self) -> Option<String> {
        self.art_url.as_deref().and_then(resolve_art_path)
    }
}

pub(crate) fn playerctl(args: &[&str]) -> Option<String> {
    let out = Command::new("playerctl").args(args).output().ok()?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    } else {
        None
    }
}

pub(crate) fn read_state() -> Option<MediaState> {
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
    let player_name =
        playerctl(&["metadata", "--format", "{{playerName}}"]).filter(|s| !s.is_empty());

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

/// Cancel flag for the running progress timer.
///
/// Only the flag is kept, never the timer's `SourceId`. The timer can end
/// itself (it returns `Break` once the flag is set, or once a position read
/// fails), which would leave a stored id dangling; removing that id later
/// panics inside a glib trampoline and aborts the process. Setting the flag
/// instead lets the timer retire itself on its next tick.
type ProgressTimer = Rc<RefCell<Option<Rc<std::cell::Cell<bool>>>>>;

pub struct MediaSection {
    root: gtk4::Box,
    widgets: Rc<Widgets>,
    state: Rc<RefCell<Option<MediaState>>>,
    progress_timer: ProgressTimer,
}

impl MediaSection {
    pub fn new() -> Self {
        // ── Root container ───────────────────────────────────────────────────
        let root = ui::group(2);
        root.add_css_class("section-group");
        root.set_visible(false);

        // ── Section title row with player badge ──────────────────────────────
        let title_row = ui::hbox(2);

        let section_title = ui::heading("Now Playing");
        section_title.set_hexpand(true);

        let player_badge = ui::text("", ui::Text::Caption, ui::Tone::Faint);
        player_badge.set_valign(gtk4::Align::Center);
        player_badge.set_visible(false);

        title_row.append(&section_title);
        title_row.append(&player_badge);
        root.append(&title_row);

        // ── Content row: album art + track info ──────────────────────────────
        let content_row = ui::hbox(4);

        // Album art (picture or fallback icon), in the thumb's rounded frame.
        let art_frame = ui::thumb();
        art_frame.set_orientation(gtk4::Orientation::Vertical);
        art_frame.add_css_class("media-section-art");

        let art_image = gtk4::Picture::builder()
            .content_fit(gtk4::ContentFit::Cover)
            .visible(false)
            .vexpand(true)
            .build();

        let art_fallback = gtk4::Label::builder()
            .label("󰎆")
            .halign(gtk4::Align::Center)
            .valign(gtk4::Align::Center)
            .vexpand(true)
            .visible(true)
            .build();
        ui::glyph(&art_fallback, ui::Text::DisplaySm, ui::Tone::Muted);

        art_frame.append(&art_image);
        art_frame.append(&art_fallback);
        content_row.append(&art_frame);

        // Track info column
        let info_box = ui::vbox(1);
        info_box.set_hexpand(true);
        info_box.set_valign(gtk4::Align::Center);

        let title_label = ui::text("", ui::Text::Body, ui::Tone::Fg);
        title_label.add_css_class("ui-strong");
        title_label.set_hexpand(true);
        title_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        title_label.set_max_width_chars(28);

        let artist_label = ui::text("", ui::Text::Label, ui::Tone::Muted);
        artist_label.set_hexpand(true);
        artist_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        artist_label.set_max_width_chars(28);

        info_box.append(&title_label);
        info_box.append(&artist_label);
        content_row.append(&info_box);
        root.append(&content_row);

        // ── Progress bar + time ──────────────────────────────────────────────
        let progress_row = ui::vbox(1);

        let progress_bar = ui::progress(0.0);
        progress_bar.set_hexpand(true);

        let time_label = ui::text("", ui::Text::Caption, ui::Tone::Faint);
        time_label.add_css_class("ui-numeric");
        time_label.set_halign(gtk4::Align::End);
        time_label.set_visible(false);

        progress_row.append(&progress_bar);
        progress_row.append(&time_label);
        root.append(&progress_row);

        // ── Controls row ──────────────────────────────────────────────────────
        let controls = ui::hbox(3);
        controls.set_halign(gtk4::Align::Center);
        controls.add_css_class("media-section-controls");

        let prev_btn = ui::glyph_button(icons::MEDIA_PREV, "Previous", ui::Kind::Flat);
        prev_btn.add_css_class("pill");

        let play_pause_btn =
            ui::glyph_button(icons::MEDIA_PLAY, "Play or pause", ui::Kind::Secondary);
        play_pause_btn.add_css_class("pill");

        let next_btn = ui::glyph_button(icons::MEDIA_NEXT, "Next", ui::Kind::Flat);
        next_btn.add_css_class("pill");

        // The transport glyphs read at title size, not the button's body size.
        for b in [&prev_btn, &play_pause_btn, &next_btn] {
            if let Some(l) = b.child().and_downcast::<gtk4::Label>() {
                ui::glyph(&l, ui::Text::Title, ui::Tone::Fg);
            }
        }
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
        let progress_timer: ProgressTimer = Rc::new(RefCell::new(None));

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
        progress_timer: ProgressTimer,
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
        progress_timer: ProgressTimer,
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
        progress_timer: &ProgressTimer,
    ) {
        // Cancel the existing progress timer. The flag both retires the timer
        // on its next tick and makes any in-flight spawn_work callback from it
        // (already dispatched to the background thread) drop its stale result
        // instead of overwriting the new track's just-applied state.
        if let Some(cancelled) = progress_timer.borrow_mut().take() {
            cancelled.set(true);
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
                        glib::timeout_add_local(std::time::Duration::from_millis(500), move || {
                            if cancelled_c.get() {
                                return glib::ControlFlow::Break;
                            }
                            let w_inner = w_c.clone();
                            let cancelled_inner = cancelled_c.clone();
                            spawn_work(
                                || playerctl(&["position"]).and_then(|s| s.parse::<f64>().ok()),
                                move |pos| {
                                    // A track change since this was dispatched marks
                                    // `cancelled`; drop the stale result rather than
                                    // applying it over the new track's state.
                                    if cancelled_inner.get() {
                                        return;
                                    }
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
                        });
                        *progress_timer.borrow_mut() = Some(cancelled);
                    }
                } else {
                    w.progress_bar.set_visible(false);
                    w.time_label.set_visible(false);
                }

                // Play/pause: the one primary action while something plays.
                if ms.status == PlaybackStatus::Playing {
                    w.play_pause_btn.set_label(icons::MEDIA_PAUSE);
                    w.play_pause_btn.add_css_class("primary");
                } else {
                    w.play_pause_btn.set_label(icons::MEDIA_PLAY);
                    w.play_pause_btn.remove_css_class("primary");
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
