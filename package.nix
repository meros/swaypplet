{
  lib,
  rustPlatform,
  pkg-config,
  wrapGAppsHook4,
  mold,
  clang,
  gtk4,
  gtk4-layer-shell,
  glib,
  cairo,
  pango,
  harfbuzz,
  gdk-pixbuf,
  graphene,
  hicolor-icon-theme,
  adwaita-icon-theme,
  polkit,
  pam,
  libpulseaudio,
}:

let
  runtimeDeps = [
    gtk4
    gtk4-layer-shell
    glib
    cairo
    pango
    harfbuzz
    gdk-pixbuf
    graphene
    hicolor-icon-theme
    adwaita-icon-theme
    polkit
    pam
    libpulseaudio
  ];
in
rustPlatform.buildRustPackage {
  pname = "swaypplet";
  version = "0.1.0";
  src = ./.;

  # Vendor per-crate via importCargoLock (each crate a fetchurl FOD,
  # almost all already in the store / on cache.nixos.org) instead of
  # buildRustPackage's default bulk cargo-vendor fetcher, which hits
  # the crates.io API and now 403s (blocked/rate-limited User-Agent).
  # No git deps in Cargo.lock, so lockFile alone suffices.
  cargoLock.lockFile = ./Cargo.lock;

  nativeBuildInputs = [
    pkg-config
    wrapGAppsHook4
    # pam-sys generates its libpam bindings at build time
    rustPlatform.bindgenHook
    mold
    clang
  ];

  buildInputs = runtimeDeps;
  doCheck = false;

  # Build optimization: fast release codegen + mold parallel linking
  env = {
    CARGO_PROFILE_RELEASE_CODEGEN_UNITS = "16";
    CARGO_PROFILE_RELEASE_DEBUG = "0";
    CARGO_PROFILE_RELEASE_LTO = "false";
    CARGO_PROFILE_RELEASE_OPT_LEVEL = "2";
    RUSTFLAGS = "-C linker=clang -C link-arg=-fuse-ld=mold";
  };

  postInstall = ''
    cat > $out/bin/swaypplet-toggle <<'SCRIPT'
    #!/bin/sh
    PID=$(cat "''${XDG_RUNTIME_DIR:-/tmp}/swaypplet.pid" 2>/dev/null)
    # comm check: a recycled pid must not get a (default-fatal) USR1
    if [ -n "$PID" ] && [ "$(cat /proc/$PID/comm 2>/dev/null)" = "swaypplet" ]; then
      kill -USR1 "$PID"
    else
      swaypplet &
    fi
    SCRIPT
    chmod +x $out/bin/swaypplet-toggle

    # Launcher toggle
    cat > $out/bin/swaypplet-launcher <<'SCRIPT'
    #!/bin/sh
    PID=$(cat "''${XDG_RUNTIME_DIR:-/tmp}/swaypplet.pid" 2>/dev/null)
    # comm check: a recycled pid must not get a (default-fatal) USR2
    if [ -n "$PID" ] && [ "$(cat /proc/$PID/comm 2>/dev/null)" = "swaypplet" ]; then
      kill -USR2 "$PID"
    else
      swaypplet launcher &
    fi
    SCRIPT
    chmod +x $out/bin/swaypplet-launcher

    # OSD client — drop-in replacement for swayosd-client
    cat > $out/bin/swaypplet-osd <<SCRIPT
    #!/bin/sh
    exec $out/bin/swaypplet osd "\$@"
    SCRIPT
    chmod +x $out/bin/swaypplet-osd

    # Polkit authentication agent — runs as its own process
    cat > $out/bin/swaypplet-polkit-agent <<SCRIPT
    #!/bin/sh
    exec $out/bin/swaypplet polkit-agent "\$@"
    SCRIPT
    chmod +x $out/bin/swaypplet-polkit-agent

    # Screenshots — region / screen / pick, all through the running panel
    cat > $out/bin/swaypplet-screenshot <<SCRIPT
    #!/bin/sh
    exec $out/bin/swaypplet screenshot "\$@"
    SCRIPT
    chmod +x $out/bin/swaypplet-screenshot

    # Window switcher — thumbnails of every window, one keypress away
    cat > $out/bin/swaypplet-switcher <<SCRIPT
    #!/bin/sh
    exec $out/bin/swaypplet switcher "\$@"
    SCRIPT
    chmod +x $out/bin/swaypplet-switcher

    # Keybinding sheet — `show` / `hide` edges from the Super-hold watcher
    cat > $out/bin/swaypplet-keybinds <<SCRIPT
    #!/bin/sh
    exec $out/bin/swaypplet keybinds "\$@"
    SCRIPT
    chmod +x $out/bin/swaypplet-keybinds

    # Session locker — swaylock replacement (ext-session-lock-v1 + PAM)
    cat > $out/bin/swaypplet-lock <<SCRIPT
    #!/bin/sh
    exec $out/bin/swaypplet lock "\$@"
    SCRIPT
    chmod +x $out/bin/swaypplet-lock
  '';

  meta = {
    description = "Beautiful control center for Sway";
    license = lib.licenses.mit;
    mainProgram = "swaypplet";
  };
}
