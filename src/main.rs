mod app;
mod elephant;
mod icons;
mod launcher;
mod layer_shell;
mod notifications;
mod osd;
mod panel;
mod polkit;
mod preview;
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
    match args.next().as_deref() {
        Some("polkit-agent") => polkit::run(),
        // Dev-only: render one component (or the whole panel) in a plain window
        // for visual validation. See src/preview.rs and dev/render.sh.
        Some("--preview") => preview::run(args.next().as_deref().unwrap_or("panel")),
        _ => app::run(),
    }
}
