fn main() {
    // The commit this binary was built from (crates/core/src/build.rs). Read
    // here, in the one small crate, so a new commit rebuilds only this crate.
    swaypplet_core::build::set_rev(option_env!("SWAYPPLET_REV"));
    // env_logger, plus the tail a report attaches (crates/screenshot/src/quality/logring.rs).
    swaypplet_screenshot::quality::logring::init();
    // A panic note for the crash reporter (crates/screenshot/src/quality/crash.rs).
    swaypplet_screenshot::quality::crash::install_panic_hook();

    // The polkit agent runs as its own GApplication so it coexists with
    // the main panel process. Anything else falls through to `swaypplet_app::app::run`,
    // which itself does subcommand routing for `osd` / `launcher`.
    let mut args = std::env::args();
    let _argv0 = args.next();
    match args.next().as_deref() {
        Some("polkit-agent") => swaypplet_polkit::run(),
        // ext-session-lock screen locker — standalone process, no GApplication.
        Some("lock") => swaypplet_lock::lock::run(),
        // greetd greeter — standalone process reusing the lock UI.
        Some("greet") => swaypplet_lock::greet::run(),
        // idle manager (swayidle replacement) — standalone process, no GTK.
        Some("idle") => swaypplet_lock::idle::run(),
        // The settings file from a keybind or a script — standalone, no GTK,
        // never a request to the panel (crates/settings/src/settings/cli.rs).
        Some("settings") => swaypplet_settings::settings::cli::run(args),
        // Root fingerprint agent for the greeter (see crates/lock/src/fp/agent.rs) and
        // its pam_exec token-check counterpart in greetd's PAM stack.
        Some("fp-agent") => swaypplet_lock::fp::agent::run_agent(),
        Some("fp-check") => swaypplet_lock::fp::agent::run_check(),
        // The fingerprint channel of pam_race (nixos repo, pkgs/pam-race):
        // one verification of a named user, started and killed by the PAM
        // module racing it against the camera and the password prompt.
        Some("fp-verify") => swaypplet_lock::fp::verify::run(args),
        // Fast user switching: the host command behind every picker chip, and
        // a usable CLI from a getty when the graphical side is unhappy.
        Some("switch-user") => swaypplet_lock::switch_user::run(args),
        // Root D-Bus shim standing in for GDM, so GNOME's "Switch User…"
        // reaches the same greeter as everything else.
        Some("gdm-shim") => swaypplet_lock::gdm_shim::run(),
        // Native status bar (waybar replacement) — own GApplication so it
        // can run standalone next to the panel during the migration.
        Some("bar") => swaypplet_bar::run(),
        // dmenu-style picker — thin client to the panel's picker server,
        // with a standalone GTK fallback when no panel is listening.
        Some("dmenu") => swaypplet_app::dmenu::run(args),
        // What systemd's OnFailure= runs when the panel dies: collect, sign,
        // de-duplicate, file (crates/screenshot/src/quality/crash.rs). No GTK.
        Some("crash-report") => swaypplet_screenshot::quality::crash::run(args),
        // The Cast devices on the network, from avahi (swaypplet_services::cast). No
        // GTK.
        Some("cast") => swaypplet_services::cast::cli(args),
        // The labels the bar draws for the named workspaces, as JSON. No GTK.
        Some("workspace-labels") => swaypplet_core::sway::workspace::labels_cli(args),
        // Dev-only: render one component (or the whole panel) in a plain window
        // for visual validation. See crates/app/src/preview.rs and dev/render.sh.
        Some("--preview") => swaypplet_app::preview::run(args.next().as_deref().unwrap_or("panel")),
        _ => swaypplet_app::app::run(),
    }
}
