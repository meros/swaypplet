---
name: matugen
slug: matugen
domain: theming
kind: project
platform: cross
vendor: InioX
years: 2023–present
status: current
tags: [material-you, wallpaper, templates, rust, base16, hyprland]
relevance: high
---

## What it is
A Rust CLI that runs Material's colour utilities on an image or colour and writes Material You roles (and base16) into user templates (`{{colors.primary.default.hex}}`), then optionally reloads apps and sets the wallpaper. Popular in Hyprland/niri shells (Quickshell, DankMaterialShell, end-4 dots).

## What was new
- Material 3 roles, not ANSI slots, as the template vocabulary; any scheme variant (tonal-spot, vibrant, content …) and contrast level.
- Harmonised custom colours: user-defined colours rotated toward the source, like MCU `harmonize`.
- A community template repo (matugen-themes) for GTK, Qt, kitty, foot, Firefox, Discord.

## What went wrong / limits
- Material's neutral tint and TonalSpot look are not everyone's taste; heavy-tinted surfaces are common complaints in shared rices.
- Templates duplicate per app; there is no contract that an app's template actually passes contrast.

## Lessons for swaypplet
- Closest open-source analogue on Wayland. swaypplet's differentiator is the glass-aware contrast test; matugen's differentiator is reaching every other app. The cheapest win is a matugen-compatible JSON export of swaypplet's tokens so users can reuse matugen-themes templates.

## Sources
- https://github.com/InioX/matugen — project, templates [verified via search summary]
- https://github.com/InioX/matugen/wiki/Configuration — config, custom colours [verified via search summary]
- https://danklinux.com/docs/dankmaterialshell/application-themes — shell integration [verified via search summary]
