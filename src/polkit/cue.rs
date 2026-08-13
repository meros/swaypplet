//! The look-at-the-camera cue, for elevation.
//!
//! The card is centred, because that is where a decision belongs. The camera
//! is at the top of the lid, and a face presented off-axis is the single most
//! common reason a good enrolment fails to match. So the thing that reports
//! the face check does not live on the card at all: it lives up by the lens,
//! in the same pill, at the same offset, with the same arrival animation the
//! lock screen uses.
//!
//! Reusing that surface rather than inventing a second one is the point. A
//! user who has learned that the ring by the camera means "hold still" has
//! learned it once, and it means the same thing whether they are unlocking
//! the session or authorising `sudo`.
//!
//! Its own layer surface, and deliberately not part of the polkit window.
//! `swaypplet-polkit` has a `layer_effects` entry, and `blur_ignore_transparent`
//! frosts a partially transparent box-shadow into a flat halo — which is
//! exactly what the attention glow is made of. An unblurred namespace renders
//! it as the shadow it is.

use gtk4::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer};

use crate::layer_shell::{create_layer_window, LayerShellConfig};

static CUE_CONFIG: LayerShellConfig = LayerShellConfig {
    namespace: "swaypplet-face-cue",
    layer: Layer::Overlay,
    exclusive: false,
    default_width: None,
    default_height: None,
    // Top centre: anchoring only the top edge lets the compositor centre it
    // horizontally, which is where the lens is.
    anchors: &[(Edge::Top, true)],
    margins: &[],
    // Never takes the keyboard. The card owns every key that matters, and a
    // grab here would silently steal the Enter meant for the Allow button.
    keyboard_mode: KeyboardMode::None,
};

pub struct Cue {
    window: gtk4::Window,
    pill: gtk4::Box,
    ring: gtk4::Box,
    label: gtk4::Label,
}

impl Cue {
    pub fn new(app: &gtk4::Application) -> Self {
        let window = create_layer_window(app, &CUE_CONFIG);
        window.add_css_class("face-cue");
        window.set_visible(false);

        let pill = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(10)
            .halign(gtk4::Align::Center)
            .valign(gtk4::Align::Start)
            .build();
        pill.add_css_class("face-pill");
        pill.set_margin_top(56);

        let ring = gtk4::Box::builder()
            .width_request(18)
            .height_request(18)
            .valign(gtk4::Align::Center)
            .build();
        ring.add_css_class("face-ring");
        let label = gtk4::Label::builder().label("").build();
        label.add_css_class("face-pill-label");
        pill.append(&ring);
        pill.append(&label);
        window.set_child(Some(&pill));

        Cue {
            window,
            pill,
            ring,
            label,
        }
    }

    /// Show the cue in `state`, or hide it.
    ///
    /// Hiding destroys the CSS animation state, so the next show replays the
    /// arrival from the top — which is the behaviour wanted. The cue has to
    /// arrive to be seen peripherally; a pill that was already there is a
    /// pill the user's eye has already filtered out.
    pub fn set(&self, visible: bool, state: &str, text: &str) {
        if !visible {
            self.window.set_visible(false);
            return;
        }
        for old in ["looking", "dark", "found", "ok", "fail"] {
            self.ring.remove_css_class(&format!("face-ring-{old}"));
            self.pill.remove_css_class(&format!("face-pill-{old}"));
        }
        self.ring.add_css_class(&format!("face-ring-{state}"));
        self.pill.add_css_class(&format!("face-pill-{state}"));
        self.label.set_label(text);
        self.window.set_visible(true);
        // Nothing here is clickable, so nothing here should swallow a click.
        // Without this the pill eats presses aimed at the backdrop behind it,
        // which is the cancel gesture.
        if let Some(surface) = self.window.surface() {
            let empty = gdk4::cairo::Region::create();
            surface.set_input_region(Some(&empty));
        }
    }
}
