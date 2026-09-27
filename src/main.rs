mod alpha;
mod anim;
mod app;
mod frame_stats;
mod watch;
mod auth_field;
mod bar;
#[cfg(test)]
mod design_lint;
mod dmenu;
mod face;
mod fp;
mod gdm_shim;
mod glib_unix;
mod greet;
mod handoff;
mod idle;
mod jump;
mod keybinds;
mod launcher;
mod lock;
mod notifications;
mod osd;
mod panel;
mod polkit;
mod preview;
mod quality;
mod screenshot;
mod service;
mod services;
mod settings;
mod shell;
mod spawn;
mod sway;
mod switch_user;
mod theme;
mod tokens;
mod ui;
mod widgets;

fn main() {
    // env_logger, plus the tail a report attaches (src/quality/logring.rs).
    quality::logring::init();
    // A panic note for the crash reporter (src/quality/crash.rs).
    quality::crash::install_panic_hook();

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
        // The settings file from a keybind or a script — standalone, no GTK,
        // never a request to the panel (src/settings/cli.rs).
        Some("settings") => settings::cli::run(args),
        // Root fingerprint agent for the greeter (see src/fp/agent.rs) and
        // its pam_exec token-check counterpart in greetd's PAM stack.
        Some("fp-agent") => fp::agent::run_agent(),
        Some("fp-check") => fp::agent::run_check(),
        // The fingerprint channel of pam_race (nixos repo, pkgs/pam-race):
        // one verification of a named user, started and killed by the PAM
        // module racing it against the camera and the password prompt.
        Some("fp-verify") => fp::verify::run(args),
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
        // What systemd's OnFailure= runs when the panel dies: collect, sign,
        // de-duplicate, file (src/quality/crash.rs). No GTK.
        Some("crash-report") => quality::crash::run(args),
        // Dev-only: render one component (or the whole panel) in a plain window
        // for visual validation. See src/preview.rs and dev/render.sh.
        Some("--preview") => preview::run(args.next().as_deref().unwrap_or("panel")),
        _ => app::run(),
    }
}
