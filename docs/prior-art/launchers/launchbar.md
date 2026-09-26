---
name: LaunchBar
slug: launchbar
domain: launchers
kind: product
platform: macOS
vendor: Objective Development
years: 1995–present (NeXTSTEP), 2001– (Mac OS X)
status: niche
tags: [abbreviation learning, instant send, browse, sub-search]
relevance: high
---

## What it is
The oldest keyboard launcher still sold, begun on NeXTSTEP in 1995 and ported to Mac OS X in 2001. It indexes apps, files, contacts, bookmarks and more, and lets you browse into any item with the Right arrow.

## What was new
- **Adaptive abbreviations**: LaunchBar remembers which item you chose for which abbreviation, so "ps" can mean Photoshop for one user and Preview for another, and the mapping sticks.
- **Instant Send**: hold or double-tap the hotkey with something selected in any app, and LaunchBar opens with that selection as the object, ready for an action.
- **Browse mode**: Right arrow enters a folder, contact or app's recent documents; Left goes back. Navigation and search are one gesture set.

## What went wrong / limits
- Paid, macOS-only, with a dated look; mindshare went to Alfred and Raycast although the ideas came first.

## Lessons for swaypplet
- Take abbreviation learning: key the frecency store by (query prefix, item) as well as by item (see [adaptive-input-history](adaptive-input-history.md)).
- Adapt Instant Send: a keybinding that opens the launcher with the primary selection as the query or as the object of an action.
- Right/Left to enter/leave an item generalises the existing Tab-for-windows.

## Sources
- https://www.obdev.at/products/launchbar/index.html — features, history [memory]
- https://en.wikipedia.org/wiki/LaunchBar — 1995 NeXTSTEP origin [memory]
