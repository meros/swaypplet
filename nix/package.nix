# swaypplet, one derivation per crate (crate2nix's Cargo.nix on nixpkgs'
# buildRustCrate). An edit rebuilds the crate it is in, the crates that
# depend on it and the small binary crate; every other crate, ours and
# crates.io's, comes from the store. Each workspace crate's source is only
# its own directory (crates/<name>), so its store path, and with it every
# derivation above it, changes only when a file in that directory does.
#
# Cargo.nix is generated. dev/cargo-nix.sh writes it and nix/cargo-nix.stamp;
# the build stops with a message when the stamp no longer matches the
# manifests, instead of building an old dependency graph.
{
  lib,
  pkgs,
  buildRustCrate,
  defaultCrateOverrides,
  xkeyboard_config,
  runtimeDeps,
  rev ? "",
}:

let
  manifests = [
    ../Cargo.toml
    ../Cargo.lock
  ]
  ++ map (c: ../crates + "/${c}/Cargo.toml") (lib.attrNames (builtins.readDir ../crates));
  stamp = builtins.hashString "sha256" (lib.concatMapStrings builtins.readFile manifests);
  fresh = lib.removeSuffix "\n" (builtins.readFile ./cargo-nix.stamp) == stamp;

  # buildRustCrate passes -C opt-level=3 for a release build. A later -C
  # flag wins, so these replace it: the levels of the release profile in
  # Cargo.toml, dependencies at 2 and our own crates at 1 (the frame
  # measurements behind 1 are there).
  depsOpts = [ "-C opt-level=2" ];
  ownOpts = [ "-C opt-level=1" ];
  linkOpts = [
    "-C linker=clang"
    "-C link-arg=-fuse-ld=mold"
    # mold writes no build id by default, and without one systemd-coredump
    # and debuginfod cannot tie a core to its binary.
    "-C link-arg=-Wl,--build-id"
  ];

  # Crates that link a C library, through pkg-config or bindgen.
  isNative =
    name:
    lib.hasSuffix "-sys" name
    || lib.elem name [
      "khronos-egl"
      "gbm"
      "pam"
      "libpulse-binding"
    ];
  native = attrs: {
    nativeBuildInputs = (attrs.nativeBuildInputs or [ ]) ++ [
      pkgs.pkg-config
      pkgs.rustPlatform.bindgenHook
    ];
    buildInputs = (attrs.buildInputs or [ ]) ++ runtimeDeps;
  };

  isOwn = name: lib.hasPrefix "swaypplet-" name;

  # The binary crate: main.rs alone, so the per-commit inputs below
  # recompile only it.
  bin = attrs: {
    extraRustcOpts = (attrs.extraRustcOpts or [ ]) ++ ownOpts ++ linkOpts;
    nativeBuildInputs = (attrs.nativeBuildInputs or [ ]) ++ [
      pkgs.clang
      pkgs.mold
      pkgs.wrapGAppsHook4
    ];
    buildInputs = (attrs.buildInputs or [ ]) ++ runtimeDeps;
    # The commit the Settings · System tab says is running and a crash report
    # names (crates/base/src/build.rs).
    SWAYPPLET_REV = rev;
    # Installed after the fixup phase, so wrapGAppsHook wraps only the
    # binary. The scripts exec it; wrapping each of them too cost 0.8 s
    # apiece (9 s of a 16 s rebuild).
    postFixup = ''
      ${attrs.postFixup or ""}
      install -Dm755 scripts/swaypplet-toggle.sh $out/bin/swaypplet-toggle
      install -Dm755 scripts/swaypplet-launcher.sh $out/bin/swaypplet-launcher
      # A command per verb that sway binds or another program calls
      # (crates/swaypplet/src/main.rs): the OSD client, the polkit agent,
      # screenshots, Jump (Super+Tab), naming a workspace, pinning one, the
      # keybinding sheet and the session locker.
      for verb in osd polkit-agent screenshot jump label pin keybinds lock; do
        printf '#!/bin/sh\nexec %s %s "$@"\n' "$out/bin/swaypplet" "$verb" > "$out/bin/swaypplet-$verb"
        chmod +x "$out/bin/swaypplet-$verb"
      done
      patchShebangs $out/bin
    '';
  };

  overrideFor =
    name: attrs:
    let
      base = (defaultCrateOverrides.${name} or (_: { })) attrs;
      merged = attrs // base;
      opts = extra: { extraRustcOpts = (merged.extraRustcOpts or [ ]) ++ extra; };
    in
    base
    // (
      if name == "swaypplet" then
        bin merged
      else if name == "swaypplet-model" then
        opts ownOpts
        // {
          # The xkb layout list the Input tab offers (crates/model/src/xkb.rs).
          SWAYPPLET_XKB_RULES = "${xkeyboard_config}/share/X11/xkb/rules/evdev.lst";
        }
      else if isOwn name then
        opts ownOpts
      else if isNative name then
        native merged // opts depsOpts
      else
        opts depsOpts
    );

  cargoNix = import ../Cargo.nix {
    inherit pkgs lib;
    buildRustCrateForPkgs = _: buildRustCrate.override { defaultCodegenUnits = 16; };
    defaultCrateOverrides = lib.listToAttrs (
      map (c: lib.nameValuePair c.crateName (overrideFor c.crateName)) (
        lib.attrValues cargoNix.internal.crates
      )
    );
  };
in
if fresh then
  cargoNix.workspaceMembers.swaypplet.build
else
  throw "swaypplet: Cargo.nix is older than Cargo.toml or Cargo.lock. Run dev/cargo-nix.sh and commit Cargo.nix and nix/cargo-nix.stamp."
