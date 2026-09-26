---
name: macOS Auto appearance
slug: macos-auto-appearance
domain: theming
kind: feature
platform: macOS
vendor: Apple
years: 2019–present
status: current
tags: [dark-mode, auto, schedule, night-shift, deferred-switch]
relevance: high
---

## What it is
macOS Catalina (2019) added "Auto" to Light/Dark appearance: the Mac switches with the Night Shift schedule (sunset/sunrise from Location Services). Dynamic Desktop wallpapers (Mojave, 2018) change through the day by solar position.

## What was new
- Deferred switching: Auto does not flip while you are actively using the Mac; it waits until the machine is idle or a full-screen app is not in use.
- Dynamic wallpapers carry per-image sun altitude/azimuth metadata (HEIC), so the wallpaper follows the sun at the user's location.

## What went wrong / limits
- No user control over thresholds (civil twilight vs sunset) or a manual time schedule; third-party apps (NightOwl) fill the gap.
- Depends on Location Services; with it off, falls back to fixed times.

## Lessons for swaypplet
- swaypplet's §2.1 already does the deferred switch (idle, lock, unlock, or 10 minutes with no surface open); macOS validates the design.
- Borrow Dynamic Desktop's idea: tie the wallpaper to solar elevation. swaypplet has the elevation and the wallpaper setting; a "day/night wallpaper pair" is small.

## Sources
- https://support.apple.com/guide/mac-help/use-a-light-or-dark-appearance-mchl52e1c2d2/mac — Auto appearance [memory]
- Deferral while in use (Apple's Catalina description: "adjusts ... when you're not using your Mac") [memory]
- Dynamic Desktop HEIC solar metadata [memory]
