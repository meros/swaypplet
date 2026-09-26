---
name: Android conversations and bubbles
slug: android-conversations-bubbles
domain: mobile
kind: feature
platform: Android
vendor: Google
years: 2020–present
status: current
tags: [notifications, people, priority, floating, sections]
relevance: medium
---

## What it is
Android 11 split the shade into sections, with a **Conversations** section on top for messages from people. A conversation can be marked *priority* (top of the section, avatar on the lock screen, allowed through DND) and can **bubble**: a floating avatar that opens the chat as a small overlay window above any app. Only notifications tied to a shortcut (a person) qualify.

## What was new
- **Notifications sorted by kind of sender** (people, then alerts, then silent), not by time.
- **Priority per conversation**, not per app: one person matters, not the whole app.
- Bubbles as a lightweight window that lives above the task and collapses back to a dot.

## What went wrong / limits
- Bubbles saw little adoption outside Google Messages and Telegram; Facebook's Chat Heads had covered this ground since 2013.
- The floating dot gets in the way; many users turn bubbles off.

## Lessons for swaypplet
- **Take sections in the notification centre**: people or waiting tasks first, then system, then silent. The centre could group by *what it asks of you*, not by arrival.
- **Avoid floating bubbles** on a tiling desktop: a floating dot is a window that belongs to no workspace, and it violates the bar's fixed spatial grammar (P4).

## Sources
- https://source.android.com/docs/core/display/conv-notifications — conversation section, priority effects [verified via search summary]
- https://developer.android.com/develop/ui/views/notifications/bubbles — shortcut requirement for bubbles [verified via search summary]
