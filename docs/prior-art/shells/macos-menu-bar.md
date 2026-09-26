---
name: macOS menu bar
slug: macos-menu-bar
domain: shells
kind: feature
platform: macOS
vendor: Apple
years: 1984–present
status: current
tags: [top-bar, global-menu, fitts-law, status-items, transparency]
relevance: medium
---

## What it is
A full-width strip at the top of the primary display holding the Apple menu, the focused app's menus (a global menu, not per window), and status items ("menu extras") plus the clock on the right. Since Big Sur (2020) most system toggles moved into Control Center, reached from one menu-bar icon. macOS 26 Tahoe draws no background by default; "Show menu bar background" and Reduce Transparency restore it.

## What was new
The global menu on the screen edge exploits Fitts's law: the target has infinite height, so a flick upward cannot overshoot. Status items gave third parties a small, uniform slot for ambient state, all behaving the same way (click opens a menu).

## What went wrong / limits
- The global menu is disconnected from the window on large and multiple displays; the distance to travel grows with screen size, eroding the Fitts advantage.
- No built-in overflow for status items: on notched MacBooks (2021+) items silently hide behind the notch, which spawned Bartender, Ice and similar apps. Third-party items accumulate without limits.
- Tahoe's transparent bar lost contrast over busy wallpapers; the fix was a setting, not an adaptive material.

## Lessons for swaypplet
- Avoid: an unbounded status-item area. swaypplet's BAR_VISION rule (tray shows only `NeedsAttention`, the rest in the panel) is the answer macOS lacks.
- Take: edge placement as a Fitts target. A bottom bar at 38 px keeps this if hit targets extend to the screen edge (no dead pixel row below).
- Avoid: making legibility over the wallpaper a user toggle; keep the opaque-frost default that BAR_VISION specifies.

## Sources
- https://www.idownloadblog.com/2025/06/25/how-to-toggle-mac-menu-bar-background-tutorial/ — Tahoe transparent menu bar and the background toggle [verified]
- https://dynamicmenubar.com/blog/transparent-menu-bar-macos-tahoe/ — no background unless enabled; Reduce Transparency restores it [verified]
- https://en.wikipedia.org/wiki/Menu_bar — global menu history and Fitts's law argument [memory]
- https://www.macbartender.com/ — third-party overflow for status items [memory]
