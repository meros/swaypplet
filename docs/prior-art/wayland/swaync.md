---
name: SwayNotificationCenter (swaync)
slug: swaync
domain: wayland
kind: product
platform: Wayland
vendor: Erik Reider
years: 2021–present
status: current
tags: [notifications, control-center, gtk4, grouping, inline-reply]
relevance: high
---

## What it is
A notification daemon with a control-center panel, now on GTK4 and gtk4-layer-shell. Features: grouped notifications, keyboard shortcuts, markup with images, inline replies, history panel, MPRIS album art, DND, inhibition over D-Bus or from a client, and control-center widgets (buttons, toggles with state commands, sliders, MPRIS, inhibitors list) defined in JSON.

## What was new
- A notification centre as a general quick-settings panel: toggles with an `update-command` to resync state changed elsewhere.
- Inhibitors as a visible widget listing which app holds DND.
- Inline reply for messaging notifications.

## What went wrong / limits
- Toggle state via shell commands drifts from the real state until the update command runs.
- Heavy dependency list (libadwaita, granite, libgee) for a daemon [verified].

## Lessons for swaypplet
- Take inline replies and the "who is inhibiting" row; both fit roadmap item 3 (context-aware quiet mode).
- Avoid command-driven toggles; swaypplet's services already own state directly.

## Sources
- https://github.com/ErikReider/SwayNotificationCenter — features, deps [verified via search summary]
