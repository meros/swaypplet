{
  description = "Swaypplet – a beautiful control center for Sway";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
  };

  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = nixpkgs.legacyPackages.${system};

      runtimeDeps = with pkgs; [
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
      ];
    in
    {
      packages.${system}.default = pkgs.rustPlatform.buildRustPackage {
        pname = "swaypplet";
        version = "0.1.0";
        src = ./.;

        cargoHash = "sha256-HH+Kd5usZX84RcHfCI61rY/zBpfQeU//H8cAPo41JLk=";

        nativeBuildInputs = with pkgs; [
          pkg-config
          wrapGAppsHook4
        ];

        buildInputs = runtimeDeps;

        postInstall = ''
          cat > $out/bin/swaypplet-toggle <<'SCRIPT'
          #!/bin/sh
          PID=$(cat /tmp/swaypplet.pid 2>/dev/null)
          if [ -n "$PID" ] && kill -0 "$PID" 2>/dev/null; then
            kill -USR1 "$PID"
          else
            swaypplet &
          fi
          SCRIPT
          chmod +x $out/bin/swaypplet-toggle

          # Launcher toggle
          cat > $out/bin/swaypplet-launcher <<'SCRIPT'
          #!/bin/sh
          PID=$(cat /tmp/swaypplet.pid 2>/dev/null)
          if [ -n "$PID" ] && kill -0 "$PID" 2>/dev/null; then
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
        '';

        meta = with pkgs.lib; {
          description = "Beautiful control center for Sway";
          license = licenses.mit;
          mainProgram = "swaypplet";
        };
      };

      devShells.${system}.default = pkgs.mkShell {
        nativeBuildInputs = with pkgs; [
          cargo
          rustc
          rust-analyzer
          clippy
          rustfmt
          pkg-config
        ];

        buildInputs = runtimeDeps;

        LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath runtimeDeps;
      };
    };
}
