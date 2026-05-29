use gdk4::Display;
use gtk4::CssProvider;

pub fn load_css() {
    let provider = CssProvider::new();
    // Dev override: SWAYPPLET_CSS=<path> loads the stylesheet from disk at
    // runtime instead of the baked-in copy, so the render harness can iterate
    // on style.css without recompiling. Production runs leave it unset.
    match std::env::var_os("SWAYPPLET_CSS") {
        Some(path) => provider.load_from_path(path),
        None => provider.load_from_string(include_str!("../data/style.css")),
    }

    gtk4::style_context_add_provider_for_display(
        &Display::default().expect("Could not get default display"),
        &provider,
        gtk4::STYLE_PROVIDER_PRIORITY_USER,
    );
}
