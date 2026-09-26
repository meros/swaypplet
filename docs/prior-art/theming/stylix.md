---
name: Stylix
slug: stylix
domain: theming
kind: project
platform: other
vendor: danth / nix-community
years: 2022–present
status: current
tags: [nixos, home-manager, base16, declarative, wallpaper, fonts]
relevance: high
---

## What it is
A NixOS / Home Manager / nix-darwin module that applies one base16 scheme, wallpaper, fonts, cursor and opacity to many applications ("targets": GTK, Qt, kitty, foot, sway, Firefox, Neovim, swaylock …). The scheme can be given as a base16 YAML or generated from the wallpaper at build time.

## What was new
- Theming as system configuration: one attribute set, rebuilt, and every supported program is consistent; `stylix.targets.<x>.enable` per app.
- `config.lib.stylix.colors` exposes the palette to hand-written modules.
- Polarity option (dark/light/either) fed to the wallpaper-derived scheme.

## What went wrong / limits
- Changing theme needs a rebuild; no runtime dark/light switching or following the sun.
- base16's 16 slots flatten roles; contrast depends on the scheme chosen.
- Targets break as upstream apps change config formats; maintenance load is high.

## Lessons for swaypplet
- swaypplet's settings layer (Nix defaults + runtime user file) is the runtime half Stylix lacks. A Stylix-compatible output (a base16 scheme generated from swaypplet's tokens for the current mode, written to the user's cache) lets the rest of a NixOS system follow without taking on Stylix's rebuild loop.
- Or the reverse: read a Stylix scheme as the Nix default for neutral/accent, for users who already have one.

## Sources
- https://github.com/nix-community/stylix — project, targets [verified via search summary]
- https://nix-community.github.io/stylix/configuration.html — base16Scheme, lib.stylix.colors [verified via search summary]
