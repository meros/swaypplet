---
name: Android notification channels and importance
slug: android-notification-channels
domain: notifications-wm
kind: feature
platform: Android
vendor: Google
years: 2017–present
status: current
tags: [channels, importance, per-category, user-control]
relevance: high
---

## What it is
Since Android 8.0 Oreo (API 26, 2017) every notification must be posted to a channel the app declares (for example "Direct messages", "Promotions"). Each channel has an importance level (none, min, low, default, high) that sets sound, heads-up and status-bar behaviour. Priority per notification was removed.

## What was new
The unit of user control became a category inside an app, not the app. The app picks the initial importance, but after creation only the user can change it; long-pressing a notification shows its channel and lets the user silence just that kind. This ended the all-or-nothing choice of muting an app to escape its marketing.

## What went wrong / limits
Apps create dozens of badly named channels, or put marketing in the same channel as transactional messages, so the control depends on developer honesty. Settings depth grew; many users never discover channels.

## Lessons for swaypplet
- Adapt: freedesktop has no channels, but `category` plus `app_name` is a workable key; allow silencing "this app's `email.arrived`" from the notification's menu.
- Take: a silencing action on the notification itself is how users discover control; a settings pane alone is not.
- Take: once the user sets a level, the sender cannot raise it.

## Sources
- https://developer.android.com/about/versions/oreo/android-8.0 — channels required from API 26. [verified via search summary]
- https://developer.android.com/develop/ui/compose/notifications/channels — importance applies to every notification in the channel. [verified via search summary]
- Channel misuse by apps. [memory]
