use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;

use swaypplet_system::mpris::{Control, MediaState, MprisService, PlaybackStatus, progress};
use swaypplet_ui::ui;
use swaypplet_ui::ui::icons;

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
/// itself (it returns `Break` once the flag is set, or once the position is
/// gone), which would leave a stored id dangling; removing that id later
/// panics inside a glib trampoline and aborts the process. Setting the flag
/// instead lets the timer retire itself on its next tick.
type ProgressTimer = Rc<RefCell<Option<Rc<std::cell::Cell<bool>>>>>;

/// The panel's Now Playing group. State comes from the players over D-Bus
/// (`swaypplet_system::mpris`), as the bar's media mark gets it; the
/// position, which no player signals as it advances, is counted on from the
/// last read while a track plays.
pub struct MediaSection {
    root: gtk4::Box,
    widgets: Rc<Widgets>,
    mpris: Rc<MprisService>,
    progress_timer: ProgressTimer,
}

impl MediaSection {
    pub fn new(mpris: Rc<MprisService>) -> Self {
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
        ui::glyph::adopt(&art_fallback, ui::Text::DisplaySm, ui::Tone::Muted);

        art_frame.append(&art_image);
        art_frame.append(&art_fallback);
        content_row.append(&art_frame);

        // Track info column
        let info_box = ui::vbox(1);
        info_box.set_hexpand(true);
        info_box.set_valign(gtk4::Align::Center);

        let title_label = ui::text("", ui::Text::Body, ui::Tone::Fg);
        swaypplet_ui::ui::set_weight(&title_label, swaypplet_ui::ui::Weight::Strong);
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
        swaypplet_ui::ui::set_numeric(&time_label, true);
        time_label.set_halign(gtk4::Align::End);
        time_label.set_visible(false);

        progress_row.append(&progress_bar);
        progress_row.append(&time_label);
        root.append(&progress_row);

        // ── Controls row ──────────────────────────────────────────────────────
        let controls = ui::hbox(3);
        controls.set_halign(gtk4::Align::Center);
        controls.add_css_class("media-section-controls");

        let prev_btn = ui::button_with(
            ui::Face::Glyph {
                glyph: icons::MEDIA_PREV,
                tooltip: "Previous",
            },
            ui::Kind::Flat,
            ui::Size::Normal,
        );
        prev_btn.add_css_class("pill");

        let play_pause_btn = ui::button_with(
            ui::Face::Glyph {
                glyph: icons::MEDIA_PLAY,
                tooltip: "Play or pause",
            },
            ui::Kind::Secondary,
            ui::Size::Normal,
        );
        play_pause_btn.add_css_class("pill");

        let next_btn = ui::button_with(
            ui::Face::Glyph {
                glyph: icons::MEDIA_NEXT,
                tooltip: "Next",
            },
            ui::Kind::Flat,
            ui::Size::Normal,
        );
        next_btn.add_css_class("pill");

        // The transport glyphs read at title size, not the button's body size.
        for b in [&prev_btn, &play_pause_btn, &next_btn] {
            if let Some(l) = b.child().and_downcast::<gtk4::Label>() {
                ui::glyph::adopt(&l, ui::Text::Title, ui::Tone::Fg);
            }
        }
        controls.append(&prev_btn);
        controls.append(&play_pause_btn);
        controls.append(&next_btn);
        root.append(&controls);

        // Sent on the service's own connection to the player it shows; the
        // player announces the result itself, so nothing is read back.
        for (button, control) in [
            (&prev_btn, Control::Previous),
            (&play_pause_btn, Control::PlayPause),
            (&next_btn, Control::Next),
        ] {
            let mpris = mpris.clone();
            button.connect_clicked(move |_| mpris.control(control));
        }

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

        let progress_timer: ProgressTimer = Rc::new(RefCell::new(None));

        // Pushed by the players themselves: nothing runs while nothing
        // changes.
        {
            let weak = root.downgrade();
            let w = widgets.clone();
            let pt = progress_timer.clone();
            let mpris_c = mpris.clone();
            mpris.connect_change(move || {
                // The service outlives a panel that is torn down.
                if let Some(root) = weak.upgrade() {
                    Self::apply_state(&root, &w, &mpris_c, &pt);
                }
            });
        }

        let section = MediaSection {
            root,
            widgets,
            mpris,
            progress_timer,
        };
        section.refresh();
        section
    }

    fn apply_state(
        root: &gtk4::Box,
        w: &Rc<Widgets>,
        mpris: &Rc<MprisService>,
        progress_timer: &ProgressTimer,
    ) {
        // Cancel the existing progress timer; the flag retires it on its
        // next tick.
        if let Some(cancelled) = progress_timer.borrow_mut().take() {
            cancelled.set(true);
        }

        let state: Option<MediaState> = mpris.snapshot();
        match &state {
            None => {
                root.set_visible(false);
            }
            Some(ms) => {
                root.set_visible(true);

                // Title + artist
                w.title_label.set_label(ms.display_title());
                w.artist_label.set_label(&ms.artist);
                w.artist_label.set_visible(!ms.artist.is_empty());

                // Player badge
                if let Some(name) = ms.player_label() {
                    w.player_badge.set_label(&name);
                    w.player_badge.set_visible(true);
                } else {
                    w.player_badge.set_visible(false);
                }

                // Album art
                let art_shown = if let Some(path) = ms.art_path() {
                    let file = gtk4::gio::File::for_path(&path);
                    w.art_image.set_file(Some(&file));
                    w.art_image.set_visible(true);
                    w.art_fallback.set_visible(false);
                    true
                } else {
                    false
                };
                if !art_shown {
                    w.art_image.set_visible(false);
                    w.art_fallback.set_visible(true);
                }

                // Progress bar + time
                if let (Some(pos), Some(len)) = (mpris.position(), ms.length_secs) {
                    let (fraction, time) = progress(pos, len);
                    w.progress_bar.set_fraction(fraction);
                    w.progress_bar.set_visible(true);
                    w.time_label.set_label(&time);
                    w.time_label.set_visible(true);

                    // Live-update progress while playing
                    if ms.status == PlaybackStatus::Playing {
                        let w_c = w.clone();
                        let mpris_c = mpris.clone();
                        let cancelled = Rc::new(std::cell::Cell::new(false));
                        let cancelled_c = cancelled.clone();
                        glib::timeout_add_local(std::time::Duration::from_millis(500), move || {
                            if cancelled_c.get() {
                                return glib::ControlFlow::Break;
                            }
                            let Some(pos) = mpris_c.position() else {
                                cancelled_c.set(true);
                                return glib::ControlFlow::Break;
                            };
                            let (fraction, time) = progress(pos, len);
                            w_c.progress_bar.set_fraction(fraction);
                            w_c.time_label.set_label(&time);
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
                    ui::set_button_kind(&w.play_pause_btn, ui::Kind::Primary);
                } else {
                    w.play_pause_btn.set_label(icons::MEDIA_PLAY);
                    ui::set_button_kind(&w.play_pause_btn, ui::Kind::Secondary);
                }
            }
        }
    }

    // ── Public API ────────────────────────────────────────────────────────────

    pub fn refresh(&self) {
        Self::apply_state(&self.root, &self.widgets, &self.mpris, &self.progress_timer);
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }
}
