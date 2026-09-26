---
name: Hyprland animation tree
slug: hyprland-animations
domain: motion
kind: feature
platform: Wayland
vendor: Hyprland (vaxry and contributors)
years: 2022–present
status: current
tags: [bezier, spring, animation-tree, config, deciseconds, styles]
relevance: medium
---

## What it is
Hyprland configures motion as a tree of named animations (`windows`, `windowsIn`, `workspaces`, `fade`, `border`, `layers` and children). Each leaf gets on/off, a speed, a named curve and a style (`slide`, `popin 80%`, `slidefade 20%`). Curves are user-defined Béziers and, in the current Lua config, springs with mass, stiffness and damping.

## What was new
Inheritance: set `windows` once and every child follows unless overridden, which keeps a user's config small. Named curves are shared across leaves, like tokens. `speed` is in deciseconds (1 = 100 ms).

## What went wrong / limits
- Deciseconds are coarse; 150 ms needs `1.5`, and many published configs use overshooting Béziers (`0.05, 0.9, 0.1, 1.05`) that bounce everything.
- The community "rice" culture spreads configs tuned for screenshots, not use; long bouncy animations are the norm in shared dotfiles.

## Lessons for swaypplet
- Take: the tree-with-inheritance shape for any user-facing motion setting (surface class, then surface), if swaypplet ever exposes one.
- Avoid: exposing raw Bézier control points to users; expose the token names.

## Sources
- https://github.com/hyprwm/hyprland-wiki/blob/main/content/configuring/core/animations.md — tree, deciseconds, bezier and spring curves, styles [verified]
