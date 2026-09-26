---
name: macOS Notification Center
slug: macos-notification-center
domain: notifications-wm
kind: feature
platform: macOS
vendor: Apple
years: 2012–present
status: current
tags: [notification-center, banners, alerts, grouping, widgets]
relevance: high
---

## What it is
A system notification service and a right-edge panel, added in OS X 10.8 Mountain Lion (July 2012), borrowed from iOS 5. Apps choose nothing about presentation; the user picks per app between banners (auto-dismiss), alerts (stay until acted on) or none. macOS 11 Big Sur (2020) rebuilt it as one column of grouped notifications above iOS-style widgets.

## What was new
Presentation owned by the user, per app, not by the sender. Big Sur added per-app grouping that expands on click, and interactive notifications (actions without opening the app). Notification Center absorbed the role of Growl and later of Dashboard's widgets.

## What went wrong / limits
Banners pile up in the top-right corner during bursts. The combined panel of widgets plus notifications was widely called cluttered after Big Sur, and "Clear All" on groups is hover-revealed. Until Sonoma, widgets lived only in the panel, a surface opened rarely.

## Lessons for swaypplet
- Take: the per-app presentation choice (popup, list only, off) belongs to the user and should be reachable from the notification itself.
- Take: per-app groups that expand in place; swaypplet already groups, so match the expand-in-place affordance.
- Avoid: hover-only controls such as clear buttons (BAR_VISION P8).

## Sources
- https://en.wikipedia.org/wiki/Notification_Center — Mountain Lion July 2012. [verified via search summary]
- https://www.macworld.com/article/234400/macos-big-sur-notification-center.html — Big Sur single column, grouping, widgets. [verified via search summary]
- Banner/alert styles and hover clear buttons. [memory]
