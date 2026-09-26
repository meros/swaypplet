---
name: Android notification channels
slug: android-notification-channels
domain: mobile
kind: feature
platform: Android
vendor: Google
years: 2017–present
status: current
tags: [notifications, per-category-control, importance, user-control]
relevance: high
---

## What it is
Since Android 8.0 (API 26) every notification belongs to a channel the app declares (for example "Direct messages", "Promotions"), with an importance from NONE (0) to HIGH (4). After creation the app cannot change the channel's behaviour: the user owns its sound, vibration, pop-up and badge settings.

## What was new
- **Control per category, not per app**: silence an app's promotions and keep its messages.
- **Ownership moves to the user** once the channel exists, so an app cannot raise its own importance later.
- A long press on any notification leads straight to that channel's settings: the control sits where the annoyance happens.

## What went wrong / limits
- Apps create too many channels, or dump everything in one "General" channel, which defeats the idea.
- Most users never open the channel screens, so the defaults the app picked decide the outcome.

## Lessons for swaypplet
- **Adapt channels from the freedesktop spec**: `category` and the `desktop-entry` hint give an app/category key. Muting per (app, category) from the notification's own context menu is the cheap version.
- **Take "control where the annoyance happens"**: a "mute this kind" action on the popup itself beats a settings pane.

## Sources
- https://developer.android.com/develop/ui/compose/notifications/channels — channels, importance, user owns behaviour [verified via search summary]
- https://specifications.freedesktop.org/notification-spec/latest/ — category and desktop-entry hints [memory]
