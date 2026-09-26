---
name: Android 12 Internet tile
slug: android-12-internet-tile
domain: mobile
kind: feature
platform: Android
vendor: Google
years: 2021–present
status: failed
tags: [quick-settings, toggles, consolidation, regression, user-backlash]
relevance: medium
---

## What it is
In Android 12, Google replaced the separate Wi-Fi and mobile-data toggles in Quick Settings with one "Internet" tile that opens a panel holding both. Turning Wi-Fi off went from one tap to two.

## What was new
Consolidation by intent, not by radio: Google's reasoning was that most users turn Wi-Fi off to force cellular when the Wi-Fi is bad, and then forget to turn it back on, spending mobile data. The panel shows the networks and both switches together.

## What went wrong / limits
- Heavy, lasting backlash: a one-tap action became a multi-tap chore for the people who use it most. Third-party apps ("Better Internet Tiles") exist to restore the split.
- Google's data described the average user; the power users who toggle radios deliberately lost the most.
- By 2025 reporting said Android 16 builds were testing one-tap Wi-Fi and Bluetooth tiles again, about four years later.

## Lessons for swaypplet
- **Avoid merging controls to steer a behaviour**: a toggle's click should do what its label says. Put the detail on the secondary action, not in front of the primary one.
- A shell for one power user should weight frequency of use over average-user safety.

## Sources
- https://www.xda-developers.com/google-android-12-internet-quick-settings-tile-reasoning/ — Google's stated reasoning [verified via search summary]
- https://www.androidauthority.com/android-16-one-click-quick-settings-tiles-3494720/ — one-tap toggles returning [verified via search summary]
