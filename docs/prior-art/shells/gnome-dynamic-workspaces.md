---
name: GNOME dynamic workspaces
slug: gnome-dynamic-workspaces
domain: shells
kind: pattern
platform: GNOME
vendor: GNOME Project
years: 2011–present
status: current
tags: [workspaces, dynamic, empty-workspace, spatial]
relevance: medium
---

## What it is
GNOME Shell keeps exactly one empty workspace at the end: moving a window onto it creates a new empty one after it, and a workspace left empty is removed. There is no workspace count to configure by default.

## What was new
Workspaces as a by-product of use rather than a setting: the number is always "as many as needed plus one", so there is always a free one to the right.

## What went wrong / limits
- Positions shift: closing the last window on workspace 2 renumbers every later one, which breaks "workspace 3 is mail" habits and keybindings.
- Users who think in fixed slots turn it off (a static count option exists).

## Lessons for swaypplet
- Avoid for named workspaces: swaypplet's workspaces are bound to keys and tasks (BAR_VISION keeps labels "the keybindings in print"); renumbering would break that.
- Adapt: "one free workspace always available" only for the generic set, if ever.

## Sources
- https://help.gnome.org/users/gnome-help/stable/shell-workspaces.html — dynamic workspace behaviour [memory]
- https://en.wikipedia.org/wiki/GNOME_Shell — design history [memory]
