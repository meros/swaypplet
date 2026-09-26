---
name: wofi
slug: wofi
domain: launchers
kind: product
platform: Wayland
vendor: Scoopta
years: 2019–present
status: niche
tags: [gtk3, layer-shell, rofi-like, dmenu, css]
relevance: low
---

## What it is
A GTK3 launcher and dmenu replacement for wlroots compositors, the first common "rofi for Wayland". Modes: drun, run, dmenu; styled with GTK CSS.

## What was new
Showed that a layer-shell GTK surface with CSS theming covers most rofi use on Wayland.

## What went wrong / limits
- GTK3 start-up cost is visible next to fuzzel/tofi; fuzzy matching and multi-line layout are basic.
- Slow development on sourcehut; many sway/Hyprland users moved to fuzzel, tofi or walker.

## Lessons for swaypplet
- A GTK launcher must be resident, not spawned per use: swaypplet's in-process `LauncherView` already avoids wofi's per-open start cost.

## Sources
- https://hg.sr.ht/~scoopta/wofi — project [memory]
