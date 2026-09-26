---
name: wlr-layer-shell
slug: layer-shell
domain: wayland
kind: protocol
platform: Wayland
vendor: wlroots project (swaywm/wlr-protocols)
years: 2018–present
status: current
tags: [protocol, panels, anchors, exclusive-zone, keyboard-interactivity]
relevance: high
---

## What it is
`zwlr_layer_shell_v1` lets a client place a surface in one of four layers (background, bottom, top, overlay), anchor it to screen edges, reserve an exclusive zone, and choose keyboard interactivity (none, exclusive, on-demand since v4). Every Wayland bar, dock, launcher, OSD and wallpaper tool depends on it. gtk4-layer-shell and Quickshell's `PanelWindow` wrap it.

## What was new
- A shell component became an ordinary client, so bars are interchangeable across compositors: the root of the whole ecosystem in this domain.
- Namespaces give the compositor a stable handle for per-surface rules (swayfx `layer_effects`, Hyprland `layerrule`).

## What went wrong / limits
- Never adopted by GNOME, so every layer-shell toolkit excludes GNOME; a split that has lasted seven years.
- Still `unstable` in the wlr repo; an `ext-` version has been discussed but not merged [memory].
- No way to ask for effects, exact stacking between two layer surfaces, or per-output behaviour beyond choosing an output.

## Lessons for swaypplet
- Treat namespaces as API: renaming one silently breaks glass and blur rules. Keep them in one generated table.
- On-demand keyboard interactivity is right for panels; exclusive only for lock-like modals.

## Sources
- https://wayland.app/protocols/wlr-layer-shell-unstable-v1 — protocol text [memory]
- https://github.com/ignis-sh/ignis — GNOME unsupported for lack of layer-shell [verified via search summary]
