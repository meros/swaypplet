---
name: GNOME Shell notifications
slug: gnome-shell-notifications
domain: notifications-wm
kind: feature
platform: GNOME
vendor: GNOME Project
years: 2011–present
status: current
tags: [message-tray, calendar-popover, banners, dnd, grouping, portal]
relevance: high
---

## What it is
GNOME 3.0 (2011) had a hidden message tray at the bottom edge, revealed by pushing the pointer down. GNOME 3.16 (2015) moved the notification list into the clock/calendar drop-down and kept a "legacy tray" for status icons, removed in 3.26. A Do Not Disturb switch sits in that drop-down. GNOME 46 (2024) added app name and icon headers and expandable bodies; GNOME 48 (March 2025) added per-app stacks.

## What was new
Tying notification history to the clock: time and "what happened" in one place. Banners at the top centre, one at a time. Moving new API work to the XDG notification portal (sounds, markup, updates, categories for calls and alarms).

## What went wrong / limits
The 3.0 hidden tray was undiscoverable. It took until 2025 to group by app, long after mobile. Without headers (pre-46) users could not tell which app sent what, which the Shell developers named as a security and usability problem.

## Lessons for swaypplet
- Take: always show the sending app's name and icon; spoofable summaries without attribution are a phishing vector.
- Take: one popup at a time in a fixed place, not a growing column.
- Avoid: surfaces revealed only by pointer-to-edge (BAR_VISION P8).

## Sources
- https://wiki.gnome.org/Design/OS/Notifications — design, list in the date drop-down. [verified via search summary]
- https://blogs.gnome.org/aday/2017/08/31/status-icons-and-gnome/ — legacy tray from 3.16, removed 3.26. [verified via search summary]
- https://blogs.gnome.org/shell-dev/2024/04/23/notifications-46-and-beyond/ — headers, attribution problem, portal. [verified]
- https://www.phoronix.com/news/GNOME-48-Notifications-App — per-app grouping in 48. [verified via search summary]
