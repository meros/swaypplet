---
name: Windows 11 Start menu
slug: windows-11-start-menu
domain: shells
kind: feature
platform: Windows
vendor: Microsoft
years: 2021–present
status: current
tags: [start-menu, launcher, recommended, categories, ads]
relevance: medium
---

## What it is
A floating card above the taskbar with a search field, pinned apps, a "Recommended" section of recent files and apps, and an All apps list. In late 2025 Microsoft redesigned it as one vertically scrolling surface with All apps on the first page in Category, Grid or List views, a size that scales with the display, and an optional Phone Link side pane.

## What was new
- 2025: automatic app categories (Productivity, Creativity, Games...) as the default All view.
- Search-first: typing on open searches apps, files, settings and the web.

## What went wrong / limits
- "Recommended" showed promoted Store apps, which users read as ads; it could not be removed for years.
- The 2021 design needed two clicks to see all apps, fixed by the 2025 redesign.
- Web results in local search are slow and often not wanted.

## Lessons for swaypplet
- Avoid: suggestions that are not the user's own history. A launcher's "recent" must be recent.
- Avoid: network results mixed into local search.
- Take: an All-apps view one scroll away from the pinned set, no second page.

## Sources
- https://www.windowscentral.com/microsoft/windows-11/whats-in-the-new-start-menu-on-windows-11-for-versions-25h2-and-24h2 — 2025 redesign, views [verified]
- https://windowsforum.com/threads/windows-11-start-menu-redesign-all-view-category-grid-and-phone-link.386879/ — category/grid/list, Phone Link [verified]
- https://en.wikipedia.org/wiki/Criticism_of_Microsoft_Windows — promoted apps in Recommended (2024 Insider test) [memory]
