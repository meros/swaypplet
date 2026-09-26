---
name: fractional-scale, cursor-shape and tearing-control
slug: client-fidelity-protocols
domain: wayland
kind: protocol
platform: Wayland
vendor: wayland-protocols
years: 2022–present
status: current
tags: [protocol, hidpi, fractional-scaling, cursor, tearing, latency]
relevance: low
---

## What it is
Three small staging protocols that remove old Wayland rough edges. `wp-fractional-scale-v1` (with `wp-viewporter`) tells a client the exact preferred scale (e.g. 1.25) so it renders at native pixels instead of upscaling a 2x buffer. `wp-cursor-shape-v1` lets a client name a cursor ("text", "pointer") and the compositor draws it from the user's theme. `wp-tearing-control-v1` lets a fullscreen game ask for async page flips.

## What was new
- Sharp text at fractional scales, and consistent cursors across toolkits.

## What went wrong / limits
- GTK4 renders fractional scales well only from 4.14 with the new renderers; before that, blurry text on 1.25x [memory].
- Tearing control is ignored for layer surfaces and non-fullscreen windows by design.

## Lessons for swaypplet
- Nothing to build: GTK4 uses fractional-scale and cursor-shape automatically. Check swaypplet's glass shader samples at the real fractional scale, not the integer buffer scale, so the grain is not resampled.

## Sources
- https://wayland.app/protocols/fractional-scale-v1 — protocol [memory]
- https://wayland.app/protocols/cursor-shape-v1 — protocol [memory]
