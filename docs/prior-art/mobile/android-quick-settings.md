---
name: Android Quick Settings
slug: android-quick-settings
domain: mobile
kind: feature
platform: Android
vendor: Google
years: 2012–present
status: current
tags: [quick-settings, tiles, toggles, extensibility, two-stage-pull]
relevance: high
---

## What it is
The tile grid in the notification shade. Android 4.2 added it. Android 7.0 opened it to third-party apps through `TileService`, and Android 13 added a placement API that lets an app ask the user to add its tile. Since Android 5 it opens in two stages: the first pull shows a strip of about 6 tiles above the notifications, and the second shows the full grid.

## What was new
- **Two-stage reveal**: the common tiles cost one gesture, and everything else costs a second pull on the same gesture. Progressive disclosure without a new surface.
- **Tap toggles, long press opens settings**: one convention across every tile.
- An open tile API: a tile's state (active, inactive, unavailable), label and subtitle are pushed by the app, and the system draws it.

## What went wrong / limits
- Third-party tiles saw little adoption until Android 13's placement prompt, because users never opened the edit screen.
- Third-party tiles cannot show a custom detail panel, which only system tiles get, so their long press leaves the shade.
- Android 12's larger tiles cut the first-stage strip to 4, and the Internet tile merge ([android-12-internet-tile](android-12-internet-tile.md)) made the grid slower to use.

## Lessons for swaypplet
- **Take the three tile states**, including *unavailable* (hardware absent or blocked). It matches P9 (data invalid is its own state).
- **Take one convention for primary and secondary action**: toggle on click, detail on a secondary action, the same on every tile, with the secondary action reachable by keyboard (P8).
- **Adapt the two-stage reveal**: the panel could open to a compact row with a full view one keystroke further, if the panel ever grows past one screen.

## Sources
- https://medium.com/androiddevelopers/quick-settings-tiles-e3c22daf93a8 — TileService on Android 7.0 [verified via search summary]
- https://www.androidpolice.com/android-13-allows-app-developers-to-promote-their-own-quick-settings-tiles/ — Android 13 placement API [verified via search summary]
