---
name: macOS Notification Center
slug: macos-notification-center
domain: shells
kind: feature
platform: macOS
vendor: Apple
years: 2012–present
status: current
tags: [notifications, widgets, grouping, focus, history]
relevance: medium
---

## What it is
A right-edge sheet (Mountain Lion, 2012) holding notification history and, since Big Sur, widgets below it; opened by clicking the clock or a two-finger swipe from the trackpad edge. Notifications group by app and thread. Focus modes (Monterey, 2021) filter which apps and people break through and can switch on by schedule, location or app.

## What was new
- Grouped stacks that expand on click, keeping the list short.
- Focus as a named, scheduled filter shared across devices, with "Share Focus status" telling senders you are silenced.
- Time Sensitive notifications as an explicit class allowed through Focus.

## What went wrong / limits
- Widgets and notifications share one sheet; the Today view went through three designs (2014, 2020, 2023 desktop widgets) and never settled.
- Notification banners stack at the top-right over app toolbars, a frequent complaint on small screens.
- Focus configuration is spread across Settings and hard to predict.

## Lessons for swaypplet
- Take: Focus triggered by context (ROADMAP item 3) with a summary after; macOS proves auto-triggers, but its trigger set is time and app, not "screen shared".
- Take: group by app and thread in the centre; one card per group.
- Avoid: mixing widgets into the notification list.

## Sources
- https://en.wikipedia.org/wiki/Notification_Center — history 2012, Big Sur merge [memory]
- https://support.apple.com/guide/mac-help/set-up-a-focus-to-stay-on-task-mchl613dc43f/mac — Focus schedules and triggers [memory]
