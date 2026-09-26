---
name: Virtual desktops (Rooms, Spaces, Task View, GNOME workspaces)
slug: virtual-desktops-spaces
domain: notifications-wm
kind: pattern
platform: cross
vendor: Xerox PARC, Apple, Microsoft, GNOME and others
years: 1986–present
status: current
tags: [workspaces, spaces, dynamic-workspaces, task-view]
relevance: high
---

## What it is
Several desktops, each with its own windows, switched by key or gesture. Xerox PARC's Rooms (Henderson and Card, 1986) framed them as task contexts; Unix WMs made them standard. Apple Spaces (Leopard, 2007), GNOME 3's dynamic workspaces (always one empty at the end, 2011), Windows 10 Task View (2015).

## What was new
Rooms' insight: working sets of windows per task reduce the cost of switching tasks. GNOME's dynamic workspaces removed the need to choose a count. macOS added per-Space fullscreen apps.

## What went wrong / limits
Most users of Windows and macOS never create a second desktop; discovery is poor and the mental model (where did my window go?) confuses. macOS "rearrange Spaces by most recent use" is the setting people most often turn off because it moves things.

## Lessons for swaypplet
- Take: workspaces are already the owner's primary unit, which is why Super+Tab shows workspaces, not windows; keep order stable (BAR_VISION P4), never reorder by recency.
- Take: GNOME's always-one-empty rule is worth copying for "new task" workspaces if they are created on demand.

## Sources
- https://en.wikipedia.org/wiki/Mission_Control_(macOS) — Spaces in Leopard 2007. [verified via search summary]
- https://en.wikipedia.org/wiki/Task_View — Windows Task View. [verified via search summary]
- Rooms 1986, GNOME dynamic workspaces, Spaces reordering setting. [memory]
