{
  description = "Swaypplet – a beautiful control center for Sway";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    crane.url = "github:ipetkov/crane";
  };

  outputs =
    { self, nixpkgs, crane }:
    let
      forAllSystems = nixpkgs.lib.genAttrs [
        "x86_64-linux"
        "aarch64-linux"
      ];

      pkgsFor = system: nixpkgs.legacyPackages.${system};

      runtimeDepsFor =
        pkgs: with pkgs; [
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
          # Audio: src/audio.rs speaks the PulseAudio protocol to
          # PipeWire's pulse server instead of parsing `wpctl status`.
          libpulseaudio
        ];
    in
    {
      overlays.default = final: prev: {
        swaypplet = self.packages.${final.system}.swaypplet;
      };

      packages = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
          craneLib = crane.mkLib pkgs;
          runtimeDeps = runtimeDepsFor pkgs;

          # Filter source files, ensuring proto and data directories are included
          protoOrCargo = path: type:
            (builtins.match ".*proto$" path != null) ||
            (builtins.match ".*css$" path != null) ||
            (craneLib.filterCargoSources path type);

          src = pkgs.lib.cleanSourceWith {
            src = ./.;
            filter = protoOrCargo;
          };

          commonArgs = {
            inherit src;
            strictDeps = true;
            nativeBuildInputs = with pkgs; [
              pkg-config
              wrapGAppsHook4
              rustPlatform.bindgenHook
              mold
              clang
            ];
            buildInputs = runtimeDeps;
            CARGO_PROFILE_RELEASE_CODEGEN_UNITS = "16";
            CARGO_PROFILE_RELEASE_DEBUG = "0";
            CARGO_PROFILE_RELEASE_LTO = "false";
            CARGO_PROFILE_RELEASE_OPT_LEVEL = "2";
            RUSTFLAGS = "-C linker=clang -C link-arg=-fuse-ld=mold";
          };

          # 1. Build & cache ALL third-party dependencies independently into /nix/store
          cargoArtifacts = craneLib.buildDepsOnly commonArgs;

          # 2. Build swaypplet itself in seconds by reusing cargoArtifacts
          swayppletPkg = craneLib.buildPackage (commonArgs // {
            inherit cargoArtifacts;
            postInstall = ''
              cat > $out/bin/swaypplet-toggle <<'SCRIPT'
              #!/bin/sh
              PID=$(cat "''${XDG_RUNTIME_DIR:-/tmp}/swaypplet.pid" 2>/dev/null)
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

              # Keybinding sheet — show / hide edges from the Super-hold watcher
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

              mkdir -p $out/share/swaypplet
              cp -r data/* $out/share/swaypplet/
            '';
          });
        in
        {
          default = swayppletPkg;
          swaypplet = swayppletPkg;
        }
      );

      devShells = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
          runtimeDeps = runtimeDepsFor pkgs;
        in
        {
          default = pkgs.mkShell {
            nativeBuildInputs = with pkgs; [
              cargo
              rustc
              rust-analyzer
              clippy
              rustfmt
              pkg-config
              # High-speed compilation & linking tooling
              mold
              clang
              sccache
              # pam-sys generates its libpam bindings at build time
              rustPlatform.bindgenHook

              # Visual harnesses (dev/render.sh, dev/filmstrip.sh): something
              # to grab frames with, and something to turn them into a contact
              # sheet. The compositor is deliberately NOT here: these harnesses
              # need the session's swayfx for the frost, and a plain sway on
              # PATH would shadow it and quietly render everything unblurred.
              grim
              imagemagick
              # montage labels the frames, and ImageMagick has no font to do
              # it with unless one is on the path.
              dejavu_fonts
              ffmpeg
              libnotify
              dbus
            ];

            buildInputs = runtimeDeps;

            LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath runtimeDeps;
            RUSTFLAGS = "-C linker=clang -C link-arg=-fuse-ld=mold";
            RUSTC_WRAPPER = "sccache";
          };
        }
      );

      checks = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
        in
        {
          build = pkgs.callPackage ./package.nix { };
        }
      );
    };
}
