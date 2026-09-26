---
name: Exposé and Mission Control
slug: mission-control-expose
domain: shells
kind: feature
platform: macOS
vendor: Apple
years: 2003–present
status: current
tags: [overview, window-picker, spaces, animation, spatial]
relevance: high
---

## What it is
Exposé (Panther, 2003) scaled all windows down so none overlapped, animating from their real positions, for picking one. Mission Control (Lion, 2011) merged Exposé with Spaces: windows grouped by app below a row of desktop thumbnails, reached by F3, a three-finger swipe up or a hot corner.

## What was new
- Windows fly from where they are to where they will be picked, so the user tracks each window without reading. The animation is the explanation.
- Gesture-driven and interruptible: the three-finger swipe scrubs the transition with the fingers.

## What went wrong / limits
- Grouping windows by app (Lion default) hid window layout; many users turned it off.
- The Spaces row shows only names on hover in later versions; thumbnails are small.
- Performance with many windows and multiple displays was uneven for years.

## Lessons for swaypplet
- Take: ROADMAP item 4 is this exact idea; the first frame must equal the screen (the capture) and every window must move from its real rect.
- Take: tie the transition to input (Super hold, or a gesture) so it can be cancelled midway.
- Avoid: regrouping windows by app; keep workspace layout recognizable.

## Sources
- https://en.wikipedia.org/wiki/Mission_Control_(macOS) — Exposé 2003, Mission Control 2011, grouping [memory]
