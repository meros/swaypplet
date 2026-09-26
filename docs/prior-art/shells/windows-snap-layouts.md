---
name: Snap Layouts and Snap Groups
slug: windows-snap-layouts
domain: shells
kind: feature
platform: Windows
vendor: Microsoft
years: 2021–present
status: current
tags: [window-management, tiling, zones, groups, discoverability]
relevance: medium
---

## What it is
In Windows 11, hovering the maximize button (or Win+Z) shows a small palette of layouts (four on common displays, six on ultrawide); clicking a zone snaps the window there and Snap Assist offers the other windows to fill the remaining zones. A set of windows snapped together becomes a Snap Group, restorable from the taskbar thumbnail, Task View and Alt+Tab.

## What was new
- Layout chosen by pointing at a picture of it, where the user already is (the maximize button), rather than learned shortcuts.
- The group as a remembered unit: one taskbar hover restores several windows in their zones.

## What went wrong / limits
- Hover-triggered: opens when the pointer passes over maximize, which some users find intrusive and disable.
- Groups break when a member is moved or closed and are forgotten after restart.
- Zones are fixed presets; FancyZones (PowerToys) is needed for custom grids.

## Lessons for swaypplet
- Take: visual layout picker for keyboard-less moments on a tiling WM, e.g. a panel tile that sets sway's split/tabbed/stacked for the focused container.
- Avoid: hover-to-open palettes (BAR_VISION P8).

## Sources
- https://learn.microsoft.com/en-us/answers/questions/2337358/how-to-use-snap-layouts-and-snap-groups-in-windows — hover, Win+Z, groups [verified]
- https://www.windowscentral.com/how-use-snap-assist-windows-11 — four vs six layouts on ultrawide [verified]
