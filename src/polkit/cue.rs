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

use crate::layer_shell::{self, create_layer_window, LayerShellConfig};

static CUE_CONFIG: LayerShellConfig = LayerShellConfig {
    namespace: "swaypplet-face-cue",
    layer: Layer::Overlay,
    exclusive: false,
    default_width: None,
    // Explicit, so the surface never asks the compositor to choose. A layer
    // surface that requests zero on an unanchored axis gets the whole output,
    // and a full-screen surface above the card with a live input region would
    // swallow every click aimed at it.
    default_height: Some(180),
    // Top strip, full width. The pill centres itself inside it, under the
    // lens.
    // Left and right anchored too, so the surface spans the output and its
    // geometry never depends on the pill inside it. A surface that resized
    // with its content renegotiated size mid-animation, and clipped the glow
    // -- a 26px blur with 5px spread reaches well past the pill's own box.
    anchors: &[(Edge::Top, true), (Edge::Left, true), (Edge::Right, true)],
    margins: &[],
    // Never takes the keyboard. The card owns every key that matters, and a
    // grab here would silently steal the Enter meant for the Allow button.
    keyboard_mode: KeyboardMode::None,
};

pub struct Cue {
    window: gtk4::Window,
    /// Carries the entrance animation, so a state change on the pill cannot
    /// replay it. See .face-pill-enter in the stylesheet.
    strip: gtk4::Box,
    pill: gtk4::Box,
    ring: gtk4::Box,
    label: gtk4::Label,
}

impl Cue {
    pub fn new(app: &gtk4::Application) -> Self {
        let window = create_layer_window(app, &CUE_CONFIG);
        window.add_css_class("face-cue");
        window.set_visible(false);
        // Re-applied on every map: the region lives on the GdkSurface, which
        // is created at map and re-laid-out on output changes.
        window.connect_map(|w| clear_input_region(w));

        let pill = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(10)
            .halign(gtk4::Align::Center)
            .valign(gtk4::Align::Start)
            .build();
        pill.add_css_class("face-pill");
        pill.set_margin_top(56);

        // A fixed strip to hold it. Height is the pill's offset plus its own
        // height plus room for the glow and the 18px the entrance travels;
        // sizing to the content instead would clip both and make the surface
        // resize on every animation frame.
        let strip = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .height_request(180)
            .build();
        strip.append(&pill);

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
        window.set_child(Some(&strip));

        Cue {
            window,
            strip,
            pill,
            ring,
            label,
        }
    }

    /// Show the cue in `state`, or hide it.
    ///
    /// Always on the built-in panel, because that is the screen with the lens
    /// over it. Left to the compositor this lands on whichever output has
    /// focus, and on a docked laptop that is usually the external monitor —
    /// a cue that says "look here" while pointing the user's face away from
    /// the sensor trying to read it.
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
        if self.window.is_visible() {
            // Already up: this is a state change, not an arrival. Touching the
            // entrance here is what made the pill jump on every frame state.
            return;
        }
        // Re-resolved on every show: outputs come and go, and a monitor
        // handle kept from startup can name one the compositor has since
        // forgotten. Set while hidden, which is the only time layer-shell
        // will take it.
        if let Some(monitor) = layer_shell::internal_monitor() {
            gtk4_layer_shell::LayerShell::set_monitor(&self.window, Some(&monitor));
        }
        // Map first, animate on the next frame the compositor gives us. The
        // frame clock does not tick until the surface is mapped and drawing,
        // so starting the entrance from here would spend its first frames
        // racing surface allocation -- which is visible, and always as a
        // stutter at exactly the moment the cue is trying to catch the eye.
        self.strip.remove_css_class("face-pill-enter");
        self.window.set_visible(true);
        let strip = self.strip.clone();
        self.window.add_tick_callback(move |window, _| {
            strip.add_css_class("face-pill-enter");
            // Re-applied here as well as below: the region is a property of
            // the GdkSurface, which does not exist until the window maps, and
            // the strip spans the whole width of the output. An unset region
            // means a 180px band across the top of the screen eating every
            // press aimed at whatever is behind it.
            clear_input_region(window);
            glib::ControlFlow::Break
        });
        clear_input_region(&self.window);
    }
}

/// Make a window transparent to input.
///
/// Nothing on the cue is clickable, so nothing on it should swallow a click.
/// It spans the full width of the output, so without this it is a band across
/// the top of the screen that eats presses aimed at whatever is behind it --
/// including the card's own backdrop, whose click is the cancel gesture.
fn clear_input_region(window: &gtk4::Window) {
    if let Some(surface) = window.surface() {
        surface.set_input_region(Some(&gdk4::cairo::Region::create()));
    }
}
