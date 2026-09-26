---
name: NeXTSTEP Dock
slug: nextstep-dock
domain: shells
kind: product
platform: other
vendor: NeXT
years: 1989–1997
status: discontinued
tags: [dock, historic, tiles, running-state, appicons]
relevance: low
---

## What it is
A vertical column of 64 px square tiles on the right screen edge of NeXTSTEP, holding the Workspace Manager, pinned apps and the recycler. A running app showed no ellipsis under its icon; a not-running docked app showed "..." until launched. Mini-windows (miniaturized windows) lived as separate tiles on the bottom edge.

## What was new
The first mainstream dock: fixed, square, uniformly sized tiles as a persistent launcher and running indicator. Tiles could be live (an app could draw into its tile, e.g. a clock or load meter).

## What went wrong / limits
- Fixed size and position; the Dock filled up and overflowed off-screen with no scrolling.
- Mini-windows scattered along the bottom edge, separate from the Dock, and were covered by windows.

## Lessons for swaypplet
- Take: fixed square cells whose position never changes (BAR_VISION P4) are a 35-year-old idea that held up.
- Take: app-drawn live tiles are the ancestor of swaypplet's board bays drawing their own state.

## Sources
- https://en.wikipedia.org/wiki/Dock_(computing) — NeXTSTEP dock description [memory]
- https://en.wikipedia.org/wiki/NeXTSTEP — platform context [memory]
