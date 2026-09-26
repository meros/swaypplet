---
name: Windows Snap, Snap Layouts and Snap Groups
slug: windows-snap-layouts
domain: notifications-wm
kind: feature
platform: Windows
vendor: Microsoft
years: 2009–present
status: current
tags: [snapping, tiling, layouts, groups, maximize-hover]
relevance: medium
---

## What it is
Aero Snap (Windows 7, 2009) halved a window by dragging to an edge. Windows 10 added Snap Assist (suggest a window for the other half). Windows 11 (2021) added Snap Layouts, a grid of 4 or 6 layouts shown on hovering the maximise button or Win+Z, and Snap Groups: windows snapped together are restored together from the taskbar.

## What was new
Tiling for people who never configure a tiling WM: pick a layout, then fill each zone from a picker. Groups remember which windows belong together.

## What went wrong / limits
Hover on the maximise button is discoverable but accidental; a setting exists to disable it. Layouts are fixed presets tied to screen width.

## Lessons for swaypplet
- Note: sway already tiles; the idea worth borrowing is the "fill the remaining zone" picker, i.e. after splitting, offer recent windows to fill the new container.
- Adapt: Win+Z is a keyboard path to a layout picker; a keyboard-first layout picker (split, tabbed, stacked, 3-column) for the focused workspace would teach sway layouts to a newcomer. Low priority for the owner.

## Sources
- https://support.microsoft.com/en-us/windows/experience/snap-your-windows — Snap features. [verified via search summary]
- https://www.windowscentral.com/how-use-snap-assist-windows-11 — Snap Assist. [verified via search summary]
- https://www.elevenforum.com/t/enable-or-disable-snap-layouts-for-maximize-button-in-windows-11.61/ — hover setting. [verified via search summary]
