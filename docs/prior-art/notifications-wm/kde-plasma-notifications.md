---
name: KDE Plasma notifications
slug: kde-plasma-notifications
domain: notifications-wm
kind: feature
platform: KDE
vendor: KDE
years: 2019–present
status: current
tags: [dnd, history, grouping, screen-mirroring, job-progress, fullscreen]
relevance: high
---

## What it is
Plasma 5.16 (June 2019) rewrote the notification system: Do Not Disturb, history grouped by app, and critical notifications shown over fullscreen apps. Plasma 5.17 turned DND on automatically while screens are mirrored, overridable from the applet. File transfers and other jobs render as progress notifications.

## What was new
Presentation detection as a default, not a setting to find: mirroring is the proxy for "presenting". Critical-over-fullscreen is the explicit breakthrough rule. Job progress (KJob) in the same surface as notifications.

## What went wrong / limits
Mirroring is a weak proxy: presenting over a video call's screen share or on an extended display is not caught. Grouping by app only, no threads.

## Lessons for swaypplet
- Take: automatic DND with a one-click override that states why it is on ("on while screens are mirrored").
- Adapt: use the stronger signals swaypplet has: screen capture active (the privacy indicator) and a fullscreen window on the focused output, in addition to mirroring.
- Take: critical breaks through, everything else waits.

## Sources
- https://kde.org/announcements/plasma/5/5.16.0/ — rewrite, DND, grouping, critical in fullscreen. [verified via search summary]
- https://kde.org/announcements/plasma/5/5.17.0/ — DND when mirroring. [verified via search summary]
- https://phabricator.kde.org/D22856 — the patch, overridable in the applet. [verified via search summary]
