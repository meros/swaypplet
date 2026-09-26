---
name: Plasma floating panel
slug: kde-floating-panel
domain: shells
kind: feature
platform: KDE
vendor: KDE
years: 2022–present
status: current
tags: [panel, floating, maximize, adaptive, dodge-windows]
relevance: high
---

## What it is
Plasma 5.25 (2022) added a floating panel mode, default since Plasma 6.0: the panel sits a few pixels off the screen edge with rounded corners, and when a window is maximized or touches it, it animates flush to the edge and becomes a normal attached panel. Plasma 6 also added "dodge windows" intelligent auto-hide.

## What was new
The bar's shape responds to the window layout: floating when there is space around it, attached when a window needs the full edge. The attach/detach transition keeps the edge as a Fitts target once a window is maximized.

## What went wrong / limits
- Floating wastes a strip of pixels and breaks the infinite-edge target while floating (the gap under the panel is not clickable in early versions).
- The margin plus a maximized window's own gap can look inconsistent across apps.

## Lessons for swaypplet
- Adapt: on a tiling WM windows almost always touch the bar, so the attached state is the norm; a floating state makes sense only on an empty workspace. That is a possible state change on the bar that happens rarely (workspace empty/non-empty), compatible with P1.
- Take: whatever the float state, clicks in the edge gap should land on the bar.

## Sources
- https://9to5linux.com/kde-plasma-6-to-ship-with-floating-panel-by-default-double-click-for-opening-files — Plasma 6 floating default [verified]
- https://www.omgubuntu.co.uk/2024/02/kde-plasma-6-0-new-features — un-floats on maximize, dodge windows [verified]
