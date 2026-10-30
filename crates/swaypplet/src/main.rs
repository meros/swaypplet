fn main() {
    // The commit this binary was built from (crates/base/src/build.rs). Read
    // here, in the one small crate, so a new commit rebuilds only this crate.
    swaypplet_base::build::set_rev(option_env!("SWAYPPLET_REV"));
    // env_logger, plus the tail a report attaches (crates/screenshot/src/quality/logring.rs).
    swaypplet_screenshot::quality::logring::init();
    // A panic note for the crash reporter (crates/screenshot/src/quality/crash.rs).
    swaypplet_screenshot::quality::crash::install_panic_hook();

    // The words below run a process of their own (the polkit agent, the
    // lock screen, the bar on its own…). Anything else falls through to
    // `swaypplet_app::run`, which hands it to the running panel as a
    // request (`launcher`, `osd`, `jump`, `pin`, …; crates/app/src/command.rs).
    let mut args = std::env::args();
    let _argv0 = args.next();
    match args.next().as_deref() {
        Some("polkit-agent") => swaypplet_polkit::run(),
        // ext-session-lock screen locker — standalone process, no GApplication.
        Some("lock") => swaypplet_lock::run(),
        // greetd greeter — standalone process reusing the lock UI.
        Some("greet") => swaypplet_greet::run(),
        // idle manager (swayidle replacement) — standalone process, no GTK.
        Some("idle") => swaypplet_idle::run(),
        // The settings file from a keybind or a script — standalone, no GTK,
        // never a request to the panel (crates/app/src/settings_cli.rs).
        Some("settings") => swaypplet_app::settings_cli::run(args),
        // Root fingerprint agent for the greeter (see crates/fp/src/agent.rs) and
        // its pam_exec token-check counterpart in greetd's PAM stack.
        Some("fp-agent") => swaypplet_fp::agent::run_agent(),
        Some("fp-check") => swaypplet_fp::agent::run_check(),
        // The fingerprint channel of pam_race (nixos repo, pkgs/pam-race):
        // one verification of a named user, started and killed by the PAM
        // module racing it against the camera and the password prompt.
        Some("fp-verify") => swaypplet_fp::verify::run(args),
        // Fast user switching: the host command behind every picker chip, and
        // a usable CLI from a getty when the graphical side is unhappy.
        Some("switch-user") => swaypplet_switch_user::run(args),
        // Root D-Bus shim standing in for GDM, so GNOME's "Switch User…"
        // reaches the same greeter as everything else.
        Some("gdm-shim") => swaypplet_switch_user::gdm_shim::run(),
        // The status bar as its own GApplication. The panel hosts the same
        // bars in-process; this one is for a session without them
        // (`SWAYPPLET_NO_BAR=1`) or without the panel.
        Some("bar") => swaypplet_bar::run(),
        // dmenu-style picker — thin client to the panel's picker server,
        // with a standalone GTK fallback when no panel is listening.
        Some("dmenu") => swaypplet_dmenu::run(args),
        // What systemd's OnFailure= runs when the panel dies: collect, sign,
        // de-duplicate, file (crates/screenshot/src/quality/crash.rs). No GTK.
        Some("crash-report") => swaypplet_screenshot::quality::crash::run(args),
        // The Cast devices on the network, from avahi (swaypplet_cast). No
        // GTK.
        Some("cast") => swaypplet_cast::cli(args),
        // The labels the bar draws for the named workspaces, as JSON. No GTK.
        Some("workspace-labels") => swaypplet_base::sway::workspace::labels_cli(args),
        // Dev-only: render one component (or the whole panel) in a plain window
        // for visual validation. See crates/app/src/preview.rs and dev/render.sh.
        Some("--preview") => swaypplet_app::preview::run(args.next().as_deref().unwrap_or("panel")),
        _ => swaypplet_app::run(),
    }
}
