---
name: KDE Plasma accent from wallpaper and scheme tinting
slug: kde-accent-from-wallpaper
domain: theming
kind: feature
platform: KDE
vendor: KDE
years: 2022–present
status: current
tags: [accent, wallpaper, tint, slideshow, colour-scheme]
relevance: high
---

## What it is
Plasma 5.25 (June 2022) added "From current wallpaper" as an accent colour option, updating with slideshow wallpapers, and a colour-scheme option "Tint all colors with the accent color" with a tint strength slider.

## What was new
- Two separate knobs: which accent (wallpaper or picked) and how much the neutrals take it (tint strength), the same split as swaypplet's `accents` vs `full`.
- Tinting is applied to any existing scheme, so it composes with community schemes.
- Also "Accent-coloured title bar" option.

## What went wrong / limits
- The extracted accent sometimes has poor contrast for text on it; there is no generator guarantee.
- Tint strength is a free slider, so high values produce unreadable, heavily coloured UIs.

## Lessons for swaypplet
- swaypplet's `full` tint holds neutral chroma in 0.010–0.025, which is what KDE's slider lacks: a bounded cast. Keep it bounded.
- Worth copying: live update on slideshow change without a user action.

## Sources
- https://kde.org/announcements/plasma/5/5.25.0/ — release notes [verified via search summary]
- https://invent.kde.org/plasma/plasma-workspace/-/merge_requests/1325 — the MR [verified via search summary]
- https://www.omgubuntu.co.uk/2022/04/kde-plasma-desktop-auto-accent-color-feature — tint strength [verified via search summary]
