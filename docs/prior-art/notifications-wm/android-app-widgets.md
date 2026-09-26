---
name: Android home screen widgets
slug: android-app-widgets
domain: notifications-wm
kind: feature
platform: Android
vendor: Google
years: 2009–present
status: current
tags: [widgets, home-screen, remoteviews, material-you]
relevance: low
---

## What it is
Android 1.5 Cupcake (2009) added third-party app widgets on the home screen, rendered by the launcher from RemoteViews the app supplies. Android 12 (2021) refreshed them with rounded shapes, dynamic colour, and stateful controls. iOS answered only in 2020 (iOS 14).

## What was new
Widgets rendered by the host from a description the app sends, so an app cannot draw arbitrary things or run in the launcher's process. Interactive (music controls, toggles) long before iOS.

## What went wrong / limits
Many apps' widgets went unmaintained for years; the RemoteViews toolkit is limited; updates are rate-limited for battery, so data goes stale.

## Lessons for swaypplet
- Take: host-rendered, description-based content (like MPRIS for media) is the safe way to show another app's state in the shell.

## Sources
- https://en.wikipedia.org/wiki/Android_version_history — Cupcake widgets, Android 12 redesign. [verified via search summary]
- https://www.computerworld.com/article/1714347/android-versions-a-living-history-from-1-0-to-today.html — history. [verified via search summary]
- RemoteViews model, update limits. [memory]
