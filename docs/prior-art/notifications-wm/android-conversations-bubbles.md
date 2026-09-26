---
name: Android conversations section and bubbles
slug: android-conversations-bubbles
domain: notifications-wm
kind: feature
platform: Android
vendor: Google
years: 2020–present
status: current
tags: [conversations, people, priority, bubbles, sections]
relevance: medium
---

## What it is
Android 11 (2020) split the shade into sections: Conversations (messages from people) on top, then Alerting, then Silent. Users mark a conversation Priority, Default or Silent by long-press; Priority conversations can break through DND and float as bubbles, chat heads that open a small window over any app.

## What was new
Sorting by what a notification is (a person talking to you) rather than by app or time. Per-conversation priority, not per-app. The shade has fixed sections with fixed meaning.

## What went wrong / limits
Bubbles never took off beyond messaging apps and are easy to dismiss by accident. Classification relies on apps using the MessagingStyle/shortcut APIs.

## Lessons for swaypplet
- Adapt: fixed sections in the notification list (from people, other, quiet/low) give order without AI; `category` `im.*` and `call.*` identify people-traffic.
- Avoid: floating chat heads; they fight the tiling layout.

## Sources
- https://developer.android.com/develop/ui/views/notifications/conversations — conversation section, bubbles. [verified via search summary]
- https://www.androidcentral.com/android-11-heres-how-chat-bubbles-and-conversation-notifications-work — Priority/Alerting/Silent. [verified via search summary]
