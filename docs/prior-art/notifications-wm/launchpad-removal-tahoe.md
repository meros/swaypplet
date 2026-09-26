---
name: Launchpad removal in macOS Tahoe
slug: launchpad-removal-tahoe
domain: notifications-wm
kind: feature
platform: macOS
vendor: Apple
years: 2011–2025
status: discontinued
tags: [launcher, app-grid, spotlight, removal, backlash]
relevance: high
---

## What it is
Launchpad (OS X Lion, 2011) was an iOS-style full-screen grid of apps with folders. macOS 26 Tahoe (September 2025) removed it and replaced it with an Apps view inside Spotlight that categorises apps automatically; the Dock icon became "Apps".

## What was new
Removal as a design decision: unify launching around search.

## What went wrong / limits
Users who launched by spatial memory (this icon, top left of page 2) lost their arrangement and folders; automatic categories cannot be edited. Third-party grids (AppGrid, Launchie, LaunchOS) appeared within weeks. Search-first works for people who know names, not for people who recognise icons.

## Lessons for swaypplet
- Take: a launcher needs both paths, type-to-search and a stable spatial layout the user arranges; keep the start menu's pinned grid even as search improves (relevant to the `launcher` branch).
- Avoid: automatic reordering or categorisation of things the user arranged.

## Sources
- https://piunikaweb.com/2025/09/17/macos-26-launchpad-removed-backlash/ — backlash. [verified via search summary]
- https://cleanmymac.com/blog/macos-tahoe-launchpad — Apps replaces Launchpad in Spotlight. [verified via search summary]
- https://developer.apple.com/forums/thread/801089 — request to restore custom organisation. [verified via search summary]
