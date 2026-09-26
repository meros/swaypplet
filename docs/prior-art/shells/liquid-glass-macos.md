---
name: Liquid Glass on macOS
slug: liquid-glass-macos
domain: shells
kind: feature
platform: macOS
vendor: Apple
years: 2025–present
status: current
tags: [glass, material, translucency, legibility, accessibility]
relevance: high
---

## What it is
Apple's cross-platform material introduced with the version-26 releases (WWDC 2025): a translucent, refracting glass for controls, toolbars, the Dock, menus and Control Center, with specular highlights and lensing at edges. macOS 26.1 added a Clear/Tinted choice; macOS 27 (fall 2026) replaces it with a slider under Appearance and reworks opacity and edge treatment.

## What was new
Glass that bends and highlights what is behind it rather than only blurring it, applied to the controls layer while content stays opaque. It adapts tint between light and dark over the content under it.

## What went wrong / limits
- Legibility: text over clear glass on busy backgrounds was the dominant complaint from beta onward.
- The 26.1 "Tinted" option made little visible difference in light mode according to reviewers.
- Apple walked parts back in 27: less rounded window corners, edge-to-edge sidebars, sidebar icons coloured again, a transparency slider. The clearest setting no longer matches the WWDC 2025 demo.

## Lessons for swaypplet
- Take: glass on the chrome layer only, one layer per surface (already design-system principle 1).
- Avoid: a user-facing clear/tinted switch as the legibility fix. Hold a contrast floor in the material itself (APCA tests, the adaptive frost term in research/adaptive-glass.md).
- Note: even Apple retreated toward more body and frost within one year; tune for text first.

## Sources
- https://www.macrumors.com/2026/06/09/macos-golden-gate-liquid-glass/ — macOS 27 slider and design walk-backs [verified]
- https://eclecticlight.co/2025/11/09/last-week-on-my-mac-tahoe-26-1-disappointments/ — Tinted option ineffective [verified]
- https://osxdaily.com/2025/11/10/how-to-switch-from-clear-or-tinted-appearance-in-macos-tahoe/ — 26.1 Clear/Tinted [verified]
- https://www.apple.com/newsroom/2025/06/apple-introduces-a-delightful-and-elegant-new-software-design/ — Apple's description [memory]
