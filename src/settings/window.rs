//! Settings' own surface: a glass card the height of the screen, the
//! sidebar and the chosen pane side by side (`SettingsSection`).
//!
//! It wears the Helm's layer namespace, and so the Helm's glass: the Glass
//! pane's sliders change the material of the card they sit on, which is
//! why settings once lived inside the Helm at all. The two are never shown
//! together; the app hides one to show the other.
//!
//! Esc clears a search, then closes. A click on the desktop around the card
//! closes it too.

use std::rc::Rc;

use gtk4::prelude::*;

use super::{Open, SettingsSection};
use crate::shell::{Namespace, Surface};

/// The widest the card grows. Past it, rows of a pane stretch into lines
/// too long to read, and the sidebar drifts from the setting it names.
const MAX_WIDTH: i32 = 1180;
/// Between the card and the screen's edges (the bar's exclusive zone is
/// already outside the surface).
const MARGIN: i32 = 32;

pub struct SettingsWindow {
    surface: Surface,
    section: Rc<SettingsSection>,
}

impl SettingsWindow {
    pub fn new(app: &gtk4::Application) -> Rc<SettingsWindow> {
        let surface = Surface::builder(app, Namespace::Panel)
            .fill()
            .keyboard(gtk4_layer_shell::KeyboardMode::Exclusive)
            .card(crate::ui::Card::Floating)
            .slide(gtk4::Orientation::Vertical, crate::anim::SLIDE_PX)
            .build();
        let section = SettingsSection::new();

        // The card fills the height between the margins; its width follows
        // the output (`fit`).
        let card = surface.card().clone();
        card.add_css_class("settings-card");
        card.set_halign(gtk4::Align::Center);
        card.set_valign(gtk4::Align::Fill);
        card.set_vexpand(true);
        card.set_margin_top(MARGIN);
        card.set_margin_bottom(MARGIN);
        if let Some(slide) = surface.slide() {
            slide.set_vexpand(true);
        }
        card.append(section.widget());
        surface.set_content(section.widget());

        let this = Rc::new(SettingsWindow { surface, section });
        this.wire();
        this
    }

    fn wire(self: &Rc<Self>) {
        let window = self.surface.window().clone();
        {
            let card = self.surface.card().clone();
            window.connect_map(move |w| fit(w, &card));
        }
        {
            // wl_surface.enter comes after the map, and says which output
            // the card is really on.
            let card = self.surface.card().clone();
            window.connect_realize(move |w| {
                let Some(gdk_surface) = w.surface() else {
                    return;
                };
                let (w, card) = (w.clone(), card.clone());
                gdk_surface.connect_enter_monitor(move |_, _| fit(&w, &card));
            });
        }
        {
            let weak = Rc::downgrade(self);
            let keys = gtk4::EventControllerKey::new();
            keys.set_propagation_phase(gtk4::PropagationPhase::Capture);
            keys.connect_key_pressed(move |_, key, _, _| {
                if key != gtk4::gdk::Key::Escape {
                    return glib::Propagation::Proceed;
                }
                if let Some(this) = weak.upgrade()
                    && !this.section.clear_search()
                {
                    this.hide();
                }
                glib::Propagation::Stop
            });
            window.add_controller(keys);
        }
        {
            let weak = Rc::downgrade(self);
            self.surface.connect_backdrop_click(move || {
                if let Some(this) = weak.upgrade() {
                    this.hide();
                }
            });
        }
    }

    /// Show settings, on `what`. The panes re-read what they edit once the
    /// card is in, as the Helm's sections do.
    pub fn open(&self, what: Open) {
        if !self.is_shown() {
            self.surface.show();
            let section = self.section.clone();
            glib::timeout_add_local_once(
                std::time::Duration::from_millis(
                    (crate::anim::duration(crate::anim::ENTER_MS) + 60.0) as u64,
                ),
                move || section.refresh(),
            );
        }
        self.section.open(&what);
        self.section.focus_search();
        // Harness hook: the nested session in dev/render.sh has no
        // keyboard, so `SWAYPPLET_SETTINGS_QUERY` types a search on open,
        // for a shot of the hits.
        if let Ok(query) = std::env::var("SWAYPPLET_SETTINGS_QUERY")
            && !query.is_empty()
        {
            let section = self.section.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(300), move || {
                section.search_text(&query)
            });
        }
    }

    pub fn hide(&self) {
        if self.is_shown() {
            self.surface.hide();
        }
    }

    pub fn is_shown(&self) -> bool {
        self.surface.is_shown() && self.surface.window().is_visible()
    }

    /// What a Helm page among the search hits opens: the app hides settings
    /// and shows the Helm on it.
    pub fn set_on_page(&self, f: impl Fn(&str) + 'static) {
        self.section.set_on_page(f);
    }

    pub fn window(&self) -> &gtk4::Window {
        self.surface.window()
    }
}

/// The card's width on the output the window is on.
fn fit(window: &gtk4::Window, card: &gtk4::Box) {
    let monitor = window
        .surface()
        .and_then(|s| gtk4::prelude::WidgetExt::display(window).monitor_at_surface(&s));
    let Some(monitor) = monitor else { return };
    let width = (monitor.geometry().width() - 2 * MARGIN).min(MAX_WIDTH);
    card.set_width_request(width.max(480));
}
