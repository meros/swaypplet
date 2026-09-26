---
name: Windows toasts, Action Center and Notification Center
slug: windows-action-center-toasts
domain: notifications-wm
kind: feature
platform: Windows
vendor: Microsoft
years: 2012–present
status: current
tags: [toasts, action-center, quick-settings, grouping]
relevance: medium
---

## What it is
Windows 8 introduced toasts for Store apps; Windows 10 (2015) added Action Center, a right-edge pane combining a per-app, chronological notification history with quick-action buttons, replacing the Charms bar. Windows 11 split it again into a Notification Center (with the calendar) and a separate Quick Settings flyout.

## What was new
Toasts with adaptive templates (buttons, inputs, progress bars) and a clear lifecycle: a toast shows, then "ghosts" into Action Center history. Apps can update or remove their own entries. Notifications grouped per app, ordered by time.

## What went wrong / limits
Windows 10 merged settings and history in one pane, then Windows 11 unmerged them, a sign that the combination never settled. Toast floods from Store and system nags trained users to disable notifications; Microsoft's own promotional tips arrived through the same channel.

## Lessons for swaypplet
- Take: the explicit popup-then-history lifecycle; a popup that times out should visibly go somewhere.
- Avoid: using the notification channel for the shell's own promotions or tips; one nag poisons the channel.
- Note: swaypplet's panel already mixes controls and notifications; keep them as clearly separate sections so neither crowds the other.

## Sources
- https://learn.microsoft.com/en-us/archive/blogs/tiles_and_toasts/toast-notification-and-action-center-overview-for-windows-10 — toast and Action Center model, per-app grouping. [verified via search summary]
- https://en.wikipedia.org/wiki/Action_Center — Windows 11 split into Quick Settings and Notification Center. [verified via search summary]
- https://newatlas.com/windows-8-features-not-in-windows-10/36252/ — Action Center replaced Charms. [verified via search summary]
