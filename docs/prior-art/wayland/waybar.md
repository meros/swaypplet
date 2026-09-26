---
name: Waybar
slug: waybar
domain: wayland
kind: product
platform: Wayland
vendor: Alexis Rouillard (Alexays) and contributors
years: 2018–present
status: current
tags: [bar, gtkmm, json-config, css, modules, polling]
relevance: high
---

## What it is
The default Wayland bar: C++ with gtkmm (GTK3), JSON config, GTK CSS styling, built-in modules for sway, Hyprland, niri, river, dwl, tray, PulseAudio, network and more, plus `custom` modules that run a script on an interval or read its stdout continuously.

## What was new
- Per-compositor workspace modules plus a generic one on `ext-workspace-v1` (ported from the wlr draft for labwc).
- `custom` modules with JSON output (`text`, `tooltip`, `class`, `percentage`) turned any script into a bar segment; the most copied contract in the ecosystem.

## What went wrong / limits
- Performance problems cluster around polling and exec: reports of one core at 100 % from a custom module, 75 % of a core from the CPU module at a 30 s interval, `waitpid failed` loops.
- GTK3 CSS without transitions between states, so no motion.
- Every module is its own polling loop; nothing coordinates timers.

## Lessons for swaypplet
- Avoid exec-per-tick modules; keep services event-driven (zbus signals, file monitors) as the "no 1-second polling" roadmap item does.
- If swaypplet ever takes third-party segments, take Waybar's JSON contract but run it as a long-lived stream, never an interval exec.

## Sources
- https://github.com/Alexays/Waybar/issues/5303 — one core at 100 % with a custom module [verified via search summary]
- https://github.com/Alexays/Waybar/issues/2631 — waybar eating CPU [verified via search summary]
- https://github.com/Alexays/Waybar/pull/4016 — ext-workspace port [verified via search summary]
