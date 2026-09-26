---
name: Apple Focus modes
slug: ios-focus-modes
domain: notifications-wm
kind: feature
platform: iOS
vendor: Apple
years: 2021–present
status: current
tags: [focus, dnd, context, filters, automation, status-sharing]
relevance: high
---

## What it is
iOS 15 and macOS Monterey (2021) replaced a single Do Not Disturb with named Focuses (Work, Sleep, Personal, custom), each with its own allow-list of people and apps, schedule or location/app triggers, and sync across devices. iOS 16 (2022) added Focus filters (Mail shows only the work account, Calendar only work calendars), a Lock Screen and Home Screen page linked to each Focus, and Focus status sharing ("notifications silenced") to contacts.

## What was new
Quiet is a context with a scope, not a switch. A Focus changes what the device shows, not just what it rings for. Automatic triggers (time, place, opening an app) remove the need to remember to turn it on.

## What went wrong / limits
Configuration is deep and spread across screens; sync across devices surprises people (a Mac Focus silencing the phone, reported again in macOS Tahoe with screen mirroring). Many users keep only DND and Sleep.

## Lessons for swaypplet
- Take: automatic triggers are the part that pays; roadmap item 3's fullscreen and screen-share triggers are the right first two.
- Avoid: multiple named Focuses with per-app allow-lists as a first version; one quiet mode with context triggers and a breakthrough rule (critical, and optionally named apps) covers a single-user desktop.
- Take: show that quiet is on in the one place it matters (panel), never as a constant bar mark (BAR_VISION P1).

## Sources
- https://www.macrumors.com/guide/ios-16-focus/ — Focus filters, Lock Screen linking, status sharing. [verified via search summary]
- https://support.apple.com/guide/iphone/set-up-a-focus-iphd6288a67f/ios — Focus setup and triggers. [verified via search summary]
- https://discussions.apple.com/thread/256207562 — DND on mirroring propagating across devices in Tahoe. [verified via search summary]
