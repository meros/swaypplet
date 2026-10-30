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
          # org.gnome.desktop.interface, which `theme::apps` writes the
          # appearance to for the desktop portal to serve.
          gsettings-desktop-schemas
          polkit
          pam
          # Audio: crates/services/src/audio.rs speaks the PulseAudio protocol to
          # PipeWire's pulse server instead of parsing `wpctl status`.
          libpulseaudio
          # Live frames on the GPU (crates/jump/src/jump/gpu.rs): EGL through libglvnd,
          # buffers through GBM.
          libglvnd
          libgbm
        ];
    in
    {
      overlays.default = final: prev: {
        swaypplet = self.packages.${final.stdenv.hostPlatform.system}.swaypplet;
      };

      packages = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
          # One derivation per crate (nix/package.nix): a rebuild compiles
          # only what changed.
          swayppletPkg = pkgs.callPackage ./nix/package.nix {
            runtimeDeps = runtimeDepsFor pkgs;
            rev = self.rev or self.dirtyRev or "";
          };
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
              # Cargo.nix, the per-crate Nix build (dev/cargo-nix.sh)
              crate2nix
              # pam-sys generates its libpam bindings at build time
              rustPlatform.bindgenHook

              # Visual harnesses (dev/render.sh, dev/filmstrip.sh): something
              # to grab frames with, and something to turn them into a contact
              # sheet. The compositor is deliberately NOT here: these harnesses
              # need the session's swayfx for the frost, and a plain sway on
              # PATH would shadow it and quietly render everything unblurred.
              grim
              # dev/render.sh's SWPP_KEYS: keys typed into the nested
              # compositor through its virtual keyboard.
              wtype
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
          craneLib = crane.mkLib pkgs;
          root = ./.;
          # The workspace as `cargo test` sees it: every crate with its data,
          # and the design spec the design lint reads.
          src = pkgs.lib.cleanSourceWith {
            src = root;
            filter =
              path: type:
              let
                rel = pkgs.lib.removePrefix (toString root + "/") (toString path);
              in
              rel == "Cargo.toml"
              || rel == "Cargo.lock"
              || rel == "crates"
              || (pkgs.lib.hasPrefix "crates/" rel && builtins.baseNameOf rel != "target")
              || rel == "docs"
              || rel == "docs/design-system.md";
          };
          commonArgs = {
            inherit src;
            pname = "swaypplet";
            version = "0.1.0";
            strictDeps = true;
            nativeBuildInputs = with pkgs; [
              pkg-config
              rustPlatform.bindgenHook
            ];
            buildInputs = runtimeDepsFor pkgs;
          };
        in
        {
          build = self.packages.${system}.swaypplet;

          # Every `#[test]` in the workspace, the way `cargo test` runs them.
          # The crates.io dependencies build once into their own derivation
          # (crane's buildDepsOnly) and stay cached across edits.
          #
          # The `#[ignore]`d tests stay ignored: they want a live sway socket
          # or a sound server, which a build sandbox has neither of.
          tests = craneLib.cargoTest (
            commonArgs
            // {
              cargoArtifacts = craneLib.buildDepsOnly commonArgs;
              cargoTestExtraArgs = "--workspace";
            }
          );
        }
      );
    };
}
