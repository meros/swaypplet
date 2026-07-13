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
