---
name: Forge and Tiling Shell (GNOME tiling extensions)
slug: gnome-tiling-extensions
domain: wayland
kind: product
platform: GNOME
vendor: jmmaranan (Forge); Domenico Ferraro (Tiling Shell)
years: 2021–present
status: current
tags: [gnome-extension, tiling, snap-assistant, fancyzones, maintainer]
relevance: low
---

## What it is
Forge adds i3/sway-style tree tiling with split containers and vim keys to GNOME. Tiling Shell adds layout-based tiling: Windows 11's Snap Assistant (drag a window to the top edge to pick a tile), PowerToys FancyZones, a layout editor, and multi-monitor support with mixed scaling.

## What was new
- Tiling Shell: the first GNOME port of Snap Assistant; tiling for mouse users, tiles can be spanned.

## What went wrong / limits
- Forge is looking for a new maintainer; an AI-maintained fork carries fixes meanwhile.
- Both carry the GNOME-release breakage cost described under gnome-shell-mutter.

## Lessons for swaypplet
- The snap-assistant gesture (drag to an edge, see targets) is a good pointer affordance for moving a window to a task workspace.

## Sources
- https://github.com/forge-ext/forge — maintainer wanted [verified via search summary]
- https://github.com/domferr/tilingshell — features [verified via search summary]
