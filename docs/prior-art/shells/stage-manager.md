---
name: Stage Manager
slug: stage-manager
domain: shells
kind: feature
platform: macOS
vendor: Apple
years: 2022–present
status: niche
tags: [window-management, groups, recent-apps, failed-idea]
relevance: low
---

## What it is
An optional window mode (macOS Ventura and iPadOS 16, 2022) that puts the current app or group of windows centre stage and shelves other apps as live thumbnails in a strip on the left. Windows can be grouped into sets by dragging.

## What was new
Window sets as a first-class, visual unit, and automatic decluttering: everything not in the current set leaves the screen without being minimized.

## What went wrong / limits
- The strip shows at most about five recent sets; beyond that apps disappear from view.
- Adding an app not already in the strip to a set takes several steps; mixed sets only let the top window be dragged out.
- No keyboard shortcuts, no AppleScript or Shortcuts support at launch.
- Adoption stayed low; reviewers called it laborious to set up.

## Lessons for swaypplet
- Avoid: a window-grouping model that is mouse-only and has no keyboard verbs. sway workspaces already are named window sets.
- Note: the recent strip failing at five items is the same limit a Super+Tab tile strip hits; plan paging or scaling before the count grows.

## Sources
- https://wormsandviruses.com/2022/10/stage-manager-on-macos-ventura/ — adding to sets, drag limits [verified]
- https://macmost.com/an-in-depth-look-at-macos-ventura-stage-manager.html — strip item limit [verified]
- https://mjtsai.com/blog/2022/10/25/stage-manager-in-macos-13-0/ — critical round-up [verified]
