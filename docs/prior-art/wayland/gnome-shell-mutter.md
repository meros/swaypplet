---
name: GNOME Shell on Mutter
slug: gnome-shell-mutter
domain: wayland
kind: product
platform: GNOME
vendor: GNOME Foundation
years: 2011–present
status: current
tags: [in-process-shell, gjs, extensions, no-layer-shell]
relevance: medium
---

## What it is
The shell UI is JavaScript (GJS) running inside the compositor process on Mutter and Clutter. Extensions are JS that monkey-patch the shell at runtime. GNOME does not implement wlr-layer-shell, so no third-party bar, dock or notification daemon runs on it.

## What was new
- One process for compositor and shell, so shell animations (overview zoom from real windows, workspace slides) are frame-perfect with window state.
- The Activities overview: windows animate from their positions into a grid, still one of the best overview entrances.

## What went wrong / limits
- A slow or buggy JS path stalls the compositor; for years GC pauses and extensions caused frame drops [memory].
- Extensions break with every six-monthly release; the GNOME 45 move to ES modules broke all of them at once [memory]. Pop Shell, Forge and Material Shell all suffered this churn.
- Refusing layer-shell fragments the ecosystem: Ignis, AGS, Quickshell and Waybar explicitly do not support GNOME.
- Mutter gained `ext-background-effect-v1` support in 2026, its first opening to client-requested blur.

## Lessons for swaypplet
- Take the overview entrance (windows fly from real positions), which the roadmap already plans from a frame capture.
- Avoid the in-process shell: a GTK stall in swaypplet only freezes swaypplet.

## Sources
- https://gitlab.gnome.org/GNOME/mutter/-/merge_requests/5071 — ext-background-effect in Mutter [verified via search summary]
- https://github.com/ignis-sh/ignis — "not supported on GNOME" [verified via search summary]
