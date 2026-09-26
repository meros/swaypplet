---
name: Variable refresh rate for desktop UI
slug: variable-refresh-desktop
domain: motion
kind: feature
platform: cross
vendor: VESA, AMD, NVIDIA; sway, KWin, Mutter
years: 2014–present
status: current
tags: [vrr, adaptive-sync, freesync, lfc, flicker, frame-pacing]
relevance: medium
---

## What it is
Adaptive Sync (VESA, FreeSync, G-Sync) lets the display wait for a frame instead of refreshing on a fixed clock. On Linux, sway has `output * adaptive_sync on`, KWin offers "Automatic/Always", Mutter gained experimental VRR in GNOME 46, Hyprland has `misc:vrr` with fullscreen-only modes.

## What was new
For games, a late frame shows a few ms late rather than a whole refresh late. For desktops, VRR can lower refresh during idle and save power, and a slightly late animation frame shows as a small hitch instead of a doubled frame.

## What went wrong / limits
- Many panels flicker in brightness when refresh swings, especially at low rates and with dark UI; compositors default VRR to fullscreen-only for that reason (Hyprland `vrr 2`).
- Cursor and animation smoothness need a steady rate; low-framerate compensation doubles frames below the panel's minimum.
- Frame-time gates designed around a fixed interval misclassify frames under VRR.

## Lessons for swaypplet
- Avoid: VRR for desktop animation by default; the flicker is worse than the hitch.
- Take: make frame-bench measure interval jitter against the output's nominal refresh and note in its output whether adaptive sync was on.

## Sources
- https://github.com/hyprwm/hyprland-wiki/blob/main/content/configuring/core/config-options.md — `vrr` modes 0–3 [verified]
- https://man.archlinux.org/man/sway-output.5 — `adaptive_sync` [memory]
- https://www.phoronix.com/news/GNOME-46-VRR — experimental VRR in Mutter [memory]
