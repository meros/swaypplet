---
name: macOS Control Center
slug: macos-control-center
domain: shells
kind: feature
platform: macOS
vendor: Apple
years: 2020–present
status: current
tags: [quick-settings, modules, toggles, customization, menu-bar]
relevance: high
---

## What it is
A panel opened from one menu-bar icon, introduced in Big Sur (2020), grouping Wi-Fi, Bluetooth, AirDrop, Focus, display, sound and media into modules. Each module expands in place into a detail view. In Tahoe (26) it became fully user-customizable, with third-party controls and control groups.

## What was new
- Collapsed the many menu extras into one entry point while letting any module be pinned back to the menu bar individually.
- Expand-in-place: a module grows into its detail list inside the same panel rather than opening a separate window.
- Tahoe: third-party controls through a Controls API shared with iOS.

## What went wrong / limits
- Big Sur's module layout was fixed for five years; users could only pin modules to the menu bar, not rearrange the panel.
- Expand-in-place loses context: the rest of the panel disappears while a detail view is open.

## Lessons for swaypplet
- Take: a module may be both a panel tile and an optional bar item, with one source of state; this matches swaypplet's panel-holds-controls rule.
- Adapt: expand-in-place for the panel sections, with the shared-element motion from ROADMAP item 1 (tile grows into its section).

## Sources
- https://www.howtogeek.com/macos-26-tahoe-settings-to-change-right-away/ — Tahoe Control Center customization [verified]
- https://www.macforce.com/blog/notable-user-interface-changes-to-expect-in-macos-26 — Tahoe rebuilt Control Center, third-party support [verified]
- https://en.wikipedia.org/wiki/MacOS_Big_Sur — Control Center introduced 2020 [memory]
