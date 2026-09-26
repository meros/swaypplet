---
name: Budgie and Raven
slug: budgie-raven
domain: shells
kind: product
platform: other
vendor: Solus, then Buddies of Budgie
years: 2013–present
status: current
tags: [sidebar, notifications, applets, gtk, wayland-port]
relevance: medium
---

## What it is
Budgie is a GTK desktop started for Solus in 2013, maintained by Buddies of Budgie since 2022. It has configurable panels with applets and Raven, a right-edge sidebar that holds widgets (calendar, media controls, sound devices) and, in a second tab, notifications. The 10.x series ran on Magpie, a Mutter fork; a Wayland port on labwc has been the project's focus.

## What was new
Raven puts media, audio output selection and notification history in one tall sidebar that slides from the edge, so the panel stays small.

## What went wrong / limits
- Small team, a long toolkit transition (the planned Qt/EFL rewrite for Budgie 11 was abandoned) and a slow Wayland port.
- Raven's two tabs split one sidebar into two unrelated jobs.

## Lessons for swaypplet
- Take: media and output device selection in one panel section; swaypplet's media popover could hold the audio output picker.
- Avoid: a sidebar that is two unrelated tools behind tabs.
- Note: Budgie's move to labwc shows wlroots-based compositors becoming a normal base for a full desktop, as swayfx is for swaypplet.

## Sources
- https://en.wikipedia.org/wiki/Budgie_(desktop_environment) — history, Magpie [verified]
- https://buddiesofbudgie.org/blog — Wayland (labwc) port plans [memory]
