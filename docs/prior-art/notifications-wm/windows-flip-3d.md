---
name: Windows Flip 3D
slug: windows-flip-3d
domain: notifications-wm
kind: feature
platform: Windows
vendor: Microsoft
years: 2007–2012
status: discontinued
tags: [switcher, 3d, eye-candy, aero]
relevance: medium
---

## What it is
Windows Vista's Win+Tab switcher showed every window as a tilted 3D stack that scrolled with Tab or the mouse wheel. Present in Windows 7; removed in Windows 8, where Win+Tab switched Metro apps.

## What was new
A showcase for the new composited desktop (DWM).

## What went wrong / limits
The perspective stack hid most of each window behind the one in front, so it was slower for recognition than the flat Alt-Tab grid that shipped beside it. It showed off the compositor rather than helping the user; its removal went largely unmourned.

## Lessons for swaypplet
- Avoid: 3D or perspective arrangements in Super+Tab or the overview; flat, non-overlapping thumbnails at the largest readable size win.
- Take: every motion in a switcher must explain where something is or goes (MOTION.md's meaning per duration).

## Sources
- https://www.thewindowsclub.com/flip-3d-windows-7-vista — Flip 3D description. [verified via search summary]
- https://www.intowindows.com/enable-flip-3d-in-windows-8/ — removed in Windows 8. [verified via search summary]
- Recognition cost of the stack. [memory]
