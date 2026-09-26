---
name: kanshi, shikane and way-displays
slug: output-profile-tools
domain: wayland
kind: product
platform: Wayland
vendor: emersion (kanshi); hw0lff (shikane); alex-courtis (way-displays)
years: 2019–present
status: current
tags: [displays, profiles, hotplug, output-management]
relevance: high
---

## What it is
Daemons that apply output layouts on hotplug via `wlr-output-management`. kanshi matches the set of connected outputs to the first profile that claims them all. shikane generates every compatible (display, output, mode) combination ranked by exactness, allows several match rules per output, and can export the current setup as config. way-displays arranges outputs automatically (left-to-right, scale by DPI) with a small rules file [memory].

## What was new
- Profiles keyed on the set of connected monitors, applied atomically.
- shikane's ranked matching and "export current state as config", which removes hand-writing profiles.
- way-displays: automatic scale from physical DPI.

## What went wrong / limits
- kanshi's matching is inexact for identical monitors and partial matches; shikane was written to fix it.
- All are config-file tools with no UI; users combine them with nwg-displays.

## Lessons for swaypplet
- For roadmap item 2, take shikane's ranking and "save current layout as profile" as the primary way to create profiles, and way-displays' DPI-derived default scale.

## Sources
- https://github.com/hw0lff/shikane — features vs kanshi [verified via search summary]
- https://github.com/emersion/kanshi — kanshi [verified via search summary]
- https://github.com/alex-courtis/way-displays — way-displays [memory]
