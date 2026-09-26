---
name: Windows 8 Charms bar and full-screen Start
slug: windows-8-charms-start-screen
domain: notifications-wm
kind: product
platform: Windows
vendor: Microsoft
years: 2012–2015
status: failed
tags: [charms, start-screen, hot-corners, touch-first, convergence]
relevance: high
---

## What it is
Windows 8 (2012) replaced the Start menu with a full-screen Start screen of Live Tiles and put system commands (Search, Share, Start, Devices, Settings) in a Charms bar summoned from the right edge or corner. Windows 8.1 (2013) brought back a Start button; Windows 10 (2015) removed Charms and restored a Start menu.

## What was new
One interface for touch and desktop; edge swipes as system gestures.

## What went wrong / limits
Desktop users lost the Start menu with no transition; hot corners were invisible and triggered by accident; the full-screen Start covered the work a user was launching from. Two worlds (Metro and desktop) with different rules. Third-party Start menu replacements sold widely.

## Lessons for swaypplet
- Avoid: removing a learned surface without a direct replacement in the same place.
- Avoid: invisible edge-triggered surfaces as the only path (BAR_VISION P8).
- Take: a launcher overlay should not hide the context it launches into more than it must.

## Sources
- https://en.wikipedia.org/wiki/List_of_features_removed_in_Windows_8 — Start replaced, Charms. [verified via search summary]
- https://tech.slashdot.org/story/15/01/26/1532203/windows-10-charms-bar-removed-no-start-screen-for-desktops — Windows 10 removal. [verified via search summary]
- https://en.wikipedia.org/wiki/Comparison_of_Start_menu_replacements_for_Windows_8 — replacement market. [verified via search summary]
