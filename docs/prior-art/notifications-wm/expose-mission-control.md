---
name: Exposé and Mission Control
slug: expose-mission-control
domain: notifications-wm
kind: feature
platform: macOS
vendor: Apple
years: 2003–present
status: current
tags: [overview, window-overview, spaces, spatial-continuity, animation]
relevance: high
---

## What it is
Exposé (Mac OS X 10.3 Panther, 2003) shrank every open window into a non-overlapping layout with one key or a hot corner, animating each from its real position. Lion (2011) merged Exposé, Spaces and Dashboard into Mission Control: windows grouped by app, with a strip of Spaces along the top. GNOME's Activities overview (2011) and Windows Task View (2015) follow the same model.

## What was new
Spatial continuity: windows fly from where they are to where they will be, and back, so the user tracks the one they want by motion, not by reading. It was fast because it used the compositor's existing window textures (Quartz Extreme).

## What went wrong / limits
Mission Control's grouping of windows by app stacked them, hiding windows that Exposé had shown individually; many users reverted via a setting. With many windows the thumbnails become too small to read.

## Lessons for swaypplet
- Take: roadmap item 4 (overview whose first frame is the real screen, windows fly to a grid) is exactly Exposé's principle; do not ship it without the continuity.
- Avoid: stacking windows by app in the overview; show each window.
- Take: the Spaces strip on top is the model for showing workspaces and their windows in one view.

## Sources
- https://en.wikipedia.org/wiki/Mission_Control_(macOS) — Exposé 2003, merge in Lion. [verified via search summary]
- https://www.cultofmac.com/news/revert-mission-control-to-expose-style-in-mountain-lion-os-x-tips — users reverting app grouping. [verified via search summary]
- Quartz Extreme use, GNOME/Windows successors. [memory]
