---
name: Android Live Updates (ProgressStyle and status-bar chips)
slug: android-live-updates
domain: mobile
kind: feature
platform: Android
vendor: Google
years: 2025–present
status: current
tags: [ongoing-activity, status-bar, chip, progress, notifications]
relevance: high
---

## What it is
Android 16's answer to Live Activities. A promoted ongoing notification built with the `ProgressStyle` template shows at the top of the shade, on the lock screen and always-on display, and as a small **chip** in the status bar carrying short text (`setShortCriticalText`: "5 min", "Turn left"). The full feature landed in Android 16 QPR1.

## What was new
- **Progress as a template, not free-form UI**: segments, points (stops) and a tracker icon on one bar, so every app's delivery or ride reads the same.
- **The status-bar chip**: a tiny live widget in the status bar in place of a static app icon, for example the next turn while you use another app.
- Built on notifications, so channels, permissions and DND apply unchanged.

## What went wrong / limits
- It shipped in stages: the chip and lock-screen presentation were missing from the first stable Android 16 release.
- A template limits expression; only progress-shaped activities fit.
- There is little public data yet on adoption or on how many chips the bar tolerates.

## Lessons for swaypplet
- **Take "template over free-form"**: a progress segment in the bar with a fixed shape (bar, stops, short text) keeps the bar's P4 grammar while carrying many sources (a build, a download, a Claude session).
- **Take "live info replaces the icon"**: the chip is the Android version of a bar segment whose content is the state (P5's two-layer contract).
- Status-bar room is scarce: cap the number of chips at 1–2, ordered by priority, as the bar's decision slot already is.

## Sources
- https://www.howtogeek.com/live-update-notifications-android-16/ — what Live Updates look like [verified via search summary]
- https://x.com/MishaalRahman/status/1891560432579620881 — setShortCriticalText, status-bar chip [verified via search summary]
- https://developer.android.com/about/versions/16/features/progress-centric-notifications — ProgressStyle template [memory]
