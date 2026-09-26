---
name: Android notification cooldown
slug: android-notification-cooldown
domain: notifications-wm
kind: feature
platform: Android
vendor: Google
years: 2024–present
status: current
tags: [rate-limit, flood, volume, group-chat]
relevance: high
---

## What it is
A feature first seen in the Android 15 developer preview (February 2024), pulled from beta, and shipped later: the first notification from an app or conversation alerts at full volume, and each one that follows within a short window is quieter and stops vibrating. After a few minutes of silence the level resets.

## What was new
Attenuation instead of a binary mute: the first message still gets through, the burst does not. Options: all apps, conversations only, off.

## What went wrong / limits
Shipped late and was briefly removed; how-to sites reported the setting disappearing on some builds. Only covers sound and vibration, not visual popups.

## Lessons for swaypplet
- Take: apply the same decay to popups. The first notification from an app pops; further ones within N seconds update the existing group popup in place (count, latest line) rather than stacking new cards. S-sized and addresses the chat-flood case directly.
- Take: reset after a quiet interval, so nothing important is lost for long.

## Sources
- https://9to5google.com/2024/02/16/android-15-notification-cooldown/ — DP1 introduction. [verified via search summary]
- https://www.androidauthority.com/notification-cooldown-android-15-qpr1-3481384/ — removed from beta, return in QPR1, behaviour. [verified via search summary]
- https://www.howtogeek.com/google-already-killed-my-favorite-android-15-feature/ — removal reported. [verified via search summary]
