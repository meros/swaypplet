use gdk4::Display;
use gtk4::CssProvider;

pub fn load_css() {
    let provider = CssProvider::new();
    let css = include_str!("../data/style.css");
    provider.load_from_string(css);

    gtk4::style_context_add_provider_for_display(
        &Display::default().expect("Could not get default display"),
        &provider,
        gtk4::STYLE_PROVIDER_PRIORITY_USER,
    );
}
