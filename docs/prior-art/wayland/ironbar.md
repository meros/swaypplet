---
name: Ironbar
slug: ironbar
domain: wayland
kind: product
platform: Wayland
vendor: Jake Stanger
years: 2022–present
status: current
tags: [bar, rust, gtk4, modules, ipc]
relevance: high
---

## What it is
A customisable Wayland bar in Rust on GTK4 (the closest sibling to swaypplet's stack). Modules include workspaces, launcher/taskbar via foreign-toplevel, tray, MPRIS, clipboard, notifications (via swaync), volume, and custom widgets defined in config; its own IPC lets scripts set variables and open popups.

## What was new
- Every module works the same on every compositor with no extra config, by preferring protocols to compositor IPC.
- Config in any of TOML, YAML, JSON or Corn, with hot-reloaded CSS.
- Ironvars: named variables set over IPC that any widget can bind to.

## What went wrong / limits
- One main developer; features are held back until stable, so progress is slow [memory].
- GTK4's CSS lacks many transitions users expect from QML shells.

## Lessons for swaypplet
- Take: prove GTK4 plus Rust can hot-reload CSS safely; ironbar does it.
- Compare its foreign-toplevel launcher module with swaypplet's pins for activate/close handling.

## Sources
- https://github.com/JakeStanger/ironbar/blob/master/README.md — philosophy, modules [verified via search summary]
