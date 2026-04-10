mod app;
mod elephant;
mod icons;
mod launcher;
mod layer_shell;
mod notifications;
mod osd;
mod panel;
mod polkit;
mod spawn;
mod theme;
mod widgets;

fn main() {
    env_logger::init();

    // The polkit agent runs as its own GApplication so it coexists with
    // the main panel process. Anything else falls through to `app::run`,
    // which itself does subcommand routing for `osd` / `launcher`.
    let mut args = std::env::args();
    let _argv0 = args.next();
    if matches!(args.next().as_deref(), Some("polkit-agent")) {
        polkit::run();
    } else {
        app::run();
    }
}
