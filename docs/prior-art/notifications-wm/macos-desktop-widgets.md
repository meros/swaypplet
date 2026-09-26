---
name: macOS desktop widgets (Sonoma)
slug: macos-desktop-widgets
domain: notifications-wm
kind: feature
platform: macOS
vendor: Apple
years: 2023–present
status: current
tags: [widgets, desktop, monochrome, fade, focus-aware]
relevance: medium
---

## What it is
macOS 14 Sonoma (2023) let WidgetKit widgets live on the desktop, including interactive ones and, via Continuity, widgets from the user's iPhone. When a window is active, desktop widgets desaturate and take on the wallpaper's tone; clicking the desktop brings them back to full colour. The style can be set to automatic, always monochrome or always full colour.

## What was new
Focus-aware ambient information: the surface dims itself when it is not the subject. Widgets are declarative timelines rendered by the system, not running code, so they cost little and cannot misbehave like gadgets.

## What went wrong / limits
Covered by windows most of the time, so seen only when the desktop is shown. Grey widgets confused users who thought they were broken.

## Lessons for swaypplet
- Take: ambient surfaces go achromatic when they are not the subject; this is BAR_VISION P1 applied to any peripheral surface, including pins.
- Take: declarative, system-rendered content instead of third-party code.

## Sources
- https://www.macrumors.com/guide/how-widgets-work-macos-sonoma/ — desktop widgets, interactivity, fade. [verified via search summary]
- https://macmost.com/macos-sonoma-desktop-widgets-preview.html — the three widget style options. [verified via search summary]
- https://discussions.apple.com/thread/255279930 — users confused by grey widgets. [verified via search summary]
