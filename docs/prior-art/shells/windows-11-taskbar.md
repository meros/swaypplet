---
name: Windows 11 taskbar
slug: windows-11-taskbar
domain: shells
kind: feature
platform: Windows
vendor: Microsoft
years: 2021–present
status: current
tags: [taskbar, centered, customization-regression, bar-position]
relevance: medium
---

## What it is
The Windows 11 taskbar was rewritten in 2021: icons centred by default, Start, search, Task View and Widgets buttons beside them, and Quick Settings and notifications behind the clock and status icons. The rewrite dropped features: moving the bar to the top or sides, drag-and-drop onto taskbar icons (restored 2022), ungrouped window labels (restored 2023), small icons.

## What was new
Centred icons that stay stable as the bar's contents change; a clean split of system status into two click targets (Quick Settings, notification centre).

## What went wrong / limits
- Removing long-standing options produced five years of complaints. Microsoft announced in March 2026 that top, left and right positions return, calling repositioning "one of the top asks".
- Centering moves the Start button as icons are added, breaking the fixed corner target that Windows 95 established.

## Lessons for swaypplet
- Avoid: an element whose position depends on how many other items are present (the centred Start button). BAR_VISION P4 fixed slots.
- Note: a rewrite that drops rarely used but strongly held options costs more trust than it saves code.

## Sources
- https://www.windowslatest.com/2026/03/21/microsoft-confirms-vertical-and-top-taskbar-update-for-windows-11-shares-first-look/ — position options return, Microsoft's quote [verified]
- https://www.elevenforum.com/t/bring-back-the-ability-to-move-taskbar-to-top-and-sides.28459/ — user demand [verified]
- https://en.wikipedia.org/wiki/Features_new_to_Windows_11 — taskbar changes and restorations [memory]
