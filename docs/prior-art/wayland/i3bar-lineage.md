---
name: i3bar protocol, i3status, i3blocks, polybar, lemonbar
slug: i3bar-lineage
domain: wayland
kind: protocol
platform: cross
vendor: i3 project; community
years: 2009–present
status: current
tags: [status-protocol, json-stream, polling, x11, text-bars]
relevance: medium
---

## What it is
The X11-era bar lineage. lemonbar reads formatted text on stdin. i3bar reads a JSON stream of blocks (`full_text`, `color`, `urgent`, `separator`) from a status command and sends click events back on stdin. i3status and i3blocks produce that stream; polybar is a configurable X11 bar with modules. swaybar still speaks the i3bar protocol.

## What was new
- A tiny, language-agnostic contract between a renderer and a data producer, with click events: any script is a status source.
- `urgent: true` as a first-class, protocol-level state.
- i3blocks: per-block intervals and refresh on a Unix signal, so a keybinding can update one block instantly.

## What went wrong / limits
- Text-only; no icons beyond fonts, no motion.
- i3status polls every interval for everything.
- polybar never got Wayland support; its users migrated to Waybar.

## Lessons for swaypplet
- Take "refresh on signal" as the model for segments: a change pushes, no interval.
- The `urgent` bit maps to BAR_VISION's act-now tier; the lineage never had a "data invalid" state, which P9 adds.

## Sources
- https://i3wm.org/docs/i3bar-protocol.html — protocol [memory]
- https://github.com/vivien/i3blocks — signals [memory]
