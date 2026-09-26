---
name: elementary OS Pantheon
slug: elementary-pantheon
domain: shells
kind: product
platform: other
vendor: elementary, Inc.
years: 2011–present
status: current
tags: [top-panel, dock, dark-schedule, consistency, gtk]
relevance: high
---

## What it is
Pantheon is elementary OS's desktop: Wingpanel (a top panel with the Applications menu, clock and indicators), a dock (Plank, later elementary's own), the Gala window manager and a curated set of apps following one HIG. Wingpanel is translucent over the wallpaper and becomes solid when a window is maximized.

## What was new
- Dark style can be scheduled "Sunset to Sunrise" (elementary OS 6, 2021), using the location, next to a manual toggle; apps follow the system preference through a freedesktop portal setting that elementary helped define.
- Accent colour can be chosen automatically from the wallpaper (elementary OS 6).
- Panel material changes with window state (translucent at rest, solid when a window is maximized).

## What went wrong / limits
- Small team; releases are infrequent (years between majors) and the app ecosystem stayed small.
- Wingpanel indicators are in-process plugins with limited third-party support.

## Lessons for swaypplet
- Take: validation for swaypplet's sun-driven mode; elementary uses sunset/sunrise times, swaypplet's elevation-with-hysteresis rule is stricter and avoids the dim-twilight flip.
- Take: the `org.freedesktop.appearance.color-scheme` portal key so apps follow swaypplet's mode.
- Take: wallpaper-derived accent is an established precedent for the tint.

## Sources
- https://blog.elementary.io/elementary-os-6-odin-released/ — dark style schedule at sunset/sunrise, accent picked from wallpaper [verified]
- https://github.com/flatpak/xdg-desktop-portal/blob/main/data/org.freedesktop.impl.portal.Settings.xml — color-scheme key [memory]
- https://en.wikipedia.org/wiki/Pantheon_(desktop_environment) — components [memory]
