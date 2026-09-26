---
name: iOS interruption levels (Passive, Active, Time Sensitive, Critical)
slug: ios-interruption-levels
domain: notifications-wm
kind: feature
platform: iOS
vendor: Apple
years: 2021–present
status: current
tags: [interruption-level, time-sensitive, critical, passive, focus]
relevance: high
---

## What it is
iOS 15 (2021) gave every notification one of four interruption levels. Passive: no sound, no screen wake, straight to the list. Active: the default. Time Sensitive: breaks through Focus and the scheduled summary. Critical: breaks through the mute switch too; needs an Apple entitlement (weather, health, security).

## What was new
The sender declares how urgent it is, the user decides whether to believe it. Time Sensitive is a separate per-app permission the user can revoke, and iOS asks after a while whether to keep allowing an app's Time Sensitive notifications. Critical is gated by review.

## What went wrong / limits
Apps over-claim Time Sensitive for marketing; the revoke prompt is the patch. Passive is under-used because apps want attention.

## Lessons for swaypplet
- Take: model three effective levels from freedesktop urgency: low = passive (list only), normal = active, critical = breaks through quiet modes and never times out.
- Take: allow per-app demotion of critical ("this app's critical is not critical"), because freedesktop has no entitlement gate and some apps send critical for trivia.

## Sources
- https://developer.apple.com/videos/play/wwdc2021/10091/ — the four levels and their behaviour. [verified via search summary]
- https://documentation.onesignal.com/docs/en/ios-focus-modes-and-interruption-levels — Time Sensitive breaks Focus and summary. [verified via search summary]
- iOS prompt to keep allowing Time Sensitive per app. [memory]
