---
name: iPadOS Stage Manager and iPadOS 26 windowing
slug: ipados-stage-manager
domain: mobile
kind: feature
platform: iOS
vendor: Apple
years: 2022–present
status: current
tags: [windowing, window-sets, workspaces, recents, touch-to-desktop]
relevance: medium
---

## What it is
iPadOS 16 (2022) added Stage Manager: up to four overlapping windows in a central stage, with recent **window sets** as thumbnails in a strip at the left edge; clicking a set swaps the whole stage. Early versions had fixed layout rules and high memory requirements. iPadOS 26 (2025) replaced the rules with free windowing (move, resize, traffic-light buttons, a menu bar on swipe down) and kept Stage Manager as an optional mode for grouping windows into sets.

## What was new
- **Window sets as the unit of switching**: a strip of task groups, each restoring its own arrangement.
- A live preview of each set in the strip.

## What went wrong / limits
- The first version was widely criticised: unpredictable window placement and sizes, windows moving on their own, and a four-window cap. Apple rebuilt it three years later.
- It needed an M1 iPad at launch, and features kept changing between releases.
- Reports on iPadOS 26 still mention windows that minimise unexpectedly when another one moves.

## Lessons for swaypplet
- **Avoid a window manager that "helps" by moving windows**: sway's deterministic tiling is the right base; auto-arrangement cost Apple credibility.
- **Take the left-edge strip of sets**: the Super+Tab strip of workspace tiles is the same idea, and task workspaces are Stage Manager sets with names and colours.

## Sources
- https://www.macstories.net/stories/ios-and-ipados-26-the-macstories-review/11/ — iPadOS 26 windowing, Stage Manager optional [verified via search summary]
- https://en.wikipedia.org/wiki/IPadOS_26 — menu bar, traffic lights [verified via search summary]
- https://www.macstories.net/stories/ipados-16-the-macstories-review/ — Stage Manager criticism in 2022 [memory]
