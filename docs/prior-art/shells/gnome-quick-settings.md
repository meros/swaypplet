---
name: GNOME quick settings
slug: gnome-quick-settings
domain: shells
kind: feature
platform: GNOME
vendor: GNOME Project
years: 2022–present
status: current
tags: [quick-settings, toggles, pills, submenus, panel]
relevance: high
---

## What it is
Since GNOME 43 (2022) the system menu at the top-right is a grid of pill-shaped toggles (Wi-Fi, Bluetooth, power mode, dark style, night light) with sliders for volume and brightness above. A pill with an arrow opens a submenu that expands in place over the grid (network list, audio device list). GNOME 44 added a Bluetooth device list and background-apps list.

## What was new
- Split pill: the body toggles, the arrow opens detail; one control carries both jobs.
- Dark style as a quick toggle next to connectivity.
- Background apps (Flatpak portal) listed in the menu with a stop button.

## What went wrong / limits
- Fixed set; extensions are the only way to add or reorder toggles, and they break across releases.
- Submenus cover the grid; only one open at a time.
- Accent colour for "on" was the only state cue at first; low-contrast themes made off and on hard to tell.

## Lessons for swaypplet
- Take: split toggle (body toggles, chevron opens detail), with "on" shown by shape or fill plus colour, not colour alone (BAR_VISION P3).
- Take: a background-apps row; swaypplet's panel could list the tray's `Passive` items there.

## Sources
- https://release.gnome.org/43/ — quick settings introduced [verified]
- https://www.debugpoint.com/gnome-43-quick-settings/ — pill buttons, arrow submenu [verified]
- https://release.gnome.org/44/ — Bluetooth list, background apps [memory]
