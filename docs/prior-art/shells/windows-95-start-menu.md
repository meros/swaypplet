---
name: Windows 95 Start menu and taskbar
slug: windows-95-start-menu
domain: shells
kind: product
platform: Windows
vendor: Microsoft
years: 1995–present
status: current
tags: [start-menu, taskbar, historic, discoverability, tray]
relevance: medium
---

## What it is
Windows 95 introduced a bottom taskbar with one button per window, a Start button on the left opening a cascading menu of programs, and a notification area ("system tray") with the clock on the right. The layout survived in every Windows version since.

## What was new
- A single labelled "Start" button: usability testing showed new users did not know where to begin, and one button answered that.
- One button per top-level window made every open window visible and one click away.

## What went wrong / limits
- Cascading submenus required steering through narrow paths (the "tunnel" problem); later versions added delay tolerance and then replaced cascades with scrolling lists.
- The notification area became a dumping ground for background apps; Windows XP added auto-hide of inactive icons.
- Shutting down by clicking "Start" was a famous inconsistency.

## Lessons for swaypplet
- Take: one always-present start target at a screen corner is proven; swaypplet has it.
- Avoid: cascading menus.
- Note: the tray-clutter problem took Windows six years to begin addressing; filter by status from the start (BAR_VISION tray rule).

## Sources
- https://en.wikipedia.org/wiki/Start_menu — history, Windows 95 design [memory]
- https://en.wikipedia.org/wiki/Taskbar — notification area history [memory]
- https://en.wikipedia.org/wiki/Windows_95 — usability testing behind the Start button (Kent Sullivan, "The Windows 95 User Interface: A Case Study in Usability Engineering", CHI 1996) [memory]
