---
name: Stage Manager
slug: stage-manager
domain: notifications-wm
kind: feature
platform: macOS
vendor: Apple
years: 2022–present
status: current
tags: [window-groups, strip, thumbnails, focus]
relevance: medium
---

## What it is
macOS 13 Ventura and iPadOS 16 (2022): the current app or group of windows sits centre stage; other groups shrink to live thumbnails in a strip at the left edge. Dragging a thumbnail onto the stage adds it to the group.

## What was new
Window groups as a first-class, user-made unit between "a window" and "a Space", with live thumbnails of recently used groups.

## What went wrong / limits
Reviews were mixed to negative: no keyboard shortcuts or automation, the desktop hidden, clicking the wallpaper exits the stage, groups clumsy to build, and a busy animated strip. Users spent time managing Stage Manager instead of working. It overlaps with Spaces without replacing them.

## Lessons for swaypplet
- Note: sway workspaces already are user-made groups, keyboard-first; Stage Manager shows what happens when grouping is pointer-only.
- Avoid: an always-visible strip of live thumbnails; live previews belong in the transient Super+Tab surface.

## Sources
- https://wormsandviruses.com/2022/10/stage-manager-on-macos-ventura/ — critique. [verified via search summary]
- https://www.macstories.net/stories/macos-ventura-the-macstories-review/2/ — no shortcuts or automation. [verified via search summary]
- https://numericcitizen.me/critical-thoughts-on-apple-stage-manager/ — time managing groups. [verified via search summary]
