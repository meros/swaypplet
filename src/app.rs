use std::cell::RefCell;
use std::fs;
use std::rc::Rc;

use glib::unix_signal_add_local;
use gtk4::prelude::*;
use gtk4::Application;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

use crate::panel::Panel;
use crate::theme;

const APP_ID: &str = "dev.swaypplet.panel";

pub fn run() {
    let app = Application::builder().application_id(APP_ID).build();

    let panel: Rc<RefCell<Option<Panel>>> = Rc::new(RefCell::new(None));

    let panel_clone = panel.clone();
    app.connect_startup(move |_app| {
        // Write PID file
        let _ = fs::write("/tmp/swaypplet.pid", std::process::id().to_string());
        theme::load_css();

        // SIGUSR1 toggles visibility
        let p = panel_clone.clone();
        unix_signal_add_local(10 /* SIGUSR1 */, move || {
            if let Some(panel) = p.borrow().as_ref() {
                panel.toggle();
            }
            glib::ControlFlow::Continue
        });
    });

    let panel_clone = panel.clone();
    app.connect_activate(move |app| {
        let mut panel_ref = panel_clone.borrow_mut();
        if let Some(p) = panel_ref.as_ref() {
            p.toggle();
            return;
        }

        let window = gtk4::Window::builder()
            .application(app)
            .default_width(380)
            .resizable(false)
            .build();
        window.add_css_class("panel");

        // Layer shell MUST be initialized before the window is mapped
        window.init_layer_shell();
        window.set_layer(Layer::Overlay);
        window.set_namespace("swaypplet");
        window.set_anchor(Edge::Bottom, true);
        window.set_anchor(Edge::Right, true);
        window.set_margin(Edge::Bottom, 48);
        window.set_margin(Edge::Right, 8);
        window.set_keyboard_mode(KeyboardMode::OnDemand);

        let p = Panel::new(window);
        p.window.present();
        *panel_ref = Some(p);
    });

    app.connect_shutdown(|_| {
        let _ = fs::remove_file("/tmp/swaypplet.pid");
    });

    app.run();
}
