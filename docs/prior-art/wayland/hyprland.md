---
name: Hyprland
slug: hyprland
domain: wayland
kind: product
platform: Wayland
vendor: Vaxry (vaxerski) and hyprwm
years: 2022–present
status: current
tags: [compositor, animations, dynamic-tiling, ipc, ecosystem]
relevance: medium
---

## What it is
A dynamic tiling compositor built for looks: bezier-curve animations for windows, workspaces, layers and fades, blur, rounded corners, shadows, gradients on borders. It left wlroots in 2024 for its own backend (aquamarine) and grew an ecosystem of `hypr*` tools (hyprlock, hypridle, hyprpaper, hyprsunset, hyprpicker) and C++ plugins.

## What was new
- Animation curves as first-class config (`bezier = name, x1, y1, x2, y2`; `animation = windows, 1, 7, name, popin 80%`), including layer-shell surfaces, so shells get enter/exit motion for free.
- Two IPC sockets: a request socket (`hyprctl`, JSON) and `socket2`, a line-oriented event stream that bars can read with `socat`.
- Per-namespace `layerrule` (blur, ignorealpha threshold) that solves "blur only the opaque part of a layer".

## What went wrong / limits
- Config syntax and window rules change across releases; dotfiles and shells (HyprPanel, end-4) break on upgrades.
- Compositor-specific features pull shells into Hyprland lock-in; most riced shells had to add niri/sway backends later.
- Single-maintainer-led governance has drawn community friction over the years [memory].

## Lessons for swaypplet
- Adapt the `ignorealpha` idea: blur where alpha exceeds a threshold, which is the fix for swayfx's binary frost on fading surfaces.
- Avoid config churn: keep swaypplet's settings schema versioned and migrated, never renamed silently.

## Sources
- https://wiki.hypr.land/ — animations, layer rules, IPC [memory]
- https://github.com/hyprwm/Hyprland — aquamarine backend since 0.42 [memory]
