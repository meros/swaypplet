---
name: StandBy
slug: standby-mode
domain: mobile
kind: feature
platform: iOS
vendor: Apple
years: 2023–present
status: current
tags: [ambient, idle, clock, night-mode, red-shift, glanceable-at-distance]
relevance: medium
---

## What it is
Since iOS 17, an iPhone that is charging, locked and lying on its side turns into a full-screen bedside display with three views: widget pairs, photos or a large clock. It remembers a view per charger (MagSafe). In low ambient light a **Night Mode** turns everything monochrome red.

## What was new
- **The device's resting state as a designed surface**, set off by posture and power, not by a setting.
- **Night Mode from the ambient light sensor**: red on black keeps the room dark and spares night vision, with no configuration.
- Built for reading from a distance: large type, few items, two widget stacks.

## What went wrong / limits
- The always-on part needs an always-on display (iPhone 14 Pro and later); on other models the screen sleeps after about 20 seconds, which halves its use.
- Red can make photos and colour-coded widgets unreadable.

## Lessons for swaypplet
- **Adapt the idle surface**: a docked laptop left idle on AC (lid open, screen not yet locked) could show a StandBy-like clock before blanking, using the idle hint the lock already uses.
- **Take red night mode**: the lock screen at night could shift to a warm, low-luminance palette instead of normal dark mode, reusing the night light's sun position (`services::gamma`).

## Sources
- https://support.apple.com/guide/iphone/use-standby-iph878d77632/ios — StandBy conditions and views [verified via search summary]
- https://www.macrumors.com/how-to/use-standby-mode-iphone/ — Night Mode red tint in low light [verified via search summary]
