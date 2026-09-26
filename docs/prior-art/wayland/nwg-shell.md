---
name: nwg-shell
slug: nwg-shell
domain: wayland
kind: project
platform: Wayland
vendor: Piotr Miller (nwg-piotr)
years: 2021–present
status: current
tags: [python, gtk3, panel, drawer, dock, displays, settings]
relevance: medium
---

## What it is
A set of Python/GTK3 tools that turn sway or Hyprland into a desktop: nwg-panel, nwg-drawer (app grid), nwg-dock, nwg-bar, nwg-look (GTK settings), nwg-displays (drag-and-drop output layout that writes sway or Hyprland config), azote (wallpapers), and a shell-wide settings app.

## What was new
- GUI configuration for every piece of a tiling desktop, including a visual output arranger, years before COSMIC.
- Each tool stands alone, so users adopt one at a time.

## What went wrong / limits
- One maintainer for a dozen tools; the look is dated and GTK3-bound.
- nwg-displays writes compositor config files instead of applying live profiles.

## Lessons for swaypplet
- Take nwg-displays' drag-to-arrange UI for roadmap item 2 (display profiles), but apply through `wlr-output-management` with test-then-apply, not config writes.

## Sources
- https://github.com/nwg-piotr/nwg-shell — suite [memory]
- https://github.com/nwg-piotr/nwg-displays — output arranger [memory]
