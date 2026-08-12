{
  description = "Swaypplet – a beautiful control center for Sway";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
  };

  outputs =
    { self, nixpkgs }:
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
          # libwebauthn (src/passkey/) pulls hidapi for the USB authenticator
          # transport, whose build script wants libudev. We only use the cable
          # transport, but the dependency is not feature-gated upstream.
          udev
        ];
    in
    {
      overlays.default = final: prev: {
        swaypplet = final.callPackage ./package.nix { };
      };

      packages = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
        in
        {
          default = pkgs.callPackage ./package.nix { };
          swaypplet = pkgs.callPackage ./package.nix { };
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
