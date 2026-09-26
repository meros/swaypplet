---
name: Unity HUD
slug: unity-hud
domain: shells
kind: feature
platform: other
vendor: Canonical
years: 2012–2017
status: discontinued
tags: [command-palette, menus, search, keyboard, alt-key]
relevance: medium
---

## What it is
The Head-Up Display (Ubuntu 12.04, 2012): tap Alt and type, and the HUD searches the focused app's menu items (exported through the global menu) and system indicators, ranked with fuzzy matching and learned frequency. Enter runs the item.

## What was new
A system-wide command palette over every app's menus, years before command palettes became common in editors. It turned deep menu trees into a two-second keyboard action.

## What went wrong / limits
- Only worked for apps that exported menus through the global menu protocol; GTK 3 apps with header bars and client-side menus had nothing to search.
- Tap-Alt conflicted with apps using Alt; discoverability relied on users knowing to tap it.
- Disappeared with Unity; a KDE analogue (the global menu search in KRunner/Plasma HUD) stays niche.

## Lessons for swaypplet
- Adapt: a launcher mode that searches shell actions (panel toggles, sway commands, settings panes) is the part swaypplet controls fully, unlike app menus.
- Avoid: depending on app-exported menus; most Wayland apps do not export them.

## Sources
- https://techcrunch.com/2012/01/25/mark-shuttleworth-unveils-new-head-up-display-for-ubuntu-12-04 — announcement [verified]
- https://www.omgubuntu.co.uk/2017/04/best-unity-desktop-features — tap Alt, menu search [verified]
