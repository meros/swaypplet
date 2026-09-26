---
name: Android bundled notifications and direct reply
slug: android-bundled-notifications
domain: notifications-wm
kind: feature
platform: Android
vendor: Google
years: 2016–present
status: current
tags: [grouping, bundles, inline-reply, summary]
relevance: high
---

## What it is
Android 7.0 Nougat (2016) grouped an app's notifications into a bundle with a summary line when collapsed and the individual children when expanded, and added direct reply: a text field inside a message notification.

## What was new
The app supplies the summary ("3 new messages from 2 chats"), so a collapsed group is informative rather than a count. Children can be expanded, acted on individually and "unbundled". Grouping made per-conversation inline reply meaningful.

## What went wrong / limits
Apps that do not set a group key get auto-grouped by the system at four or more, with a generic summary. Inline reply depends on app support.

## Lessons for swaypplet
- Take: a collapsed per-app group should show a synthesized summary (count plus most recent senders or titles), not just the latest item.
- Take: individual items stay actionable inside the group.
- Avoid for now: inline reply; freedesktop has no standard for it and the gain is small on a desktop with a keyboard.

## Sources
- https://developer.android.com/about/versions/nougat/android-7.0 — bundled notifications and direct reply. [verified via search summary]
- https://saket.me/nougat-bundled-notifications/ — summary when collapsed, children when expanded. [verified via search summary]
- System auto-grouping at four notifications. [memory]
