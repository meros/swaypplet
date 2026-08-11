mod alpha;
mod anim;
mod app;
mod avatar;
mod bar;
mod clipboard;
mod dmenu;
mod elephant;
mod fp;
mod gdm_shim;
mod glib_unix;
mod greet;
mod icons;
mod idle;
mod keybinds;
mod launcher;
mod layer_shell;
mod lock;
mod notifications;
mod osd;
mod panel;
mod polkit;
mod preview;
mod service;
mod spawn;
mod surface;
mod sway_ipc;
mod switch_user;
mod task_state;
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
        // ext-session-lock screen locker — standalone process, no GApplication.
        Some("lock") => lock::run(),
        // greetd greeter — standalone process reusing the lock UI.
        Some("greet") => greet::run(),
        // idle manager (swayidle replacement) — standalone process, no GTK.
        Some("idle") => idle::run(),
        // Root fingerprint agent for the greeter (see src/fp/agent.rs) and
        // its pam_exec token-check counterpart in greetd's PAM stack.
        Some("fp-agent") => fp::agent::run_agent(),
        Some("fp-check") => fp::agent::run_check(),
        // Fast user switching: the host command behind every picker chip, and
        // a usable CLI from a getty when the graphical side is unhappy.
        Some("switch-user") => switch_user::run(args),
        // Root D-Bus shim standing in for GDM, so GNOME's "Switch User…"
        // reaches the same greeter as everything else.
        Some("gdm-shim") => gdm_shim::run(),
        // Native status bar (waybar replacement) — own GApplication so it
        // can run standalone next to the panel during the migration.
        Some("bar") => bar::run(),
        // dmenu-style picker — thin client to the panel's picker server,
        // with a standalone GTK fallback when no panel is listening.
        Some("dmenu") => dmenu::run(args),
        // Dev-only: render one component (or the whole panel) in a plain window
        // for visual validation. See src/preview.rs and dev/render.sh.
        Some("--preview") => preview::run(args.next().as_deref().unwrap_or("panel")),
        _ => app::run(),
    }
}
